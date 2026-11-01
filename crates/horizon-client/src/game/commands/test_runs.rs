//! `TestCommand` over the test instance blocks of the level in place and the
//! game's test instances: `run`, `runmultiple`, `runthese`, `runclosest`,
//! `runthat`, `runfailed`, `resetclosest`/`these`/`that`,
//! `clearthat`/`these`/`all` and `stop`, with `TestFinder`'s searches.
//!
//! Retries are `RetryOptions(numberOfTimes, untilFailed)`.
use super::{Arg, Context, Game, R, err};
use crate::game::{
    Dim,
    game_tests::{Grid, Info, Retry, RunnerOptions, verify_batches},
    test_instances::TestDef,
};
use horizon_client::text::{self, Text};
use horizon_world::worldgen::structure::BoundingBox;
use horizon_world::worldgen::structure::Rotation;

type Pos = (i32, i32, i32);

/// `TestFinder.Builder`'s finders that the command picks.
enum Finder {
    /// `radius(c, r)`.
    Radius(i32),
    /// `nearest(c)`.
    Nearest,
    /// `allNearby(c)`.
    AllNearby,
    /// `lookedAt(c)`.
    LookedAt,
    /// `failedTests(c, onlyRequiredTests)`.
    Failed(bool),
    /// `byResourceSelection`, `copies` times over.
    Selection { tests: Vec<String>, copies: i32 },
}

/// `StructureUtils.getRotationForRotationSteps`.
fn rotation_for_steps(steps: i32) -> Option<Rotation> {
    usize::try_from(steps)
        .ok()
        .and_then(|i| Rotation::ALL.get(i).copied())
}

impl Game<'_> {
    /// `CommandSourceStack.sendFailure`.
    fn test_failure(&mut self, message: Text) {
        if self.command_source.is_some_and(|s| s.silent) {
            return;
        }
        self.system_message(message.with_color(text::RED));
    }

    /// `StructureUtils.findTestBlocks`: the test instance blocks within
    /// `radius` of `center` (the POI search is a sphere).
    pub(super) fn find_test_instance_blocks(&self, center: Pos, radius: i32) -> Vec<Pos> {
        let limit = i64::from(radius) * i64::from(radius);
        let mut found: Vec<Pos> = self
            .world
            .block_entities
            .iter()
            .filter(|(_, e)| {
                matches!(
                    e,
                    horizon_client::containers::BlockEntity::TestInstanceBlock(_)
                )
            })
            .map(|(p, _)| *p)
            .filter(|p| {
                let d = [p.0 - center.0, p.1 - center.1, p.2 - center.2].map(i64::from);
                d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= limit
            })
            .collect();
        found.sort_unstable();
        found
    }

    /// The block of the source's position (`BlockPos.containing`).
    pub(super) fn test_source_block(&self) -> Pos {
        let p = self.source_position();
        (
            p[0].floor() as i32,
            p[1].floor() as i32,
            p[2].floor() as i32,
        )
    }

    /// `StructureUtils.lookedAtTestPos`: the test whose box the player's
    /// view ray (250 blocks) meets, the nearest to the source.
    fn looked_at_test(&self, center: Pos) -> Option<Pos> {
        let player = self.player.as_ref()?;
        let eye = [
            player.position.x,
            player.position.y + f64::from(player.dimensions.eye_height),
            player.position.z,
        ];
        let (yaw, pitch) = (
            f64::from(self.yaw).to_radians(),
            f64::from(self.pitch).to_radians(),
        );
        let look = [
            -yaw.sin() * pitch.cos(),
            -pitch.sin(),
            yaw.cos() * pitch.cos(),
        ];
        let end = [0, 1, 2].map(|i| eye[i] + look[i] * 250.0);
        self.find_test_instance_blocks(center, 250)
            .into_iter()
            .filter(|&p| {
                self.test_structure_box(p).is_some_and(|b| {
                    let min = b.min.map(f64::from);
                    let max = b.max.map(|v| f64::from(v) + 1.0);
                    ray_hits_box(eye, end, min, max)
                })
            })
            .min_by_key(|p| {
                let d = [p.0 - center.0, p.1 - center.1, p.2 - center.2].map(i64::from);
                d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
            })
    }

    /// The test instance blocks a finder gives (`findTestPos`).
    fn finder_positions(&self, finder: &Finder) -> Vec<Pos> {
        let center = self.test_source_block();
        match finder {
            Finder::Radius(radius) => self.find_test_instance_blocks(center, *radius),
            Finder::Nearest => self
                .find_test_instance_blocks(center, 15)
                .into_iter()
                .min_by_key(|p| {
                    (p.0 - center.0).abs() + (p.1 - center.1).abs() + (p.2 - center.2).abs()
                })
                .into_iter()
                .collect(),
            Finder::AllNearby => self.find_test_instance_blocks(center, 250),
            Finder::LookedAt => self.looked_at_test(center).into_iter().collect(),
            Finder::Failed(_) | Finder::Selection { .. } => Vec::new(),
        }
    }

    /// The tests a finder gives (`findTests`), by id.
    fn finder_tests(&self, finder: &Finder) -> Vec<(String, TestDef)> {
        let keys: Vec<String> = match finder {
            Finder::Failed(only_required) => self
                .game_tests
                .failed
                .iter()
                .filter(|k| !only_required || self.test_def(k).is_some_and(|def| def.required))
                .cloned()
                .collect(),
            Finder::Selection { tests, copies } => {
                (0..*copies).flat_map(|_| tests.iter().cloned()).collect()
            }
            _ => Vec::new(),
        };
        keys.into_iter()
            .filter_map(|k| self.test_def(&k).map(|def| (k, def)))
            .collect()
    }

    /// `verifyStructureExists`.
    fn verify_test_structure(&mut self, structure: &str) -> bool {
        if self.template_entry(structure).is_some() {
            return true;
        }
        self.test_failure(Text::translate(
            "commands.test.error.structure_not_found",
            &[Text::literal(structure.to_owned())],
        ));
        false
    }

    /// `createGameTestInfo`: the test of a test instance block.
    fn test_info_at(&mut self, pos: Pos, retry: Retry) -> Option<Info> {
        let key = self.test_instance_at(pos).and_then(|b| b.data.test.clone());
        if self.test_instance_at(pos).is_none() {
            self.test_failure(Text::translate(
                "commands.test.error.test_instance_not_found.position",
                &[pos.0, pos.1, pos.2].map(|v| Text::literal(v.to_string())),
            ));
            return None;
        }
        let Some(def) = key.as_deref().and_then(|k| self.test_def(k)) else {
            let name = self.test_name(pos);
            self.test_failure(Text::translate(
                "commands.test.error.non_existant_test",
                &[name],
            ));
            return None;
        };
        let key = key.unwrap_or_default();
        let key = if key.contains(':') {
            key
        } else {
            format!("minecraft:{key}")
        };
        if !self.verify_test_structure(&def.structure) {
            return None;
        }
        let rotation = self.test_rotation(pos);
        let id = self.game_tests.next_id();
        Some(Info::new(id, key, def, Some(pos), self.dim, rotation).with_retry(retry))
    }

    /// `toGameTestInfo`: a test without a block, in the level in place.
    fn test_info_for(
        &mut self,
        key: String,
        def: TestDef,
        steps: i32,
        retry: Retry,
    ) -> R<Option<Info>> {
        if !self.verify_test_structure(&def.structure) {
            return Ok(None);
        }
        // `server.getLevel(dimension)`: the server's levels; a client-only
        // game (deprecated in-frame mode) has the one it is in.
        let dim = match def.dimension.as_str() {
            "minecraft:overworld" | "minecraft:the_nether" | "minecraft:the_end" => {
                Some(Dim::from_id(&def.dimension))
            }
            _ => None,
        }
        .filter(|d| *d == self.dim || !self.options.execution_role.is_client());
        let Some(dim) = dim else {
            self.test_failure(Text::literal(format!(
                "Could not resolve level for dimension: {}",
                def.dimension
            )));
            return Ok(None);
        };
        let rotation = rotation_for_steps(steps).ok_or_else(|| err("command.failed", &[]))?;
        let id = self.game_tests.next_id();
        Ok(Some(
            Info::new(id, key, def, None, dim, rotation).with_retry(retry),
        ))
    }

    /// The column `createTestPositionAround` starts from: the source's.
    pub(super) fn test_around(&self) -> (i32, i32) {
        let p = self.source_position();
        (p[0].floor() as i32, p[2].floor() as i32)
    }

    /// `createTestPositionAround`: three blocks south of the source, on the
    /// surface of the level in place (`playerAndTestInfo` loads the chunk of
    /// the source's column in that level first).
    pub(in crate::game) fn test_position_in_level(&mut self, (x, z): (i32, i32)) -> Pos {
        self.load_test_chunks(&BoundingBox::new([x, 0, z], [x, 0, z]));
        let facts = self.res.facts();
        let surface = self.world.world_surface(x, z, |s| facts.is_air(s));
        (x, surface, z + 3)
    }

    /// `createTestPositionAround` in the level in place.
    pub(super) fn test_position_around(&mut self) -> Pos {
        let around = self.test_around();
        self.test_position_in_level(around)
    }

    /// `TestCommand.run`.
    fn run_tests(&mut self, finder: &Finder, retry: Retry, steps: i32, per_row: i32) -> R<i32> {
        self.clear_game_tests();
        let mut infos = Vec::new();
        for pos in self.finder_positions(finder) {
            infos.extend(self.test_info_at(pos, retry));
        }
        for (key, def) in self.finder_tests(finder) {
            infos.extend(self.test_info_for(key, def, steps, retry)?);
        }
        if infos.is_empty() {
            self.feedback_unbroadcast(Text::translate("commands.test.no_tests", &[]));
            return Ok(0);
        }
        self.game_tests.failed.clear();
        self.feedback_unbroadcast(Text::translate(
            "commands.test.run.running",
            &[Text::literal(infos.len().to_string())],
        ));
        let grid = Grid::new(self.test_around(), per_row);
        self.track_and_start_runner(infos, false, Some(grid));
        Ok(1)
    }

    /// `TestCommand.verify`.
    pub(super) fn test_verify(&mut self, ctx: &Context) -> R<i32> {
        self.clear_game_tests();
        let tests = match ctx.get("tests") {
            Some(Arg::Str(pattern)) => horizon_client::command::args::select_resources(
                self.ui.registries.ids("minecraft:test_instance"),
                pattern,
            )
            .into_iter()
            .cloned()
            .collect(),
            _ => Vec::new(),
        };
        let finder = Finder::Selection { tests, copies: 1 };
        let mut infos = Vec::new();
        for pos in self.finder_positions(&finder) {
            infos.extend(self.test_info_at(pos, Retry::none()));
        }
        for (key, def) in self.finder_tests(&finder) {
            infos.extend(self.test_info_for(key, def, 0, Retry::none())?);
        }
        self.game_tests.failed.clear();
        let batches = verify_batches(&infos, || self.game_tests.next_id());
        let grid = Grid::new(self.test_around(), 10).clearing();
        self.track_and_start_batches(batches, RunnerOptions::verify(grid));
        Ok(1)
    }

    /// `TestCommand.reset`.
    fn reset_tests(&mut self, finder: &Finder) -> R<i32> {
        self.clear_game_tests();
        let mut count = 0;
        for pos in self.finder_positions(finder) {
            if self.test_info_at(pos, Retry::none()).is_some() {
                // resetGameTestInfo: `TestInstanceBlockEntity.resetTest`.
                self.reset_test(pos);
                count += 1;
            }
        }
        if count == 0 {
            return Err(err("commands.test.clear.error.no_tests", &[]));
        }
        self.feedback(Text::translate(
            "commands.test.reset.success",
            &[Text::literal(count.to_string())],
        ));
        Ok(count)
    }

    /// `TestCommand.clear`.
    fn clear_tests(&mut self, finder: &Finder) -> R<i32> {
        self.clear_game_tests();
        let tests = self.finder_positions(finder);
        for &pos in &tests {
            if let Some(area) = self.test_bounding_box(pos) {
                self.clear_test_space(&area);
            }
            self.remove_test_barriers(pos);
            // level.destroyBlock(pos, false)
            self.set_test_block_state(
                pos,
                "minecraft:air",
                horizon_world::level::flags::UPDATE_ALL,
            );
        }
        if tests.is_empty() {
            return Err(err("commands.test.clear.error.no_tests", &[]));
        }
        self.feedback(Text::translate(
            "commands.test.clear.success",
            &[Text::literal(tests.len().to_string())],
        ));
        Ok(tests.len() as i32)
    }

    /// The `/test` subcommands that run, reset and clear tests.
    pub(super) fn cmd_test_runs(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let int = |name: &str| match ctx.get(name) {
            Some(Arg::Int(v)) => Some(*v),
            _ => None,
        };
        // runWithRetryOptions: `RetryOptions(numberOfTimes, untilFailed)`, else
        // no retries.
        let retry = match int("numberOfTimes") {
            Some(n) => Retry::new(n, matches!(ctx.get("untilFailed"), Some(Arg::Bool(true)))),
            None => Retry::none(),
        };
        let has = |word: &str| ctx.literals.iter().any(|l| l == word);
        let looks = has("runthat") || has("resetthat") || has("clearthat");
        if looks {
            self.source_player()?;
        }
        match id {
            "test.stop" => {
                self.clear_game_tests();
                Ok(1)
            }
            "test.run" => {
                let tests = match ctx.get("tests") {
                    Some(Arg::Str(pattern)) => horizon_client::command::args::select_resources(
                        self.ui.registries.ids("minecraft:test_instance"),
                        pattern,
                    )
                    .into_iter()
                    .cloned()
                    .collect(),
                    _ => Vec::new(),
                };
                let copies = if has("runmultiple") {
                    int("amount").unwrap_or(1)
                } else {
                    1
                };
                let finder = Finder::Selection { tests, copies };
                self.run_tests(
                    &finder,
                    if has("runmultiple") {
                        Retry::none()
                    } else {
                        retry
                    },
                    int("rotationSteps").unwrap_or(0),
                    int("testsPerRow").unwrap_or(8),
                )
            }
            "test.run.found" => {
                let finder = if has("runclosest") {
                    Finder::Nearest
                } else if has("runthat") {
                    Finder::LookedAt
                } else if has("runfailed") {
                    let only = matches!(ctx.get("onlyRequiredTests"), Some(Arg::Bool(true)));
                    Finder::Failed(only)
                } else {
                    Finder::AllNearby
                };
                self.run_tests(
                    &finder,
                    retry,
                    int("rotationSteps").unwrap_or(0),
                    int("testsPerRow").unwrap_or(8),
                )
            }
            _ => {
                let finder = if has("resetclosest") {
                    Finder::Nearest
                } else if has("resetthese") || has("clearthese") {
                    Finder::AllNearby
                } else if looks {
                    Finder::LookedAt
                } else {
                    // clearall [radius]: clamped to 0..=1024, 250 by default.
                    Finder::Radius(int("radius").map_or(250, |r| r.clamp(0, 1024)))
                };
                if id == "test.clear"
                    && (has("resetclosest") || has("resetthese") || has("resetthat"))
                {
                    self.reset_tests(&finder)
                } else {
                    self.clear_tests(&finder)
                }
            }
        }
    }
}

/// `AABB.clip(from, to).isPresent()`: the segment meets the box.
fn ray_hits_box(from: [f64; 3], to: [f64; 3], min: [f64; 3], max: [f64; 3]) -> bool {
    let (mut near, mut far) = (0.0f64, 1.0f64);
    for i in 0..3 {
        let d = to[i] - from[i];
        if d.abs() < 1.0e-7 {
            if from[i] < min[i] || from[i] > max[i] {
                return false;
            }
            continue;
        }
        let (a, b) = ((min[i] - from[i]) / d, (max[i] - from[i]) / d);
        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
        near = near.max(lo);
        far = far.min(hi);
        if near > far {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_steps_run_from_zero_to_three() {
        assert_eq!(rotation_for_steps(0), Some(Rotation::None));
        assert_eq!(rotation_for_steps(1), Some(Rotation::Clockwise90));
        assert_eq!(rotation_for_steps(3), Some(Rotation::Counterclockwise90));
        assert_eq!(rotation_for_steps(4), None);
        assert_eq!(rotation_for_steps(-1), None);
    }

    #[test]
    fn a_ray_meets_a_box_it_crosses_and_misses_one_beside_it() {
        let (min, max) = ([0.0, 0.0, 0.0], [2.0, 2.0, 2.0]);
        assert!(ray_hits_box([-5.0, 1.0, 1.0], [5.0, 1.0, 1.0], min, max));
        assert!(!ray_hits_box([-5.0, 3.0, 1.0], [5.0, 3.0, 1.0], min, max));
        assert!(!ray_hits_box([-5.0, 1.0, 1.0], [-1.0, 1.0, 1.0], min, max));
    }
}
