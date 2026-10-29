//! Landing impacts for falling blocks, before their placement/drop decision.
use super::{FallingBlock, Game, Landing};
use horizon_client::{falling_block, items::ItemStack};
use horizon_core::{geometry::Vec3, position::BlockPos, random::RandomSource};
use horizon_world::level::LevelEvent;

impl Game<'_> {
    pub(super) fn finish_falling_block(&mut self, mut block: FallingBlock, pos: [i32; 3]) {
        let name = self.res.facts().get(block.state).name.clone();
        let anvil = name.ends_with("anvil");
        let stalactite = matches!(
            &*name,
            "minecraft:pointed_dripstone" | "minecraft:sulfur_spike"
        );
        let sound_event = match &*name {
            "minecraft:pointed_dripstone" => Some(1045),
            "minecraft:sulfur_spike" => Some(1052),
            _ => None,
        };
        if let Some(impact) = block.damage.impact(block.fall_distance) {
            if impact.amount > 0.0 {
                let mob_kind = if stalactite {
                    horizon_client::mobs::DamageKind::FallingStalactite
                } else {
                    horizon_client::mobs::DamageKind::FallingBlock
                };
                falling_block::hurt_mobs_with_kind(
                    &mut self.mobs,
                    block.position,
                    impact.amount,
                    mob_kind,
                );
                let kind = if stalactite {
                    "minecraft:falling_stalactite"
                } else if anvil {
                    "minecraft:falling_anvil"
                } else {
                    "minecraft:falling_block"
                };
                self.falling_hurt_player(block.position, impact.amount, kind);
                if let Some(mut host) = self.lan.take() {
                    let bounds = falling_block::bounds(block.position);
                    let conns = host
                        .hub
                        .guests
                        .iter()
                        .filter(|g| {
                            host.hub.here(g)
                                && g.loaded
                                && !g.dead
                                && !g.creative()
                                && !g.spectator()
                                && bounds.intersects(super::lan_mobs::guest_bounds(g))
                        })
                        .map(|g| g.conn)
                        .collect::<Vec<_>>();
                    for conn in conns {
                        self.guest_hit_by(&mut host, conn, |g| {
                            g.falling_hurt_player(block.position, impact.amount, kind)
                        });
                    }
                    self.lan = Some(host);
                }
            }
            if anvil
                && impact.wear_chance > 0.0
                && self.world.random.next_float() < impact.wear_chance
                && let Some(mut state) = self.pack.assets.registry.state(block.state).cloned()
            {
                if falling_block::wear_anvil(&mut state) {
                    block.state = self
                        .pack
                        .assets
                        .registry
                        .resolve(&state)
                        .unwrap_or(block.state);
                } else {
                    block.damage.cancel_drop = true;
                }
            }
        }
        if block.damage.cancel_drop {
            if let Some(id) = sound_event {
                self.level_events.push(LevelEvent::LevelEvent {
                    id,
                    pos: BlockPos::new(pos[0], pos[1], pos[2]),
                    data: 0,
                });
            } else if anvil {
                self.level_events.push(LevelEvent::LevelEvent {
                    id: 1029,
                    pos: BlockPos::new(pos[0], pos[1], pos[2]),
                    data: 0,
                });
            } else if self
                .res
                .facts()
                .get(block.state)
                .name
                .starts_with("minecraft:suspicious_")
            {
                // BrushableBlock.onBrokenAfterFall.
                let at = BlockPos::new(pos[0], pos[1], pos[2]);
                self.level_events.push(LevelEvent::LevelEvent {
                    id: 2001,
                    pos: at,
                    data: block.state.0 as i32,
                });
                self.level_events.push(LevelEvent::GameEvent {
                    pos: at,
                    event: "minecraft:block_destroy",
                    state: None,
                });
            }
            return;
        }
        let landing = self.with_level(|b, level, ticks| {
            b.falling_block_landed(
                level,
                ticks,
                BlockPos::new(pos[0], pos[1], pos[2]),
                block.state,
                false,
            )
        });
        if matches!(landing, Landing::Broken)
            && block.drop_item
            && self.ui.rules.bool("entity_drops")
        {
            if let Some(id) = sound_event {
                self.level_events.push(LevelEvent::LevelEvent {
                    id,
                    pos: BlockPos::new(pos[0], pos[1], pos[2]),
                    data: 0,
                });
            }
            let item = self.res.facts().get(block.state).item.clone();
            if &*item != "minecraft:air" {
                self.pop_resource(
                    pos,
                    ItemStack {
                        item,
                        count: 1,
                        damage: 0,
                        extra: None,
                    },
                );
            }
        }
    }
    fn falling_hurt_player(&mut self, position: Vec3, amount: f32, kind: &str) {
        if self.creative || self.spectator() {
            return;
        }
        if self.player.as_ref().is_none_or(|p| {
            p.health.dead_or_dying() || !falling_block::bounds(position).intersects(p.bounds())
        }) {
            return;
        }
        let amount = if self.armor[0].is_some() {
            amount * 0.75
        } else {
            amount
        };
        if let Some(p) = self.player.as_mut()
            && let Some(id) = self.motion.health.damage.id(kind)
        {
            let _ = p.hurt(&self.motion.health, self.world.health_rules, id, amount);
        }
    }
}
