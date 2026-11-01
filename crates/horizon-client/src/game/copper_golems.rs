//! Copper golems on the game side: what their brain (`mobs::transport_items`)
//! does to chests (opener counts, the lid block events, slots), the openers
//! recheck, turning into statues, and the hand-over, waxing, scraping and
//! shearing interactions.
use super::Game;
use horizon_client::{
    items::ItemStack,
    mobs::{
        Mob, Species,
        transport_items::{held, set_held},
    },
};
use horizon_core::random::RandomSource;
use horizon_world::chunk::BlockState;

/// `ContainerOpenersCounter.CHECK_TICK_DELAY`.
const RECHECK_TICKS: i64 = 5;

impl Game<'_> {
    /// A copper golem opened or closed a chest half: the counter changes and
    /// the clients see the block event (`openerCountChanged`).
    pub(super) fn golem_chest_open(&mut self, chest: [i32; 3], open: bool) {
        let key = (chest[0], chest[1], chest[2]);
        let entry = self.world.chest_openers.entry(key).or_insert((0, 0.0));
        if open {
            entry.0 += 1;
            entry.1 = entry.1.max(3.0);
        } else {
            entry.0 -= 1;
            if entry.0 == 0 {
                entry.1 = 0.0;
            }
        }
        let count = entry.0;
        if count <= 0 {
            self.world.chest_openers.remove(&key);
        }
        self.block_entity_event(chest, 1, count.max(0));
        // The guests get the same `ClientboundBlockEventPacket` (the lid).
        self.lan_block_event(chest, 1, count.max(0));
    }
    /// A copper golem took from or filled a chest slot.
    pub(super) fn golem_chest_slot(
        &mut self,
        chest: [i32; 3],
        slot: usize,
        stack: Option<ItemStack>,
    ) {
        let key = (chest[0], chest[1], chest[2]);
        if let Some(e) = self.world.block_entities.get_mut(&key)
            && let Some(s) = e.slots_mut().get_mut(slot)
        {
            *s = stack;
            // TransportItemsBetweenContainers calls Container.setChanged.
            self.container_changed(key);
        }
    }
    /// `ContainerOpenersCounter.recheckOpeners` every five ticks: the golems
    /// whose opened chest is this one, within its range plus 4; when none
    /// is left the chest closes (a double chest the golem left open).
    pub(super) fn recheck_golem_openers(&mut self) {
        if self.world.game_time % RECHECK_TICKS != 0 {
            return;
        }
        let open: Vec<(i32, i32, i32)> = self.world.chest_openers.keys().copied().collect();
        for key in open {
            let p = [key.0, key.1, key.2];
            let other = self.chest_other_half(key).map(|o| [o.0, o.1, o.2]);
            let range = self.world.chest_openers.get(&key).map_or(0.0, |o| o.1) + 4.0;
            let area = horizon_core::geometry::Aabb::new(
                horizon_core::geometry::Vec3::new(
                    f64::from(p[0]) - range,
                    f64::from(p[1]) - range,
                    f64::from(p[2]) - range,
                ),
                horizon_core::geometry::Vec3::new(
                    f64::from(p[0]) + 1.0 + range,
                    f64::from(p[1]) + 1.0 + range,
                    f64::from(p[2]) + 1.0 + range,
                ),
            );
            let count = self
                .mobs
                .iter()
                .filter(|m| {
                    m.species() == Species::CopperGolem
                        && !m.removed
                        && m.extra
                            .opened_chest
                            .is_some_and(|o| o == p || Some(o) == other)
                        && m.bounds().intersects(area)
                })
                .count() as i32;
            let before = self.world.chest_openers.get(&key).map_or(0, |o| o.0);
            if count == before {
                continue;
            }
            if count == 0 {
                self.world.chest_openers.remove(&key);
                self.golem_chest_sound(p, false);
            } else if let Some(o) = self.world.chest_openers.get_mut(&key) {
                o.0 = count;
            }
            self.block_entity_event(p, 1, count);
            self.lan_block_event(p, 1, count);
        }
    }
    /// `ChestBlockEntity.playSound` for a chest the recheck closed.
    fn golem_chest_sound(&mut self, p: [i32; 3], open: bool) {
        let name = self
            .res
            .facts()
            .get(self.world.block(p[0], p[1], p[2]))
            .name
            .clone();
        let base = name
            .trim_start_matches("minecraft:")
            .trim_start_matches("waxed_");
        let kind = if !base.ends_with("copper_chest") {
            "chest"
        } else if base.starts_with("weathered") {
            "copper_chest_weathered"
        } else if base.starts_with("oxidized") {
            "copper_chest_oxidized"
        } else {
            "copper_chest"
        };
        let side = if open { "open" } else { "close" };
        let at = [
            f64::from(p[0]) + 0.5,
            f64::from(p[1]) + 0.5,
            f64::from(p[2]) + 0.5,
        ];
        let pitch = self.world.random.next_float() * 0.1 + 0.9;
        self.audio.play(
            &format!("minecraft:block.{kind}.{side}"),
            Some(at),
            0.5,
            pitch,
        );
    }
    /// `CopperGolem.turnToStatue`, which its weathering asked for.
    pub(super) fn golem_become_statue(&mut self, id: u32) {
        let Some(i) = self.mobs.iter().position(|m| m.id == id) else {
            return;
        };
        let mut m = self.mobs[i].clone();
        self.golem_turn_to_statue(&mut m);
        if let Some(i) = self.mobs.iter().position(|x| x.id == id) {
            self.mobs[i] = m;
        }
    }
    fn golem_turn_to_statue(&mut self, m: &mut Mob) {
        let pos = [
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        ];
        // The pose comes from the golem's own random (`this.random.nextInt(0, 4)`).
        let pose = ["standing", "sitting", "running", "star"]
            [m.random.next_int_bound(4).unwrap_or(0) as usize];
        // Direction.fromYRot.
        let facing = ["south", "west", "north", "east"]
            [((f64::from(m.yaw) / 90.0 + 0.5).floor() as i32 & 3) as usize];
        let state = BlockState {
            name: "minecraft:oxidized_copper_golem_statue".into(),
            properties: [
                ("facing".to_string(), facing.to_string()),
                ("copper_golem_pose".to_string(), pose.to_string()),
                ("waterlogged".to_string(), "false".to_string()),
            ]
            .into(),
        };
        let registry = self.pack.assets.registry;
        let Some(id) = registry.id(&state) else {
            return;
        };
        if self.level_set(pos, id).is_err() {
            return;
        }
        self.golem_drop_equipment(m);
        m.extra.opened_chest = None;
        if m.extra.leash.is_some() {
            self.break_leash(m.id, true, false);
            m.extra.leash = None;
        }
        m.removed = true;
        self.audio.play(
            "minecraft:entity.copper_golem_become_statue",
            Some([m.position.x, m.position.y, m.position.z]),
            1.0,
            1.0,
        );
    }
    /// `Mob.dropPreservedEquipment`: the guaranteed drops in `EquipmentSlot`
    /// order, the carried stack (main hand) then the antenna flower (saddle),
    /// each `spawnAtLocation` at the golem's feet.
    pub(super) fn golem_drop_equipment(&mut self, m: &mut Mob) {
        let at = m.position;
        if let Some(stack) = held(m).cloned() {
            set_held(m, None);
            self.spawn_at_location(at, 0.0, stack);
        }
        if let Some(antenna) = m.saddle_item() {
            m.set_saddle(None);
            self.spawn_at_location(at, 0.0, antenna);
        }
    }
    // ---- interactions -----------------------------------------------------
    /// `WeatheringCopperGolemStatueBlock.useItemOn`: an axe on an unoxidized
    /// statue turns it back into a golem (`CopperGolemStatueBlockEntity.removeStatue`),
    /// wearing the axe. Weathered statues are scraped by the axe's own
    /// transform instead.
    pub(super) fn use_golem_statue(&mut self, pos: [i32; 3], held: Option<&ItemStack>) -> bool {
        if !held.is_some_and(|s| s.item.ends_with("_axe")) {
            return false;
        }
        let registry = self.pack.assets.registry;
        let Some(state) = registry.state(self.world.block(pos[0], pos[1], pos[2])) else {
            return false;
        };
        if state.name != "minecraft:copper_golem_statue" {
            return false;
        }
        // `Direction.toYRot` of the statue's facing.
        let yaw = match state.properties.get("facing").map(String::as_str) {
            Some("west") => 90.0,
            Some("north") => 180.0,
            Some("east") => 270.0,
            _ => 0.0,
        };
        let waterlogged = state
            .properties
            .get("waterlogged")
            .is_some_and(|w| w == "true");
        // initCopperGolem: at the block's feet, facing as the statue does.
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]),
            f64::from(pos[2]) + 0.5,
        ];
        let Some(id) = self.spawn_mob("minecraft:copper_golem", at, true) else {
            return false;
        };
        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
            m.yaw = yaw;
            m.previous_yaw = yaw;
            m.body_yaw = yaw;
            m.previous_body_yaw = yaw;
            m.head_yaw = yaw;
            m.previous_head_yaw = yaw;
        }
        self.damage_held(1);
        self.audio
            .play("minecraft:entity.copper_golem.spawn", Some(at), 1.0, 1.0);
        // Level.removeBlock(pos, false): air, or the water the statue stood in.
        let replacement = registry.default_state(if waterlogged {
            "minecraft:water"
        } else {
            "minecraft:air"
        });
        if let Some(replacement) = replacement {
            let _ = self.level_set(pos, replacement);
        }
        self.world.block_entities.remove(&(pos[0], pos[1], pos[2]));
        true
    }
    /// `CopperGolem.mobInteract`; true when handled.
    pub(super) fn interact_copper_golem(&mut self, index: usize, item: &str) -> bool {
        if self.mobs[index].species() != Species::CopperGolem || !self.mobs[index].alive() {
            return false;
        }
        let at = {
            let m = &self.mobs[index];
            [m.position.x, m.position.y, m.position.z]
        };
        if item.is_empty() {
            // Empty hand: the golem throws what it carries back.
            let Some(h) = held(&self.mobs[index]).cloned() else {
                return false;
            };
            set_held(&mut self.mobs[index], None);
            // BehaviorUtils.throwItem: from 0.3 below the golem's eyes towards
            // the player's feet at 0.3 blocks a tick, default pickup delay.
            let p = self.player.as_ref().map(|p| p.position);
            let eye = self.mobs[index].eye_height();
            let from = [at[0], at[1] + eye - f64::from(0.3f32), at[2]];
            let v = p.map_or([0.0; 3], |p| {
                let d = [p.x - at[0], p.y - at[1], p.z - at[2]];
                let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                if len < 1.0e-5 {
                    return [0.0; 3];
                }
                let k = f64::from(0.3f32);
                [d[0] / len * k, d[1] / len * k, d[2] / len * k]
            });
            self.spawn_item(h, from, v, 10);
            return true;
        }
        let name = item;
        let is_axe = name.ends_with("_axe");
        let waxed = self.mobs[index].extra.vector[0] as i64 == -2;
        if name == "minecraft:shears" && self.golem_shearable(index) {
            // CopperGolem.shear: the sound, the antenna slot emptied (an
            // unequip) and the flower dropped 1.5 up.
            self.audio
                .play("minecraft:entity.copper_golem.shear", Some(at), 1.0, 1.0);
            let antenna = self.mobs[index].saddle_item();
            self.mobs[index].set_saddle(None);
            let id = self.mobs[index].id;
            let ctx = self.who_context(Some(horizon_client::mobs::Who::Mob(id)), None);
            self.post_game_event("minecraft:unequip", at, ctx);
            if let Some(a) = antenna {
                let p = self.mobs[index].position;
                self.spawn_at_location(p, 1.5, a);
            }
            self.damage_held_tool();
            return true;
        }
        let block = {
            let p = self.mobs[index].position;
            [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32]
        };
        if name == "minecraft:honeycomb" && !waxed {
            self.mobs[index].extra.vector[0] = -2.0;
            self.level_event_at(3003, block, 0);
            self.audio
                .play("minecraft:item.honeycomb.wax_on", Some(at), 1.0, 1.0);
            self.consume_held();
            return true;
        }
        if is_axe && waxed {
            self.mobs[index].extra.vector[0] = -1.0;
            self.audio
                .play("minecraft:item.axe.scrape", Some(at), 1.0, 1.0);
            self.level_event_at(3004, block, 0);
            self.damage_held_tool();
            return true;
        }
        if is_axe && self.mobs[index].extra.variant > 0 {
            let m = &mut self.mobs[index];
            m.extra.vector[0] = -1.0;
            m.extra.variant = horizon_client::mobs::copper_golem::weather_previous(m.extra.variant);
            self.audio
                .play("minecraft:item.axe.scrape", Some(at), 1.0, 1.0);
            self.level_event_at(3005, block, 0);
            self.damage_held_tool();
            return true;
        }
        false
    }
    /// `readyForShearing`: the antenna holds a flower shears take.
    pub(super) fn golem_shearable(&self, index: usize) -> bool {
        self.mobs[index].saddle_item().is_some_and(|a| {
            self.res
                .items()
                .item_tags
                .get("minecraft:shearable_from_copper_golem")
                .is_some_and(|t| t.contains(&*a.item))
        })
    }
}
