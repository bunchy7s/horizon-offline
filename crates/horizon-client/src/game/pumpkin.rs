//! `PumpkinBlock.useItemOn`: shears carve a pumpkin into a carved pumpkin
//! facing the clicked side (the side facing the player on the top and
//! bottom), and the `carve/pumpkin` seeds pop out of that side.
use super::Game;
use horizon_client::{Result, items::ItemStack};
use horizon_core::random::RandomSource;

impl Game<'_> {
    /// `PumpkinBlock.useItemOn` with `held` in the hand that acts, the block
    /// at `pos` clicked on `face`: true when shears carved it (SUCCESS).
    pub(super) fn use_pumpkin(
        &mut self,
        pos: [i32; 3],
        face: [i32; 3],
        held: Option<&ItemStack>,
    ) -> Result<bool> {
        let Some(shears) = held.filter(|s| &*s.item == "minecraft:shears") else {
            return Ok(false);
        };
        let state = self.world.block(pos[0], pos[1], pos[2]);
        if &*self.res.facts().get(state).name != "minecraft:pumpkin" {
            return Ok(false);
        }
        // Up or down: player.getDirection().getOpposite().
        let (dx, dz) = if face[1] != 0 {
            match ((self.yaw / 90.0 + 0.5).floor() as i32).rem_euclid(4) {
                0 => (0, -1),
                1 => (1, 0),
                2 => (0, 1),
                _ => (-1, 0),
            }
        } else {
            (face[0], face[2])
        };
        let facing = match (dx, dz) {
            (0, -1) => "north",
            (0, 1) => "south",
            (-1, 0) => "west",
            _ => "east",
        };
        // dropFromBlockInteractLootTable(CARVE_PUMPKIN): each stack as an
        // ItemEntity in front of the carved side, moved by the level random
        // (no pickup delay).
        let (dx, dz) = (f64::from(dx), f64::from(dz));
        let table = "minecraft:carve/pumpkin";
        let mut drops = self.sequence_loot(table, table, Some(shears));
        let origin = pos.map(|c| f64::from(c) + 0.5);
        self.native_use_drops(
            table,
            origin,
            Some((pos, state)),
            None,
            Some(shears),
            &mut drops,
        );
        for seeds in drops {
            let at = [
                f64::from(pos[0]) + 0.5 + dx * 0.65,
                f64::from(pos[1]) + 0.1,
                f64::from(pos[2]) + 0.5 + dz * 0.65,
            ];
            let r = &mut self.world.random;
            let vx = 0.05 * dx + r.next_double() * 0.02;
            let vz = 0.05 * dz + r.next_double() * 0.02;
            self.spawn_item(seeds, at, [vx, 0.05, vz], 0);
        }
        self.audio.play(
            "minecraft:block.pumpkin.carve",
            Some([
                f64::from(pos[0]) + 0.5,
                f64::from(pos[1]) + 0.5,
                f64::from(pos[2]) + 0.5,
            ]),
            1.0,
            1.0,
        );
        let registry = self.pack.assets.registry;
        let carved = registry
            .default_state("minecraft:carved_pumpkin")
            .and_then(|d| {
                let mut s = registry.state(d)?.clone();
                s.properties.insert("facing".into(), facing.into());
                registry.resolve(&s).ok()
            });
        if let Some(carved) = carved {
            self.set_golem_head(pos, carved)?;
        }
        self.damage_held(1);
        self.post_block_game_event("minecraft:shear", pos, self.sculk_actor(), None);
        self.stats.add("used", "minecraft:shears", 1);
        // ServerPlayerGameMode.useItemOn: the action was consumed.
        self.trigger_block("minecraft:item_used_on_block", pos, shears);
        Ok(true)
    }
}
