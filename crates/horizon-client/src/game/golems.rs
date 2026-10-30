//! Building golems from blocks (`CarvedPumpkinBlock.trySpawnGolem`): a carved
//! pumpkin or jack o'lantern on two snow blocks makes a snow golem, and on a
//! T of four iron blocks (arms along X or Z, nothing beside the legs) an iron
//! golem. Players placing the pumpkin and dispensers share it.
use super::Game;
use horizon_client::Result;

impl Game<'_> {
    fn golem_block(&self, p: [i32; 3]) -> &str {
        let state = self.world.block(p[0], p[1], p[2]);
        &self.res.facts().get(state).name
    }
    fn is_golem_head(&self, p: [i32; 3]) -> bool {
        matches!(
            self.golem_block(p),
            "minecraft:carved_pumpkin" | "minecraft:jack_o_lantern"
        )
    }
    /// The blocks of the golem completed at `head` and its spawn base
    /// (`BlockPattern.find`), snow golems first: the snow column ("^", "#",
    /// "#") spawns at its bottom, the iron T ("~^~", "###", "~#~") under its
    /// head. The blocks come column by column, top first, as
    /// `clearPatternBlocks` visits them. With `base`, the head's cell takes
    /// any block, as in the base patterns (" ", "#", "#") and ("~ ~", "###",
    /// "~#~") that `canSpawnGolem` checks before the head is there.
    fn find_golem(
        &self,
        head: [i32; 3],
        base: bool,
    ) -> Option<(&'static str, Vec<[i32; 3]>, [i32; 3])> {
        let air = |p: [i32; 3]| self.res.facts().is_air(self.world.block(p[0], p[1], p[2]));
        let snow = |c: char, p: [i32; 3]| match c {
            '^' => base || self.is_golem_head(p),
            _ => self.golem_block(p) == "minecraft:snow_block",
        };
        let iron = |c: char, p: [i32; 3]| match c {
            '^' => base || self.is_golem_head(p),
            '#' => self.golem_block(p) == "minecraft:iron_block",
            _ => air(p),
        };
        if let Some(cell) = find_pattern(head, &["^", "#", "#"], snow) {
            let blocks = (0..3).map(|y| cell(0, y)).collect();
            return Some(("minecraft:snow_golem", blocks, cell(0, 2)));
        }
        let cell = find_pattern(head, &["~^~", "###", "~#~"], iron)?;
        let blocks = (0..3)
            .flat_map(|x| (0..3).map(move |y| (x, y)))
            .map(|(x, y)| cell(x, y))
            .collect();
        Some(("minecraft:iron_golem", blocks, cell(1, 2)))
    }
    /// `CarvedPumpkinBlock.canSpawnGolem`: a snow or iron golem base, or a
    /// copper block (`#minecraft:copper`) below.
    pub(super) fn can_spawn_golem(&self, head: [i32; 3]) -> bool {
        self.find_golem(head, true).is_some() || self.copper_golem_base(head)
    }
    /// The copper golem pattern ("^", "#"): a block of `#minecraft:copper` under the head.
    fn copper_golem_base(&self, head: [i32; 3]) -> bool {
        let below = [head[0], head[1] - 1, head[2]];
        let name = self.golem_block(below);
        self.res
            .items()
            .block_tags
            .get("minecraft:copper")
            .is_some_and(|t| t.contains(name))
    }
    /// `CarvedPumpkinBlock.trySpawnGolem`, after a pumpkin was placed.
    pub(super) fn check_golem_spawn(&mut self, head: [i32; 3]) -> Result<()> {
        if !self.is_golem_head(head) {
            return Ok(());
        }
        let Some((kind, blocks, base)) = self.find_golem(head, false) else {
            return self.try_spawn_copper_golem(head);
        };
        // CarvedPumpkinBlock.spawnGolemInWorld
        self.clear_pattern_blocks(&blocks)?;
        let at = [
            f64::from(base[0]) + 0.5,
            f64::from(base[1]) + 0.05,
            f64::from(base[2]) + 0.5,
        ];
        let id = self.create_triggered(kind, at, 0.0);
        // CarvedPumpkinBlock.trySpawnGolem: setPlayerCreated(true) on the iron golem.
        if kind == "minecraft:iron_golem"
            && let Some(m) = id.and_then(|id| self.mobs.iter_mut().find(|m| m.id == id))
        {
            m.extra.player_created = true;
        }
        self.trigger_summoned(id, 5.0);
        self.update_pattern_blocks(&blocks)
    }
    /// SUMMONED_ENTITY for each player whose box is within `reach` blocks of
    /// the built mob's box (the local player here): 5 for golems
    /// (`CarvedPumpkinBlock.spawnGolemInWorld`), 50 for the wither
    /// (`WitherSkullBlock.checkSpawn`).
    pub(super) fn trigger_summoned(&mut self, id: Option<u32>, reach: f64) {
        let Some(built) = id
            .and_then(|id| self.mobs.iter().find(|m| m.id == id))
            .cloned()
        else {
            return;
        };
        let me = self.player_entity();
        let half = f64::from(built.width()) / 2.0;
        let height = f64::from(built.height());
        let [x, y, z] = me.position;
        let p = built.position;
        // The player's box is 0.6 wide and 1.8 tall.
        let near = x + 0.3 >= p.x - half - reach
            && x - 0.3 <= p.x + half + reach
            && y + 1.8 >= p.y - reach
            && y <= p.y + height + reach
            && z + 0.3 >= p.z - half - reach
            && z - 0.3 <= p.z + half + reach;
        if near {
            let view = self.mob_entity(&built);
            self.trigger_entity("minecraft:summoned_entity", "entity", &view);
        }
    }
    /// Whether placing a golem head at `head` completes a golem.
    pub(super) fn completes_golem(&self, head: [i32; 3]) -> bool {
        self.find_golem(head, true).is_some() || self.copper_golem_base(head)
    }
    /// `Level.setBlock` of the golem head `state` at `head`, then
    /// `CarvedPumpkinBlock.onPlace`. When the head completes a golem, onPlace
    /// builds it inside setBlock and the spot is air again by the time
    /// setBlock would update neighbours, so it skips them.
    pub(super) fn set_golem_head(
        &mut self,
        head: [i32; 3],
        state: horizon_world::registry::BlockStateId,
    ) -> Result<()> {
        if self.completes_golem(head) {
            let at = horizon_core::position::BlockPos::new(head[0], head[1], head[2]);
            self.with_level(|b, level, ticks| b.set_block(level, ticks, at, state, 2 | 16));
            self.process_level_events()?;
        } else {
            self.level_set(head, state)?;
        }
        self.check_golem_spawn(head)
    }
    /// `EntityType.create(level, TRIGGERED)` and `snapTo`: a built golem or
    /// wither has no spawn finalization and draws nothing from the level.
    pub(super) fn create_triggered(&mut self, kind: &str, at: [f64; 3], yaw: f32) -> Option<u32> {
        let t = self.res.mob_types().get(kind).cloned()?;
        self.next_entity += 1;
        let mut mob = horizon_client::mobs::Mob::new(
            self.next_entity,
            t,
            horizon_core::geometry::Vec3::new(at[0], at[1], at[2]),
            yaw,
        );
        mob.head_yaw = yaw;
        mob.body_yaw = yaw;
        mob.previous_yaw = yaw;
        mob.previous_body_yaw = yaw;
        mob.previous_head_yaw = yaw;
        let id = mob.id;
        self.mobs.push(mob);
        Some(id)
    }
    /// `CarvedPumpkinBlock.clearPatternBlocks`: every cell of the match
    /// becomes air (flags 2) with its destroy event (2001).
    pub(super) fn clear_pattern_blocks(&mut self, cells: &[[i32; 3]]) -> Result<()> {
        let air = self.world.air;
        for &p in cells {
            let state = self.world.block(p[0], p[1], p[2]);
            let pos = horizon_core::position::BlockPos::new(p[0], p[1], p[2]);
            self.with_level(|b, level, ticks| b.set_block(level, ticks, pos, air, 2));
            self.level_events
                .push(horizon_world::level::LevelEvent::LevelEvent {
                    id: 2001,
                    pos,
                    data: state.0 as i32,
                });
        }
        self.process_level_events()
    }
    /// `CarvedPumpkinBlock.updatePatternBlocks`: neighbours of every cell
    /// hear that air arrived.
    pub(super) fn update_pattern_blocks(&mut self, cells: &[[i32; 3]]) -> Result<()> {
        let air = self.world.air;
        for &p in cells {
            let pos = horizon_core::position::BlockPos::new(p[0], p[1], p[2]);
            self.with_level(|b, level, ticks| {
                b.update_neighbors_at(level, ticks, pos, b.block(air))
            });
        }
        self.process_level_events()
    }
    /// The copper golem (`CarvedPumpkinBlock.trySpawnGolem`, after the snow
    /// and iron golems): both blocks clear, the golem appears where the
    /// pumpkin was, the copper block becomes the matching copper chest facing
    /// the pumpkin's way (`replaceCopperBlockWithChest`, flags 2) and the
    /// golem takes the block's weathering (`CopperGolem.spawn`).
    fn try_spawn_copper_golem(&mut self, head: [i32; 3]) -> Result<()> {
        if !self.copper_golem_base(head) {
            return Ok(());
        }
        let below = [head[0], head[1] - 1, head[2]];
        let copper = self.world.block(below[0], below[1], below[2]);
        let pumpkin = self.world.block(head[0], head[1], head[2]);
        let registry = self.pack.assets.registry;
        let facing = registry
            .state(pumpkin)
            .and_then(|s| s.properties.get("facing").cloned())
            .unwrap_or_else(|| "north".into());
        // getWeatherStateFromPattern: waxed blocks weather like their unwaxed form.
        let base = self
            .golem_block(below)
            .trim_start_matches("minecraft:")
            .trim_start_matches("waxed_");
        let weather = ["exposed", "weathered", "oxidized"]
            .iter()
            .position(|w| base.starts_with(w))
            .map_or(0, |i| i as u8 + 1);
        let cells = [head, below];
        self.clear_pattern_blocks(&cells)?;
        let at = [
            f64::from(head[0]) + 0.5,
            f64::from(head[1]) + 0.05,
            f64::from(head[2]) + 0.5,
        ];
        let id = self.create_triggered("minecraft:copper_golem", at, 0.0);
        self.trigger_summoned(id, 5.0);
        self.update_pattern_blocks(&cells)?;
        let direction = match facing.as_str() {
            "south" => horizon_core::geometry::Direction::South,
            "west" => horizon_core::geometry::Direction::West,
            "east" => horizon_core::geometry::Direction::East,
            _ => horizon_core::geometry::Direction::North,
        };
        let pos = horizon_core::position::BlockPos::new(below[0], below[1], below[2]);
        let chest = self.with_level(|b, level, ticks| {
            b.copper_chest_from_block(level, ticks, copper, direction, pos)
        });
        if let Some(chest) = chest {
            self.with_level(|b, level, ticks| b.set_block(level, ticks, pos, chest, 2));
            if let Some(entity) = horizon_client::containers::BlockEntity::for_block(
                &self.res.facts().get(chest).name,
            ) {
                self.world
                    .block_entities
                    .insert((below[0], below[1], below[2]), entity);
            }
        }
        if let Some(m) = id.and_then(|id| self.mobs.iter_mut().find(|m| m.id == id)) {
            m.extra.variant = weather;
            let p = [m.position.x, m.position.y, m.position.z];
            self.audio
                .play("minecraft:entity.copper_golem.spawn", Some(p), 1.0, 1.0);
        }
        self.process_level_events()
    }
}

/// `BlockPattern.find` for a one-aisle pattern (`rows`, top first): the
/// pattern's corner at every block from `pos` to `pos + size - 1` (x fastest,
/// then y, then z), turned every way (`forwards`, then `up`, in `Direction`
/// order, never along each other), until every cell passes `test`. The match
/// gives the block of column x, row y (`BlockPatternMatch.getBlock(x, y, 0)`).
fn find_pattern(
    pos: [i32; 3],
    rows: &[&str],
    test: impl Fn(char, [i32; 3]) -> bool,
) -> Option<impl Fn(i32, i32) -> [i32; 3]> {
    // Direction.values(): down, up, north, south, west, east.
    const DIRECTIONS: [[i32; 3]; 6] = [
        [0, -1, 0],
        [0, 1, 0],
        [0, 0, -1],
        [0, 0, 1],
        [-1, 0, 0],
        [1, 0, 0],
    ];
    let size = rows.len().max(rows[0].len()) as i32;
    let corners =
        (0..size).flat_map(|z| (0..size).flat_map(move |y| (0..size).map(move |x| [x, y, z])));
    for d in corners {
        let origin = [pos[0] + d[0], pos[1] + d[1], pos[2] + d[2]];
        for forwards in DIRECTIONS {
            for up in DIRECTIONS {
                if up == forwards || up == forwards.map(|v| -v) {
                    continue;
                }
                // translateAndRotate: right is forwards × up, rows go down `up`.
                let right = [
                    forwards[1] * up[2] - forwards[2] * up[1],
                    forwards[2] * up[0] - forwards[0] * up[2],
                    forwards[0] * up[1] - forwards[1] * up[0],
                ];
                let cell =
                    move |x: i32, y: i32| [0, 1, 2].map(|i| origin[i] - up[i] * y + right[i] * x);
                let matches = rows.iter().enumerate().all(|(y, row)| {
                    row.chars()
                        .enumerate()
                        .all(|(x, c)| test(c, cell(x as i32, y as i32)))
                });
                if matches {
                    return Some(cell);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::find_pattern;
    use std::collections::HashMap;

    const SNOW: [&str; 3] = ["^", "#", "#"];
    const IRON: [&str; 3] = ["~^~", "###", "~#~"];

    /// `find_pattern` over `blocks` (anything else is air): the cells of the
    /// match, column by column, top first.
    fn find(rows: &[&str], head: [i32; 3], blocks: &[([i32; 3], char)]) -> Option<Vec<[i32; 3]>> {
        let world: HashMap<_, _> = blocks.iter().copied().collect();
        let at = |p: [i32; 3]| world.get(&p).copied().unwrap_or('~');
        let cell = find_pattern(head, rows, |c, p| at(p) == c)?;
        let width = rows[0].len() as i32;
        Some(
            (0..width)
                .flat_map(|x| (0..rows.len() as i32).map(move |y| (x, y)))
                .map(|(x, y)| cell(x, y))
                .collect(),
        )
    }

    #[test]
    fn a_snow_golem_column_counts_upright_lying_and_upside_down() {
        let head = [5, 64, 5];
        for step in [
            [0, -1, 0],
            [1, 0, 0],
            [-1, 0, 0],
            [0, 0, 1],
            [0, 0, -1],
            [0, 1, 0],
        ] {
            let at = |n: i32| [0, 1, 2].map(|i| head[i] + step[i] * n);
            let blocks = [(at(0), '^'), (at(1), '#'), (at(2), '#')];
            assert_eq!(
                find(&SNOW, head, &blocks),
                Some(vec![at(0), at(1), at(2)]),
                "snow blocks going {step:?} from the head"
            );
        }
    }

    #[test]
    fn a_bent_or_short_snow_column_is_no_golem() {
        let head = [0, 0, 0];
        let bent = [(head, '^'), ([0, -1, 0], '#'), ([1, -1, 0], '#')];
        assert_eq!(find(&SNOW, head, &bent), None);
        assert_eq!(find(&SNOW, head, &[(head, '^'), ([0, -1, 0], '#')]), None);
    }

    #[test]
    fn an_iron_golem_t_counts_lying_on_its_back() {
        // Head at the origin, body along +x, arms along z, all at y 0.
        let head = [0, 0, 0];
        let blocks = [
            (head, '^'),
            ([1, 0, -1], '#'),
            ([1, 0, 0], '#'),
            ([1, 0, 1], '#'),
            ([2, 0, 0], '#'),
        ];
        let cells = find(&IRON, head, &blocks).expect("a lying iron golem");
        // Column 1, rows 0 to 2: the head, the body and the legs.
        assert_eq!(&cells[3..6], &[[0, 0, 0], [1, 0, 0], [2, 0, 0]]);
        let mut arms_out = blocks.to_vec();
        arms_out.push(([2, 0, 1], '#'));
        assert_eq!(
            find(&IRON, head, &arms_out),
            None,
            "a block beside the legs"
        );
    }
}
