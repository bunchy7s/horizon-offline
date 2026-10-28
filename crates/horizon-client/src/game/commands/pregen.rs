//! `/pregen`, Horizon's own command: generate, light and save every chunk of a
//! square or a circle ahead of the players (docs/pregeneration.md). The task
//! lives on the server and hands its band to the level's streaming, whose
//! workers do the work; chunks behind the band unload and are saved as any
//! chunk the players leave.
use super::{CommandError, Context, Dim, Game, R};
use horizon_client::{
    lan::data::component_nbt,
    pregen::{Pregen, Shape},
    text::{self, Text},
};
use horizon_protocol::packets::play::{self, Clientbound as C};
use std::time::{Duration, Instant};

/// How often the progress is reported.
const REPORT_EVERY: Duration = Duration::from_secs(5);
/// How long the band may stay put before the task looks for what holds it.
const STALL: Duration = Duration::from_secs(30);

pub(in crate::game) struct PregenTask {
    pregen: Pregen,
    dim: Dim,
    paused: bool,
    /// Time spent running, up to the last pause, and when it last resumed.
    ran: Duration,
    resumed: Instant,
    reported: Instant,
    /// When the band last moved, to notice a task that stopped advancing.
    moved: Instant,
    /// The player who started it, told the progress too (a guest's connection).
    owner: Option<u32>,
}

impl PregenTask {
    fn running_for(&self) -> Duration {
        if self.paused {
            self.ran
        } else {
            self.ran + self.resumed.elapsed()
        }
    }
    fn progress(&self) -> Text {
        let p = &self.pregen;
        let seconds = self.running_for().as_secs_f64();
        let rate = p.done as f64 / seconds.max(1e-3);
        let left = p.total - p.done;
        let eta = if rate > 0.0 {
            duration(left as f64 / rate)
        } else {
            "unknown".into()
        };
        Text::literal(format!(
            "Pregenerating {}: {} of {} chunks ({:.1}%), {rate:.1} chunks/s, {} left",
            self.dim.id(),
            p.done,
            p.total,
            100.0 * p.done as f64 / p.total.max(1) as f64,
            if p.done == p.total {
                "nothing".into()
            } else {
                eta
            },
        ))
    }
}

fn failure(message: &str) -> CommandError {
    CommandError::new(Text::literal(message))
}

/// `1h 02m 03s`.
fn duration(seconds: f64) -> String {
    let s = seconds.round() as u64;
    match (s / 3600, s / 60 % 60, s % 60) {
        (0, 0, s) => format!("{s}s"),
        (0, m, s) => format!("{m}m {s:02}s"),
        (h, m, s) => format!("{h}h {m:02}m {s:02}s"),
    }
}

impl Game<'_> {
    pub(super) fn cmd_pregen(&mut self, id: &str, ctx: &Context) -> R<i32> {
        match id {
            "pregen.start" => {
                if self.cmds.pregen.is_some() {
                    return Err(failure(
                        "Pregeneration is already running: /pregen cancel stops it",
                    ));
                }
                let radius = Self::int(ctx, "radius").unwrap_or(0);
                let shape = if ctx.literals.iter().any(|l| l == "circle") {
                    Shape::Circle
                } else {
                    Shape::Square
                };
                let [x, z] = if ctx.get("center").is_some() {
                    self.column_arg(ctx, "center")?
                } else {
                    let p = self.source_position();
                    [p[0].floor() as i32, p[2].floor() as i32]
                };
                let pregen = Pregen::new((x >> 4, z >> 4), radius >> 4, shape);
                let now = Instant::now();
                let total = pregen.total;
                self.feedback(Text::literal(format!(
                    "Pregenerating {total} chunks of {}: a {} of radius {radius} around {x} {z}",
                    self.dim.id(),
                    match shape {
                        Shape::Square => "square",
                        Shape::Circle => "circle",
                    },
                )));
                self.cmds.pregen = Some(PregenTask {
                    pregen,
                    dim: self.dim,
                    paused: false,
                    ran: Duration::ZERO,
                    resumed: now,
                    reported: now,
                    moved: now,
                    owner: self.lan_acting,
                });
                Ok(total as i32)
            }
            "pregen.pause" | "pregen.continue" => {
                let pause = id == "pregen.pause";
                let task = self
                    .cmds
                    .pregen
                    .as_mut()
                    .ok_or_else(|| failure("No pregeneration is running"))?;
                if task.paused != pause {
                    task.paused = pause;
                    if pause {
                        task.ran += task.resumed.elapsed();
                    } else {
                        task.resumed = Instant::now();
                    }
                }
                let (progress, dim) = (task.progress(), task.dim);
                if pause && self.dim == dim {
                    self.streaming.set_band(None);
                }
                self.feedback(
                    Text::literal(if pause { "Paused. " } else { "Continuing. " }).append(progress),
                );
                Ok(1)
            }
            "pregen.cancel" => {
                let task = self
                    .cmds
                    .pregen
                    .take()
                    .ok_or_else(|| failure("No pregeneration is running"))?;
                if self.dim == task.dim {
                    self.streaming.set_band(None);
                }
                self.feedback(Text::literal("Cancelled. ").append(task.progress()));
                Ok(task.pregen.done as i32)
            }
            _ => {
                let task = self
                    .cmds
                    .pregen
                    .as_ref()
                    .ok_or_else(|| failure("No pregeneration is running"))?;
                let progress = task.progress();
                let done = task.pregen.done as i32;
                self.feedback(progress);
                Ok(done)
            }
        }
    }

    /// Move the task's band past the rows that are lit and hand it to the
    /// level's streaming, and report the progress now and then. Runs before
    /// each streaming update of the level in place.
    pub(in crate::game) fn tick_pregen(&mut self) {
        let Some(task) = self.cmds.pregen.as_mut() else {
            return;
        };
        if task.dim != self.dim || task.paused {
            return;
        }
        // A row is done once its chunks are lit and post-processed
        // (`postprocess_chunks`, on the tick), as vanilla's are by the time
        // they are FULL: then they may unload and be saved.
        let (streaming, chunks) = (&mut self.streaming, &self.world.chunks);
        if task.pregen.advance(|x, z| {
            streaming.is_lit(x, z)
                && chunks
                    .get(&(x, z))
                    .is_some_and(|c| c.postprocess.is_empty())
        }) {
            task.moved = Instant::now();
        }
        streaming.set_band(task.pregen.band());
        if task.moved.elapsed() >= STALL {
            // Nothing advanced for a while: say what the first row waits
            // for, and queue light again for chunks that lost their job.
            task.moved = Instant::now();
            let row = task.pregen.first_row();
            let waiting = row
                .iter()
                .filter(|k| chunks.get(k).is_some_and(|c| !c.postprocess.is_empty()))
                .count();
            let [missing, lighting, unlit] = streaming.unstick_light(&mut self.world, &row);
            log::warn!(
                "Horizon: pregeneration waits on row {:?}: {missing} not loaded, {lighting} being lit, \
                 {unlit} unlit with no light job (queued again), {waiting} not post-processed; {:?}",
                row.first(),
                streaming.stages()
            );
        }
        let finished = task.pregen.finished();
        if finished || task.reported.elapsed() >= REPORT_EVERY {
            task.reported = Instant::now();
            let (owner, mut message) = (task.owner, task.progress());
            if finished {
                message = Text::literal("Done. ")
                    .append(message)
                    .with_color(text::GREEN);
                self.cmds.pregen = None;
            }
            self.pregen_report(owner, message);
        }
    }

    /// Whether the level in place has a running task, which the server pumps
    /// between its ticks (`pump_chunks`).
    pub(in crate::game) fn pregen_runs_here(&self) -> bool {
        self.cmds
            .pregen
            .as_ref()
            .is_some_and(|t| t.dim == self.dim && !t.paused)
    }

    /// The level a running task needs ticked, whoever is in it.
    pub(in crate::game) fn pregen_level(&self) -> Option<Dim> {
        self.cmds.pregen.as_ref().map(|t| t.dim)
    }

    /// Keep the workers fed until `until`: collect what they finished and
    /// hand them the next jobs as soon as each one is done, rather than once
    /// a tick.
    pub(in crate::game) fn pump_chunks(&mut self, until: Instant, paused: bool) {
        // Force-loaded chunks too: decorating in vanilla's order, a chunk
        // waits for its neighbours far more often than once a tick allows.
        // And the players' chunks while they load, as vanilla's server runs
        // its chunk tasks while it waits for the next tick
        // (`MinecraftServer.waitUntilNextTick`): once a tick, a few jobs per
        // worker (`schedule_generation`) capped joining a world at about 120
        // columns a second with two workers, the workers idle most of the
        // time. Not while a respawn search waits for a chunk: its area is
        // only asked for by the tick's own `update_world`.
        let players = !paused && self.spawn_wait.is_empty();
        while self.pregen_runs_here()
            || ((players || !self.streaming.forced.is_empty()) && self.streaming.busy())
        {
            let now = Instant::now();
            if now >= until {
                break;
            }
            self.streaming
                .wait((until - now).min(Duration::from_millis(5)));
            self.update_world();
        }
    }

    /// The console and the player who started the task get the progress.
    fn pregen_report(&mut self, owner: Option<u32>, message: Text) {
        if self.options.dedicated.is_some() {
            super::super::dedicated::log("INFO", &message.plain());
        } else {
            log::info!("{}", message.plain());
        }
        if let Some(host) = &self.lan {
            if let Some(g) = owner.and_then(|c| host.hub.guest(c)) {
                g.send(&C::SystemChat(play::SystemChat {
                    content: component_nbt(&message.component()),
                    overlay: false,
                }));
            }
        } else if self.options.dedicated.is_none() {
            self.system_message(message);
        }
    }
}
