//! `ServerExplosion`: exploded positions (vanilla ray march over a Java
//! HashSet, so iteration and the following shuffle match vanilla's random
//! draws), entity damage with exposure and knockback, block interaction with
//! merged drops and TNT chain priming, and optional fire.
use super::Game;
use horizon_client::{
    Result, blocks::Fluid, collisions::SceneCollisions, items::ItemStack, mobs::Who, text::Text,
};
use horizon_core::{
    geometry::{Vec3, VoxelShape},
    position::BlockPos,
    random::{LegacyRandom, RandomSource},
};
use horizon_world::{
    collision_context::{CollisionContext, CollisionEntity, CollisionItem},
    entity_obstruction::EntityIdentity,
    java_hash_map::{JavaHashMap, JavaKey},
    registry::BlockStateId,
    world_ray::{RayBlockMode, RayFluidMode, RayWorld, clip_world},
};
use std::{cmp::Ordering, io};

/// `Level.clip` with `ClipContext.Block.COLLIDER`, no fluids and the
/// collision context of the entity whose exposure is measured.
struct ColliderRays<'a, 'r> {
    scene: SceneCollisions<'a, 'r>,
    context: CollisionContext<'static>,
}
impl RayWorld for ColliderRays<'_, '_> {
    type Error = io::Error;
    type Block = BlockStateId;
    type Fluid = ();
    fn block_state(&mut self, p: BlockPos) -> io::Result<BlockStateId> {
        Ok(self.scene.world.block(p.x, p.y, p.z))
    }
    fn fluid_state(&mut self, _: BlockPos) -> io::Result<()> {
        Ok(())
    }
    fn block_shape(
        &mut self,
        state: BlockStateId,
        p: BlockPos,
        _: RayBlockMode,
    ) -> io::Result<VoxelShape> {
        let binding = self.scene.binding;
        binding
            .collision_shape_with_context(&mut self.scene, state, p, &self.context)
            .map_err(|e| io::Error::other(format!("{e:?}")))
    }
    fn interaction_shape(&mut self, _: BlockStateId, _: BlockPos) -> io::Result<VoxelShape> {
        Ok(VoxelShape::empty())
    }
    fn fluid_is_source(&mut self, _: ()) -> io::Result<bool> {
        Ok(false)
    }
    fn fluid_is_empty(&mut self, _: ()) -> io::Result<bool> {
        Ok(true)
    }
    fn fluid_is_water(&mut self, _: ()) -> io::Result<bool> {
        Ok(false)
    }
    fn fluid_shape(&mut self, _: (), _: BlockPos) -> io::Result<VoxelShape> {
        Ok(VoxelShape::empty())
    }
}

/// `BlockPos` with Java `Vec3i.hashCode` / `compareTo`.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Key([i32; 3]);
impl JavaKey for Key {
    fn java_hash(&self) -> i32 {
        let [x, y, z] = self.0;
        y.wrapping_add(z.wrapping_mul(31))
            .wrapping_mul(31)
            .wrapping_add(x)
    }
    fn comparable_compare(&self, other: &Self) -> Option<Ordering> {
        let ([x, y, z], [ox, oy, oz]) = (self.0, other.0);
        Some(y.cmp(&oy).then(z.cmp(&oz)).then(x.cmp(&ox)))
    }
    fn tie_break(&self, _other: &Self) -> Ordering {
        Ordering::Less
    }
}

/// `Explosion.BlockInteraction` for the explosions Horizon produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BlockInteraction {
    Keep,
    Destroy,
    DestroyWithDecay,
}

/// `ServerExplosion.hurtEntities`: an entity is hurt when its position is
/// within the blast's diameter of the centre.
pub(super) fn blast_reaches(center: [f64; 3], diameter: f64, at: [f64; 3]) -> bool {
    let d =
        ((at[0] - center[0]).powi(2) + (at[1] - center[1]).powi(2) + (at[2] - center[2]).powi(2))
            .sqrt();
    d / diameter <= 1.0
}

/// `ItemEntity.ignoreExplosion` through
/// `ServerExplosion.shouldAffectBlocklikeEntities`: an explosion that does not
/// affect blocklike entities is ignored entirely by a dropped item. With
/// `mobGriefing` off that is every blast that keeps blocks; with it on, none
/// (no wind charge goes through `server_explode`).
fn items_ignore_explosion(mob_griefing: bool, interaction: BlockInteraction) -> bool {
    !mob_griefing && interaction == BlockInteraction::Keep
}

/// Whether a blast leaves item frames, paintings, cushions and armour stands
/// alone. Without `mobGriefing`, a blast that keeps blocks is ignored
/// (`ServerExplosion.shouldAffectBlocklikeEntities` through their
/// `ignoreExplosion`; no wind charge goes through `server_explode`), and one a
/// mob causes does no harm (`BlockAttachedEntity.hurtServer` and
/// `ArmorStand.hurtServer`: `source.getEntity() instanceof Mob`).
pub(super) fn blast_spares_decorations(
    mob_griefing: bool,
    interaction: BlockInteraction,
    by_mob: bool,
    source_in_water: bool,
) -> bool {
    // `BlockAttachedEntity.ignoreExplosion`: a source in water first.
    source_in_water || (!mob_griefing && (interaction == BlockInteraction::Keep || by_mob))
}

/// A mob in a blast's reach: `hurtServer` when its team allows the damage
/// (`hurtable`), then `pushFromExplosion` whatever that returned, so a mob
/// that refuses the hit (invulnerable, in its hurt cooldown, a wither and an
/// undead `cause`) is still pushed. `Entity.push` sets `needsSync`.
fn hurt_and_push_mob(
    m: &mut horizon_client::mobs::Mob,
    hurtable: bool,
    cause: Option<horizon_client::mobs::Species>,
    (amount, source): (f32, horizon_client::mobs::Damage),
    push: [f64; 3],
) {
    let shielded =
        cause.is_some_and(|c| horizon_client::mobs::wither::shielded_from(m.species(), c));
    if hurtable && !shielded {
        m.damage(amount, source);
    }
    m.velocity = Vec3::new(
        m.velocity.x + push[0],
        m.velocity.y + push[1],
        m.velocity.z + push[2],
    );
    m.needs_sync = true;
}

/// A blast's causing entity and its name (`Explosion.getIndirectSourceEntity`):
/// the mob that owns the exploding projectile while it is in the level
/// (`Projectile.getOwner` is null once it is removed), else the exploding
/// mob. `live` names a mob in the level, `exploded` the mobs whose blast
/// went off this tick and that already left it.
fn blast_cause(
    owner: Option<u32>,
    exploding: Option<u32>,
    live: impl Fn(u32) -> Option<Text>,
    exploded: &[(u32, Text)],
) -> Option<(u32, Text)> {
    if let Some(owner) = owner {
        return live(owner).map(|name| (owner, name));
    }
    let id = exploding?;
    let name = live(id).or_else(|| {
        exploded
            .iter()
            .rfind(|(m, _)| *m == id)
            .map(|(_, n)| n.clone())
    })?;
    Some((id, name))
}

thread_local! {
    /// Client-side randomness (explosion sound pitch), separate from the level random.
    static CLIENT_RANDOM: std::cell::RefCell<LegacyRandom> = std::cell::RefCell::new(LegacyRandom::new(0x5eed));
}
pub(super) fn client_float() -> f32 {
    CLIENT_RANDOM.with(|r| r.borrow_mut().next_float())
}

impl Game<'_> {
    /// `blast_cause` for the blast going off now.
    pub(super) fn blast_attacker(&self) -> Option<(u32, Text)> {
        // `Explosion.getIndirectSourceEntity`: a primed TNT's owner names the
        // blast before any mob does, which makes the damage type
        // `minecraft:player_explosion` (`server_explode` asks this).
        self.combat.blast_owner.clone().or_else(|| {
            blast_cause(
                self.combat.explosion_owner,
                self.combat.explosion_source,
                |id| self.mob_display_name(id),
                &self.combat.exploded,
            )
        })
    }
    /// `ServerExplosion.getSeenPercent` for an entity box.
    pub(super) fn seen_percent(&self, center: [f64; 3], min: [f64; 3], max: [f64; 3]) -> f32 {
        let xs = 1.0 / ((max[0] - min[0]) * 2.0 + 1.0);
        let ys = 1.0 / ((max[1] - min[1]) * 2.0 + 1.0);
        let zs = 1.0 / ((max[2] - min[2]) * 2.0 + 1.0);
        let x_offset = (1.0 - (1.0 / xs).floor() * xs) / 2.0;
        let z_offset = (1.0 - (1.0 / zs).floor() * zs) / 2.0;
        if xs < 0.0 || ys < 0.0 || zs < 0.0 {
            return 0.0;
        }
        // ClipContext(..., entity): the entity standing at the box's bottom.
        let mut rays = ColliderRays {
            scene: SceneCollisions {
                world: &self.world,
                binding: self.collision,
                subject: None,
            },
            context: CollisionContext::for_entity(
                CollisionEntity {
                    identity: EntityIdentity(0),
                    y: min[1],
                    descending: false,
                    main_hand: Some(CollisionItem::EMPTY),
                },
                false,
            ),
        };
        let (mut hits, mut count) = (0, 0);
        let mut xx = 0.0;
        while xx <= 1.0 {
            let mut yy = 0.0;
            while yy <= 1.0 {
                let mut zz = 0.0;
                while zz <= 1.0 {
                    let x = min[0] + (max[0] - min[0]) * xx;
                    let y = min[1] + (max[1] - min[1]) * yy;
                    let z = min[2] + (max[2] - min[2]) * zz;
                    let from = Vec3::new(x + x_offset, y, z + z_offset);
                    let to = Vec3::new(center[0], center[1], center[2]);
                    let missed = clip_world(
                        &mut rays,
                        from,
                        to,
                        RayBlockMode::Collider,
                        RayFluidMode::None,
                        4096,
                    )
                    .is_ok_and(|r| r.miss);
                    if missed {
                        hits += 1;
                    }
                    count += 1;
                    zz += zs;
                }
                yy += ys;
            }
            xx += xs;
        }
        hits as f32 / count as f32
    }
    /// `ServerExplosion.calculateExplodedPositions`; `resists` overrides one
    /// block's resistance (`ExplosionDamageCalculator.getBlockExplosionResistance`).
    fn exploded_positions(
        &mut self,
        center: [f64; 3],
        radius: f32,
        resists: Option<([i32; 3], f32)>,
    ) -> Vec<[i32; 3]> {
        let (min_y, max_y) = self.with_level(|_, level, _| {
            use horizon_world::level::LevelAccess;
            (level.min_y(), level.max_y())
        });
        // WitherSkull.getBlockExplosionResistance: a dangerous skull's blast
        // caps a block the wither may destroy at 0.8.
        let cap = self.combat.blast_resistance_cap;
        let wither_immune = cap.and_then(|_| {
            self.res
                .items()
                .block_tags
                .get("minecraft:wither_immune")
                .cloned()
        });
        let mut set: JavaHashMap<Key, ()> = JavaHashMap::default();
        for xx in 0..16 {
            for yy in 0..16 {
                for zz in 0..16 {
                    if !(xx == 0 || xx == 15 || yy == 0 || yy == 15 || zz == 0 || zz == 15) {
                        continue;
                    }
                    let mut xd = f64::from(xx as f32 / 15.0 * 2.0 - 1.0);
                    let mut yd = f64::from(yy as f32 / 15.0 * 2.0 - 1.0);
                    let mut zd = f64::from(zz as f32 / 15.0 * 2.0 - 1.0);
                    let d = (xd * xd + yd * yd + zd * zd).sqrt();
                    xd /= d;
                    yd /= d;
                    zd /= d;
                    let mut power = radius * (0.7 + self.world.random.next_float() * 0.6);
                    let (mut xp, mut yp, mut zp) = (center[0], center[1], center[2]);
                    while power > 0.0 {
                        let pos = [xp.floor() as i32, yp.floor() as i32, zp.floor() as i32];
                        if pos[1] < min_y
                            || pos[1] >= max_y
                            || !(-30_000_000..30_000_000).contains(&pos[0])
                            || !(-30_000_000..30_000_000).contains(&pos[2])
                        {
                            break;
                        }
                        let state = self.world.block(pos[0], pos[1], pos[2]);
                        let f = self.res.facts().get(state);
                        let fluid = if f.fluid != Fluid::None {
                            100.0f32
                        } else {
                            0.0
                        };
                        // A primed TNT minecart's rails and the blocks they
                        // stand on: no resistance, and never blown up.
                        let spared = self.combat.rail_proof_blast
                            && (self.rail(pos[0], pos[1], pos[2]).is_some()
                                || self.rail(pos[0], pos[1] + 1, pos[2]).is_some());
                        match resists {
                            Some((at, resistance)) if at == pos => {
                                power -= (resistance + 0.3) * 0.3;
                            }
                            _ if !self.res.facts().is_air(state) || f.fluid != Fluid::None => {
                                let base = if spared {
                                    0.0
                                } else {
                                    f.explosion_resistance.max(fluid)
                                };
                                let immune =
                                    wither_immune.as_ref().is_some_and(|t| t.contains(&*f.name));
                                power -= (blast_resistance(base, cap, immune) + 0.3) * 0.3;
                            }
                            _ => {}
                        }
                        if power > 0.0 && !spared {
                            set.put(Key(pos), ());
                        }
                        xp += xd * f64::from(0.3f32);
                        yp += yd * f64::from(0.3f32);
                        zp += zd * f64::from(0.3f32);
                        power -= 0.225_000_01;
                    }
                }
            }
        }
        set.entries().map(|(k, _)| k.0).collect()
    }
    /// `ServerExplosion.hurtEntities` for the player: damage scaled by
    /// distance and exposure, through a raised shield (`hurtServer` blocks
    /// explosions like any attack from where the blast is), then the push
    /// (less with explosion knockback resistance; none for spectators or a
    /// creative player flying).
    ///
    /// Returns the push their client adds from the explosion packet
    /// (`hitPlayers`), unless a hurt marked their velocity: that goes to them
    /// whole, this push in it, once their server copy has travelled
    /// (Entity.markHurt, then ServerEntity.sendChanges), and replaces what the
    /// packet would add.
    ///
    /// `cause` is the blast's source as predicates see it, read before the
    /// guests' turns swap their own combat state in.
    pub(super) fn explosion_hurt_player(
        &mut self,
        center: [f64; 3],
        radius: f32,
        damage_type: &str,
        exploding: Option<u32>,
        attacker: Option<&(u32, Text)>,
        cause: Option<&horizon_client::advancements::EntityView>,
    ) -> Option<[f64; 3]> {
        let diameter = f64::from(radius * 2.0);
        // Blast protection: `EXPLOSION_KNOCKBACK_RESISTANCE` (0.15 per level and
        // piece, so two Blast Protection IV pieces cancel it).
        let resistance = self
            .player
            .as_mut()
            .and_then(|p| p.attribute("minecraft:explosion_knockback_resistance").ok())
            .unwrap_or(0.0);
        let p = self.player.as_ref()?;
        let feet = [p.position.x, p.position.y, p.position.z];
        let dist = ((feet[0] - center[0]).powi(2)
            + (feet[1] - center[1]).powi(2)
            + (feet[2] - center[2]).powi(2))
        .sqrt()
            / diameter;
        if dist > 1.0 || p.health.dead_or_dying() {
            return None;
        }
        // ServerPlayer.onExplosionHit: the blast that hit the player is theirs.
        self.adv.marks.blast = Some((feet, cause.cloned()));
        let w = f64::from(p.dimensions.width) / 2.0;
        let h = f64::from(p.dimensions.height);
        let eye = [
            feet[0],
            feet[1] + f64::from(p.dimensions.eye_height),
            feet[2],
        ];
        let exposure = self.seen_percent(
            center,
            [feet[0] - w, feet[1], feet[2] - w],
            [feet[0] + w, feet[1] + h, feet[2] + w],
        );
        let pow = (1.0 - dist) * f64::from(exposure);
        let amount = ((pow * pow + pow) / 2.0 * 7.0 * diameter + 1.0) as f32;
        let d = [eye[0] - center[0], eye[1] - center[1], eye[2] - center[2]];
        let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let dir = if l < 1.0e-4 {
            [0.0; 3]
        } else {
            [d[0] / l, d[1] / l, d[2] / l]
        };
        let power = pow * (1.0 - resistance);
        // `source.doTeamsAllowDamage(entity)`.
        let hurtable = exploding.is_none_or(|id| {
            horizon_client::scoreboard::teams_allow_damage(self.mob_team(id), self.player_team())
        });
        let blocking = self.player_blocking(Some(center), damage_type, false);
        let raised = blocking.is_some();
        let (creative, spectator) = (self.creative, self.spectator());
        let flying = self.player.as_ref().is_some_and(|p| p.flying);
        let mut outcome = horizon_client::health::BlockOutcome::default();
        let mut pushed = None;
        // The death message names the blast's causing entity. It goes to this
        // player's tracker, the host's or a guest's.
        if let Some((id, name)) = attacker.filter(|_| !creative && !spectator && hurtable) {
            self.note_named_attacker(damage_type, *id, name.clone());
        }
        if let Some(p) = self.player.as_mut()
            && let Some(id) = self.motion.health.damage.id(damage_type)
        {
            // Creative players are invulnerable to explosions.
            if !creative
                && !spectator
                && hurtable
                && let Ok(o) = p.hurt_caused_by(
                    &self.motion.health,
                    self.world.health_rules,
                    id,
                    amount,
                    blocking,
                    attacker.is_some(),
                    // The blast's causing entity.
                    attacker.map_or(0, |(id, _)| *id as i32),
                )
            {
                outcome = o;
                // dealDefaultKnockback's `indicateDamage` points at the blast
                // (the source position), not the no-direction fallback.
                if o.knockback && o.blocked <= 0.0 {
                    p.health
                        .indicate_damage(center[0] - feet[0], center[2] - feet[2], p.yaw);
                }
            }
            if !spectator && !(creative && flying) {
                let push = [dir[0] * power, dir[1] * power, dir[2] * power];
                p.velocity = Vec3::new(
                    p.velocity.x + push[0],
                    p.velocity.y + push[1],
                    p.velocity.z + push[2],
                );
                pushed = (!p.health.sync_velocity).then_some(push);
            }
        }
        self.combat.forget_noted(outcome.canceled);
        if raised {
            let source = horizon_client::health::HurtSource {
                damage_type,
                causing: attacker.map_or(0, |(id, _)| *id as i32),
                direct: 0,
            };
            self.after_block(outcome, (None, false), source);
        }
        pushed
    }

    /// `ServerExplosion.hurtEntities` (player, mobs, items, primed TNT, minecarts);
    /// the player takes `damage_type`. Returns the players whose clients add a
    /// push from the explosion packet, and how much (`explosion_hurt_player`).
    fn explosion_hurt_entities(
        &mut self,
        center: [f64; 3],
        radius: f32,
        damage_type: &str,
        interaction: BlockInteraction,
    ) -> Vec<(u32, [f64; 3])> {
        if radius < 1.0e-5 {
            return Vec::new();
        }
        self.explosion_hits_special(center, radius);
        let diameter = f64::from(radius * 2.0);
        let distance = |p: [f64; 3]| {
            ((p[0] - center[0]).powi(2) + (p[1] - center[1]).powi(2) + (p[2] - center[2]).powi(2))
                .sqrt()
        };
        let damage = |dist: f64, exposure: f32| {
            let pow = (1.0 - dist) * f64::from(exposure);
            ((pow * pow + pow) / 2.0 * 7.0 * diameter + 1.0) as f32
        };
        let direction = |origin: [f64; 3]| {
            let d = [
                origin[0] - center[0],
                origin[1] - center[1],
                origin[2] - center[2],
            ];
            let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if l < 1.0e-4 {
                [0.0; 3]
            } else {
                [d[0] / l, d[1] / l, d[2] / l]
            }
        };
        // `source.doTeamsAllowDamage(entity)`: the exploding mob spares its
        // team mates when its team has no friendly fire (they are still pushed).
        let exploding = self.combat.explosion_source;
        let teams_allow = |g: &Self, victim: Option<&horizon_client::scoreboard::Team>| {
            exploding.is_none_or(|id| {
                horizon_client::scoreboard::teams_allow_damage(g.mob_team(id), victim)
            })
        };
        // The player, then every other player in the level (LAN guests),
        // each swapped in as the player. In a guest's turn (their hit on an
        // end crystal) the player is that guest. The blast's source is the
        // same for all of them (`onExplosionHit(this.source)`).
        let attacker = self.blast_attacker();
        let blast_source = self.combat.explosion_cause.clone();
        let own = self.explosion_hurt_player(
            center,
            radius,
            damage_type,
            exploding,
            attacker.as_ref(),
            blast_source.as_ref(),
        );
        let mut pushed: Vec<_> = self.lan_acting.zip(own).into_iter().collect();
        if let Some(mut host) = self.lan.take() {
            let guests: Vec<u32> = host
                .hub
                .guests
                .iter()
                .filter(|g| host.hub.here(g) && g.loaded && !g.dead)
                .map(|g| g.conn)
                .collect();
            for conn in guests {
                let push = self.as_guest(&mut host, conn, |g| {
                    g.explosion_hurt_player(
                        center,
                        radius,
                        damage_type,
                        exploding,
                        attacker.as_ref(),
                        blast_source.as_ref(),
                    )
                });
                if let Some(Some(push)) = push {
                    pushed.push((conn, push));
                }
            }
            self.lan = Some(host);
        }
        // Mobs. `WitherBoss.hurtServer`: a wither shrugs off an undead cause.
        let cause = self
            .combat
            .explosion_owner
            .or(exploding)
            .and_then(|id| self.mobs.iter().find(|m| m.id == id))
            .map(|m| m.species());
        // Mobs, but not the one exploding (`getEntities(source, box)`): a
        // wither's spawn blast neither hurts nor pushes it. `Warden.ignoreExplosion`:
        // nor does any blast hurt or push a digging or emerging warden.
        let spared = exploding.filter(|_| self.combat.explosion_source_named);
        let mut mobs = std::mem::take(&mut self.mobs);
        for m in mobs.iter_mut().filter(|m| {
            m.alive()
                && spared != Some(m.id)
                && !(m.species() == horizon_client::mobs::Species::Warden
                    && horizon_client::mobs::warden::digging_or_emerging(m))
        }) {
            let hurtable = teams_allow(self, self.mob_team(m.id));
            let feet = [m.position.x, m.position.y, m.position.z];
            let dist = distance(feet) / diameter;
            if dist > 1.0 {
                continue;
            }
            let (w, h) = (m.kind.width / 2.0, m.kind.height);
            let eye = [feet[0], feet[1] + m.kind.eye_height, feet[2]];
            let exposure = self.seen_percent(
                center,
                [feet[0] - w, feet[1], feet[2] - w],
                [feet[0] + w, feet[1] + h, feet[2] + w],
            );
            // The exploding mob is the damage source's entity, so victims
            // it kills drop its charged-creeper loot (`Creeper.killedEntity`).
            let source = horizon_client::mobs::Damage {
                kind: horizon_client::mobs::DamageKind::Explosion,
                // A projectile's owner (`getIndirectSourceEntity`), else
                // the exploding mob itself.
                attacker: attacker
                    .as_ref()
                    .map(|&(id, _)| id)
                    .filter(|&o| o != m.id)
                    .map(horizon_client::mobs::Who::Mob),
                from: Some(center),
            };
            // `EXPLOSION_KNOCKBACK_RESISTANCE`, not `KNOCKBACK_RESISTANCE`: its
            // base is 0 for every mob type, so iron golems and ravagers are pushed.
            let dir = direction(eye);
            let power = (1.0 - dist) * f64::from(exposure);
            hurt_and_push_mob(
                m,
                hurtable,
                cause,
                (damage(dist, exposure), source),
                [dir[0] * power, dir[1] * power, dir[2] * power],
            );
        }
        self.mobs = mobs;
        // Dropped items: 5 health, destroyed by the explosion damage (nether stars resist).
        // `ItemEntity.ignoreExplosion` asks `shouldAffectBlocklikeEntities`
        // first, so a blast that keeps blocks leaves every item untouched.
        let items_ignore = items_ignore_explosion(self.ui.rules.bool("mob_griefing"), interaction);
        let before = self.dropped_ids();
        let mut items = std::mem::take(&mut self.dropped);
        items.retain_mut(|item| {
            if items_ignore {
                return true;
            }
            let feet = [item.position.x, item.position.y, item.position.z];
            let dist = distance(feet) / diameter;
            if dist > 1.0 {
                return true;
            }
            let half = 0.125;
            let exposure = self.seen_percent(
                center,
                [feet[0] - half, feet[1], feet[2] - half],
                [feet[0] + half, feet[1] + 0.25, feet[2] + half],
            );
            if &*item.stack.item != "minecraft:nether_star" && damage(dist, exposure) >= 5.0 {
                return false;
            }
            let dir = direction([feet[0], feet[1] + 0.2125, feet[2]]);
            let power = (1.0 - dist) * f64::from(exposure);
            item.velocity = Vec3::new(
                item.velocity.x + dir[0] * power,
                item.velocity.y + dir[1] * power,
                item.velocity.z + dir[2] * power,
            );
            true
        });
        self.dropped = items;
        self.native_items_left_since(&before);
        // Experience orbs: `ExperienceOrb.hurtServer` takes the damage off
        // their 5 health, and `pushFromExplosion` moves the ones that stay
        // (a 0.5 x 0.5 box, the eyes at 0.85 of its height).
        let orbs_before = self.orb_ids();
        let mut orbs = std::mem::take(&mut self.orbs);
        orbs.retain_mut(|o| {
            let feet = [o.position.x, o.position.y, o.position.z];
            let dist = distance(feet) / diameter;
            if dist > 1.0 {
                return true;
            }
            let half = 0.25;
            let exposure = self.seen_percent(
                center,
                [feet[0] - half, feet[1], feet[2] - half],
                [feet[0] + half, feet[1] + 0.5, feet[2] + half],
            );
            if o.hurt(damage(dist, exposure)) {
                return false;
            }
            let dir = direction([feet[0], feet[1] + f64::from(0.5f32 * 0.85f32), feet[2]]);
            let power = (1.0 - dist) * f64::from(exposure);
            o.velocity = Vec3::new(
                o.velocity.x + dir[0] * power,
                o.velocity.y + dir[1] * power,
                o.velocity.z + dir[2] * power,
            );
            true
        });
        self.orbs = orbs;
        self.native_orbs_left_since(&orbs_before);
        // Primed TNT: knockback only, from its position (not its eyes).
        let mut tnts = std::mem::take(&mut self.tnts);
        for t in &mut tnts {
            let feet = [t.position.x, t.position.y, t.position.z];
            let dist = distance(feet) / diameter;
            if dist > 1.0 {
                continue;
            }
            let half = 0.49;
            let exposure = self.seen_percent(
                center,
                [feet[0] - half, feet[1], feet[2] - half],
                [feet[0] + half, feet[1] + 0.98, feet[2] + half],
            );
            let dir = direction(feet);
            let power = (1.0 - dist) * f64::from(exposure);
            t.velocity = Vec3::new(
                t.velocity.x + dir[0] * power,
                t.velocity.y + dir[1] * power,
                t.velocity.z + dir[2] * power,
            );
        }
        self.tnts = tnts;
        // `VehicleEntity.ignoreExplosion`: a mob's blast (not a primed TNT's)
        // spares boats and minecarts while mob griefing is off.
        let vehicles_ignore = self.combat.blast_owner.is_none()
            && self.blast_attacker().is_some()
            && !self.ui.rules.bool("mob_griefing");
        // Minecarts: VehicleEntity.hurtServer (explosions ignite TNT carts),
        // then the push from their eyes (0.85 of their height).
        let ids: Vec<u32> = if vehicles_ignore {
            Vec::new()
        } else {
            self.carts.iter().map(|c| c.id).collect()
        };
        for id in ids {
            let Some(i) = self.carts.iter().position(|c| c.id == id) else {
                continue;
            };
            let bounds = self.carts[i].bounds();
            let feet = self.carts[i].position;
            let dist = distance([feet.x, feet.y, feet.z]) / diameter;
            if dist > 1.0 {
                continue;
            }
            let (lo, hi) = (bounds.min_position(), bounds.max_position());
            let exposure = self.seen_percent(center, [lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]);
            self.hurt_cart(i, damage(dist, exposure), false, true);
            let Some(c) = self.carts.iter_mut().find(|c| c.id == id) else {
                continue;
            };
            let eye_height = f64::from((hi.y - lo.y) as f32 * 0.85f32);
            let dir = direction([feet.x, feet.y + eye_height, feet.z]);
            let power = (1.0 - dist) * f64::from(exposure);
            c.velocity = Vec3::new(
                c.velocity.x + dir[0] * power,
                c.velocity.y + dir[1] * power,
                c.velocity.z + dir[2] * power,
            );
        }
        // Boats: `VehicleEntity.hurtServer` (damage * 10, broken past 40), then the
        // push from their eyes (`EntityType.OAK_BOAT & co`: the full 0.5625).
        let ids: Vec<u32> = if vehicles_ignore {
            Vec::new()
        } else {
            self.boats.iter().map(|b| b.id).collect()
        };
        for id in ids {
            let Some(i) = self.boats.iter().position(|b| b.id == id) else {
                continue;
            };
            let bounds = self.boats[i].bounds();
            let feet = self.boats[i].position;
            let dist = distance([feet.x, feet.y, feet.z]) / diameter;
            if dist > 1.0 {
                continue;
            }
            let (lo, hi) = (bounds.min_position(), bounds.max_position());
            let exposure = self.seen_percent(center, [lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]);
            self.hurt_boat(i, damage(dist, exposure), false);
            let Some(b) = self.boats.iter_mut().find(|b| b.id == id) else {
                continue;
            };
            let dir = direction([feet.x, feet.y + 0.5625, feet.z]);
            let power = (1.0 - dist) * f64::from(exposure);
            b.velocity = Vec3::new(
                b.velocity.x + dir[0] * power,
                b.velocity.y + dir[1] * power,
                b.velocity.z + dir[2] * power,
            );
        }
        // Falling blocks: `Entity.hurtServer` does nothing, `pushFromExplosion`
        // moves them from their eyes (0.85 of the 0.98 box).
        let mut falling = std::mem::take(&mut self.falling);
        for f in &mut falling {
            let feet = f.position;
            let dist = distance([feet.x, feet.y, feet.z]) / diameter;
            if dist > 1.0 {
                continue;
            }
            let b = horizon_client::falling_block::bounds(feet);
            let (lo, hi) = (b.min_position(), b.max_position());
            let exposure = self.seen_percent(center, [lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]);
            let eye = f64::from((hi.y - lo.y) as f32 * 0.85f32);
            let dir = direction([feet.x, feet.y + eye, feet.z]);
            let power = (1.0 - dist) * f64::from(exposure);
            f.velocity = Vec3::new(
                f.velocity.x + dir[0] * power,
                f.velocity.y + dir[1] * power,
                f.velocity.z + dir[2] * power,
            );
        }
        self.falling = falling;
        // Arrows, fire charges, snowballs and the rest: `Entity.pushFromExplosion`
        // (`push`) from their eyes (0.85 of their height). `AbstractArrow.push`
        // does nothing while it is in the ground; `hurtServer` never hurts them.
        let mut projectiles = std::mem::take(&mut self.creatures.projectiles);
        for p in projectiles
            .iter_mut()
            .filter(|p| !p.stuck && !p.kind.ignores_blast())
        {
            let (lo, hi) = (p.bounds().min_position(), p.bounds().max_position());
            let dist = distance([p.position.x, p.position.y, p.position.z]) / diameter;
            if dist > 1.0 {
                continue;
            }
            let exposure = self.seen_percent(center, [lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]);
            let eye_height = p.kind.eye_height();
            let dir = direction([p.position.x, p.position.y + eye_height, p.position.z]);
            let power = (1.0 - dist) * f64::from(exposure);
            p.velocity = Vec3::new(
                p.velocity.x + dir[0] * power,
                p.velocity.y + dir[1] * power,
                p.velocity.z + dir[2] * power,
            );
            p.needs_sync = true;
        }
        self.creatures.projectiles = projectiles;
        // Firework rockets (0.25 wide, so 0.2125 to the eyes).
        let mut rockets = std::mem::take(&mut self.rockets);
        for r in &mut rockets {
            let feet = r.position;
            let dist = distance([feet.x, feet.y, feet.z]) / diameter;
            if dist > 1.0 {
                continue;
            }
            let exposure = self.seen_percent(
                center,
                [feet.x - 0.125, feet.y, feet.z - 0.125],
                [feet.x + 0.125, feet.y + 0.25, feet.z + 0.125],
            );
            let dir = direction([feet.x, feet.y + 0.2125, feet.z]);
            let power = (1.0 - dist) * f64::from(exposure);
            r.velocity = Vec3::new(
                r.velocity.x + dir[0] * power,
                r.velocity.y + dir[1] * power,
                r.velocity.z + dir[2] * power,
            );
        }
        self.rockets = rockets;
        // Thrown tridents live apart from the other projectiles (`game/trident.rs`): 0.5
        // wide and high, eyes at 0.13 (`EntityType.TRIDENT`), and like any arrow
        // (`AbstractArrow.push`) not pushed while in the ground.
        let mut tridents = std::mem::take(&mut self.tridents);
        for t in tridents.iter_mut().filter(|t| !t.in_ground) {
            let feet = t.position;
            let dist = distance([feet.x, feet.y, feet.z]) / diameter;
            if dist > 1.0 {
                continue;
            }
            let exposure = self.seen_percent(
                center,
                [feet.x - 0.25, feet.y, feet.z - 0.25],
                [feet.x + 0.25, feet.y + 0.5, feet.z + 0.25],
            );
            let dir = direction([feet.x, feet.y + f64::from(0.13_f32), feet.z]);
            let power = (1.0 - dist) * f64::from(exposure);
            t.velocity = Vec3::new(
                t.velocity.x + dir[0] * power,
                t.velocity.y + dir[1] * power,
                t.velocity.z + dir[2] * power,
            );
        }
        self.tridents = tridents;
        // Frames, paintings, cushions and armour stands last: vanilla lists
        // the entities in reach before hurting any, so the items they drop
        // here are neither destroyed nor pushed by the same blast.
        self.explosion_breaks_frames(center, diameter, interaction);
        self.explosion_breaks_decorations(center, diameter, interaction);
        pushed
    }
    /// `blast_spares_decorations` for the blast going off now: its causing
    /// mob (`blast_attacker`) is the source's entity.
    pub(super) fn blast_spares_decorations(&self, interaction: BlockInteraction) -> bool {
        let by_mob = self.blast_attacker().is_some();
        blast_spares_decorations(
            self.ui.rules.bool("mob_griefing"),
            interaction,
            by_mob,
            self.combat.explosion_in_water,
        )
    }
    /// `Entity.isInWater` for an entity box: `updateFluidInteraction` over the box.
    pub(super) fn bounds_in_water(&self, bounds: horizon_core::geometry::Aabb) -> bool {
        self.mob_world_base(&[])
            .fluids(bounds, bounds.max_position().y)
            .in_water
    }
    /// `ServerExplosion.hurtEntities` for paintings, cushions and armour
    /// stands (item frames: `explosion_breaks_frames`). A painting or cushion
    /// in reach is killed and dropped (`BlockAttachedEntity.hurtServer`,
    /// `Painting.dropItem`, `Cushion.dropItem`). An armour stand that is
    /// neither invisible nor a marker breaks (`ArmorStand.hurtServer` with
    /// `IS_EXPLOSION`: `brokenByAnything`, then `kill`): its equipment drops,
    /// the stand itself does not.
    fn explosion_breaks_decorations(
        &mut self,
        center: [f64; 3],
        diameter: f64,
        interaction: BlockInteraction,
    ) {
        // An armour stand is no `BlockAttachedEntity`: a source in water does not spare it.
        let by_mob = self.blast_attacker().is_some();
        let spares_stands = blast_spares_decorations(
            self.ui.rules.bool("mob_griefing"),
            interaction,
            by_mob,
            false,
        );
        if !self.blast_spares_decorations(interaction) {
            self.explosion_breaks_hanging(center, diameter);
        }
        if spares_stands {
            return;
        }
        let mut i = 0;
        while i < self.armor_stands.len() {
            let a = &self.armor_stands[i];
            let p = a.position;
            if !super::armor_stands::spared_by_blast(a)
                && blast_reaches(center, diameter, [p.x, p.y, p.z])
            {
                self.break_armor_stand_by_anything(i);
            } else {
                i += 1;
            }
        }
    }
    /// Paintings and cushions in reach (`BlockAttachedEntity.hurtServer`).
    fn explosion_breaks_hanging(&mut self, center: [f64; 3], diameter: f64) {
        let mut i = 0;
        while i < self.paintings.len() {
            // HangingEntity.recalculateBoundingBox: a painting stands at the
            // centre of its box.
            let (lo, hi) = self.paintings[i].bounds();
            let at = [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2.0);
            if blast_reaches(center, diameter, at) {
                self.break_painting(i, false);
            } else {
                i += 1;
            }
        }
        let mut i = 0;
        while i < self.cushions.len() {
            let p = self.cushions[i].position;
            if blast_reaches(center, diameter, [p.x, p.y, p.z]) {
                self.break_cushion(i, false);
            } else {
                i += 1;
            }
        }
    }
    /// `ServerLevel.getDestroyType`: a drop decay rule picks between decaying
    /// and plain destruction.
    pub(super) fn destroy_type(&self, rule: &str) -> BlockInteraction {
        if self.ui.rules.bool(rule) {
            BlockInteraction::DestroyWithDecay
        } else {
            BlockInteraction::Destroy
        }
    }
    /// Runs an explosion whose source empties the hives it destroys
    /// (`BeehiveBlock.getDrops`: primed TNT, creeper, wither skull, wither,
    /// TNT minecart).
    pub(super) fn with_hive_emptying_blast<R>(&mut self, blast: impl FnOnce(&mut Self) -> R) -> R {
        self.combat.hive_emptying_blast = true;
        let result = blast(self);
        self.combat.hive_emptying_blast = false;
        result
    }
    /// `Level.explode` on the server: positions, entities, blocks, fire; then
    /// the client's explosion sound.
    pub(super) fn server_explode(
        &mut self,
        center: [f64; 3],
        radius: f32,
        interaction: BlockInteraction,
        fire: bool,
    ) -> Result<()> {
        let damage_type = super::creatures::damage_type(
            horizon_client::mobs::DamageKind::Explosion,
            self.blast_attacker().is_some(),
        );
        self.server_explode_as(center, radius, interaction, fire, damage_type, None)
    }
    /// `server_explode` with the damage type the player takes and a block
    /// whose explosion resistance the damage calculator overrides.
    pub(super) fn server_explode_as(
        &mut self,
        center: [f64; 3],
        radius: f32,
        interaction: BlockInteraction,
        fire: bool,
        damage_type: &str,
        resists: Option<([i32; 3], f32)>,
    ) -> Result<()> {
        if let Some(p) = &self.player {
            let d = [
                p.position.x - center[0],
                p.position.y - center[1],
                p.position.z - center[2],
            ];
            let distance = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            self.pad_rumble(horizon_client::gamepad::Rumble::Explosion { distance, radius });
        }
        // Explosion.explode: the blast is heard.
        let ctx = horizon_client::sculk::Context::default();
        self.post_game_event("minecraft:explode", center, ctx);
        let mut targets = self.exploded_positions(center, radius, resists);
        let dimension = self.dim.id();
        self.native_explosion_detonate(dimension, center, &mut targets);
        let pushed = self.explosion_hurt_entities(center, radius, damage_type, interaction);
        if interaction != BlockInteraction::Keep {
            // Util.shuffle(list, level.random)
            for i in (2..=targets.len()).rev() {
                let swap = self.world.random.next_int_bound(i as i32).unwrap_or(0) as usize;
                targets.swap(i - 1, swap);
            }
            let mut stacks: Vec<([i32; 3], ItemStack)> = Vec::new();
            for &pos in &targets {
                self.explosion_hit_block(pos, radius, interaction, &mut stacks)?;
            }
            for (pos, stack) in stacks {
                self.pop_resource(pos, stack);
            }
        }
        if fire {
            for &pos in &targets {
                if self.world.random.next_int_bound(3).unwrap_or(1) == 0
                    && self
                        .res
                        .facts()
                        .is_air(self.world.block(pos[0], pos[1], pos[2]))
                    && self
                        .res
                        .facts()
                        .get(self.world.block(pos[0], pos[1] - 1, pos[2]))
                        .solid_render
                {
                    // BaseFireBlock.getState: soul fire on its base blocks,
                    // fire shaped by its neighbours elsewhere.
                    let at = BlockPos::new(pos[0], pos[1], pos[2]);
                    let fire =
                        self.with_level(|b, level, ticks| b.fire_state_for(level, ticks, at));
                    self.level_set(pos, fire)?;
                }
            }
        }
        let pitch = (1.0 + (client_float() - client_float()) * 0.2) * 0.7;
        self.audio
            .play("minecraft:entity.generic.explode", Some(center), 4.0, pitch);
        // ServerLevel.explode sends toBlow.size() whatever the interaction, and
        // `ServerExplosion.isSmall` also covers an explosion that keeps blocks.
        let block_count = targets.len();
        let small = radius < 2.0 || interaction == BlockInteraction::Keep;
        let (world, facts) = (&self.world, self.res.facts());
        self.particles
            .explosion(center, small, block_count, radius, &|b| {
                facts.is_air(world.block(b[0], b[1], b[2]))
            });
        self.lan_explode(center, small, radius, block_count, pushed);
        Ok(())
    }
    /// `BlockBehaviour.onExplosionHit` (TNT: `wasExploded` primes a short fuse).
    fn explosion_hit_block(
        &mut self,
        pos: [i32; 3],
        radius: f32,
        interaction: BlockInteraction,
        stacks: &mut Vec<([i32; 3], ItemStack)>,
    ) -> Result<()> {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        if self.res.facts().is_air(state) {
            return Ok(());
        }
        let name = self.res.facts().get(state).name.clone();
        let is_tnt = &*name == "minecraft:tnt";
        // CreakingHeartBlock.onExplosionHit: its creaking dies (no
        // experience, as Horizon does not track a TNT's owner).
        if &*name == "minecraft:creaking_heart" {
            self.creaking_heart_broken(pos, false);
        }
        // Block.onExplosionHit: the drops copy from the block entity, which
        // then spills its contents as the block turns to air.
        let key = (pos[0], pos[1], pos[2]);
        let is_hive = matches!(&*name, "minecraft:beehive" | "minecraft:bee_nest");
        // BeehiveBlock.getDrops: the bees of a hive blown up by TNT, a
        // creeper, a wither skull, a wither or a TNT minecart come out.
        if is_hive && self.combat.hive_emptying_blast {
            let facing = self
                .pack
                .assets
                .registry
                .state(state)
                .and_then(|s| s.properties.get("facing").cloned());
            self.empty_hive(
                pos,
                horizon_client::beehive::facing_step(facing.as_deref()),
                horizon_client::beehive::Release::Emergency,
            );
        }
        if let Some(entity) = self.world.block_entities.get(&key).cloned() {
            let custom = self.world.block_names.get(&key).cloned();
            self.pending.record(pos, &entity, &name, custom);
        } else if let Some(tag) = super::spill::saved_entity(&self.world, pos) {
            self.pending.record_saved(pos, &tag);
        }
        if !is_tnt {
            let dynamic = self.pending.dynamic(pos);
            // LootContextParams.EXPLOSION_RADIUS under DESTROY_WITH_DECAY: the
            // tables' survives_explosion and explosion_decay draw from their
            // random sequence.
            let decay = (interaction == BlockInteraction::DestroyWithDecay).then_some(radius);
            let place = self.loot_place_at(
                pos.map(|v| f64::from(v) + 0.5),
                self.loot_block_entity(pos, &name),
            );
            let mut drops = horizon_world::loot::with_native_loot_place(place, || {
                world_items!(self).block_drops_exploded(
                    self.pack.assets.registry,
                    state,
                    None,
                    &dynamic,
                    &mut self.world.random,
                    Some((&mut self.rand_sequences, self.level.seed)),
                    decay,
                    false,
                )
            });
            self.native_block_drops(pos, state, None, None, Some(radius), &mut drops);
            for mut stack in drops {
                if &*stack.item == "minecraft:air" || stack.count == 0 {
                    continue;
                }
                self.pending.attach(pos, &mut stack);
                let items = self.res.items();
                add_or_append_stack(stacks, stack, pos, &|s| {
                    items.get(&s.item).map_or(64, |i| i.max_stack)
                });
            }
        }
        self.pending.clear(pos);
        self.level_set(pos, self.world.air)?;
        // BeehiveBlock.onExplosionHit: the bees about turn on the player.
        if is_hive {
            self.anger_nearby_bees(pos);
        }
        // TntBlock.wasExploded: new PrimedTnt (its push drawn first), then
        // getRandomShortFuse(80); no game event.
        if is_tnt && self.ui.rules.bool("tnt_explodes") {
            let r = &mut self.world.random;
            let rot = r.next_double() * f64::from(std::f32::consts::TAU);
            let fuse = r.next_int_bound(20).unwrap_or(0) + 10;
            // TntBlock.wasExploded hands the new TNT the blast's indirect
            // source, so a chain reaction keeps the player who started it.
            let (owner, guest) = match &self.combat.blast_owner {
                Some(_) => (Some(Who::Player), self.combat.blast_guest),
                None => (None, None),
            };
            self.add_primed_tnt(
                BlockPos::new(pos[0], pos[1], pos[2]),
                rot,
                fuse,
                owner,
                guest,
            );
        }
        Ok(())
    }
}

/// `ServerExplosion.addOrAppendStack`: `ItemEntity.areMergable` (same item and
/// components, combined count within the item's max stack size), then
/// `ItemEntity.merge(.., 16)`.
fn add_or_append_stack(
    stacks: &mut Vec<([i32; 3], ItemStack)>,
    mut stack: ItemStack,
    pos: [i32; 3],
    max_stack: &dyn Fn(&ItemStack) -> u32,
) {
    let max = i64::from(max_stack(&stack));
    for (_, existing) in stacks.iter_mut() {
        if existing.same_item_same_components(&stack)
            && i64::from(existing.count) + i64::from(stack.count) <= max
        {
            let moved = (max.min(16) - i64::from(existing.count))
                .min(i64::from(stack.count))
                .max(0) as u32;
            existing.count += moved;
            stack.count -= moved;
            if stack.count == 0 {
                return;
            }
        }
    }
    stacks.push((pos, stack));
}

/// `WitherSkull.getBlockExplosionResistance`: `isDangerous() &&
/// WitherBoss.canDestroy(state) ? min(0.8, resistance) : resistance`. Air has
/// no resistance to begin with, and `#wither_immune` blocks keep theirs.
fn blast_resistance(base: f32, cap: Option<f32>, wither_immune: bool) -> f32 {
    match cap {
        Some(cap) if !wither_immune => base.min(cap),
        _ => base,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dangerous wither skull's blast treats a block as at most 0.8 harder to
    /// break, but a `#wither_immune` block keeps its own resistance and any
    /// other blast is unchanged.
    #[test]
    fn a_dangerous_wither_skull_caps_block_resistance() {
        assert_eq!(blast_resistance(4.0, Some(0.8), false), 0.8);
        assert_eq!(blast_resistance(0.3, Some(0.8), false), 0.3, "already low");
        assert_eq!(blast_resistance(4.0, Some(0.8), true), 4.0, "wither immune");
        assert_eq!(blast_resistance(4.0, None, false), 4.0, "another blast");
    }

    /// A dropped item survives a mob's blast untouched while `mobGriefing` is
    /// off, and is thrown and hurt as usual by one that destroys blocks or by
    /// any blast with the rule back on.
    #[test]
    fn items_ignore_a_blast_that_keeps_blocks_with_griefing_off() {
        assert!(items_ignore_explosion(false, BlockInteraction::Keep));
        assert!(!items_ignore_explosion(false, BlockInteraction::Destroy));
        assert!(!items_ignore_explosion(
            false,
            BlockInteraction::DestroyWithDecay
        ));
        assert!(!items_ignore_explosion(true, BlockInteraction::Keep));
    }

    #[test]
    fn blasts_reach_entities_within_their_diameter() {
        // A TNT blast: radius 4, so the entity pass reaches 8 blocks out.
        let center = [0.5, 64.0, 0.5];
        assert!(blast_reaches(center, 8.0, [8.5, 64.0, 0.5]));
        assert!(blast_reaches(center, 8.0, [-3.5, 68.0, 4.5]));
        assert!(!blast_reaches(center, 8.0, [8.6, 64.0, 0.5]));
    }

    /// A zombie's type with only what a hurt and a push read.
    fn zombie_type() -> std::sync::Arc<horizon_client::mobs::MobType> {
        use horizon_client::mobs::{Anim, Brain, MobType, Part, Species};
        std::sync::Arc::new(MobType {
            id: "minecraft:zombie".into(),
            species: Species::Zombie,
            width: 0.6,
            height: 1.95,
            eye_height: 1.74,
            category: "monster".into(),
            max_health: 20.0,
            speed: 0.23,
            flying_speed: 0.02,
            attack_damage: 3.0,
            attack_knockback: 0.0,
            follow_range: 35.0,
            step_height: 0.6,
            armor: 2.0,
            armor_toughness: 0.0,
            knockback_resistance: 0.0,
            jump_strength: 0.42,
            attributes: Default::default(),
            gravity: 0.08,
            safe_fall_distance: 3.0,
            fall_damage_multiplier: 1.0,
            water_efficiency: 0.0,
            water_slowdown: 0.8,
            air_speed: 0.02,
            baby_scale: 0.5,
            head_rot_speed: 10.0,
            max_head_x_rot: 40.0,
            max_head_y_rot: 75.0,
            ambient_interval: 80,
            flags: Default::default(),
            model: std::sync::Arc::new(Part {
                name: "root".into(),
                pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                polygons: Vec::new(),
                children: Vec::new(),
            }),
            layers: Default::default(),
            texture: String::new(),
            brain: Brain::Passive,
            anim: Anim::Humanoid { zombie_arms: true },
            sounds: ["", "", ""],
            step_sound: None,
            food: Default::default(),
            tempt: Default::default(),
        })
    }

    #[test]
    fn a_mob_that_refuses_the_hit_is_still_pushed() {
        use horizon_client::mobs::{Damage, DamageKind, Mob, Species};
        let hit = (10.0, Damage::of(DamageKind::Explosion));
        let push = [0.5, 0.25, -0.5];
        let mob = |id, species| {
            let mut kind = (*zombie_type()).clone();
            kind.species = species;
            Mob::new(id, std::sync::Arc::new(kind), Vec3::ZERO, 0.0)
        };
        let pushed = |m: &Mob| {
            assert_eq!(m.velocity, Vec3::new(0.5, 0.25, -0.5));
            assert!(m.needs_sync);
        };
        // An ordinary mob is hurt and pushed.
        let mut m = mob(1, Species::Zombie);
        let health = m.health;
        hurt_and_push_mob(&mut m, true, None, hit, push);
        assert!(m.health < health);
        pushed(&m);
        // An invulnerable one refuses the hit, and is pushed all the same.
        let mut m = mob(2, Species::Zombie);
        m.invulnerable_flag = true;
        let health = m.health;
        hurt_and_push_mob(&mut m, true, None, hit, push);
        assert_eq!(m.health, health);
        pushed(&m);
        // A team mate of the exploding mob is not hurt, but pushed.
        let mut m = mob(3, Species::Zombie);
        let health = m.health;
        hurt_and_push_mob(&mut m, false, None, hit, push);
        assert_eq!(m.health, health);
        pushed(&m);
        // A wither shrugs off a blast an undead caused, and is pushed.
        let mut m = mob(4, Species::Wither);
        let health = m.health;
        hurt_and_push_mob(&mut m, true, Some(Species::Skeleton), hit, push);
        assert_eq!(m.health, health);
        pushed(&m);
    }

    #[test]
    fn mob_griefing_off_spares_decorations_from_kept_and_mob_blasts() {
        use BlockInteraction::{Destroy, DestroyWithDecay, Keep};
        // mobGriefing on: every blast breaks them, a creeper's included.
        assert!(!blast_spares_decorations(
            true,
            DestroyWithDecay,
            true,
            false
        ));
        assert!(!blast_spares_decorations(true, Keep, true, false));
        // Off: TNT still breaks them (ExplosionInteraction.TNT destroys blocks).
        assert!(!blast_spares_decorations(false, Destroy, false, false));
        assert!(!blast_spares_decorations(
            false,
            DestroyWithDecay,
            false,
            false
        ));
        // Off: a blast that keeps blocks, or one a mob causes, does not.
        assert!(blast_spares_decorations(false, Keep, false, false));
        assert!(blast_spares_decorations(false, Destroy, true, false));
        // A source in water spares them whatever the rules (`BlockAttachedEntity.ignoreExplosion`).
        assert!(blast_spares_decorations(
            true,
            DestroyWithDecay,
            false,
            true
        ));
        assert!(blast_spares_decorations(true, Destroy, true, true));
    }

    /// The type a blast hurts with and the attacker its death message names
    /// come from one cause (`DamageSources.explosion`): `player_explosion`
    /// "blown up by" that mob, or `explosion` with no one when it has none.
    fn blast_hurt(
        owner: Option<u32>,
        exploding: Option<u32>,
        live: &[(u32, Text)],
        exploded: &[(u32, Text)],
    ) -> (&'static str, Option<(u32, Text)>) {
        let find = |id| live.iter().find(|(m, _)| *m == id).map(|(_, n)| n.clone());
        let cause = blast_cause(owner, exploding, find, exploded);
        let kind = horizon_client::mobs::DamageKind::Explosion;
        (
            super::super::creatures::damage_type(kind, cause.is_some()),
            cause,
        )
    }

    #[test]
    fn a_caused_blast_names_its_cause_under_player_explosion() {
        let creeper = Text::translate("entity.minecraft.creeper", &[]);
        let ghast = Text::translate("entity.minecraft.ghast", &[]);
        // A creeper's blast: it left the mob list, its event named it.
        assert_eq!(
            blast_hurt(None, Some(7), &[], &[(7, creeper.clone())]),
            ("minecraft:player_explosion", Some((7, creeper)))
        );
        // A ghast's fireball: its owner is still in the level.
        assert_eq!(
            blast_hurt(Some(3), None, &[(3, ghast.clone())], &[]),
            ("minecraft:player_explosion", Some((3, ghast.clone())))
        );
        // The ghast was removed before its fireball landed: no owner.
        assert_eq!(
            blast_hurt(Some(3), None, &[], &[]),
            ("minecraft:explosion", None)
        );
        // Nothing caused it.
        assert_eq!(
            blast_hurt(None, None, &[], &[]),
            ("minecraft:explosion", None)
        );
        // A projectile's owner wins over the exploding mob, and a removed
        // owner gives no cause even when a mob is exploding.
        let zombie = Text::translate("entity.minecraft.zombie", &[]);
        let live = [(3, ghast.clone()), (5, zombie.clone())];
        assert_eq!(
            blast_hurt(Some(3), Some(5), &live, &[]),
            ("minecraft:player_explosion", Some((3, ghast)))
        );
        assert_eq!(
            blast_hurt(Some(4), Some(5), &live, &[]),
            ("minecraft:explosion", None)
        );
        // A mob still in the level is named as it is now, before its name
        // in the exploded list.
        let named = Text::literal("Bob".to_owned());
        assert_eq!(
            blast_hurt(None, Some(5), &live, &[(5, named)]),
            ("minecraft:player_explosion", Some((5, zombie)))
        );
    }

    #[test]
    fn block_pos_hash_matches_vec3i() {
        // Vec3i.hashCode: (y + z * 31) * 31 + x
        assert_eq!(Key([1, 2, 3]).java_hash(), (2 + 3 * 31) * 31 + 1);
        assert_eq!(Key([-5, 64, -7]).java_hash(), (64 - 7 * 31) * 31 - 5);
        assert_eq!(
            Key([0, 1, 0]).comparable_compare(&Key([5, 0, 9])),
            Some(Ordering::Greater)
        );
        assert_eq!(
            Key([1, 0, 2]).comparable_compare(&Key([0, 0, 3])),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn drops_merge_up_to_sixteen() {
        let stack = |item: &str| ItemStack {
            item: item.into(),
            count: 1,
            damage: 0,
            extra: None,
        };
        let max = |s: &ItemStack| {
            if &*s.item == "minecraft:diamond_sword" {
                1
            } else {
                64
            }
        };
        let mut stacks = Vec::new();
        for _ in 0..20 {
            add_or_append_stack(&mut stacks, stack("minecraft:dirt"), [0, 0, 0], &max);
        }
        add_or_append_stack(&mut stacks, stack("minecraft:stone"), [1, 0, 0], &max);
        let counts: Vec<u32> = stacks.iter().map(|(_, s)| s.count).collect();
        assert_eq!(counts, vec![16, 4, 1]);
    }

    #[test]
    fn unstackable_drops_stay_separate() {
        let sword = ItemStack {
            item: "minecraft:diamond_sword".into(),
            count: 1,
            damage: 0,
            extra: None,
        };
        let max = |_: &ItemStack| 1;
        let mut stacks = Vec::new();
        add_or_append_stack(&mut stacks, sword.clone(), [0, 0, 0], &max);
        add_or_append_stack(&mut stacks, sword, [0, 0, 0], &max);
        let counts: Vec<u32> = stacks.iter().map(|(_, s)| s.count).collect();
        assert_eq!(counts, vec![1, 1]);
    }

    #[test]
    fn drops_with_different_damage_stay_separate() {
        let pick = |damage| ItemStack {
            item: "minecraft:iron_pickaxe".into(),
            count: 1,
            damage,
            extra: None,
        };
        let max = |_: &ItemStack| 64;
        let mut stacks = Vec::new();
        add_or_append_stack(&mut stacks, pick(0), [0, 0, 0], &max);
        add_or_append_stack(&mut stacks, pick(5), [0, 0, 0], &max);
        assert_eq!(stacks.len(), 2);
    }
}
