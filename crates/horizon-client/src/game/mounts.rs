//! Riding horses, donkeys and mules: feeding (`handleEating`), saddles,
//! armour and chests, mounting, the rider's control and jump bar
//! (`LocalPlayer.aiStep`'s riding jump), positioning, dismounting, and the
//! mount's health and jump bar on the HUD.
use super::Game;
use horizon_client::{
    hud::HudBatch,
    items::ItemStack,
    mobs::{MobEvent, Species, equine},
    sprint::Keys,
};
use horizon_core::{geometry::Vec3, random::RandomSource};

/// `passengerAttachments` minus the player's 0.6 vehicle attachment.
fn seat(s: Species) -> f64 {
    let attach = match s {
        Species::Pig => 0.86875,
        Species::Strider => 1.7,
        Species::Horse => 1.44375,
        Species::Llama | Species::TraderLlama => 1.37,
        Species::Donkey => 1.1125,
        Species::Mule => 1.2125,
        Species::Nautilus | Species::ZombieNautilus => 1.1375,
        _ => 1.31875,
    };
    attach - 0.6
}
#[derive(Default)]
pub(super) struct MountState {
    /// `jumpRidingTicks`: -10 cooling down after a jump, >= 0 charging.
    pub(super) jump_ticks: i32,
    /// `jumpRidingScale` (0..1).
    pub(super) jump_scale: f32,
    was_jumping: bool,
}
impl MountState {
    pub(super) fn reset_jump(&mut self) {
        self.jump_ticks = 0;
        self.jump_scale = 0.0;
    }
}
impl Game<'_> {
    fn mount_index(&self) -> Option<usize> {
        let id = self.riding?;
        self.mobs.iter().position(|m| m.id == id)
    }
    /// `Pig` / `Strider.mobInteract`: saddle an adult, or ride a saddled one.
    fn interact_steerable(&mut self, index: usize, item: &str) -> bool {
        let held = self.inventory[self.selected].clone();
        let m = &mut self.mobs[index];
        if m.is_baby() || !m.alive() || Some(m.id) == self.riding {
            return false;
        }
        let at = m.center();
        if item == "minecraft:saddle" && !m.extra.saddled {
            m.set_saddle(held);
            let sound = if m.species() == Species::Strider {
                "minecraft:entity.strider.saddle"
            } else {
                "minecraft:entity.pig.saddle"
            };
            self.audio.play(sound, Some(at), 0.5, 1.0);
            self.consume_held();
            self.world
                .modified
                .insert((at[0].floor() as i32 >> 4, at[2].floor() as i32 >> 4));
            return true;
        }
        if !m.extra.saddled || self.keys.contains(&winit::keyboard::KeyCode::ShiftLeft) {
            return false;
        }
        self.riding = Some(m.id);
        self.trigger_player("minecraft:started_riding");
        true
    }
    /// `FoodOnAStickItem.use` while riding: boost the mount, wear the stick.
    pub(super) fn use_steering_stick(&mut self, item: &str) -> bool {
        let (species, wear) = match item {
            "minecraft:carrot_on_a_stick" => (Species::Pig, 7),
            "minecraft:warped_fungus_on_a_stick" => (Species::Strider, 1),
            _ => return false,
        };
        let Some(i) = self.mount_index() else {
            return false;
        };
        if self.mobs[i].species() != species || !equine::boost(&mut self.mobs[i]) {
            return false;
        }
        if !self.creative
            && let Some(stack) = self.inventory[self.selected].clone()
        {
            self.trigger_item_durability(&stack);
            if let Some(stack) = self.inventory[self.selected].as_mut() {
                stack.damage += wear;
                let max = self
                    .res
                    .items()
                    .get(&stack.item)
                    .map_or(25, |i| i.max_damage);
                if stack.damage >= max {
                    self.inventory[self.selected] =
                        Some(ItemStack::new("minecraft:fishing_rod", 1));
                }
            }
        }
        self.swing_arm();
        true
    }
    /// `AbstractHorse.mobInteract`. Returns whether the click was used.
    pub(super) fn interact_mount(&mut self, index: usize, item: &str) -> bool {
        let held = self.held().cloned();
        let m = &self.mobs[index];
        let species = m.species();
        // A server player rides it already (`isVehicle`: no seat, no feeding).
        if self.ridden_by_guest(m.id) {
            return false;
        }
        if matches!(species, Species::Pig | Species::Strider) {
            return self.interact_steerable(index, item);
        }
        if horizon_client::mobs::llama::is_llama(species) {
            return self.interact_llama(index, item);
        }
        if species == Species::HappyGhast {
            return self.interact_happy_ghast(index, item);
        }
        if horizon_client::mobs::nautilus::is_nautilus(species) {
            return self.interact_nautilus(index, item);
        }
        if !equine::is_equine(species) || !m.alive() || Some(m.id) == self.riding {
            return false;
        }
        // AbstractHorse.mobInteract: a horse a mob rides (`isVehicle`) has no
        // seat, inventory or feeding to give. Camels seat two and check their
        // passengers themselves.
        let id = m.id;
        if !matches!(species, Species::Camel | Species::CamelHusk)
            && self
                .mobs
                .iter()
                .any(|p| p.extra.mount == Some(id) && !p.removed && p.alive())
        {
            return false;
        }
        // SkeletonHorse.mobInteract: nothing for an untamed one.
        if species == Species::SkeletonHorse && !m.extra.tame {
            return false;
        }
        let sneaking = self.keys.contains(&winit::keyboard::KeyCode::ShiftLeft)
            || self.keys.contains(&winit::keyboard::KeyCode::ShiftRight);
        // Horse / ZombieHorse.mobInteract: food is fed, and anything else in
        // hand makes an untamed one mad instead of seating the player.
        let dandelion = [self.held(), self.offhand.as_ref()]
            .into_iter()
            .flatten()
            .any(|s| &*s.item == "minecraft:golden_dandelion");
        let mut events = Vec::new();
        match equine::held_item(
            &mut self.mobs[index],
            item,
            sneaking,
            dandelion,
            &mut events,
        ) {
            Some(equine::HeldItem::Feed) => return self.feed_mount(index, item),
            Some(equine::HeldItem::Refused) => {
                let _ = self.handle_mob_events(events);
                return true;
            }
            None => {}
        }
        let m = &self.mobs[index];
        // AbstractHorse.mobInteract: shift + use opens a tamed adult's inventory.
        if !m.is_baby() && m.extra.tame && sneaking {
            self.open_mount_menu(index);
            return true;
        }
        if self.feed_mount(index, item) {
            return true;
        }
        let m = &mut self.mobs[index];
        if m.is_baby() {
            return false;
        }
        let at = m.center();
        let tamed = m.extra.tame;
        let sound = if item == "minecraft:saddle" && tamed && !m.extra.saddled {
            m.set_saddle(held);
            // Camel.getSaddleSound (the equip sound of a camel's saddle).
            Some(match species {
                Species::Camel => "minecraft:entity.camel.saddle",
                Species::CamelHusk => "minecraft:entity.camel_husk.saddle",
                _ => "minecraft:entity.horse.saddle",
            })
        } else if item.ends_with("_horse_armor")
            && species == Species::Horse
            && tamed
            && m.extra.head.is_none()
        {
            m.extra.head = Some(item.into());
            Some("minecraft:entity.horse.armor")
        } else if item == "minecraft:chest"
            && matches!(species, Species::Donkey | Species::Mule)
            && tamed
            && !m.extra.chested
        {
            m.extra.chested = true;
            Some("minecraft:entity.donkey.chest")
        } else {
            None
        };
        if let Some(sound) = sound {
            let pitch =
                (self.world.random.next_float() - self.world.random.next_float()) * 0.2 + 1.0;
            self.audio.play(sound, Some(at), 0.5, pitch);
            self.consume_held();
            self.world
                .modified
                .insert((at[0].floor() as i32 >> 4, at[2].floor() as i32 >> 4));
            return true;
        }
        // doPlayerRide
        let m = &mut self.mobs[index];
        m.extra.flag = false;
        m.extra.counter = 0;
        self.yaw = m.yaw;
        self.riding = Some(m.id);
        self.mount.jump_ticks = 0;
        self.mount.jump_scale = 0.0;
        self.trigger_player("minecraft:started_riding");
        true
    }
    /// `AbstractHorse.handleEating`.
    fn feed_mount(&mut self, index: usize, item: &str) -> bool {
        let species = self.mobs[index].species();
        let camel = species == Species::Camel;
        // CamelHusk.isFood: rabbit's feet (and it never falls in love).
        if species == Species::CamelHusk && item != "minecraft:rabbit_foot" {
            return false;
        }
        if species != Species::CamelHusk && camel != (item == "minecraft:cactus") {
            return false;
        }
        let (heal, age_up, temper, love) = match item {
            "minecraft:rabbit_foot" if species == Species::CamelHusk => (2.0, 10, 0, false),
            // Camel.handleEating.
            "minecraft:cactus" => (2.0, 10, 0, true),
            "minecraft:wheat" => (2.0, 20, 3, false),
            "minecraft:sugar" => (1.0, 30, 3, false),
            "minecraft:hay_block" => (20.0, 180, 0, false),
            "minecraft:apple" => (3.0, 60, 3, false),
            "minecraft:red_mushroom" => (3.0, 0, 3, false),
            "minecraft:carrot" => (3.0, 60, 3, false),
            "minecraft:golden_carrot" => (4.0, 60, 5, true),
            "minecraft:golden_apple" | "minecraft:enchanted_golden_apple" => (10.0, 240, 10, true),
            _ => return false,
        };
        let m = &mut self.mobs[index];
        if matches!(m.species(), Species::SkeletonHorse | Species::ZombieHorse) {
            return false;
        }
        let mut used = false;
        let mut loved = false;
        if love && m.extra.tame && m.extra.age == 0 && m.extra.in_love <= 0 {
            m.extra.in_love = 600;
            m.extra.love_player = true;
            used = true;
            loved = true;
        }
        if m.health < m.attrs.max_health && heal > 0.0 {
            m.health = (m.health + heal).min(m.attrs.max_health);
            used = true;
        }
        if m.is_baby() && age_up > 0 && !m.extra.age_locked {
            m.extra.age = (m.extra.age + age_up * 20).min(0);
            used = true;
        }
        if temper > 0 && (used || !m.extra.tame) && m.extra.variant3 < 100 {
            m.extra.variant3 = (m.extra.variant3 + temper).min(100);
            used = true;
        }
        if !used {
            return false;
        }
        // eating(): the mouth opens and the horse eats.
        m.anim.v[2] = 1.0;
        let at = m.center();
        let id = m.id;
        let sound = match m.species() {
            Species::Camel => "minecraft:entity.camel.eat",
            Species::CamelHusk => "minecraft:entity.camel_husk.eat",
            Species::Donkey => "minecraft:entity.donkey.eat",
            Species::Mule => "minecraft:entity.mule.eat",
            Species::Horse if m.is_baby() => "minecraft:entity.baby_horse.eat",
            _ => "minecraft:entity.horse.eat",
        };
        let pitch = (self.world.random.next_float() - self.world.random.next_float()) * 0.2 + 1.0;
        self.audio.play(sound, Some(at), 1.0, pitch);
        // Animal.setInLove: entity event 18.
        if loved {
            self.love_cause.insert(id, self.lan_acting);
            self.mob_event_particles(id, 18);
        }
        self.consume_held();
        true
    }
    /// `LocalPlayer.aiStep`'s riding jump: the bar charges while jump is
    /// held; on release, the jump's strength (0 when there is none).
    pub(super) fn charge_mount_jump(&mut self, i: usize, keys: Keys) -> f32 {
        let saddled = self.mobs[i].extra.saddled && Self::jumps(self.mobs[i].species());
        let mut jump = 0.0;
        if saddled {
            let st = &mut self.mount;
            if st.jump_ticks < 0 {
                st.jump_ticks += 1;
                if st.jump_ticks == 0 {
                    st.jump_scale = 0.0;
                }
            }
            if st.was_jumping && !keys.jump {
                st.jump_ticks = -10;
                let amount = (st.jump_scale * 100.0).floor() as i32;
                // PlayerRideableJumping.getPlayerJumpPendingScale
                jump = if amount >= 90 {
                    1.0
                } else {
                    0.4 + 0.4 * amount as f32 / 90.0
                };
            } else if !st.was_jumping && keys.jump {
                st.jump_ticks = 0;
                st.jump_scale = 0.0;
            } else if st.was_jumping {
                st.jump_ticks += 1;
                st.jump_scale = if st.jump_ticks < 10 {
                    st.jump_ticks as f32 * 0.1
                } else {
                    0.8 + 2.0 / (st.jump_ticks - 9) as f32 * 0.1
                };
            }
            st.was_jumping = keys.jump;
        }
        jump
    }
    /// The rider's input for this tick; returns false when not on a mount.
    pub(super) fn tick_mount_riding(&mut self, keys: Keys) -> bool {
        let Some(i) = self.mount_index() else {
            return false;
        };
        if keys.shift || !self.mobs[i].alive() {
            self.dismount_mount();
            return true;
        }
        let strafe = f32::from(u8::from(keys.left)) - f32::from(u8::from(keys.right));
        let forward = f32::from(u8::from(keys.forward)) - f32::from(u8::from(keys.backward));
        // LocalPlayer.aiStep: the riding jump charges while space is held.
        let held = self.held().map(|s| s.item.clone());
        let steer = match self.mobs[i].species() {
            Species::Pig => held.as_deref() == Some("minecraft:carrot_on_a_stick"),
            Species::Strider => held.as_deref() == Some("minecraft:warped_fungus_on_a_stick"),
            _ => false,
        };
        let jump = self.charge_mount_jump(i, keys);
        let m = &mut self.mobs[i];
        if jump > 0.0 {
            // onPlayerJump: rear while jumping (horses).
            if matches!(
                m.species(),
                Species::Horse | Species::SkeletonHorse | Species::ZombieHorse
            ) {
                m.extra.counter = 20;
            }
            // The nautilus plays its dash sound itself.
            if equine::is_equine(m.species()) {
                // playJumpSound: Donkey and Mule have their own; a skeleton
                // horse in water splashes.
                let sound = match m.species() {
                    Species::Donkey => "minecraft:entity.donkey.jump",
                    Species::Mule => "minecraft:entity.mule.jump",
                    Species::SkeletonHorse if m.fluids.in_water => {
                        "minecraft:entity.skeleton_horse.jump_water"
                    }
                    _ => "minecraft:entity.horse.jump",
                };
                let at = m.center();
                self.audio.play(sound, Some(at), 0.4, 1.0);
            }
        }
        let m = &mut self.mobs[i];
        let pending = m.rider.map_or(0.0, |r| r.jump);
        m.rider = Some(equine::Rider {
            strafe,
            forward,
            yaw: self.yaw,
            pitch: self.pitch,
            jump: if jump > 0.0 { jump } else { pending },
            steer,
            sprint: keys.sprint || self.keys.contains(&winit::keyboard::KeyCode::ControlLeft),
            jumping: keys.jump,
        });
        true
    }
    /// `positionRider`: the player sits on the mount after it moves.
    pub(super) fn position_rider(&mut self) {
        let Some(i) = self.mount_index() else {
            return;
        };
        let m = &self.mobs[i];
        let mut at = Vec3::new(m.position.x, m.position.y + seat(m.species()), m.position.z);
        if horizon_client::mobs::camel::is_camel(m.species()) {
            // Camel.getPassengerAttachmentPoint: the driver half a block
            // forward, at the body anchor (lower while sitting).
            let scale = if m.is_baby() { 0.6 } else { 1.0 };
            let anchor = if horizon_client::mobs::camel::sitting(m) {
                (2.375 - 1.43) * scale - if m.is_baby() { 0.09375 } else { 0.375 } + 0.2 * scale
            } else {
                2.375 * scale - if m.is_baby() { 0.09375 } else { 0.375 }
            };
            let yaw = m.yaw.to_radians();
            at = Vec3::new(
                m.position.x - f64::from(0.5 * scale * yaw.sin()),
                m.position.y + f64::from(anchor) - 0.6,
                m.position.z + f64::from(0.5 * scale * yaw.cos()),
            );
        }
        if m.species() == Species::HappyGhast {
            // The first passenger attachment (0, 4, 1.7) of the harnessed back.
            let yaw = f64::from(m.yaw.to_radians());
            at = Vec3::new(
                m.position.x - 1.7 * yaw.sin(),
                m.position.y + 4.0 - 0.6,
                m.position.z + 1.7 * yaw.cos(),
            );
        }
        if horizon_client::mobs::llama::is_llama(m.species()) {
            // The passenger attachment (0, 1.37, -0.3), half of it for a baby.
            let (y, z) = if m.is_baby() {
                (0.81, -0.15)
            } else {
                (1.37, -0.3)
            };
            let yaw = f64::from(m.yaw.to_radians());
            at = Vec3::new(
                m.position.x - z * yaw.sin(),
                m.position.y + y - 0.6,
                m.position.z + z * yaw.cos(),
            );
        }
        if let Some(p) = self.player.as_mut() {
            self.previous_position = p.position;
            p.position = at;
            p.velocity = Vec3::ZERO;
            p.fall_distance = 0.0;
        }
    }
    /// `removePassenger` + `getDismountLocationForPassenger`: beside the mount.
    pub(super) fn dismount_mount(&mut self) {
        let Some(id) = self.riding.take() else {
            return;
        };
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        m.rider = None;
        if m.species() == Species::HappyGhast {
            let id = m.id;
            self.dismount_happy_ghast(id);
            return;
        }
        let yaw = m.yaw.to_radians();
        let side = Vec3::new(f64::from(yaw.cos()), 0.0, f64::from(yaw.sin()));
        let w = f64::from(m.width()) / 2.0 + 0.4;
        let base = m.position;
        let candidates = [
            Vec3::new(base.x - side.x * w, base.y, base.z - side.z * w),
            Vec3::new(base.x + side.x * w, base.y, base.z + side.z * w),
            Vec3::new(base.x, base.y + f64::from(m.height()), base.z),
        ];
        let spot = candidates
            .into_iter()
            .find(|c| {
                let f = |dy: i32| {
                    self.res
                        .facts()
                        .get(self.world.block(
                            c.x.floor() as i32,
                            c.y.floor() as i32 + dy,
                            c.z.floor() as i32,
                        ))
                        .full_collision
                };
                !f(0) && !f(1)
            })
            .unwrap_or(candidates[2]);
        if let Some(p) = self.player.as_mut() {
            p.position = spot;
            p.velocity = Vec3::ZERO;
        }
    }
    pub(super) fn mount_event(&mut self, e: &MobEvent) {
        match *e {
            MobEvent::Tamed { mob } => {
                let owner = self.level.player_uuid.or(Some([0, 0, 0, 1]));
                if let Some(view) = self
                    .mobs
                    .iter()
                    .find(|m| m.id == mob)
                    .map(|m| self.mob_entity(m))
                {
                    self.trigger_entity("minecraft:tame_animal", "entity", &view);
                }
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                    m.extra.owner = owner;
                    m.persistent = true;
                    // AbstractHorse.tameWithName: entity event 7, spawnTamingParticles(true).
                    self.mob_event_particles(mob, 7);
                }
            }
            MobEvent::Buck { mob } => {
                if self.riding == Some(mob) {
                    self.dismount_mount();
                } else {
                    // ejectPassengers: a server's player gets off too
                    // (leave_guest tells their client).
                    let rider = self.lan.as_ref().and_then(|h| {
                        h.hub
                            .guests
                            .iter()
                            .find(|g| g.vehicle == Some(mob))
                            .map(|g| g.conn)
                    });
                    if let Some(conn) = rider
                        && let Some(mut host) = self.lan.take()
                    {
                        self.as_guest(&mut host, conn, |g| g.dismount_mount());
                        self.lan = Some(host);
                    }
                }
                // RunAroundLikeCrazyGoal.tick: entity event 6, smoke.
                self.mob_event_particles(mob, 6);
            }
            _ => {}
        }
    }
    /// `PlayerRideableJumping`: horses, camels and nautili (the dash).
    fn jumps(species: Species) -> bool {
        equine::is_equine(species) || horizon_client::mobs::nautilus::is_nautilus(species)
    }
    /// `Player.jumpableVehicle`: a saddled mount that jumps (not a pig or a
    /// strider, which a saddle only steers).
    pub(super) fn jumpable_mount(&self) -> bool {
        self.mount_index()
            .is_some_and(|i| self.mobs[i].extra.saddled && Self::jumps(self.mobs[i].species()))
    }
    /// `willPrioritizeJumpInfo`: charging a jump or recharging.
    pub(super) fn mount_jumping(&self) -> bool {
        self.mount.jump_scale > 0.0 || self.mount.jump_ticks < 0
    }
    /// `Hud.extractVehicleHealth`, and the jump bar when it is the
    /// contextual bar (`jump_bar`). Returns the mount's heart count
    /// (`getVehicleMaxHearts`).
    pub(super) fn draw_mount_hud(
        &self,
        batch: &mut HudBatch,
        w: i32,
        h: i32,
        jump_bar: bool,
    ) -> i32 {
        let Some(i) = self.mount_index() else {
            return 0;
        };
        let m = &self.mobs[i];
        let atlas = &self.gui.atlas;
        let hearts = horizon_client::hud_layout::vehicle_max_hearts(m.attrs.max_health);
        let draws = horizon_client::hud_layout::vehicle_health(w, h, hearts, m.health);
        horizon_client::hud_layout::paint(batch, atlas, &draws);
        if jump_bar {
            // JumpableVehicleBar: `getJumpCooldown`, which only the camel's and
            // the nautilus' dash have; a horse always shows the progress.
            let cooldown = i32::from(
                self.mount.jump_ticks < 0
                    && (matches!(m.species(), Species::Camel | Species::CamelHusk)
                        || horizon_client::mobs::nautilus::is_nautilus(m.species())),
            );
            let draws =
                horizon_client::hud_overlays::jump_bar(w, h, cooldown, self.mount.jump_scale);
            horizon_client::hud_layout::paint(batch, atlas, &draws);
        }
        hearts
    }
}
