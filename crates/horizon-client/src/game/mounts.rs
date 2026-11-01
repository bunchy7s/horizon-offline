//! Riding horses, donkeys and mules: feeding (`handleEating`), saddles,
//! armour and chests, mounting, the rider's control and jump bar
//! (`LocalPlayer.aiStep`'s riding jump), positioning, dismounting, and the
//! mount's health and jump bar on the HUD.
use super::Game;
use horizon_client::{
    blocks::Fluid,
    hud::HudBatch,
    items::ItemStack,
    mobs::{MobEvent, Species, Who, equine},
    motion::{MotionWorld, PlayerMotion},
    pose::{Dimensions, Pose},
    sprint::Keys,
};
use horizon_core::{
    geometry::{Aabb, Vec3, VoxelShape, trig},
    random::RandomSource,
};

fn steering_item(species: Species, main: Option<&str>, offhand: Option<&str>) -> bool {
    let item = match species {
        Species::Pig => "minecraft:carrot_on_a_stick",
        Species::Strider => "minecraft:warped_fungus_on_a_stick",
        _ => return false,
    };
    main == Some(item) || offhand == Some(item)
}

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
        // Pig.mobInteract: held food goes to Animal.mobInteract, not a ride.
        let food = m.kind.food.iter().any(|f| f.as_str() == item);
        if !m.extra.saddled || food || self.keys.contains(&winit::keyboard::KeyCode::ShiftLeft) {
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
            self.trigger_item_durability(&stack, stack.damage + wear);
            if let Some(stack) = self.inventory[self.selected].as_mut() {
                stack.damage += wear;
                let max = self
                    .res
                    .items()
                    .get(&stack.item)
                    .map_or(25, |i| i.max_damage);
                if stack.damage >= max {
                    let broken = stack.clone();
                    self.inventory[self.selected] =
                        Some(ItemStack::new("minecraft:fishing_rod", 1));
                    // ItemStack.hurtAndBreak: onEquippedItemBroken, then the replacement.
                    self.player_equipment_broke(
                        horizon_world::player_inventory::EquipmentSlot::MainHand,
                        &broken,
                    );
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
        let steer = steering_item(
            self.mobs[i].species(),
            self.held().map(|s| s.item.as_ref()),
            self.offhand.as_ref().map(|s| s.item.as_ref()),
        );
        let jump = self.charge_mount_jump(i, keys);
        let m = &mut self.mobs[i];
        if jump > 0.0 {
            // AbstractHorse.onPlayerJump: slide on the keys and rear (horses,
            // donkeys and mules).
            if equine::is_equine(m.species()) {
                equine::on_player_jump(m);
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
            // A server's player (a LAN guest, single player's window) drives
            // a horse from its own client (`handleMoveVehicle`).
            client: self.lan_acting.is_some(),
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
            // forward, at the body anchor (lower while sitting), turned with
            // the camel.
            let a = horizon_client::mobs::camel::passenger_attachment(m);
            let turned = Vec3::new(a[0], a[1], a[2])
                .rotate_y(-m.yaw * (std::f64::consts::PI / 180.0) as f32);
            at = Vec3::new(
                m.position.x + turned.x,
                m.position.y + turned.y - 0.6,
                m.position.z + turned.z,
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
        // AbstractHorse.getPassengerAttachmentPoint: the rider slides back and
        // up while the horse rears.
        let rear = equine::rear_offset(m);
        let rear = Vec3::new(rear[0], rear[1], rear[2])
            .rotate_y(-m.yaw * (std::f64::consts::PI / 180.0) as f32);
        at = Vec3::new(at.x + rear.x, at.y + rear.y, at.z + rear.z);
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
        let mount = Mount {
            species: m.species(),
            position: m.position,
            bounds: m.bounds(),
            width: m.width(),
            height: m.height(),
            yaw: m.yaw,
        };
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let rider = Rider {
            yaw: self.yaw,
            width: p.dimensions.width,
            poses: rider_poses(p),
        };
        let (spot, pose) = dismount_location(&mount, &rider, &GameDismountWorld(self));
        if let Some(p) = self.player.as_mut() {
            p.position = spot;
            p.velocity = Vec3::ZERO;
            // LivingEntity.setPose: the dismount pose that fit.
            if let Some(pose) = pose
                && let Err(e) = p.set_pose(pose)
            {
                log::warn!("dismount pose: {e}");
            }
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
            // `LivingEntity.stabAttack`: `target.stopRiding()`.
            MobEvent::StabDismount { target } => match target {
                Who::Player => self.stop_riding(),
                Who::Mob(id) => self.dismount_mob(id),
            },
            _ => {}
        }
    }
    /// `Entity.propagateFallToPassengers`: whoever rides the mob `mob` takes
    /// `causeFallDamage` of its fall, as the mount's own damage does. The local
    /// player, the server players and the mob on its back each get it, with
    /// their own safe distance and armour.
    pub(super) fn fall_to_passengers(&mut self, mob: u32, distance: f64, modifier: f32) {
        if self.riding == Some(mob) {
            self.rider_fall_damage(distance, modifier);
        }
        let guests: Vec<u32> = self.lan.as_ref().map_or_else(Vec::new, |h| {
            h.hub
                .guests
                .iter()
                .filter(|g| g.vehicle == Some(mob) && Some(g.conn) != self.lan_acting)
                .map(|g| g.conn)
                .collect()
        });
        if !guests.is_empty()
            && let Some(mut host) = self.lan.take()
        {
            for conn in guests {
                self.guest_hit_by(&mut host, conn, |g| g.rider_fall_damage(distance, modifier));
            }
            self.lan = Some(host);
        }
        let rider = self
            .mobs
            .iter()
            .find(|m| m.id == mob)
            .and_then(|m| m.extra.rider);
        if let Some(rider) = rider
            && let Some(r) = self.mobs.iter_mut().find(|m| m.id == rider)
        {
            let mut events = Vec::new();
            r.cause_fall_damage_from(distance, modifier, &mut events);
            let _ = self.handle_mob_events(events);
        }
    }
    /// The player this game runs (the local one, or the server player it acts
    /// as) takes the fall of the mount under it. `cause_mount_fall` runs the
    /// landing code's own fall damage for the player: creative flight is
    /// exempt, and the statistic, the sounds and the damage records wait in the
    /// player's events for its next tick, as a landing of its own leaves them.
    fn rider_fall_damage(&mut self, distance: f64, modifier: f32) {
        let Some(player) = self.player.as_mut() else {
            return;
        };
        let _ = MotionWorld {
            entities: &[],
            player,
            scene: &mut self.world,
            assets: &self.pack.assets,
            collision_binding: self.collision,
            resources: self.motion,
        }
        .cause_mount_fall(distance, modifier);
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

/// The mount a rider gets off: its kind, where it stands, its box,
/// `getBbWidth`, `getBbHeight` and body yaw.
struct Mount {
    species: Species,
    position: Vec3,
    bounds: Aabb,
    width: f32,
    height: f32,
    yaw: f32,
}
/// The rider getting off: `getYRot`, `getBbWidth`, and
/// `getLocalBoundsForPose` for each of `Player.getDismountPoses`.
struct Rider {
    yaw: f32,
    width: f32,
    poses: Vec<(Pose, Dimensions)>,
}
/// `Player.getDismountPoses`: standing, crouching, then crawling.
const DISMOUNT_POSES: [Pose; 3] = [Pose::Standing, Pose::Crouching, Pose::Swimming];
/// The rider's box in each dismount pose, at their `minecraft:scale`.
fn rider_poses(p: &PlayerMotion) -> Vec<(Pose, Dimensions)> {
    // EntityDimensions.scale: every player pose scales alike, so the
    // standing width gives the scale without a mutable attribute lookup.
    let scale = p.dimensions.width / Pose::Standing.dimensions(1.0).width;
    DISMOUNT_POSES
        .iter()
        .map(|&pose| (pose, pose.dimensions(scale)))
        .collect()
}
/// The level queries a dismount makes.
trait DismountWorld {
    /// A full collision block (the generic rule's test).
    fn full_collision(&self, p: [i32; 3]) -> bool;
    /// `getFluidState(pos).is(FluidTags.LAVA)`.
    fn lava(&self, p: [i32; 3]) -> bool;
    /// `BlockGetter.getBlockFloorHeight(pos)`.
    fn floor_height(&self, p: [i32; 3]) -> f64;
    /// `DismountHelper.canDismountTo(level, passenger, box)`.
    fn can_dismount_to(&self, bounds: Aabb) -> bool;
}
struct GameDismountWorld<'a, 'g>(&'a Game<'g>);
impl DismountWorld for GameDismountWorld<'_, '_> {
    fn full_collision(&self, p: [i32; 3]) -> bool {
        let g = self.0;
        g.res
            .facts()
            .get(g.world.block(p[0], p[1], p[2]))
            .full_collision
    }
    fn lava(&self, p: [i32; 3]) -> bool {
        let g = self.0;
        g.res.facts().get(g.world.block(p[0], p[1], p[2])).fluid == Fluid::Lava
    }
    fn floor_height(&self, p: [i32; 3]) -> f64 {
        let shape = self.0.block_collision_shape(p);
        let below = std::cell::OnceCell::<VoxelShape>::new();
        horizon_core::geometry::block_floor_height(&shape, || {
            below.get_or_init(|| self.0.block_collision_shape([p[0], p[1] - 1, p[2]]))
        })
    }
    fn can_dismount_to(&self, bounds: Aabb) -> bool {
        self.0.can_dismount_to(bounds)
    }
}
/// `getDismountLocationForPassenger`, and the pose the rider takes there.
fn dismount_location(
    mount: &Mount,
    rider: &Rider,
    world: &impl DismountWorld,
) -> (Vec3, Option<Pose>) {
    if mount.species == Species::Strider {
        return strider_dismount_location(mount, rider, world);
    }
    // Beside the mount, else on top of it.
    let yaw = mount.yaw.to_radians();
    let side = Vec3::new(f64::from(yaw.cos()), 0.0, f64::from(yaw.sin()));
    let w = f64::from(mount.width) / 2.0 + 0.4;
    let base = mount.position;
    let candidates = [
        Vec3::new(base.x - side.x * w, base.y, base.z - side.z * w),
        Vec3::new(base.x + side.x * w, base.y, base.z + side.z * w),
        Vec3::new(base.x, base.y + f64::from(mount.height), base.z),
    ];
    let spot = candidates
        .into_iter()
        .find(|c| {
            let f = |dy: i32| {
                world.full_collision([
                    c.x.floor() as i32,
                    c.y.floor() as i32 + dy,
                    c.z.floor() as i32,
                ])
            };
            !f(0) && !f(1)
        })
        .unwrap_or(candidates[2]);
    (spot, None)
}
/// `Entity.getCollisionHorizontalEscapeVector`: out of the vehicle's box
/// along `yaw`, to the edge of the square it spans.
fn escape_vector(vehicle_width: f64, passenger_width: f64, yaw: f32) -> Vec3 {
    // 1.0E-5F: a float literal widened to double.
    let distance = (vehicle_width + passenger_width + f64::from(1.0e-5_f32)) / 2.0;
    let radians = f64::from(yaw * 0.017_453_292);
    let (sin, cos) = (-trig::sin(radians), trig::cos(radians));
    let max = f64::from(sin.abs().max(cos.abs()));
    Vec3::new(
        f64::from(sin) * distance / max,
        0.0,
        f64::from(cos) * distance / max,
    )
}
/// `Strider.getDismountLocationForPassenger`: the columns along the rider's
/// facing and 22.5 and 45 degrees either side of it, each from the top of
/// the strider's box down to half a block below its feet. The first block
/// with no lava, a valid floor (`DismountHelper.isBlockFloorValid`) and room
/// for one of the rider's dismount poses (`canDismountTo`) wins, with that
/// pose; without one, on top of the strider with the pose unchanged.
fn strider_dismount_location(
    strider: &Mount,
    rider: &Rider,
    world: &impl DismountWorld,
) -> (Vec3, Option<Pose>) {
    let (vehicle, passenger) = (f64::from(strider.width), f64::from(rider.width));
    let top = strider.bounds.max_position().y;
    let bottom = strider.bounds.min_position().y - 0.5;
    // A LinkedHashSet: each block once, in the order first reached.
    let mut blocks: Vec<[i32; 3]> = Vec::new();
    for turn in [0.0, -22.5, 22.5, -45.0, 45.0] {
        let offset = escape_vector(vehicle, passenger, rider.yaw + turn);
        let (x, z) = (
            (strider.position.x + offset.x).floor() as i32,
            (strider.position.z + offset.z).floor() as i32,
        );
        let mut y = top;
        let mut block_y = top.floor() as i32;
        while y > bottom {
            if !blocks.contains(&[x, block_y, z]) {
                blocks.push([x, block_y, z]);
            }
            block_y -= 1;
            y -= 1.0;
        }
    }
    for p in blocks {
        if world.lava(p) {
            continue;
        }
        // DismountHelper.isBlockFloorValid.
        let floor = world.floor_height(p);
        if floor.is_infinite() || floor >= 1.0 {
            continue;
        }
        // Vec3.upFromBottomCenterOf.
        let at = Vec3::new(
            f64::from(p[0]) + 0.5,
            f64::from(p[1]) + floor,
            f64::from(p[2]) + 0.5,
        );
        for (pose, dimensions) in &rider.poses {
            if world.can_dismount_to(dimensions.bounds(at)) {
                return (at, Some(*pose));
            }
        }
    }
    (Vec3::new(strider.position.x, top, strider.position.z), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn steering_accepts_the_correct_item_in_either_hand() {
        let carrot = "minecraft:carrot_on_a_stick";
        let fungus = "minecraft:warped_fungus_on_a_stick";
        assert!(steering_item(Species::Pig, Some(carrot), None));
        assert!(steering_item(Species::Pig, None, Some(carrot)));
        assert!(steering_item(Species::Strider, None, Some(fungus)));
        assert!(!steering_item(Species::Pig, Some(fungus), None));
        assert!(!steering_item(Species::Horse, Some(carrot), Some(fungus)));
    }

    /// A lava sea up to y 30 with full blocks placed in it; air above.
    struct Sea {
        blocks: HashMap<[i32; 3], ()>,
    }
    impl Sea {
        fn with(blocks: &[[i32; 3]]) -> Self {
            Self {
                blocks: blocks.iter().map(|&b| (b, ())).collect(),
            }
        }
        fn solid(&self, p: [i32; 3]) -> bool {
            self.blocks.contains_key(&p)
        }
    }
    impl DismountWorld for Sea {
        fn full_collision(&self, p: [i32; 3]) -> bool {
            self.solid(p)
        }
        fn lava(&self, p: [i32; 3]) -> bool {
            p[1] <= 30 && !self.solid(p)
        }
        fn floor_height(&self, p: [i32; 3]) -> f64 {
            if self.solid(p) {
                1.0
            } else if self.solid([p[0], p[1] - 1, p[2]]) {
                0.0
            } else {
                f64::NEG_INFINITY
            }
        }
        fn can_dismount_to(&self, bounds: Aabb) -> bool {
            let (lo, hi) = (bounds.min_position(), bounds.max_position());
            let cells = |a: f64, b: f64| a.floor() as i32..=(b - 1.0e-7).floor() as i32;
            !cells(lo.x, hi.x).any(|x| {
                cells(lo.y, hi.y).any(|y| cells(lo.z, hi.z).any(|z| self.solid([x, y, z])))
            })
        }
    }
    /// The top of a lava source at y 30 (`FlowingFluid.getOwnHeight`, 8/9).
    const SURFACE: f64 = 30.0 + 8.0 / 9.0;
    /// A strider (0.9 by 1.7) standing on the lava at x, z, facing south.
    fn strider_at(x: f64, z: f64) -> Mount {
        Mount {
            species: Species::Strider,
            position: Vec3::new(x, SURFACE, z),
            bounds: Aabb::new(
                Vec3::new(x - 0.45, SURFACE, z - 0.45),
                Vec3::new(x + 0.45, SURFACE + 1.7, z + 0.45),
            ),
            width: 0.9,
            height: 1.7,
            yaw: 0.0,
        }
    }
    fn rider(yaw: f32) -> Rider {
        Rider {
            yaw,
            width: 0.6,
            poses: DISMOUNT_POSES
                .iter()
                .map(|&pose| (pose, pose.dimensions(1.0)))
                .collect(),
        }
    }
    fn block_of(at: Vec3) -> [i32; 3] {
        [
            at.x.floor() as i32,
            at.y.floor() as i32,
            at.z.floor() as i32,
        ]
    }

    #[test]
    fn a_strider_in_open_lava_puts_its_rider_on_its_back() {
        // #3513: getting off in a lava sea used to put the player beside the
        // strider, in the lava; with no floor in reach vanilla puts the rider
        // on top of the strider instead.
        let sea = Sea::with(&[]);
        let (at, pose) = dismount_location(&strider_at(0.5, 0.5), &rider(0.0), &sea);
        assert!(!sea.lava(block_of(at)), "dismounted into lava at {at:?}");
        assert_eq!(at, Vec3::new(0.5, SURFACE + 1.7, 0.5));
        assert_eq!(pose, None);
    }

    #[test]
    fn a_strider_by_the_shore_puts_its_rider_on_the_shore() {
        // Netherrack 45 degrees to the right of a rider facing south (+z);
        // the straight-ahead and 22.5 degree columns are lava.
        let sea = Sea::with(&[[-1, 30, 1]]);
        let (at, pose) = dismount_location(&strider_at(0.5, 0.5), &rider(0.0), &sea);
        assert_eq!(at, Vec3::new(-0.5, 31.0, 1.5));
        assert_eq!(pose, Some(Pose::Standing));
    }

    #[test]
    fn a_low_ceiling_on_the_shore_makes_the_rider_crawl_off() {
        let sea = Sea::with(&[[0, 30, 1], [0, 32, 1]]);
        let (at, pose) = dismount_location(&strider_at(0.5, 0.5), &rider(0.0), &sea);
        assert_eq!(at, Vec3::new(0.5, 31.0, 1.5));
        assert_eq!(pose, Some(Pose::Swimming));
    }

    #[test]
    fn the_escape_vector_reaches_the_edge_of_the_square() {
        let edge = (0.9 + 0.6 + f64::from(1.0e-5_f32)) / 2.0;
        let v = escape_vector(0.9, 0.6, 0.0);
        assert!((v.z - edge).abs() < 1.0e-6 && v.x.abs() < 1.0e-6);
        // Facing west of south: the corner of the square.
        let v = escape_vector(0.9, 0.6, 45.0);
        assert!((v.x + edge).abs() < 1.0e-3 && (v.z - edge).abs() < 1.0e-3);
    }
}
