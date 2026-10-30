//! Vaults in the game: `VaultBlockEntity.Server.tick` for every vault
//! (`vault`), the block's state as it changes, the rewards it ejects, the
//! keys players insert (`VaultBlock.useItemOn`) and the display item spinning
//! inside (`VaultRenderer`).
use super::Game;
use horizon_assets::item_model::DisplayContext;
use horizon_client::{
    Result,
    block_entity_render::vertex_light,
    containers::BlockEntity,
    item_render,
    items::ItemStack,
    mesher::Vertex,
    vault::{Vault, VaultEvent, VaultLevel, VaultPlayer, VaultState},
};
use horizon_core::random::LegacyRandom;

type Pos = (i32, i32, i32);

/// What a vault reads: the game time and the players.
struct Players {
    time: i64,
    players: Vec<VaultPlayer>,
}
impl VaultLevel for Players {
    fn game_time(&self) -> i64 {
        self.time
    }
    fn players(&self) -> Vec<VaultPlayer> {
        self.players.clone()
    }
}

impl Game<'_> {
    fn vault_players(&self) -> Players {
        let host = self.player.as_ref().map(|p| VaultPlayer {
            uuid: self.acting_uuid(),
            block: [p.position.x, p.position.y, p.position.z].map(|v| v.floor() as i32),
            spectator: self.spectator(),
        });
        let guests = self.lan_present().into_iter().map(|g| VaultPlayer {
            uuid: g.uuid,
            block: g.position.map(|v| v.floor() as i32),
            spectator: g.spectator,
        });
        Players {
            time: self.world.game_time,
            players: host.into_iter().chain(guests).collect(),
        }
    }
    /// The vault at `key` with its block's state and ominous flag.
    pub(super) fn take_vault(&mut self, key: Pos) -> Option<(Box<Vault>, VaultState, bool)> {
        let state = self.world.block(key.0, key.1, key.2);
        let block = self.pack.assets.registry.state(state)?;
        if block.name != "minecraft:vault" {
            return None;
        }
        let current = block
            .properties
            .get("vault_state")
            .and_then(|s| VaultState::from_name(s))
            .unwrap_or_default();
        let ominous = block.properties.get("ominous").is_some_and(|v| v == "true");
        let Some(BlockEntity::Vault(vault)) = self.world.block_entities.take(&key) else {
            return None;
        };
        Some((vault, current, ominous))
    }
    /// Runs `f` on the vault at `key` with the level random and a loot roll,
    /// then applies its events.
    fn with_vault<T>(
        &mut self,
        key: Pos,
        f: impl FnOnce(
            &mut Vault,
            &Players,
            &mut LegacyRandom,
            VaultState,
            bool,
            &mut horizon_client::vault::Roll<LegacyRandom>,
            &mut Vec<VaultEvent>,
        ) -> T,
    ) -> Option<Result<T>> {
        let (mut vault, current, ominous) = self.take_vault(key)?;
        let players = self.vault_players();
        let mut random = std::mem::replace(&mut self.world.random, LegacyRandom::new(0));
        let mut sequences = std::mem::take(&mut self.rand_sequences);
        let loot = horizon_client::structure_blocks::loot_data(
            &self.options.resources,
            &self.pack_files(),
        );
        let seed = self.level.seed;
        let mut events = Vec::new();
        let out = {
            // LootTable.getRandomItems, stacks split; the reward rolls with the
            // table's random sequence.
            let mut roll = |table: &str, random: &mut LegacyRandom, sequence: bool| {
                let Some(loot) = &loot else {
                    return Vec::new();
                };
                let drops = match loot.random_sequence(table).filter(|_| sequence) {
                    Some(s) => loot.roll_split(table, sequences.get(s, seed)),
                    None => loot.roll_split(table, random),
                };
                drops
                    .iter()
                    .map(horizon_client::structure_blocks::loot_item_with_components)
                    .collect()
            };
            f(
                &mut vault,
                &players,
                &mut random,
                current,
                ominous,
                &mut roll,
                &mut events,
            )
        };
        self.world.random = random;
        self.rand_sequences = sequences;
        self.world
            .block_entities
            .put(key, BlockEntity::Vault(vault));
        Some(self.vault_events(key, events).map(|()| out))
    }
    /// The server tick of the vault at `key`, then its client tick when there
    /// is a window.
    pub(super) fn tick_vault(&mut self, key: Pos) -> Result<()> {
        let pos = [key.0, key.1, key.2];
        let ticked = self.with_vault(key, |v, l, random, state, ominous, roll, events| {
            v.server_tick(l, random, pos, state, ominous, roll, events);
        });
        if let Some(result) = ticked {
            result?;
        }
        if self.options.dedicated.is_none() {
            self.client_tick_vault(key);
        }
        Ok(())
    }
    /// `VaultBlock.useItemOn` with `held` in the hand that acts: true when the
    /// vault took the click (an active vault, with anything in the hand).
    pub(super) fn use_vault(&mut self, pos: [i32; 3], held: Option<&ItemStack>) -> Result<bool> {
        let Some(stack) = held.cloned() else {
            return Ok(false);
        };
        let player = self.acting_uuid();
        let used = self.with_vault(
            (pos[0], pos[1], pos[2]),
            |v, l, random, state, ominous, roll, events| {
                v.use_item_on(
                    l,
                    random,
                    pos,
                    state,
                    ominous,
                    player,
                    Some(&stack),
                    roll,
                    events,
                )
            },
        );
        let Some(taken) = used.transpose()?.flatten() else {
            return Ok(false);
        };
        if taken > 0 {
            // awardStat(ITEM_USED), then stackToInsert.consume(count, player).
            self.stats.add("used", &stack.item, 1);
            for _ in 0..taken {
                self.consume_held();
            }
        }
        // ServerPlayerGameMode.useItemOn: the action was consumed.
        self.trigger_block("minecraft:item_used_on_block", pos, &stack);
        Ok(true)
    }
    fn vault_events(&mut self, key: Pos, events: Vec<VaultEvent>) -> Result<()> {
        let pos = [key.0, key.1, key.2];
        let center = pos.map(|v| f64::from(v) + 0.5);
        for event in events {
            match event {
                VaultEvent::State(state) => self.set_vault_state(key, state)?,
                VaultEvent::BlockUpdated => self.mark_block_entity_updated(key),
                VaultEvent::LevelEvent(id, data) => self.level_event_at(id, pos, data),
                VaultEvent::Sound(name, volume, pitch) => {
                    self.audio
                        .play(&format!("minecraft:{name}"), Some(center), volume, pitch)
                }
                // Vec3.atBottomCenterOf(pos).relative(UP, 1.2), lowered by spawnItem.
                VaultEvent::Eject(stack, v) => {
                    self.spawn_item(stack, [center[0], center[1] + 0.7 - 0.125, center[2]], v, 0)
                }
            }
        }
        Ok(())
    }
    /// `setBlockAndUpdate` with the new `vault_state`.
    fn set_vault_state(&mut self, key: Pos, state: VaultState) -> Result<()> {
        let current = self.world.block(key.0, key.1, key.2);
        let Some(mut block) = self.pack.assets.registry.state(current).cloned() else {
            return Ok(());
        };
        block
            .properties
            .insert("vault_state".into(), state.name().into());
        let id = self.pack.assets.registry.resolve(&block)?;
        self.level_set([key.0, key.1, key.2], id)
    }
    /// `VaultRenderer`: the display item of every vault near the camera that
    /// shows one, turning above its floor.
    pub(super) fn vault_geometry(&self, camera: glam::DVec3, alpha: f64, out: &mut Vec<Vertex>) {
        for (key, entity) in &self.world.block_entities {
            let BlockEntity::Vault(vault) = entity else {
                continue;
            };
            let Some(stack) = &vault.display_item else {
                continue;
            };
            let at = glam::DVec3::new(f64::from(key.0), f64::from(key.1), f64::from(key.2));
            if at.distance_squared(camera) >= 64.0 * 64.0 {
                continue;
            }
            let state = self.resolve_item(stack, DisplayContext::Ground, None, 0);
            if state.is_empty() {
                continue;
            }
            let pose =
                glam::Mat4::from_translation(
                    (at - camera).as_vec3() + glam::Vec3::new(0.5, 0.4, 0.5),
                ) * glam::Mat4::from_rotation_y(vault.display_spin(alpha as f32).to_radians());
            let light = self.world.light(key.0, key.1, key.2);
            let seed =
                i64::from(self.pack.item_renderer.item_id(&stack.item)) + i64::from(stack.damage);
            for copy in item_render::cluster_poses(
                &state,
                pose,
                item_render::rendered_amount(stack.count),
                seed,
            ) {
                self.submit_item_world(&state, copy, vertex_light(light), out);
            }
        }
    }
}
