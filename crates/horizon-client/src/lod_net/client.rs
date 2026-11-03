//! The client side of `horizon:lod`: far terrain a server samples, as the
//! `LodSource` of a client of another machine's world. Each sampling thread
//! queues one tile request, which the game thread sends with its own packets
//! ([`Inbox::requests`]), and waits for its reply, which the network thread
//! hands over ([`Inbox::deliver`]), within the server's [`Offer`]. A tile
//! refused, lost or unreadable comes back as no columns: `Lod` asks again.
use super::{Bucket, Offer, Request, SIDE, decode_reply, reply_id};
use crate::lod::{Column, Surface};
use crate::streaming::{LodSampler, LodSource};
use std::{
    collections::HashMap,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

/// How long a client waits for a tile before asking again.
const TIMEOUT: Duration = Duration::from_secs(30);

/// A connection's far terrain requests: their replies come in on the network
/// thread ([`Self::deliver`]) and go to the sampler waiting for each. One per
/// connection, so the server's limits hold across the far terrains (one per
/// dimension) a connection has in turn.
#[derive(Default)]
pub struct Inbox {
    state: Mutex<InboxState>,
    changed: Condvar,
}
#[derive(Default)]
struct InboxState {
    /// Requests sent and not answered: when each went, and the sampler
    /// waiting for it (None once given up).
    waiting: HashMap<u32, (Instant, Option<mpsc::Sender<Vec<u8>>>)>,
    /// Requests for the game thread to send, by id.
    outbox: Vec<(u32, Vec<u8>)>,
    next: u32,
    bucket: Option<Bucket>,
    /// Bumped by [`Inbox::cancel`]: samplers of an older one stop.
    epoch: u64,
}
impl Inbox {
    /// A reply from the server: to its sampler, and its slot freed.
    pub fn deliver(&self, data: Vec<u8>) {
        let Some(id) = reply_id(&data) else {
            return;
        };
        let mut state = self.state.lock().unwrap();
        if let Some((_, Some(tx))) = state.waiting.remove(&id) {
            let _ = tx.send(data);
        }
        drop(state);
        self.changed.notify_all();
    }
    /// The requests to send now, from the game thread.
    pub fn requests(&self) -> Vec<Vec<u8>> {
        let mut state = self.state.lock().unwrap();
        state.outbox.drain(..).map(|(_, data)| data).collect()
    }
    /// The far terrain ends (another dimension, a disconnect): its samplers
    /// stop waiting at once. Requests still at the server keep their slot
    /// until it answers; those not sent yet never go.
    pub fn cancel(&self) {
        let mut state = self.state.lock().unwrap();
        state.epoch += 1;
        for (id, _) in std::mem::take(&mut state.outbox) {
            state.waiting.remove(&id);
        }
        for (_, tx) in state.waiting.values_mut() {
            *tx = None;
        }
        drop(state);
        self.changed.notify_all();
    }
    /// The connection now talks to another server (a proxy's reconfiguration):
    /// what the old one was asked can never come back, so its slots and its
    /// rate go too. Ids keep increasing, so a late reply matches nothing.
    pub fn reset(&self) {
        self.cancel();
        let mut state = self.state.lock().unwrap();
        state.waiting.clear();
        state.bucket = None;
        drop(state);
        self.changed.notify_all();
    }
    fn epoch(&self) -> u64 {
        self.state.lock().unwrap().epoch
    }
    /// A slot for `request` (its id set), within the offer's limits, once
    /// there is one, and the request queued; None once the far terrain of
    /// `epoch` was cancelled.
    fn send(
        &self,
        epoch: u64,
        offer: Offer,
        mut request: Request,
    ) -> Option<mpsc::Receiver<Vec<u8>>> {
        let mut state = self.state.lock().unwrap();
        loop {
            if state.epoch != epoch {
                return None;
            }
            let now = Instant::now();
            // A request the server never answered (a reconfiguration lost
            // it) frees its slot in time.
            state
                .waiting
                .retain(|_, (sent, _)| now.duration_since(*sent) < TIMEOUT);
            let rate = f64::from(offer.per_second);
            let free = state.waiting.len() < offer.in_flight as usize;
            let bucket = state
                .bucket
                .get_or_insert_with(|| Bucket::new(rate, rate, now));
            if free && bucket.take(now) {
                request.id = state.next;
                state.next = state.next.wrapping_add(1);
                let (tx, rx) = mpsc::channel();
                state.waiting.insert(request.id, (now, Some(tx)));
                state.outbox.push((request.id, request.encode()));
                return Some(rx);
            }
            state = self
                .changed
                .wait_timeout(state, Duration::from_millis(20))
                .unwrap()
                .0;
        }
    }
}

/// Far terrain sampled by the server: the `LodSource` of a client of
/// another machine's world.
pub struct RemoteSource {
    inbox: Arc<Inbox>,
    epoch: u64,
    offer: Offer,
    /// This client's biome indices by name.
    biomes: HashMap<String, u16>,
    /// Block states this client knows.
    states: u32,
    /// Samplers made: no more than requests may wait.
    samplers: AtomicUsize,
}
impl RemoteSource {
    pub fn new(
        inbox: Arc<Inbox>,
        offer: Offer,
        biomes: HashMap<String, u16>,
        states: u32,
    ) -> Arc<Self> {
        Arc::new(Self {
            epoch: inbox.epoch(),
            inbox,
            offer,
            biomes,
            states,
            samplers: AtomicUsize::new(0),
        })
    }
    /// One tile's columns; none when the server refused it, it was lost,
    /// or the far terrain ended first: `Lod` asks again later.
    fn tile(&self, x: i32, z: i32, stride: i32, approximate: bool) -> Vec<Column> {
        let request = Request {
            id: 0,
            x,
            z,
            stride,
            approximate,
        };
        let Some(reply) = self.inbox.send(self.epoch, self.offer, request) else {
            return Vec::new();
        };
        let Ok(data) = reply.recv_timeout(TIMEOUT) else {
            return Vec::new();
        };
        match decode_reply(&data, &self.biomes, self.states) {
            Ok(columns) => columns.unwrap_or_default(),
            Err(e) => {
                log::warn!("Horizon: far terrain from the server: {e}");
                Vec::new()
            }
        }
    }
}
impl LodSource for RemoteSource {
    fn sampler(self: Arc<Self>) -> Box<dyn LodSampler> {
        self.samplers.fetch_add(1, Ordering::Relaxed);
        Box::new(RemoteSampler {
            source: self,
            approximate: false,
        })
    }
}
struct RemoteSampler {
    source: Arc<RemoteSource>,
    approximate: bool,
}
impl LodSampler for RemoteSampler {
    /// Each sampling thread waits for one request: no more threads than the
    /// server lets requests wait.
    fn fork(&self) -> Option<Box<dyn LodSampler>> {
        let samplers = &self.source.samplers;
        if samplers.load(Ordering::Relaxed) >= self.source.offer.in_flight as usize {
            return None;
        }
        Some(self.source.clone().sampler())
    }
    fn set_approximate(&mut self, approximate: bool) {
        self.approximate = approximate;
    }
    fn sample(&mut self, _x: i32, _z: i32) -> Option<Surface> {
        None
    }
    fn grid(&mut self, x: i32, z: i32, stride: i32, side: usize) -> Vec<Column> {
        if side != SIDE {
            return vec![Column::default(); side * side];
        }
        self.source.tile(x, z, stride, self.approximate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lod_net::refusal;

    fn request(x: i32) -> Request {
        Request {
            id: 0,
            x,
            z: 0,
            stride: 1,
            approximate: false,
        }
    }

    #[test]
    fn a_cancelled_inbox_frees_its_samplers() {
        let inbox = Arc::new(Inbox::default());
        let offer = Offer::new(64);
        let epoch = inbox.epoch();
        let sent = inbox.send(epoch, offer, request(0)).unwrap();
        let id = Request::decode(&inbox.requests()[0]).unwrap().id;
        // Queued, not sent: cancelling forgets it.
        let _queued = inbox.send(epoch, offer, request(16)).unwrap();
        inbox.cancel();
        assert!(sent.recv_timeout(Duration::from_secs(1)).is_err());
        assert!(inbox.send(epoch, offer, request(32)).is_none());
        assert!(inbox.requests().is_empty());
        // The late reply still frees the slot the sent one held.
        assert_eq!(inbox.state.lock().unwrap().waiting.len(), 1);
        inbox.deliver(refusal(id));
        assert!(inbox.state.lock().unwrap().waiting.is_empty());
        // A reconfiguration frees the slots still at the old server.
        let epoch = inbox.epoch();
        assert!(inbox.send(epoch, offer, request(48)).is_some());
        assert_eq!(inbox.requests().len(), 1);
        inbox.reset();
        let state = inbox.state.lock().unwrap();
        assert!(state.waiting.is_empty() && state.outbox.is_empty() && state.bucket.is_none());
        drop(state);
    }

    #[test]
    fn a_refused_tile_is_asked_again() {
        let inbox = Arc::new(Inbox::default());
        let source = RemoteSource::new(inbox.clone(), Offer::new(64), HashMap::new(), 10);
        let mut sampler = source.clone().sampler();
        let server = std::thread::spawn(move || {
            loop {
                if let Some(data) = inbox.requests().pop() {
                    let id = Request::decode(&data).unwrap().id;
                    inbox.deliver(refusal(id));
                    return;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        // No columns, not 256 empty ones: Lod doesn't keep it.
        assert!(sampler.grid(0, 0, 1, SIDE).is_empty());
        server.join().unwrap();
    }
}
