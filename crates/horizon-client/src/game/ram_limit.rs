//! The window's side of the RAM limit (`horizon_client::ram_limit`): a look at the memory once a
//! second, the render distance and far terrain in use, and `/ramlimit`'s answer.
use super::Game;
use horizon_client::ram_limit::{self, Step};
use horizon_client::text::Text;
use horizon_threads::capped;
use std::time::Instant;

const MIB: u64 = 1024 * 1024;

impl Game<'_> {
    /// The render distance in use: the player's, unless memory holds it lower.
    pub(super) fn render_distance_in_use(&self) -> i32 {
        self.ram_limit.distance(self.settings.render_distance)
    }

    /// Once a second, on the window's game while a world is open.
    pub(super) fn tick_ram_limit(&mut self) {
        if !self.ticks.is_multiple_of(20) {
            return;
        }
        let chosen = self.settings.render_distance;
        let Some(limit) = ram_limit::limit() else {
            if self.ram_limit.reset() {
                self.apply_render_distance_in_use();
            }
            return;
        };
        let has_far = self.settings.horizon.lod_distance > 0;
        let pressure = ram_limit::pressure(limit, self.fps);
        // Every ten seconds, what the limit sees: a report can show the trace.
        if self.ticks.is_multiple_of(200) {
            let (commit, _) = horizon_threads::allocator_commit();
            log::info!(
                "Horizon: memory: allocated {} MB of {} MB, commit {} MB, distance {}, {pressure:?}",
                capped::live_bytes() / MIB,
                limit / MIB,
                commit as u64 / MIB,
                self.render_distance_in_use()
            );
        }
        let step = self
            .ram_limit
            .step(Instant::now(), pressure, chosen, has_far);
        if !self.ram_limit.at_lowest(chosen, has_far) {
            ram_limit::can_shrink();
        }
        let Some(step) = step else {
            return;
        };
        self.apply_render_distance_in_use();
        let said = match step {
            Step::FarTerrainOff => "Far terrain is off to stay under the RAM limit.".to_owned(),
            Step::FarTerrainBack => "Far terrain is back on.".to_owned(),
            Step::Lowered(d) => {
                format!("Render distance lowered to {d} to stay under the RAM limit.")
            }
            Step::Raised(d) if d >= chosen => format!("Render distance is back to {d}."),
            // Going back up step by step needs no line for each step.
            Step::Raised(_) => return,
        };
        log::info!("Horizon: {said} {pressure:?}");
        self.system_message(Text::literal(said.as_str()));
    }

    /// The streaming radius follows the distance in use, within what the server sends; the server
    /// hears of it with the next `ClientInformation` or `set_distances`.
    fn apply_render_distance_in_use(&mut self) {
        let distance = self.render_distance_in_use();
        self.streaming.radius = match &self.remote {
            Some(r) if r.logged_in => distance.min(r.view_distance + 1).max(2),
            _ => distance,
        };
    }

    /// `/ramlimit`, answered here on any server. False: another command.
    pub(super) fn ram_limit_command(&mut self, command: &str) -> bool {
        let Some(answer) = ram_limit::run(command) else {
            return false;
        };
        self.system_message(Text::literal(answer.as_str()));
        let (in_use, chosen) = (self.render_distance_in_use(), self.settings.render_distance);
        if in_use < chosen {
            let line = format!("Render distance is {in_use} for now; yours is {chosen}.");
            self.system_message(Text::literal(line.as_str()));
        }
        true
    }
}
