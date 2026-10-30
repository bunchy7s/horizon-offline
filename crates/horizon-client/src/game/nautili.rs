//! Nautili on the game side (`AbstractNautilus.mobInteract`): taming with
//! pufferfish, feeding, the saddle and armour, riding, and the zombie
//! nautilus a drowned with a trident sometimes rides.
use super::Game;
use horizon_client::{
    items::ItemStack,
    mobs::{Species, nautilus},
};
use horizon_core::random::RandomSource;

impl Game<'_> {
    fn item_tagged(&self, tag: &str, item: &str) -> bool {
        self.res
            .items()
            .item_tags
            .get(tag)
            .is_some_and(|t| t.contains(item))
    }
    /// `AbstractNautilus.usePlayerItem`: bucketed fish leave a water bucket.
    fn use_nautilus_food(&mut self, item: &str) {
        let bucket = self.item_tagged("minecraft:nautilus_bucket_food", item);
        self.consume_held();
        if bucket && !self.creative {
            let water = ItemStack::new("minecraft:water_bucket", 1);
            if self.inventory[self.selected].is_none() {
                self.inventory[self.selected] = Some(water);
            } else if let Some(rest) = self.give(water) {
                let at = self.eye_position().unwrap_or([0.0; 3]);
                self.spawn_item(rest, at, [0.0; 3], 40);
            }
        }
    }
    fn nautilus_sound(&mut self, index: usize, sound: &str) {
        let at = self.mobs[index].center();
        self.audio
            .play(&format!("minecraft:{sound}"), Some(at), 1.0, 1.0);
    }
    /// `AbstractNautilus.mobInteract`. Returns whether the click was used.
    pub(super) fn interact_nautilus(&mut self, index: usize, item: &str) -> bool {
        let m = &self.mobs[index];
        if !m.alive() || Some(m.id) == self.riding {
            return false;
        }
        let taming: Vec<String> = self
            .res
            .items()
            .item_tags
            .get("minecraft:nautilus_taming_items")
            .map(|t| t.iter().cloned().collect())
            .unwrap_or_default();
        let food = nautilus::is_food(m, item, &taming);
        let sneaking = self.keys.contains(&winit::keyboard::KeyCode::ShiftLeft)
            || self.keys.contains(&winit::keyboard::KeyCode::ShiftRight);
        let tame = m.extra.tame;
        if m.is_baby() {
            // Animal.mobInteract: food grows a baby up by a tenth.
            if !food {
                return false;
            }
            let m = &mut self.mobs[index];
            if !m.extra.age_locked {
                let secs = ((-m.extra.age / 20) as f32 * 0.1) as i32;
                horizon_client::mobs::ai::age_up(m, secs, true);
            }
            let sound = nautilus::eat_sound(m);
            self.use_nautilus_food(item);
            self.nautilus_sound(index, sound);
            return true;
        }
        // AbstractNautilus.mobInteract: shift + use on a tame adult opens its
        // saddle and armour screen (openCustomInventoryScreen).
        if tame && sneaking {
            self.open_mount_menu(index);
            return true;
        }
        if !item.is_empty() {
            if !tame && food {
                // tryToTame: one time in three.
                self.use_nautilus_food(item);
                let m = &mut self.mobs[index];
                let tamed = m.random.next_int_bound(3).unwrap_or(1) == 0;
                if tamed {
                    m.extra.tame = true;
                    m.extra.owner = self.level.player_uuid.or(Some([0, 0, 0, 1]));
                    m.persistent = true;
                    m.nav_stop();
                    m.ai.target = None;
                }
                let sound = nautilus::eat_sound(m);
                // TamableAnimal.spawnTamingParticles (entity event 7 or 6).
                self.mob_event_particles(self.mobs[index].id, if tamed { 7 } else { 6 });
                self.nautilus_sound(index, sound);
                if tamed {
                    let view = self.mob_entity(&self.mobs[index]);
                    self.trigger_entity("minecraft:tame_animal", "entity", &view);
                }
                return true;
            }
            let m = &self.mobs[index];
            if food && m.health < m.attrs.max_health {
                // feed(player, hand, stack, 2, 1).
                let sound = nautilus::eat_sound(m);
                let m = &mut self.mobs[index];
                m.health = (m.health + 2.0).min(m.attrs.max_health);
                self.use_nautilus_food(item);
                self.nautilus_sound(index, sound);
                return true;
            }
            // Equippable.equipOnInteract: a saddle or nautilus armour on a
            // tame adult.
            let m = &mut self.mobs[index];
            if item == "minecraft:saddle" && tame && !m.extra.saddled {
                let held = self.inventory[self.selected].clone();
                m.set_saddle(held);
                let sound = nautilus::saddle_sound(m);
                self.nautilus_sound(index, sound);
                self.consume_held();
                return true;
            }
            if item.ends_with("_nautilus_armor") && tame && m.extra.head.is_none() {
                m.extra.head = Some(item.into());
                self.nautilus_sound(index, "item.armor.equip_nautilus");
                self.consume_held();
                return true;
            }
        }
        if tame && !food {
            // doPlayerRide.
            let id = self.mobs[index].id;
            self.break_leash(id, true, false);
            self.yaw = self.mobs[index].yaw;
            self.riding = Some(id);
            self.mount.reset_jump();
            self.trigger_player("minecraft:started_riding");
            return true;
        }
        // Animal.mobInteract: a tame adult fed falls in love (not a zombie).
        let m = &mut self.mobs[index];
        if food && m.species() == Species::Nautilus && m.extra.age == 0 && m.extra.in_love <= 0 {
            m.extra.in_love = 600;
            m.extra.love_player = true;
            let sound = nautilus::eat_sound(m);
            let id = m.id;
            self.love_cause.insert(id, self.lan_acting);
            self.use_nautilus_food(item);
            self.nautilus_sound(index, sound);
            // Animal.setInLove: entity event 18.
            self.mob_event_particles(self.mobs[index].id, 18);
            return true;
        }
        false
    }
}
