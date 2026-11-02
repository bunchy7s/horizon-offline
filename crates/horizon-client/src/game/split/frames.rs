//! The frame-time readout (`HORIZON_SPLIT_STATS=1`, docs/extras.md). The players render one
//! after another: on threads, their renders slowed each other down (the GPU driver's per-process
//! state) more than they overlapped.
use super::*;

impl<'a> Split<'a> {
    /// `HORIZON_SPLIT_STATS=1`: every 5 s, where a frame's time goes (the players' turns with
    /// their renders, the windows' presents, the GPU wait, outside the loop) and each player's
    /// frame rate and CPU and GPU time a frame, in the log.
    pub(super) fn print_stats(&mut self) {
        let Some(since) = self.stats else {
            return;
        };
        if since.elapsed() < Duration::from_secs(5) {
            return;
        }
        self.stats = Some(Instant::now());
        let seats: Vec<usize> = (0..self.players()).map(|i| self.seat_of(i)).collect();
        let games =
            std::iter::once(&mut self.host).chain(self.guests.iter_mut().map(|g| &mut *g.game));
        let frames = self.spent_frames.max(1);
        let [turns, presents, wait, outside] =
            std::mem::take(&mut self.spent).map(|d| d.as_secs_f64() * 1000.0 / f64::from(frames));
        self.spent_frames = 0;
        let mut line = format!(
            " a frame {:.2} ms: turns {turns:.2} (with the renders), presents {presents:.2}, gpu wait {wait:.2}, outside {outside:.2}.",
            turns + presents + wait + outside,
        );
        for (i, game) in games.enumerate() {
            let s = game.frame_stats.summary();
            let ms = |k: &str| s[k]["avg_ms"].as_f64().unwrap_or(0.0);
            line += &format!(
                "  P{}: {:.0} fps, cpu {:.2} ms (render {:.2} ms), gpu {:.2} ms",
                seats[i],
                s["avg_fps"].as_f64().unwrap_or(0.0),
                ms("cpu"),
                s["phases_avg_ms"]["render"].as_f64().unwrap_or(0.0),
                ms("gpu"),
            );
            game.frame_stats.reset();
        }
        log::info!("Horizon: splitscreen frames:{line}");
    }
}
