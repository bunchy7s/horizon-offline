//! Llamas on the game side: `AbstractChestedHorse.mobInteract` and
//! `Llama.handleEating` (wheat and hay), chests, carpets, shearing a carpet
//! off and riding.
use super::Game;
use horizon_client::{
    items::ItemStack,
    mobs::{equine, llama},
};
use horizon_core::random::RandomSource;

impl Game<'_> {
    fn llama_sound(&mut self, index: usize, sound: &str, volume: f32) {
        let at = self.mobs[index].center();
        let pitch = (self.world.random.next_float() - self.world.random.next_float()) * 0.2 + 1.0;
        self.audio
            .play(&format!("minecraft:{sound}"), Some(at), volume, pitch);
    }
    /// `Llama.handleEating`: wheat and hay bales heal, age up a baby, raise
    /// the temper and, from a hay bale, put a tame adult in love.
    fn feed_llama(&mut self, index: usize, item: &str) -> bool {
        let (age_up, temper, heal) = match item {
            "minecraft:wheat" => (10, 3, 2.0),
            "minecraft:hay_block" => (90, 6, 10.0),
            _ => return false,
        };
        let m = &mut self.mobs[index];
        let mut used = false;
        let mut loved = false;
        if item == "minecraft:hay_block" && m.extra.tame && m.extra.age == 0 && m.extra.in_love <= 0
        {
            m.extra.in_love = 600;
            m.extra.love_player = true;
            used = true;
            loved = true;
        }
        if m.health < m.attrs.max_health {
            m.health = (m.health + heal).min(m.attrs.max_health);
            used = true;
        }
        let mut happy = false;
        if m.is_baby() && !m.extra.age_locked {
            m.extra.age = (m.extra.age + age_up * 20).min(0);
            used = true;
            happy = true;
        }
        if (used || !m.extra.tame) && i32::from(m.extra.variant3) < llama::MAX_TEMPER {
            m.extra.variant3 = (i32::from(m.extra.variant3) + temper).min(llama::MAX_TEMPER) as u8;
            used = true;
        }
        if happy {
            // AbstractHorse.handleEating: one at rest about the body.
            let id = m.id;
            self.particles_around_mob(
                id,
                "happy_villager",
                horizon_client::particles::Burst::HORSE_EATING,
            );
        }
        if !used {
            return false;
        }
        // Animal.setInLove: entity event 18.
        if loved {
            self.love_cause.insert(self.mobs[index].id, self.lan_acting);
            self.mob_event_particles(self.mobs[index].id, 18);
        }
        self.llama_sound(index, "entity.llama.eat", 1.0);
        self.consume_held();
        true
    }
    /// `AbstractChestedHorse.mobInteract` and `AbstractHorse.mobInteract` for
    /// llamas and trader llamas. Returns whether the click was used.
    pub(super) fn interact_llama(&mut self, index: usize, item: &str) -> bool {
        let m = &self.mobs[index];
        if !m.alive() || Some(m.id) == self.riding {
            return false;
        }
        let sneaking = self.keys.contains(&winit::keyboard::KeyCode::ShiftLeft);
        let (baby, tamed, vehicle) = (m.is_baby(), m.extra.tame, m.rider.is_some());
        let open_inventory = !baby && tamed && sneaking;
        if !vehicle && !open_inventory && !item.is_empty() {
            if self.mobs[index].kind.food.iter().any(|f| f == item) {
                return self.feed_llama(index, item);
            }
            if !tamed {
                let mut events = Vec::new();
                equine::make_mad(&mut self.mobs[index], &mut events);
                let _ = self.handle_mob_events(events);
                return true;
            }
            if !self.mobs[index].extra.chested && item == "minecraft:chest" {
                self.mobs[index].extra.chested = true;
                self.llama_sound(index, "entity.llama.chest", 1.0);
                self.consume_held();
                return true;
            }
        }
        // AbstractHorse.mobInteract.
        if vehicle || baby {
            return false;
        }
        if tamed && sneaking {
            // AbstractHorse.openCustomInventoryScreen.
            self.open_mount_menu(index);
            return true;
        }
        let m = &self.mobs[index];
        let carpet = |i: &str| i.ends_with("_carpet") && i.starts_with("minecraft:");
        if !item.is_empty() {
            if item == "minecraft:shears" && m.extra.head.as_deref().is_some_and(carpet) {
                // Equippable.canBeSheared: the carpet comes off.
                let at = m.center();
                let carpet = self.mobs[index].extra.head.take().unwrap_or_default();
                self.audio
                    .play("minecraft:item.llama_carpet.unequip", Some(at), 1.0, 1.0);
                self.spawn_item(ItemStack::new(&carpet, 1), at, [0.0, 0.2, 0.0], 10);
                self.damage_held_tool();
                return true;
            }
            if carpet(item) && m.extra.head.is_none() {
                self.mobs[index].extra.head = Some(item.into());
                self.llama_sound(index, "entity.llama.swag", 1.0);
                self.consume_held();
                return true;
            }
        }
        // TraderLlama.doPlayerRide: not while a wandering trader leads it.
        if self.is_led_by_trader(index) {
            return true;
        }
        // doPlayerRide; mounting drops a lead.
        let id = self.mobs[index].id;
        self.break_leash(id, true, false);
        let m = &mut self.mobs[index];
        m.extra.flag = false;
        m.extra.counter = 0;
        self.yaw = m.yaw;
        self.riding = Some(m.id);
        self.mount.reset_jump();
        self.trigger_player("minecraft:started_riding");
        true
    }
    fn is_led_by_trader(&self, index: usize) -> bool {
        let m = &self.mobs[index];
        m.species() == horizon_client::mobs::Species::TraderLlama
            && matches!(
                m.extra.leash,
                Some(horizon_client::mobs::leash::Holder::Mob(id))
                    if self.mobs.iter().any(|o| o.id == id
                        && o.species() == horizon_client::mobs::Species::WanderingTrader)
            )
    }
}
