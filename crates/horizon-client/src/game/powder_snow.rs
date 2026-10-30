//! Powder snow around the player: its snowflakes, fire put out by it, and the
//! bucket that picks it up (`BucketItem.use` on a `BucketPickup` block). The
//! sinking, slowing, climbing and freezing are `PlayerMotion`'s
//! (`crate::motion`); placing it is the powder snow bucket's `BlockItem.place`.
use super::Game;
use horizon_client::{
    Result,
    items::ItemStack,
    mobs::Who,
    particles::{ParticleOptions, ParticleType},
};
use horizon_core::{geometry::Vec3, position::BlockPos, random::RandomSource};

impl Game<'_> {
    /// `ClientPacketListener.handleParticleEvent` for the one ALTERNATIVE
    /// snowflake `PowderSnowBlock.entityInside` sends: no spread, its speed.
    pub(super) fn snowflake(&mut self, position: Vec3, velocity: Vec3) {
        let r = self.particles.random();
        // nextDouble() * dist for each axis, the distances being 0.
        let offset = [
            r.next_double() * 0.0,
            r.next_double() * 0.0,
            r.next_double() * 0.0,
        ];
        let options = ParticleOptions::simple(ParticleType::Snowflake);
        self.particles.add(
            &options,
            [
                position.x + offset[0],
                position.y + offset[1],
                position.z + offset[2],
            ],
            [velocity.x, velocity.y, velocity.z],
        );
    }
    /// The EXTINGUISH effect of the powder snow the player went into this
    /// tick: burning, it destroys those blocks (a player always may), then
    /// its fire goes out (`Entity.clearFire`) with the extinguish sound.
    pub(super) fn powder_snow_fire(&mut self) -> Result<()> {
        let Some(player) = self.player.as_mut() else {
            return Ok(());
        };
        let entered = std::mem::take(&mut player.powder_snow_entered);
        let at = [player.position.x, player.position.y, player.position.z];
        if entered.is_empty() || player.fire_ticks <= 0 {
            return Ok(());
        }
        for pos in entered {
            self.with_level(|b, level, ticks| b.destroy_block(level, ticks, pos, false));
            self.world.modified.insert((pos.x >> 4, pos.z >> 4));
        }
        self.process_level_events()?;
        if let Some(player) = self.player.as_mut() {
            player.fire_ticks = 0;
        }
        self.extinguish_sound(at);
        Ok(())
    }
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
    /// The in-window player's rain extinguish (a server's player has its
    /// fire put out by the server).
    pub(super) fn rain_extinguish_in_frame(&mut self) {
        if self.remote.is_none() {
            self.rain_extinguish_player();
        }
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
        let picked = self.with_level(|b, level, ticks| b.bucket_pickup(level, ticks, at));
        self.process_level_events()?;
        let Some(filled) = picked else {
            return Ok(());
        };
        self.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
        self.stats.add("used", "minecraft:bucket", 1);
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
    fn filled_result(&mut self, filled: ItemStack) {
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
