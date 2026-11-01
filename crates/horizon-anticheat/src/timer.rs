//! The timer check's balance: how far a client's ticks are behind (positive) or ahead (negative) of the
//! server's clock, in milliseconds.
//!
//! The server adds the wall-clock time of each of its ticks, and every ServerboundClientTickEndPacket
//! takes away the length of one tick at the server's tick rate (vanilla's client ticks at that rate,
//! `TickRateManager.millisecondsPerTick`). The balance is capped at `max_lag_ms` *after* the tick's
//! packets are counted: a slow server tick that handles a second of packets at once also added that
//! second, and a quiet connection banks at most `max_lag_ms` for the burst that ends a lag spike.

use crate::TimerConfig;

#[derive(Clone, Debug, Default)]
pub struct Balance {
    pub ms: f64,
}

impl Balance {
    /// The start of a server tick that came `elapsed_ms` after the previous one.
    pub fn elapse(&mut self, elapsed_ms: f64, config: &TimerConfig) {
        self.ms = self.ms.min(config.max_lag_ms) + elapsed_ms;
    }

    /// One client tick ended, at a tick rate of `tick_rate` a second. Some(how far ahead, in ms) when the
    /// client is further ahead than the tolerance; the balance starts again from zero then, so a
    /// sustained speed-up is flagged again each time it gets that far ahead.
    pub fn tick_end(&mut self, tick_rate: f32, config: &TimerConfig) -> Option<f64> {
        self.ms -= 1000.0 / f64::from(tick_rate.max(1.0));
        if self.ms < -config.tolerance_ms {
            let ahead = -self.ms;
            self.ms = 0.0;
            return Some(ahead);
        }
        None
    }

    /// The tick rate changed or the player arrived: start again.
    pub fn reset(&mut self) {
        self.ms = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `seconds` of server ticks of `server_ms` each, with `per_tick` client ticks handled in each.
    fn run(b: &mut Balance, server_ms: f64, per_tick: f64, seconds: f64) -> usize {
        let c = TimerConfig::default();
        let mut owed = 0.0;
        let mut flags = 0;
        for _ in 0..(seconds * 1000.0 / server_ms) as usize {
            b.elapse(server_ms, &c);
            owed += per_tick;
            while owed >= 1.0 {
                owed -= 1.0;
                flags += usize::from(b.tick_end(20.0, &c).is_some());
            }
        }
        flags
    }

    #[test]
    fn a_vanilla_client_is_never_flagged() {
        let mut b = Balance::default();
        assert_eq!(run(&mut b, 50.0, 1.0, 600.0), 0);
        // A server that runs at 10 ticks a second handles two client ticks in each.
        assert_eq!(run(&mut b, 100.0, 2.0, 600.0), 0);
    }

    #[test]
    fn a_slow_server_tick_with_its_backlog_is_not_flagged() {
        let (mut b, c) = (Balance::default(), TimerConfig::default());
        b.elapse(3000.0, &c);
        for _ in 0..60 {
            assert_eq!(b.tick_end(20.0, &c), None);
        }
    }

    #[test]
    fn a_short_network_stall_is_not_flagged() {
        let (mut b, c) = (Balance::default(), TimerConfig::default());
        // 800 ms with nothing arriving, then the 16 ticks sent meanwhile.
        for _ in 0..16 {
            b.elapse(50.0, &c);
        }
        b.elapse(50.0, &c);
        for _ in 0..17 {
            assert_eq!(b.tick_end(20.0, &c), None);
        }
    }

    #[test]
    fn a_sped_up_client_is_flagged_again_and_again() {
        let mut b = Balance::default();
        // 10 % faster: 5 ms ahead each tick, 300 ms after 3 s, so close to 20 flags a minute.
        let flags = run(&mut b, 50.0, 1.1, 60.0);
        assert!((15..=20).contains(&flags), "{flags}");
        // Twice as fast: a flag every 7 ticks.
        assert!(run(&mut Balance::default(), 50.0, 2.0, 10.0) >= 25);
    }

    #[test]
    fn a_pause_cant_be_saved_for_a_burst() {
        let (mut b, c) = (Balance::default(), TimerConfig::default());
        for _ in 0..200 {
            b.elapse(50.0, &c);
        }
        let flagged = (0..60).any(|_| b.tick_end(20.0, &c).is_some());
        assert!(flagged);
    }
}
