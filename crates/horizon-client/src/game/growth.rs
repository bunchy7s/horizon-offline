//! Client glue for sapling growth and bone meal: installs the live feature
//! registry, routes `BoneMealItem.useOn` through the behaviour library, and
//! crops a growing plant's head with shears (`ShearsItem.useOn`).
use super::Game;
use horizon_client::{Result, items::ItemStack, vanillagen};
use horizon_core::{geometry::Direction, position::BlockPos};
use horizon_world::{level::classes::Class, worldgen::live::LiveFeatures};
use std::sync::Arc;

impl Game<'_> {
    /// Load the configured/placed feature registry used by saplings, fungi
    /// and bone meal (once per process; shares the generator's bundle).
    pub(super) fn install_live_features(&self) {
        install_live_features(&self.res, &self.pack, &self.options);
    }
    /// `BoneMealItem.useOn` for the targeted block.
    pub(super) fn use_bone_meal(&mut self, stack: &ItemStack) -> Result<bool> {
        if &*stack.item != "minecraft:bone_meal" {
            return Ok(false);
        }
        let Some(hit) = self.pick() else {
            return Ok(false);
        };
        let face = match hit.face {
            [0, 1, 0] => Direction::Up,
            [0, -1, 0] => Direction::Down,
            [0, 0, -1] => Direction::North,
            [0, 0, 1] => Direction::South,
            [-1, 0, 0] => Direction::West,
            _ => Direction::East,
        };
        let pos = BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]);
        let used = self.with_level(|b, level, ticks| b.use_bone_meal(level, ticks, pos, face));
        // ItemStack.causeUseVibration at the player, after the meal is used
        // up (shrunk even in creative): an emptied stack has no use effects.
        if used
            && stack.count > 1
            && let Some(p) = self.player.as_ref().map(|p| p.position)
        {
            let ctx = self.player_context(None);
            self.post_game_event("minecraft:item_interact_finish", [p.x, p.y, p.z], ctx);
        }
        self.process_level_events()?;
        if used {
            self.swing_arm();
            // ItemStack.useOn: an item interaction counts as a use.
            self.stats.add("used", &stack.item, 1);
            self.consume_held();
        }
        Ok(used)
    }
    /// `ShearsItem.useOn` on a growing plant's head below its top age (kelp,
    /// weeping and twisting vines, cave vines): the head goes to
    /// `GrowingPlantHeadBlock.getMaxAgeState` (age 25) with
    /// `block.growing_plant.crop`, the level hears a `block_change`, and the
    /// shears wear by one. True when it cropped (SUCCESS).
    pub(super) fn shears_crop(&mut self, pos: [i32; 3], stack: &ItemStack) -> Result<bool> {
        if &*stack.item != "minecraft:shears" {
            return Ok(false);
        }
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let b = self.res.behaviours();
        // A head grows and carries `AGE_25`; the plant's body below it does not.
        if !b.data(state).is(Class::GrowingPlantHead) {
            return Ok(false);
        }
        // `GrowingPlantHeadBlock.isMaxAge`: the property's top value.
        if b.table.int(state, b.props.age) == 25 {
            return Ok(false);
        }
        // `GrowingPlantHeadBlock.getMaxAgeState`.
        let max_age = b.table.set_int(state, b.props.age, 25);
        self.audio.play(
            "minecraft:block.growing_plant.crop",
            Some([
                f64::from(pos[0]) + 0.5,
                f64::from(pos[1]) + 0.5,
                f64::from(pos[2]) + 0.5,
            ]),
            1.0,
            1.0,
        );
        self.level_set(pos, max_age)?;
        self.post_block_game_event(
            "minecraft:block_change",
            pos,
            self.sculk_actor(),
            Some(max_age),
        );
        self.damage_held(1);
        // ItemStack.useOn: an item interaction counts as a use.
        self.stats.add("used", &stack.item, 1);
        self.swing_arm();
        // ServerPlayerGameMode.useItemOn: a consuming useOn reports it.
        self.trigger_block("minecraft:item_used_on_block", pos, stack);
        Ok(true)
    }
}

/// Shared by the initialization worker and fixture setup.
pub(super) fn install_live_features(
    res: &super::Resources,
    pack: &super::PackResources,
    options: &super::Options,
) {
    if res.behaviours().live_features().is_some() {
        return;
    }
    let dir = options.resources.join("worldgen");
    let started = std::time::Instant::now();
    let features = vanillagen::shared_data(&dir, pack.assets.registry)
        .map_err(|e| e.to_string())
        .and_then(|data| LiveFeatures::new(&data).map_err(|e| e.to_string()));
    match features {
        Ok(Some(features)) => {
            res.behaviours().set_live_features(Arc::new(features));
            log::info!("Horizon: live features ready in {:?}", started.elapsed());
        }
        Ok(None) => {
            log::warn!("Horizon: the worldgen bundle has no features; saplings will not grow")
        }
        Err(e) => log::warn!("Horizon: live features unavailable: {e}"),
    }
}
