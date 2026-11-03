//! The server side of `horizon:lod`: what each request gets
//! ([`Service::request`]), the one background thread that samples the tiles
//! admitted, every player in turn, and the replies it hands back to the game
//! thread ([`Service::deliver`]), which sends them with its own packets.
use super::{
    Bucket, IN_FLIGHT, MAX_DISTANCE, Offer, PER_SECOND, Request, SIDE, encode_reply, refusal,
};
use crate::streaming::{LodSampler, LodSource};
use std::{
    collections::{HashMap, VecDeque},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    thread,
    time::Instant,
};

/// Tiles the server lets a player ask for at once, past [`PER_SECOND`]: the
/// network bunches requests the client spaced out.
const SERVER_BURST: f64 = 4.0 * PER_SECOND as f64;
/// Requests a Horizon client never sends (unreadable, or past the offer's
/// rate) a player may make before they are disconnected.
pub const STRIKES: u32 = 64;

/// One player's limits at the server.
struct Player {
    bucket: Bucket,
    /// Requests admitted whose reply the player's connection hasn't taken.
    in_flight: usize,
    strikes: u32,
}

/// What a request gets.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Sampled: [`Service::push`] it.
    Serve(Request),
    /// Answered at once with this refusal: out of reach, in a level with no
    /// far terrain, or every slot taken.
    Refuse(Vec<u8>),
    /// Not answered: unreadable, or past the rate. A strike.
    Ignore,
    /// Too many strikes: disconnect the player.
    Kick,
}

/// How [`Service::deliver`]'s connection took a reply.
pub enum Delivery {
    Sent,
    /// Its backlog is long: the reply waits for the next delivery.
    Busy,
    /// The player left.
    Gone,
}

/// The server's far terrain: what each request gets ([`Self::request`]), and
/// the thread that samples the tiles admitted.
pub struct Service {
    pub distance: u32,
    players: HashMap<u32, Player>,
    biome_names: Arc<Vec<String>>,
    worker: Option<Worker>,
    /// Replies the worker finished, waiting for their connection.
    ready: VecDeque<(u32, Vec<u8>)>,
}
/// A tile admitted, waiting for the worker.
pub struct Job {
    pub conn: u32,
    /// The level's id: its sampler is kept.
    pub level: String,
    pub source: Arc<dyn LodSource>,
    pub request: Request,
}
impl Service {
    pub fn new(distance: u32, biome_names: Vec<String>) -> Self {
        Self {
            distance: distance.min(MAX_DISTANCE),
            players: HashMap::new(),
            biome_names: Arc::new(biome_names),
            worker: None,
            ready: VecDeque::new(),
        }
    }
    /// The offer Horizon clients hear.
    pub fn offer(&self) -> Offer {
        Offer::new(self.distance)
    }
    /// What `conn`'s request `data` gets from a player at `position`, in a
    /// level with far terrain when `open`. Every
    /// request takes a token, refused or not; one with no token left, or
    /// unreadable, is a strike. A served one holds a slot until its reply
    /// leaves ([`Self::deliver`]).
    pub fn request(
        &mut self,
        conn: u32,
        data: &[u8],
        position: [f64; 3],
        open: bool,
        now: Instant,
    ) -> Verdict {
        let player = self.players.entry(conn).or_insert_with(|| Player {
            bucket: Bucket::new(f64::from(PER_SECOND), SERVER_BURST, now),
            in_flight: 0,
            strikes: 0,
        });
        // The token goes first, so an unreadable request is charged too.
        let token = player.bucket.take(now);
        let request = match Request::decode(data) {
            Ok(request) if token => request,
            _ => {
                player.strikes += 1;
                return if player.strikes >= STRIKES {
                    Verdict::Kick
                } else {
                    Verdict::Ignore
                };
            }
        };
        if !open
            || !request.allowed(self.distance, position)
            || player.in_flight >= IN_FLIGHT as usize
        {
            return Verdict::Refuse(refusal(request.id));
        }
        player.in_flight += 1;
        Verdict::Serve(request)
    }
    /// Forget the players for whom `keep` is false (they left).
    pub fn retain(&mut self, keep: impl Fn(u32) -> bool) {
        self.players.retain(|&conn, _| keep(conn));
    }
    pub fn players(&self) -> usize {
        self.players.len()
    }
    /// Queue a served tile: the worker starts with the first.
    pub fn push(&mut self, job: Job) {
        let names = self.biome_names.clone();
        let worker = self.worker.get_or_insert_with(|| Worker::start(names));
        let mut queue = lock(&worker.shared.queue);
        match queue.players.iter_mut().find(|(conn, _)| *conn == job.conn) {
            Some((_, jobs)) => jobs.push_back(job),
            None => queue.players.push_back((job.conn, VecDeque::from([job]))),
        }
        drop(queue);
        worker.shared.wake.notify_one();
    }
    /// Hands the finished replies to `send`, on the game thread, so they
    /// keep their place among the game's packets. A reply its connection
    /// is too busy for waits; one taken frees its player's slot.
    pub fn deliver(&mut self, mut send: impl FnMut(u32, &mut Vec<u8>) -> Delivery) {
        if let Some(worker) = &self.worker {
            self.ready.extend(lock(&worker.shared.done).drain(..));
        }
        let players = &mut self.players;
        self.ready
            .retain_mut(|(conn, data)| match send(*conn, data) {
                Delivery::Sent => {
                    if let Some(p) = players.get_mut(conn) {
                        p.in_flight = p.in_flight.saturating_sub(1);
                    }
                    false
                }
                Delivery::Busy => true,
                Delivery::Gone => false,
            });
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Worker {
    shared: Arc<WorkerShared>,
    thread: Option<thread::JoinHandle<()>>,
}
struct WorkerShared {
    queue: Mutex<Queue>,
    wake: Condvar,
    /// Replies sampled, for [`Service::deliver`].
    done: Mutex<Vec<(u32, Vec<u8>)>>,
}
#[derive(Default)]
struct Queue {
    /// Each player's tiles, players in turn.
    players: VecDeque<(u32, VecDeque<Job>)>,
    stop: bool,
}
impl Worker {
    fn start(biome_names: Arc<Vec<String>>) -> Self {
        let shared = Arc::new(WorkerShared {
            queue: Mutex::default(),
            wake: Condvar::new(),
            done: Mutex::default(),
        });
        let worker = shared.clone();
        let thread = thread::Builder::new()
            .name("Far terrain server".into())
            .spawn(move || {
                // The ticks come first (horizon_threads).
                horizon_threads::behind_server();
                // A sampler per level, kept while its level's source is the same.
                let mut samplers: HashMap<String, (Arc<dyn LodSource>, Box<dyn LodSampler>)> =
                    HashMap::new();
                while let Some(job) = worker.next() {
                    let r = job.request;
                    // A sampler that panics is dropped, and the tile refused:
                    // every job gets a reply, which frees its slot.
                    let reply = catch_unwind(AssertUnwindSafe(|| {
                        let same = samplers
                            .get(&job.level)
                            .is_some_and(|(source, _)| same_source(source, &job.source));
                        if !same {
                            let main = job.source.clone().sampler();
                            // A fork leaves out the client's chunk decoration threads.
                            let sampler = main.fork().unwrap_or(main);
                            samplers.insert(job.level.clone(), (job.source.clone(), sampler));
                        }
                        let (_, sampler) = samplers.get_mut(&job.level).expect("inserted above");
                        sampler.set_approximate(r.approximate);
                        let columns = sampler.grid(r.x, r.z, r.stride, SIDE);
                        encode_reply(r.id, Some(&columns), &biome_names)
                    }))
                    .unwrap_or_else(|_| {
                        log::warn!("Far terrain: sampling {} failed; tile refused", job.level);
                        samplers.remove(&job.level);
                        refusal(r.id)
                    });
                    lock(&worker.done).push((job.conn, reply));
                }
            })
            .expect("far terrain server thread");
        Self {
            shared,
            thread: Some(thread),
        }
    }
}
impl WorkerShared {
    /// The next player's next tile, None once the service stops.
    fn next(&self) -> Option<Job> {
        let mut queue = lock(&self.queue);
        loop {
            if queue.stop {
                return None;
            }
            if let Some((conn, mut jobs)) = queue.players.pop_front() {
                let job = jobs.pop_front();
                if !jobs.is_empty() {
                    queue.players.push_back((conn, jobs));
                }
                if job.is_some() {
                    return job;
                }
                continue;
            }
            queue = self
                .wake
                .wait(queue)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        lock(&self.shared.queue).stop = true;
        self.shared.wake.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn same_source(a: &Arc<dyn LodSource>, b: &Arc<dyn LodSource>) -> bool {
    std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn request(id: u32, x: i32) -> Vec<u8> {
        Request {
            id,
            x,
            z: 0,
            stride: 1,
            approximate: false,
        }
        .encode()
    }

    #[test]
    fn players_are_held_to_their_rate_and_slots() {
        let now = Instant::now();
        let mut service = Service::new(64, Vec::new());
        let at = [0.0; 3];
        let served = |v: Verdict| matches!(v, Verdict::Serve(_));
        for _ in 0..IN_FLIGHT {
            assert!(served(service.request(1, &request(0, 0), at, true, now)));
        }
        // Every slot taken: refused until a reply leaves.
        assert_eq!(
            service.request(1, &request(5, 0), at, true, now),
            Verdict::Refuse(refusal(5))
        );
        assert!(served(service.request(2, &request(0, 0), at, true, now)));
        // Out of reach: refused, and charged like any request.
        assert!(matches!(
            service.request(2, &request(0, 1 << 20), at, true, now),
            Verdict::Refuse(_)
        ));
        let mut spent = IN_FLIGHT as usize + 1;
        while matches!(
            service.request(1, &request(0, 1 << 20), at, true, now),
            Verdict::Refuse(_)
        ) {
            spent += 1;
        }
        // The burst spent, every request is a strike, up to a kick.
        assert_eq!(spent, SERVER_BURST as usize);
        for _ in 1..STRIKES - 1 {
            assert_eq!(
                service.request(1, &request(0, 0), at, true, now),
                Verdict::Ignore
            );
        }
        assert_eq!(service.request(1, b"junk", at, true, now), Verdict::Kick);
        // A second later there are tokens again; slots come back as replies leave.
        let later = now + Duration::from_secs(1);
        assert!(matches!(
            service.request(1, &request(0, 0), at, true, later),
            Verdict::Refuse(_)
        ));
        service.ready.push_back((1, Vec::new()));
        service.ready.push_back((2, Vec::new()));
        service.deliver(|conn, _| {
            if conn == 1 {
                Delivery::Sent
            } else {
                Delivery::Busy
            }
        });
        assert_eq!(service.ready.len(), 1);
        assert!(served(service.request(1, &request(0, 0), at, true, later)));
        // A level with no far terrain refuses.
        assert!(matches!(
            service.request(1, &request(9, 0), at, false, later),
            Verdict::Refuse(_)
        ));
        service.retain(|conn| conn == 2);
        assert_eq!(service.players(), 1);
    }

    /// Far terrain that fails west of x = 0.
    struct Brittle;
    impl LodSampler for Brittle {
        fn sample(&mut self, x: i32, _z: i32) -> Option<crate::lod::Surface> {
            assert!(x >= 0, "a broken chunk");
            None
        }
    }
    impl LodSource for Brittle {
        fn sampler(self: Arc<Self>) -> Box<dyn LodSampler> {
            Box::new(Brittle)
        }
    }

    #[test]
    fn a_sampler_that_panics_refuses_the_tile_and_frees_the_slot() {
        let now = Instant::now();
        let mut service = Service::new(64, Vec::new());
        let source: Arc<dyn LodSource> = Arc::new(Brittle);
        let mut replies = Vec::new();
        for (id, x) in [(1, -16), (2, 0)] {
            let Verdict::Serve(request) = service.request(1, &request(id, x), [0.0; 3], true, now)
            else {
                panic!("served");
            };
            service.push(Job {
                conn: 1,
                level: "minecraft:overworld".into(),
                source: source.clone(),
                request,
            });
        }
        let start = Instant::now();
        while replies.len() < 2 && start.elapsed() < Duration::from_secs(10) {
            service.deliver(|_, data| {
                replies.push(std::mem::take(data));
                Delivery::Sent
            });
            thread::sleep(Duration::from_millis(5));
        }
        let refused = |data: &Vec<u8>| data[1] == 1;
        assert_eq!(
            replies.iter().map(refused).collect::<Vec<_>>(),
            [true, false]
        );
        assert_eq!(service.players[&1].in_flight, 0);
    }
}
