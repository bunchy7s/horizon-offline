//! Allays on the game side: spotting and picking up matching drops,
//! throwing their load to the player or note block, dancing to jukeboxes,
//! and the hand-over interaction (`Allay.mobInteract`).
use super::Game;
use horizon_client::{
    containers::BlockEntity,
    items::ItemStack,
    mobs::{Species, allay},
};
use horizon_core::random::RandomSource;

impl Game<'_> {
    pub(super) fn tick_allays(&mut self) {
        let griefing = self.ui.rules.bool("mob_griefing");
        // The playing jukeboxes, found at the first allay: no allay starts
        // or stops one, and the block entities run into the thousands.
        let mut jukeboxes: Option<Vec<(i32, i32, i32)>> = None;
        for i in 0..self.mobs.len() {
            if self.mobs[i].species() != Species::Allay || !self.mobs[i].alive() {
                continue;
            }
            let pos = self.mobs[i].position;
            // Dancing: a playing jukebox within 10 blocks.
            let jukeboxes = jukeboxes.get_or_insert_with(|| {
                self.world
                    .block_entities
                    .iter()
                    .filter(|(_, e)| matches!(e, BlockEntity::Jukebox(j) if j.playing))
                    .map(|(k, _)| *k)
                    .collect()
            });
            let nearest = jukeboxes
                .iter()
                .map(|k| {
                    (f64::from(k.0) + 0.5 - pos.x).powi(2)
                        + (f64::from(k.1) + 0.5 - pos.y).powi(2)
                        + (f64::from(k.2) + 0.5 - pos.z).powi(2)
                })
                .fold(f64::INFINITY, f64::min);
            // `Allay.setDancing` wants effective AI: a `NoAI` allay never dances.
            let panicking = horizon_client::mobs::goals::should_panic(&self.mobs[i]);
            self.mobs[i].extra.flag = allay::dances(self.mobs[i].no_ai, panicking, nearest);
            let Some(held) = self.mobs[i].extra.main_hand_stack().cloned() else {
                self.mobs[i].extra.point = None;
                continue;
            };
            let carried = self.mobs[i].extra.ints[1];
            let max = self
                .res
                .items()
                .get(&held.item)
                .map_or(64, |i| i.max_stack as i32);
            // `Mob.aiStep`'s looting loop asks for no AI: `canPickUpLoot`, the
            // inventory room in `wantsToPickUp` and mob_griefing are its gates.
            let wants = self.mobs[i].can_pick_up_loot() && griefing && carried < max;
            let reach = allay::pickup_reach(self.mobs[i].bounds());
            // GoToWantedItem: a stack it considers equal within 32 that the
            // allay sees (NearestItemSensor's hasLineOfSight, eye to eye). The
            // sensor is the brain's, so without AI only `Mob.aiStep`'s looting
            // loop runs: it takes an equal drop already in `getPickupReach`.
            let wanted = if !wants {
                None
            } else if self.mobs[i].no_ai {
                self.dropped.iter().position(|e| {
                    e.pickup_delay == 0
                        && allay::considers_item_equal(&held, &e.stack)
                        && reach.contains(e.position)
                })
            } else {
                let w = self.mob_world_base(&[]);
                let eye = horizon_core::geometry::Vec3::new(pos.x, self.mobs[i].eye_y(), pos.z);
                // The sensor looks in the allay's box grown by (32, 16, 32).
                let b = self.mobs[i].bounds();
                let (low, high) = (b.min_position().y - 16.0, b.max_position().y + 16.0);
                self.dropped
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| {
                        e.pickup_delay == 0 && allay::considers_item_equal(&held, &e.stack)
                    })
                    .map(|(j, e)| (j, e.position.distance_squared(pos)))
                    .filter(|(_, d)| *d < 32.0 * 32.0)
                    .filter(|(j, _)| {
                        let y = self.dropped[*j].position.y;
                        y <= high && y + 0.25 >= low
                    })
                    .filter(|(j, _)| {
                        // ItemEntity's eye height is 0.2125.
                        let e = &self.dropped[*j];
                        let at = horizon_core::geometry::Vec3::new(
                            e.position.x,
                            e.position.y + 0.2125,
                            e.position.z,
                        );
                        w.line_of_sight(eye, at)
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(j, _)| j)
            };
            let Some(j) = wanted else {
                self.mobs[i].extra.point = None;
                // `GiveInventoryToLookTargetPlayer` is the brain's too.
                if !self.mobs[i].no_ai {
                    self.allay_deliver(i);
                }
                continue;
            };
            let e = &self.dropped[j];
            if reach.contains(e.position) {
                let take = (max - carried).min(e.stack.count as i32);
                let (item, id) = (e.clone(), self.mobs[i].id);
                // It carries the fetched stack, potion and all, not just its id.
                self.mobs[i].extra.set_main_hand_stack(Some(ItemStack {
                    count: 1,
                    ..e.stack.clone()
                }));
                self.add_pickup(
                    super::pickups::Taken::Item(Box::new(item)),
                    super::pickups::Collector::Mob(id),
                );
                self.mobs[i].extra.ints[1] += take;
                let e = &mut self.dropped[j];
                e.stack.count -= take as u32;
                if e.stack.count == 0 {
                    self.remove_dropped(j);
                }
                let at = [pos.x, pos.y, pos.z];
                let pitch =
                    (self.world.random.next_float() - self.world.random.next_float()) * 1.4 + 2.0;
                self.audio
                    .play("minecraft:entity.item.pickup", Some(at), 0.2, pitch);
                self.mobs[i].extra.point = None;
            } else {
                self.mobs[i].extra.point = Some([e.position.x, e.position.y, e.position.z]);
            }
        }
    }
    /// The UUID of the player an allay likes (`LIKED_PLAYER`).
    fn allay_liked_uuid(&self, i: usize) -> [i32; 4] {
        // An allay saved before its player was kept belongs to the host.
        self.mobs[i]
            .extra
            .owner
            .filter(|u| *u != [0, 0, 0, 1])
            .or(self.level.player_uuid)
            .unwrap_or([0, 0, 0, 1])
    }
    /// `AllayAi.getLikedPlayer`: the player the allay was given an item by,
    /// found by UUID among the host and the guests of the level (the window
    /// is a guest on the server thread), when they are not a spectator and
    /// within 64 blocks.
    fn allay_liked_player(&self, i: usize) -> Option<horizon_core::geometry::Vec3> {
        let m = &self.mobs[i];
        if !m.extra.tame {
            return None;
        }
        let uuid = self.allay_liked_uuid(i);
        let host = self
            .player_view()
            .filter(|p| !p.spectator && self.level.player_uuid.unwrap_or([0, 0, 0, 1]) == uuid)
            .map(|p| p.position);
        let found = host.or_else(|| {
            self.lan_present()
                .into_iter()
                .find(|p| p.uuid == uuid && !p.spectator)
                .map(|p| {
                    horizon_core::geometry::Vec3::new(p.position[0], p.position[1], p.position[2])
                })
        });
        found.filter(|p| p.distance_squared(m.position) < 64.0 * 64.0)
    }
    /// `GoAndGiveItemsToTarget` / `AllayAi.throwItem`: within 3 blocks of
    /// the deposit, one item a tick, each restarting the three-second pause
    /// before it picks anything up again.
    fn allay_deliver(&mut self, i: usize) {
        let m = &self.mobs[i];
        if m.extra.ints[1] <= 0 {
            return;
        }
        // AllayAi.getItemDepositPosition: the liked note block, else the player.
        let dimension = self.dim.id();
        let noteblock = allay::noteblock_deposit(&mut self.mobs[i], dimension, |p| {
            &*self
                .res
                .facts()
                .get(self.world.block(p[0], p[1], p[2]))
                .name
                == "minecraft:note_block"
        });
        let player = self.allay_liked_player(i);
        let m = &self.mobs[i];
        let target = match noteblock {
            Some(p) => horizon_core::geometry::Vec3::new(p[0], p[1], p[2]),
            None => match player.filter(|_| m.extra.tame) {
                Some(p) => p,
                None => return,
            },
        };
        let eye = horizon_core::geometry::Vec3::new(m.position.x, m.eye_y(), m.position.z);
        if target.distance_squared(eye) >= 9.0 {
            return;
        }
        let Some(item) = m.extra.main_hand_stack().cloned() else {
            return;
        };
        let from = [
            m.position.x,
            m.position.y + m.eye_height() - 0.3,
            m.position.z,
        ];
        let d = [
            target.x - from[0],
            target.y + 1.0 - from[1],
            target.z - from[2],
        ];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
        let v = [d[0] / len * 0.2, d[1] / len * 0.3, d[2] / len * 0.2];
        // AllayAi.throwItem: the liked player hears of the block below the deposit.
        let below = [
            target.x.floor() as i32,
            (target.y - 1.0).floor() as i32,
            target.z.floor() as i32,
        ];
        let uuid = self.allay_liked_uuid(i);
        // AllayAi.throwItem: one item of the held stack, its components and all.
        let stack = ItemStack { count: 1, ..item };
        self.for_each_player(|g| {
            if g.acting_uuid() == uuid && !g.spectator() {
                g.trigger_block("minecraft:allay_drop_item_on_block", below, &stack);
            }
        });
        let m = &mut self.mobs[i];
        m.extra.ints[1] -= 1;
        m.extra.ints[3] = 60;
        // ItemEntity.setDefaultPickUpDelay.
        self.spawn_item(stack, from, v, 10);
        // BehaviorUtils.throwItem: ItemEntity.setThrower(the allay).
        let allay = self.mob_uuid(self.mobs[i].id);
        if let Some(thrown) = self.just_spawned_item() {
            thrown.thrower = Some(allay);
        }
        if self.world.game_time % 7 == 0 && self.world.random.next_double() < 0.9 {
            const PITCHES: [f32; 16] = [
                0.5625, 0.625, 0.75, 0.9375, 1.0, 1.0, 1.125, 1.25, 1.5, 1.875, 2.0, 2.25, 2.5,
                3.0, 3.75, 4.0,
            ];
            let pitch = PITCHES[self.world.random.next_int_bound(16).unwrap_or(4) as usize];
            self.audio
                .play("minecraft:entity.allay.item_thrown", Some(from), 1.0, pitch);
        }
    }
    /// `Allay.mobInteract`: duplicate when dancing with an amethyst shard,
    /// take the held item, or hand back what it holds.
    pub(super) fn interact_allay(&mut self, index: usize, item: Option<&ItemStack>) -> bool {
        let m = &self.mobs[index];
        if m.species() != Species::Allay || !m.alive() {
            return false;
        }
        let at = [m.position.x, m.position.y, m.position.z];
        let shard = item.is_some_and(|s| &*s.item == "minecraft:amethyst_shard");
        if m.extra.flag && shard && m.extra.egg_time == 0 {
            let pos = at;
            self.mobs[index].extra.egg_time = 6000;
            if let Some(id) = self.spawn_mob("minecraft:allay", pos, true)
                && let Some(c) = self.mobs.iter_mut().find(|m| m.id == id)
            {
                c.extra.egg_time = 6000;
            }
            // Allay.duplicateAllay: entity event 18, three spawnHeartParticle.
            let id = self.mobs[index].id;
            self.mob_event_particles(id, 18);
            self.audio
                .play("minecraft:block.amethyst_block.chime", Some(at), 2.0, 1.0);
            self.consume_held();
            return true;
        }
        let acting = self.acting_uuid();
        let holds = m.extra.main_hand.is_some();
        if !holds && let Some(mut given) = item.cloned() {
            let m = &mut self.mobs[index];
            // `Allay.mobInteract` hands over one item, its components and all.
            given.count = 1;
            m.extra.set_main_hand_stack(Some(given));
            m.extra.tame = true;
            // `Allay.mobInteract` remembers LIKED_PLAYER by UUID.
            m.extra.owner = Some(acting);
            self.audio
                .play("minecraft:entity.allay.item_given", Some(at), 2.0, 1.0);
            self.consume_held();
            return true;
        }
        if holds && item.is_none() {
            let m = &mut self.mobs[index];
            // The hand's stack keeps the components (a potion) the items came with.
            let held = m
                .extra
                .main_hand_stack()
                .cloned()
                .unwrap_or_else(|| ItemStack::new("minecraft:air", 1));
            m.extra.set_main_hand(None);
            let carried = std::mem::take(&mut m.extra.ints[1]);
            m.extra.tame = false;
            m.extra.owner = None;
            self.audio
                .play("minecraft:entity.allay.item_taken", Some(at), 2.0, 1.0);
            if carried > 0 {
                self.spawn_item(
                    ItemStack {
                        count: carried as u32,
                        ..held.clone()
                    },
                    at,
                    [0.0, 0.2, 0.0],
                    40,
                );
            }
            if let Some(rest) = self.give(ItemStack { count: 1, ..held })
                && let Some(eye) = self.eye_position()
            {
                self.spawn_item(rest, eye, [0.0; 3], 0);
            }
            return true;
        }
        false
    }
}
