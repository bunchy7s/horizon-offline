//! Powder snow around the player: its snowflakes, fire put out by it, and the
//! bucket that picks it up (`BucketItem.use` on a `BucketPickup` block). The
//! sinking, slowing, climbing and freezing are `PlayerMotion`'s
//! (`crate::motion`); placing it is the powder snow bucket's `BlockItem.place`.
use super::Game;
use horizon_client::{Result, items::ItemStack, mobs::Who};
use horizon_core::{position::BlockPos, random::RandomSource};

impl Game<'_> {
    /// Entity.playEntityOnFireExtinguishedSound
    pub(super) fn extinguish_sound(&mut self, at: [f64; 3]) {
        let r = self.particles.random();
        let pitch = 1.6 + (r.next_float() - r.next_float()) * 0.4;
        self.audio.play(
            "minecraft:entity.generic.extinguish_fire",
            Some(at),
            0.7,
            pitch,
        );
    }
    /// `Entity.applyEffectsFromBlocks`: a burning player in the rain
    /// (`isInRain`) has the fire put out (`clearFire`), with the extinguish
    /// sound that everyone near hears.
    pub(super) fn rain_extinguish_player(&mut self) {
        let Some(player) = self.player.as_ref() else {
            return;
        };
        let (at, top) = (player.position, player.bounds().max_position().y);
        if player.fire_ticks <= 0 || !self.in_rain(at, top) {
            return;
        }
        if let Some(player) = self.player.as_mut() {
            player.fire_ticks = 0;
        }
        self.extinguish_sound([at.x, at.y, at.z]);
    }
    /// Whether the block at `pos` is powder snow.
    pub(super) fn is_powder_snow(&self, pos: [i32; 3]) -> bool {
        &*self
            .res
            .facts()
            .get(self.world.block(pos[0], pos[1], pos[2]))
            .name
            == "minecraft:powder_snow"
    }
    /// `BucketItem.use` with an empty bucket on powder snow:
    /// `PowderSnowBlock.pickupBlock` (air, level event 2001), the used
    /// statistic, the fill sound, FLUID_PICKUP, `ItemUtils.createFilledResult`
    /// and the filled_bucket trigger.
    pub(super) fn pick_up_powder_snow(&mut self, pos: [i32; 3]) -> Result<()> {
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let creative = self.creative;
        let picked = self.with_level(|b, level, ticks| b.bucket_pickup(level, ticks, at, creative));
        self.process_level_events()?;
        let Some(filled) = picked else {
            return Ok(());
        };
        self.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
        self.award_stat("used", "minecraft:bucket", 1);
        if let Some(eye) = self
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z])
        {
            self.audio.play(
                "minecraft:item.bucket.fill_powder_snow",
                Some(eye),
                1.0,
                1.0,
            );
        }
        self.post_block_game_event("minecraft:fluid_pickup", pos, Some(Who::Player), None);
        let filled = ItemStack::new(filled, 1);
        self.filled_result(filled.clone());
        self.trigger_item("minecraft:filled_bucket", &filled);
        self.swing_arm();
        Ok(())
    }
    /// `ItemUtils.createFilledResult(held, player, filled)` for the held stack.
    pub(super) fn filled_result(&mut self, filled: ItemStack) {
        if self.creative {
            // Kept, and the filled one added unless the inventory has it.
            let has = self
                .inventory
                .iter()
                .flatten()
                .any(|s| s.same_item_same_components(&filled));
            if !has {
                let _ = self.give(filled);
            }
            return;
        }
        let held = self.held_mut();
        let last = held.as_ref().is_none_or(|s| s.count <= 1);
        if last {
            *held = Some(filled);
            return;
        }
        if let Some(s) = held.as_mut() {
            s.count -= 1;
        }
        if let Some(rest) = self.give(filled) {
            self.player_drop(rest);
        }
    }
}
