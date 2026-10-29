//! Firework rockets (`FireworkRocketItem`, `FireworkRocketEntity`): launching
//! from a block face, boosting a gliding player, the 1.15 horizontal
//! acceleration and 0.04 lift, the flight-duration lifetime, explosion
//! damage with line of sight, and the client explosion (`createFireworks`).
use super::{Game, Hit, item_draw::owner_at, view_vector};
use horizon_assets::item_model::DisplayContext;
use horizon_client::{
    block_entity_render::vertex_light,
    collisions::SceneCollisions,
    entity_nbt::RocketNbt,
    items::ItemStack,
    mesher::Vertex,
    mobs::{Damage, DamageKind, Species, Who},
    particles::{FireworkExplosion, ParticleOptions, ParticleType},
    projectiles::ProjectileWorld,
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::RandomSource,
};

pub(super) const FIREWORK_EXPLODE_EVENT: i8 = 17;

pub(super) struct Rocket {
    /// The entity id a server tracks it by (0 until it is tracked).
    pub id: u32,
    pub(super) position: Vec3,
    pub(super) previous: Vec3,
    pub(super) velocity: Vec3,
    life: i32,
    lifetime: i32,
    pub(super) explosions: Vec<FireworkExplosion>,
    /// Boosting the player (attachedToEntity).
    pub(super) attached: bool,
    /// On a server, the guest (by connection) who launched it, or whom it
    /// boosts: `owner`'s `Who::Player` then means them, not the host.
    pub(super) guest: Option<u32>,
    /// `Projectile.getOwner`: the player who launched it, credited with
    /// what its blast hurts.
    pub(super) owner: Option<Who>,
    /// `Projectile.leftOwner`: until then it can't hit its owner.
    left_owner: bool,
    /// `DATA_ATTACHED_TO_TARGET`: the server entity the rocket follows;
    /// boosting happens when it is the local player.
    pub(super) attached_to: Option<u32>,
    /// `DATA_SHOT_AT_ANGLE` (dispensers): constant velocity, exploding on
    /// the first block it hits.
    pub(super) angled: bool,
    /// `DATA_ID_FIREWORKS_ITEM`: the stack it was launched from.
    pub(super) item: ItemStack,
}
impl Rocket {
    /// A rocket a server sent (`handleAddEntity`): placed and moved by it.
    pub(super) fn remote(id: u32, position: Vec3, velocity: Vec3) -> Self {
        let item = ItemStack::new("minecraft:firework_rocket", 1);
        let (_, explosions) = fireworks_of(&item);
        Self {
            id,
            position,
            previous: position,
            velocity,
            life: 0,
            lifetime: i32::MAX,
            explosions,
            attached: false,
            guest: None,
            owner: None,
            left_owner: false,
            attached_to: None,
            angled: false,
            item,
        }
    }
    /// A rocket loaded from its NBT (`/summon`, a saved chunk).
    pub(super) fn from_nbt(r: RocketNbt) -> Self {
        let item = r
            .item
            .unwrap_or_else(|| ItemStack::new("minecraft:firework_rocket", 1));
        let (_, explosions) = fireworks_of(&item);
        Self {
            id: 0,
            position: r.position,
            previous: r.position,
            velocity: r.motion,
            life: r.life,
            lifetime: r.lifetime,
            explosions,
            attached: false,
            guest: None,
            owner: r.owner,
            left_owner: r.left_owner,
            attached_to: None,
            angled: r.angled,
            item,
        }
    }
    /// One tick of a server's client: the rocket flies on by itself between
    /// the server's updates (a rocket that follows an entity is placed by the
    /// caller).
    pub(super) fn remote_step(&mut self, scene: &mut SceneCollisions) {
        self.previous = self.position;
        if self.attached_to.is_none() {
            self.free_flight(scene);
        }
        self.life += 1;
    }
    /// The move of a rocket attached to no entity: sped up and rising, or an
    /// angled one's own motion.
    fn next_motion(&self) -> Vec3 {
        if self.angled {
            self.velocity
        } else {
            Vec3::new(
                self.velocity.x * 1.15,
                self.velocity.y + 0.04,
                self.velocity.z * 1.15,
            )
        }
    }
    /// `FireworkRocketEntity.tick` for a rocket attached to no entity: speed
    /// up (an angled one keeps its motion) and rise, then move. True when a
    /// block stopped it.
    fn free_flight(&mut self, scene: &mut SceneCollisions) -> bool {
        let h = 0.125;
        let bounds = Aabb::new(
            Vec3::new(self.position.x - h, self.position.y, self.position.z - h),
            Vec3::new(
                self.position.x + h,
                self.position.y + 2.0 * h,
                self.position.z + h,
            ),
        );
        let wanted = self.next_motion();
        let Ok(moved) = scene.resolve(bounds, wanted, 0.0, false) else {
            return false;
        };
        let blocked = (moved.x - wanted.x).abs() > 1e-7
            || (moved.y - wanted.y).abs() > 1e-7
            || (moved.z - wanted.z).abs() > 1e-7;
        let horizontal = (moved.x - wanted.x).abs() > 1e-7 || (moved.z - wanted.z).abs() > 1e-7;
        self.position = Vec3::new(
            self.position.x + moved.x,
            self.position.y + moved.y,
            self.position.z + moved.z,
        );
        self.velocity = if horizontal {
            Vec3::new(self.velocity.x, wanted.y, self.velocity.z)
        } else {
            wanted
        };
        blocked
    }
    /// Position and motion (the dispenser parity fixture).
    pub(super) fn motion(&self) -> (Vec3, Vec3) {
        (self.position, self.velocity)
    }
}
/// `damageAmount * (float)Math.sqrt((5.0 - this.distanceTo(target)) / 5.0)`
/// from the rocket at `from` to a target at `to`; `Entity.distanceTo` sums
/// the squared offsets in floats.
fn blast_damage(damage: f32, from: Vec3, to: Vec3) -> f32 {
    let (x, y, z) = (
        (from.x - to.x) as f32,
        (from.y - to.y) as f32,
        (from.z - to.z) as f32,
    );
    let distance = f64::from(x * x + y * y + z * z).sqrt() as f32;
    damage * ((5.0 - f64::from(distance)) / 5.0).sqrt() as f32
}
pub(super) fn fireworks_of(stack: &ItemStack) -> (u8, Vec<FireworkExplosion>) {
    // The item's default component: flight 1, no explosions.
    stack
        .data()
        .and_then(|d| d.fireworks.clone())
        .unwrap_or((1, Vec::new()))
}
impl Game<'_> {
    pub(super) fn new_rocket(&mut self, at: Vec3, stack: &ItemStack, attached: bool) -> Rocket {
        let (flight, explosions) = fireworks_of(stack);
        // FireworkRocketEntity: motion and lifetime come from the rocket's own random.
        let r = &mut super::dispenser::entity_random();
        let tri = |r: &mut horizon_core::random::LegacyRandom| r.triangle(0.0, 0.002297);
        let velocity = Vec3::new(tri(r), 0.05, tri(r));
        let lifetime = 10 * (1 + i32::from(flight))
            + r.next_int_bound(6).unwrap_or(0)
            + r.next_int_bound(7).unwrap_or(0);
        Rocket {
            id: 0,
            position: at,
            previous: at,
            velocity,
            life: 0,
            lifetime,
            explosions,
            attached,
            guest: None,
            owner: None,
            left_owner: false,
            attached_to: None,
            angled: false,
            item: stack.clone(),
        }
    }
    /// `FireworkRocketItem.useOn`: launch from the clicked point.
    pub(super) fn launch_firework(&mut self, hit: &Hit, stack: &ItemStack) -> bool {
        if &*stack.item != "minecraft:firework_rocket" {
            return false;
        }
        let at = Vec3::new(
            hit.point[0] + f64::from(hit.face[0]) * 0.15,
            hit.point[1] + f64::from(hit.face[1]) * 0.15,
            hit.point[2] + f64::from(hit.face[2]) * 0.15,
        );
        let mut rocket = self.new_rocket(at, stack, false);
        rocket.owner = Some(Who::Player);
        rocket.guest = self.lan_acting;
        self.rockets.push(rocket);
        if !self.creative {
            self.consume_held();
        }
        self.swing_arm();
        true
    }
    /// A rocket shot at an angle (`DATA_SHOT_AT_ANGLE`): out of a dispenser
    /// (`FireworkRocketItem.asProjectile`) or a crossbow.
    pub(super) fn launch_angled_rocket(
        &mut self,
        at: Vec3,
        velocity: Vec3,
        stack: &ItemStack,
        owner: Option<Who>,
    ) {
        let mut rocket = self.new_rocket(at, stack, false);
        rocket.velocity = velocity;
        rocket.angled = true;
        rocket.owner = owner;
        rocket.guest = self.lan_acting.filter(|_| owner == Some(Who::Player));
        self.rockets.push(rocket);
    }
    /// `FireworkRocketItem.use` while gliding: a rocket attached to the player.
    pub(super) fn boost_firework(&mut self, stack: &ItemStack) -> bool {
        if &*stack.item != "minecraft:firework_rocket"
            || !self.player.as_ref().is_some_and(|p| p.fall_flying)
        {
            return false;
        }
        let at = self.player.as_ref().map_or(Vec3::ZERO, |p| p.position);
        let mut rocket = self.new_rocket(at, stack, true);
        rocket.owner = Some(Who::Player);
        rocket.guest = self.lan_acting;
        self.rockets.push(rocket);
        if !self.creative {
            self.consume_held();
        }
        self.swing_arm();
        true
    }
    pub(super) fn tick_rockets(&mut self) {
        let targets = if self.rockets.is_empty() {
            Vec::new()
        } else {
            self.rocket_targets()
        };
        let mut i = 0;
        while i < self.rockets.len() {
            let mut r = self.rockets.remove(i);
            r.previous = r.position;
            // Projectile.tick: whether it has left its owner, before it moves.
            let owner = self.rocket_owner(&r);
            if !r.left_owner {
                let h = 0.125;
                let reach = Aabb::new(
                    Vec3::new(r.position.x - h, r.position.y, r.position.z - h),
                    Vec3::new(r.position.x + h, r.position.y + 2.0 * h, r.position.z + h),
                )
                .expand_towards(r.velocity)
                .inflate(Vec3::new(1.0, 1.0, 1.0));
                r.left_owner = owner.is_none_or(|o| {
                    !targets
                        .iter()
                        .any(|(who, b)| *who == o && b.intersects(reach))
                });
            }
            let skip = owner.filter(|_| !r.left_owner);
            let mut hit = None;
            if r.attached
                && let Some(conn) = r.guest
            {
                // A guest's rocket follows them; their own client boosts them.
                let guest = self
                    .lan
                    .as_ref()
                    .and_then(|host| host.hub.guests.iter().find(|g| g.conn == conn));
                if let Some(g) = guest {
                    r.position = Vec3::new(g.position[0], g.position[1], g.position[2]);
                }
            } else if r.attached {
                if let Some(p) = self.player.as_mut() {
                    if p.fall_flying {
                        let look = view_vector(self.yaw, self.pitch);
                        let m = p.velocity;
                        p.velocity = Vec3::new(
                            m.x + look[0] * 0.1 + (look[0] * 1.5 - m.x) * 0.5,
                            m.y + look[1] * 0.1 + (look[1] * 1.5 - m.y) * 0.5,
                            m.z + look[2] * 0.1 + (look[2] * 1.5 - m.z) * 0.5,
                        );
                    }
                    r.position = p.position;
                    r.velocity = p.velocity;
                }
                hit = self.rocket_struck(&targets, &r, r.velocity, skip);
            } else {
                // getHitResultOnMoveVector before the move.
                hit = self.rocket_struck(&targets, &r, r.next_motion(), skip);
                let mut scene = SceneCollisions {
                    world: &self.world,
                    binding: self.collision,
                    subject: None,
                };
                if r.free_flight(&mut scene) && r.angled {
                    // hitTargetOrDeflectSelf: explode on the block.
                    r.life = r.lifetime;
                }
            }
            // hitTargetOrDeflectSelf on an entity: a breeze turns it back
            // (ProjectileDeflection.REVERSE), anything else sets it off.
            match hit {
                Some(Who::Mob(id))
                    if self
                        .mobs
                        .iter()
                        .any(|m| m.id == id && m.species() == Species::Breeze) =>
                {
                    r.velocity = r.velocity.scale(-0.5);
                }
                Some(_) => r.life = r.lifetime,
                None => {}
            }
            let at = [r.position.x, r.position.y, r.position.z];
            if r.life == 0 {
                self.audio.play(
                    "minecraft:entity.firework_rocket.launch",
                    Some(at),
                    3.0,
                    1.0,
                );
            }
            r.life += 1;
            // The trail: a spark each tick drifting against the rocket's motion.
            let g = |g: &mut horizon_core::random::LegacyRandom| g.next_gaussian() * 0.05;
            let v = [
                g(&mut self.world.random),
                -r.velocity.y * 0.5,
                g(&mut self.world.random),
            ];
            self.particles
                .add(&ParticleOptions::simple(ParticleType::Firework), at, v);
            if r.life > r.lifetime {
                self.explode_rocket(&r);
                continue;
            }
            self.rockets.insert(i, r);
            i += 1;
        }
    }
    /// `FireworkRocketEntity.explode` and `dealExplosionDamage`.
    fn explode_rocket(&mut self, r: &Rocket) {
        let at = [r.position.x, r.position.y, r.position.z];
        // FireworkRocketEntity.explode broadcasts before discard. Send now:
        // tick_lan removes missing tracked entities before flushing effects.
        if r.id != 0
            && let Some(host) = &self.lan
        {
            host.hub
                .entity_event(r.id as i32, FIREWORK_EXPLODE_EVENT, at);
        }
        if !r.explosions.is_empty() {
            let damage = 5.0 + r.explosions.len() as f32 * 2.0;
            let from = r.position;
            // `attachedToEntity` takes the full damage, a guest as well; every
            // other player in range and in sight takes the blast.
            let boosted = r.guest.filter(|_| r.attached);
            if r.attached && boosted.is_none() {
                self.rocket_hurt_player(damage);
            } else {
                self.rocket_blast_player(from, damage);
            }
            if let Some(conn) = boosted
                && let Some(mut host) = self.lan.take()
            {
                self.as_guest(&mut host, conn, |g| g.rocket_hurt_player(damage));
                self.lan = Some(host);
            }
            // Every other player in the level (LAN guests, the players of a
            // server thread), each swapped in as the player.
            self.each_guest_hit_except(boosted, |g| g.rocket_blast_player(from, damage));
            let hits: Vec<(u32, f32)> = {
                let world = self.mob_world_base(&[]);
                self.mobs
                    .iter()
                    .filter(|m| m.alive())
                    .filter_map(|m| {
                        let d2 = (m.position.x - from.x).powi(2)
                            + (m.position.y - from.y).powi(2)
                            + (m.position.z - from.z).powi(2);
                        // Line of sight to the feet or the middle of the mob.
                        let sees = d2 <= 25.0
                            && (0..2).any(|step| {
                                let to = Vec3::new(
                                    m.position.x,
                                    m.position.y + f64::from(m.height()) * 0.5 * f64::from(step),
                                    m.position.z,
                                );
                                world.clear_between(from, to, false)
                            });
                        sees.then(|| (m.id, blast_damage(damage, from, m.position)))
                    })
                    .collect()
            };
            // damageSources().fireworks(this, getOwner()): a guest's rocket
            // hurts as theirs, and the mobs it hit answer to them.
            let owner = r.owner;
            if let Some(conn) = r.guest.filter(|_| owner.is_some())
                && let Some(mut host) = self.lan.take()
            {
                let ran = self
                    .as_guest(&mut host, conn, |g| g.rocket_hurt_mobs(&hits, owner, at))
                    .is_some();
                if ran {
                    for (id, _) in &hits {
                        if self.mobs.iter().any(|m| m.id == *id) {
                            host.mob_players.insert(*id, conn);
                        }
                    }
                }
                self.lan = Some(host);
                if !ran {
                    self.rocket_hurt_mobs(&hits, owner, at);
                }
            } else {
                self.rocket_hurt_mobs(&hits, owner, at);
            }
        }
        let camera = self.eye_position().unwrap_or(at);
        let v = [r.velocity.x, r.velocity.y, r.velocity.z];
        self.particles
            .fireworks(at, v, r.explosions.clone(), camera);
    }
    fn rocket_hurt_mobs(&mut self, hits: &[(u32, f32)], attacker: Option<Who>, at: [f64; 3]) {
        for &(id, amount) in hits {
            if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                m.damage(
                    amount,
                    Damage {
                        kind: DamageKind::Explosion,
                        attacker,
                        from: Some(at),
                    },
                );
            }
        }
    }
    /// The owner as `rocket_targets` names it: a guest by their entity id.
    fn rocket_owner(&self, r: &Rocket) -> Option<Who> {
        let owner = r.owner?;
        let Some(conn) = r.guest.filter(|_| owner == Who::Player) else {
            return Some(owner);
        };
        let host = self.lan.as_ref()?;
        let g = host.hub.guests.iter().find(|g| g.conn == conn)?;
        Some(Who::Mob(g.entity_id as u32))
    }
    /// What a rocket can hit (`Projectile.canHitEntity`): living mobs, the
    /// dragon's parts and crystals, the player and the guests.
    fn rocket_targets(&self) -> Vec<(Who, Aabb)> {
        let mut targets: Vec<(Who, Aabb)> = self
            .mobs
            .iter()
            .filter(|m| m.alive())
            .map(|m| (Who::Mob(m.id), m.bounds()))
            .collect();
        if let Some(p) = self.player_view()
            && p.alive
            && !self.spectator()
        {
            targets.push((Who::Player, p.bounds));
        }
        targets.extend(
            self.special_targets()
                .into_iter()
                .map(|(id, b)| (Who::Mob(id), b)),
        );
        targets.extend(self.lan_projectile_targets());
        targets
    }
    /// `ProjectileUtil.getHitResultOnMoveVector`'s entity half: the nearest
    /// target on the move, short of the block it clips, each box grown by
    /// `computeMargin` (`tickCount` is the rocket's life plus one).
    fn rocket_struck(
        &self,
        targets: &[(Who, Aabb)],
        r: &Rocket,
        motion: Vec3,
        skip: Option<Who>,
    ) -> Option<Who> {
        let from = r.position;
        let mut to = from.add_vector(motion);
        let w = self.mob_world_base(&[]);
        if let Some((at, ..)) = (super::creatures::ProjWorld {
            w: &w,
            targets: Vec::new(),
        })
        .clip(from, to)
        {
            to = at;
        }
        let margin = (f64::from(r.life - 1) / 20.0).clamp(0.0, 0.3);
        targets
            .iter()
            .filter(|(who, _)| Some(*who) != skip)
            .filter_map(|(who, b)| {
                let at = b
                    .inflate(Vec3::new(margin, margin, margin))
                    .clip(from, to)?;
                Some((*who, from.distance_squared(at)))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(who, _)| who)
    }
    /// FireworkRocketEntity.handleEntityEvent: use the synchronized item data.
    pub(super) fn remote_firework_explosion(&mut self, id: i32) {
        let camera = self.eye_position();
        if let Some(r) = self.rockets.iter().find(|r| r.id == id as u32) {
            let at = [r.position.x, r.position.y, r.position.z];
            let v = [r.velocity.x, r.velocity.y, r.velocity.z];
            self.particles
                .fireworks(at, v, r.explosions.clone(), camera.unwrap_or(at));
        }
    }
    /// `dealExplosionDamage` on a player that is not the rocket's
    /// `attachedToEntity`: within 5 blocks, when the rocket sees its feet or
    /// its middle (`ClipContext.Block.COLLIDER`, no fluids).
    fn rocket_blast_player(&mut self, from: Vec3, damage: f32) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let feet = p.position;
        let d2 = (feet.x - from.x).powi(2) + (feet.y - from.y).powi(2) + (feet.z - from.z).powi(2);
        if d2 > 25.0 {
            return;
        }
        let height = f64::from(p.dimensions.height);
        let sees = {
            let world = self.mob_world_base(&[]);
            (0..2).any(|step| {
                let to = Vec3::new(feet.x, feet.y + height * 0.5 * f64::from(step), feet.z);
                world.clear_between(from, to, false)
            })
        };
        if sees {
            self.rocket_hurt_player(blast_damage(damage, from, feet));
        }
    }
    /// `hurtServer` with `damageSources().fireworks`: armor absorbs it and
    /// wears (`hurtArmor`); creative and spectator players are immune.
    fn rocket_hurt_player(&mut self, amount: f32) {
        if self.creative || self.spectator() {
            return;
        }
        if let Some(p) = self.player.as_mut()
            && !p.health.dead_or_dying()
            && let Some(id) = self.motion.health.damage.id("minecraft:fireworks")
        {
            let _ = p.hurt(&self.motion.health, self.world.health_rules, id, amount);
        }
    }
    /// `FireworkEntityRenderer.submit`: the item (GROUND) facing the camera,
    /// lit at the rocket's light probe.
    pub(super) fn rocket_geometry(&self, camera: glam::DVec3, alpha: f64, out: &mut Vec<Vertex>) {
        let orientation = glam::Mat4::from_quat(self.camera_orientation());
        // shouldRender: not while attached to an entity (`isAttachedToEntity`).
        for r in self
            .rockets
            .iter()
            .filter(|r| !r.attached && r.attached_to.is_none())
        {
            // FireworkRocketEntity.shouldRenderAtSqrDistance: within 64 blocks;
            // EntityRenderer.shouldRender: its 0.25 box in the view.
            if !self.standing_entity_in_view(
                camera,
                super::entity_culling::at(r.previous),
                super::entity_culling::at(r.position),
                alpha,
                0.25,
                0.25,
                64.0,
                0.0,
            ) {
                continue;
            }
            let p = glam::DVec3::new(
                r.previous.x + (r.position.x - r.previous.x) * alpha,
                r.previous.y + (r.position.y - r.previous.y) * alpha,
                r.previous.z + (r.position.z - r.previous.z) * alpha,
            );
            // `updateForNonLiving`: GROUND, owned by the rocket.
            let owner = owner_at(p.to_array(), 0.0, "minecraft:firework_rocket");
            let state = self.resolve_item(&r.item, DisplayContext::Ground, Some(&owner), 0);
            if state.is_empty() {
                continue;
            }
            let mut pose = glam::Mat4::from_translation((p - camera).as_vec3()) * orientation;
            if r.angled {
                pose *= glam::Mat4::from_rotation_z(std::f32::consts::PI)
                    * glam::Mat4::from_rotation_y(std::f32::consts::PI)
                    * glam::Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2);
            }
            // getLightProbePosition: the eye, 0.85 of its 0.25 height.
            let light = self.world.light(
                p.x.floor() as i32,
                (p.y + 0.2125).floor() as i32,
                p.z.floor() as i32,
            );
            self.submit_item_world(&state, pose, vertex_light(light), out);
        }
    }
}
