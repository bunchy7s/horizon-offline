//! Wind charges: the item a player throws (`WindChargeItem`), the charges
//! breezes shoot, and the burst they make (`ServerExplosion` with
//! `TRIGGER_BLOCK`: no damage, only knockback and the blocks that react).
use super::Game;
use horizon_client::{
    Result,
    mobs::{Damage, DamageKind, Species, Who},
    particles::{ParticleOptions, ParticleType},
    projectiles::{Hit, Projectile, ProjectileKind},
};
use horizon_core::{
    geometry::{Direction, Vec3},
    position::BlockPos,
    random::RandomSource,
};
use horizon_world::level::PlayerContext;
use std::collections::BTreeSet;

/// `WindCharge.RADIUS` and the knockback multiplier of its damage calculator.
const PLAYER_RADIUS: f32 = 1.2;
const PLAYER_KNOCKBACK: f32 = 1.22;
/// `BreezeWindCharge.RADIUS`.
const BREEZE_RADIUS: f32 = 3.0;

/// A burst `wind_burst` puts off until a guest's turn is over.
pub(super) struct Burst {
    center: [f64; 3],
    radius: f32,
    multiplier: f32,
    breeze: bool,
    wind_charge: bool,
}
/// `Projectile.deflect` with `ProjectileDeflection.REVERSE`.
pub(super) fn reverse(p: &mut Projectile, jitter: f32) {
    p.velocity = Vec3::new(
        p.velocity.x * -0.5,
        p.velocity.y * -0.5,
        p.velocity.z * -0.5,
    );
    p.yaw += 170.0 + jitter * 20.0;
}
/// Whether a block reacts to a wind charge's burst by being used.
fn triggers(name: &str) -> bool {
    let n = name.trim_start_matches("minecraft:");
    // Doors and trapdoors open unless iron; gates, buttons, levers and bells react.
    (n.ends_with("_door") || n.ends_with("_trapdoor")) && !n.starts_with("iron_")
        || n.ends_with("_fence_gate")
        || n.ends_with("_button")
        || n == "lever"
        || n == "bell"
}
impl Game<'_> {
    /// `WindChargeItem.use`.
    pub(super) fn throw_wind_charge(&mut self) -> Result<bool> {
        let (Some(eye), Some(p)) = (self.eye_position(), self.player.as_ref()) else {
            return Ok(false);
        };
        let (on_ground, pos) = (p.ground.on_ground, p.position);
        let velocity = self.known_player_movement();
        let start = Vec3::new(pos.x, eye[1], pos.z);
        let mut proj = Projectile::new(ProjectileKind::WindCharge, start, Some(Who::Player));
        proj.shoot_from_rotation(
            &mut self.world.random,
            self.pitch,
            self.yaw,
            0.0,
            1.5,
            1.0,
            velocity,
            on_ground,
        );
        self.add_projectile(proj);
        let pitch = 0.4 / (self.world.random.next_float() * 0.4 + 0.8);
        self.audio
            .play("minecraft:entity.wind_charge.throw", Some(eye), 0.5, pitch);
        self.stats.add("used", "minecraft:wind_charge", 1);
        self.swing_arm();
        if !self.creative
            && let Some(s) = self.held_mut().as_mut()
        {
            s.count -= 1;
            if s.count == 0 {
                *self.held_mut() = None;
            }
        }
        Ok(true)
    }
    /// A wind charge meeting an entity or a block (or flying too high).
    pub(super) fn wind_charge_hit(&mut self, p: &Projectile, hit: Hit) -> Result<()> {
        let breeze = p.kind == ProjectileKind::BreezeWindCharge;
        let (radius, knockback) = if breeze {
            (BREEZE_RADIUS, 1.0)
        } else {
            (PLAYER_RADIUS, PLAYER_KNOCKBACK)
        };
        let center = match hit {
            Hit::Entity { who, at } => {
                self.wind_charge_damage(p, who);
                [at.x, at.y, at.z]
            }
            // onHitBlock: a quarter of a block out along the face.
            Hit::Block { at, face, .. } => [
                at.x + f64::from(face[0]) * 0.25,
                at.y + f64::from(face[1]) * 0.25,
                at.z + f64::from(face[2]) * 0.25,
            ],
            Hit::None => return Ok(()),
        };
        self.wind_burst(center, radius, knockback, breeze, !breeze)
    }
    /// `AbstractWindCharge.onHitEntity`: one point of damage.
    fn wind_charge_damage(&mut self, p: &Projectile, who: Who) {
        let owner_breeze = matches!(p.owner, Some(Who::Mob(id))
            if self.mobs.iter().any(|m| m.id == id && m.species() == Species::Breeze));
        match who {
            Who::Player => {
                if !self.creative {
                    self.mob_hits_player_kind(
                        1.0,
                        [p.previous.x, p.previous.y, p.previous.z],
                        "minecraft:wind_charge",
                    );
                }
            }
            // An ender dragon part takes the point of damage too; a crystal is
            // not hit (`AbstractWindCharge.canHitEntity`).
            Who::Mob(id) if id >= super::dragon::SPECIAL => {
                if id < super::dragon::SPECIAL + 8 {
                    self.hit_special(id, 1.0, p.owner == Some(Who::Player), false);
                }
            }
            Who::Mob(id) => {
                let source = Damage {
                    kind: DamageKind::Thrown,
                    attacker: p.owner,
                    from: Some([p.previous.x, p.previous.y, p.previous.z]),
                };
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                    // Breeze.isInvulnerableTo: its own kind's charges.
                    if owner_breeze && m.species() == Species::Breeze {
                        return;
                    }
                    m.damage(1.0, source);
                }
                if p.owner == Some(Who::Player) {
                    self.last_hurt_by_player.insert(id, self.lan_acting);
                }
            }
        }
    }
    /// The exploded positions: rays through everything but the blocks that
    /// stop wind (`blocks_wind_charge_explosions`).
    fn wind_positions(&mut self, center: [f64; 3], radius: f32) -> Vec<[i32; 3]> {
        let stops = self
            .res
            .items()
            .block_tags
            .get("minecraft:blocks_wind_charge_explosions")
            .cloned()
            .unwrap_or_default();
        let (min_y, max_y) = (horizon_client::chunk::MIN_Y, 320);
        let mut set: BTreeSet<[i32; 3]> = BTreeSet::new();
        for xx in 0..16 {
            for yy in 0..16 {
                for zz in 0..16 {
                    if !(xx == 0 || xx == 15 || yy == 0 || yy == 15 || zz == 0 || zz == 15) {
                        continue;
                    }
                    let mut d = [
                        f64::from(xx as f32 / 15.0 * 2.0 - 1.0),
                        f64::from(yy as f32 / 15.0 * 2.0 - 1.0),
                        f64::from(zz as f32 / 15.0 * 2.0 - 1.0),
                    ];
                    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                    for c in &mut d {
                        *c /= len;
                    }
                    let mut power = radius * (0.7 + self.world.random.next_float() * 0.6);
                    let mut at = center;
                    while power > 0.0 {
                        let pos = [
                            at[0].floor() as i32,
                            at[1].floor() as i32,
                            at[2].floor() as i32,
                        ];
                        if pos[1] < min_y || pos[1] >= max_y {
                            break;
                        }
                        let name = &self
                            .res
                            .facts()
                            .get(self.world.block(pos[0], pos[1], pos[2]))
                            .name;
                        if stops.contains(&**name) {
                            power -= (3_600_000.0_f32 + 0.3) * 0.3;
                        }
                        if power > 0.0 {
                            set.insert(pos);
                        }
                        for i in 0..3 {
                            at[i] += d[i] * f64::from(0.3_f32);
                        }
                        power -= 0.225_000_01;
                    }
                }
            }
        }
        set.into_iter().collect()
    }
    /// `ServerExplosion.hurtEntities` for a calculator that damages nothing:
    /// knockback scaled by distance, exposure and the multiplier. Returns the
    /// guests it pushed and by how much (`hitPlayers`), which their clients
    /// apply from the burst's packet.
    fn wind_knockback(
        &mut self,
        center: [f64; 3],
        radius: f32,
        multiplier: f32,
        wind_charge: bool,
        blocklike: bool,
    ) -> Vec<(u32, [f64; 3])> {
        let diameter = f64::from(radius * 2.0);
        let distance = |p: [f64; 3]| {
            ((p[0] - center[0]).powi(2) + (p[1] - center[1]).powi(2) + (p[2] - center[2]).powi(2))
                .sqrt()
        };
        let direction = |origin: [f64; 3]| {
            let d = [
                origin[0] - center[0],
                origin[1] - center[1],
                origin[2] - center[2],
            ];
            let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if l < 1.0e-5 {
                [0.0; 3]
            } else {
                [d[0] / l, d[1] / l, d[2] / l]
            }
        };
        // The player, then every other player in the level (LAN guests),
        // each swapped in as the player. In a guest's turn (a mace's wind
        // burst) the player is that guest.
        let own = self.wind_push_player(center, radius, multiplier, wind_charge);
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
                    g.wind_push_player(center, radius, multiplier, wind_charge)
                });
                if let Some(Some(push)) = push {
                    pushed.push((conn, push));
                }
            }
            self.lan = Some(host);
        }
        let multiplier = f64::from(multiplier);
        let mut mobs = std::mem::take(&mut self.mobs);
        for m in mobs.iter_mut().filter(|m| m.alive()) {
            let feet = [m.position.x, m.position.y, m.position.z];
            let dist = distance(feet) / diameter;
            if dist > 1.0 {
                continue;
            }
            let (w, h) = (m.kind.width / 2.0, f64::from(m.height()));
            let exposure = self.seen_percent(
                center,
                [feet[0] - w, feet[1], feet[2] - w],
                [feet[0] + w, feet[1] + h, feet[2] + w],
            );
            let eye = [feet[0], m.eye_y(), feet[2]];
            let power = (1.0 - dist) * f64::from(exposure) * multiplier;
            let dir = direction(eye);
            m.velocity = Vec3::new(
                m.velocity.x + dir[0] * power,
                m.velocity.y + dir[1] * power,
                m.velocity.z + dir[2] * power,
            );
        }
        self.mobs = mobs;
        // ArmorStand.ignoreExplosion: a visible stand feels only a burst that
        // affects block-like entities (never a wind charge's).
        if blocklike {
            let mut stands = std::mem::take(&mut self.armor_stands);
            for s in stands.iter_mut().filter(|s| !s.invisible) {
                let feet = [s.position.x, s.position.y, s.position.z];
                let dist = distance(feet) / diameter;
                if dist > 1.0 {
                    continue;
                }
                // EntityType.ARMOR_STAND's size, halved when small; a marker has none.
                let (w, h, eye) = if s.marker {
                    (0.0, 0.0, 0.0)
                } else if s.small {
                    (0.125, f64::from(0.9875f32), f64::from(0.9875f32))
                } else {
                    (0.25, f64::from(1.975f32), f64::from(1.7775f32))
                };
                let exposure = self.seen_percent(
                    center,
                    [feet[0] - w, feet[1], feet[2] - w],
                    [feet[0] + w, feet[1] + h, feet[2] + w],
                );
                let power = (1.0 - dist) * f64::from(exposure) * multiplier;
                let dir = direction([feet[0], feet[1] + eye, feet[2]]);
                s.velocity = Vec3::new(
                    s.velocity.x + dir[0] * power,
                    s.velocity.y + dir[1] * power,
                    s.velocity.z + dir[2] * power,
                );
            }
            self.armor_stands = stands;
        }
        // Arrows and other projectiles (the charges themselves ignore it).
        let push = |position: Vec3, velocity: &mut Vec3, game: &Game<'_>| {
            let feet = [position.x, position.y, position.z];
            let dist = distance(feet) / diameter;
            if dist > 1.0 {
                return;
            }
            let exposure = game.seen_percent(
                center,
                [feet[0] - 0.125, feet[1], feet[2] - 0.125],
                [feet[0] + 0.125, feet[1] + 0.25, feet[2] + 0.125],
            );
            let power = (1.0 - dist) * f64::from(exposure) * multiplier;
            let dir = direction(feet);
            *velocity = Vec3::new(
                velocity.x + dir[0] * power,
                velocity.y + dir[1] * power,
                velocity.z + dir[2] * power,
            );
        };
        let mut projectiles = std::mem::take(&mut self.creatures.projectiles);
        for p in projectiles.iter_mut().filter(|p| {
            !p.stuck
                && !matches!(
                    p.kind,
                    ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge
                )
        }) {
            push(p.position, &mut p.velocity, self);
        }
        self.creatures.projectiles = projectiles;
        let mut tnts = std::mem::take(&mut self.tnts);
        for t in &mut tnts {
            push(t.position, &mut t.velocity, self);
        }
        self.tnts = tnts;
        pushed
    }
    /// `ServerExplosion.hurtEntities` for the player (not a spectator, nor a
    /// creative player flying): the push, less with explosion knockback
    /// resistance, and `ServerPlayer.onExplosionHit`. Returns the push.
    fn wind_push_player(
        &mut self,
        center: [f64; 3],
        radius: f32,
        multiplier: f32,
        wind_charge: bool,
    ) -> Option<[f64; 3]> {
        if self.spectator() {
            return None;
        }
        let resistance = self
            .player
            .as_mut()
            .and_then(|p| p.attribute("minecraft:explosion_knockback_resistance").ok())
            .unwrap_or(0.0);
        let creative = self.creative;
        let p = self.player.as_ref()?;
        let feet = [p.position.x, p.position.y, p.position.z];
        let dist = ((feet[0] - center[0]).powi(2)
            + (feet[1] - center[1]).powi(2)
            + (feet[2] - center[2]).powi(2))
        .sqrt()
            / f64::from(radius * 2.0);
        if dist > 1.0 || creative && p.flying {
            return None;
        }
        let w = f64::from(p.dimensions.width) / 2.0;
        let h = f64::from(p.dimensions.height);
        let eye = [
            feet[0] - center[0],
            feet[1] + f64::from(p.dimensions.eye_height) - center[1],
            feet[2] - center[2],
        ];
        let exposure = self.seen_percent(
            center,
            [feet[0] - w, feet[1], feet[2] - w],
            [feet[0] + w, feet[1] + h, feet[2] + w],
        );
        let power = (1.0 - dist) * f64::from(exposure) * f64::from(multiplier) * (1.0 - resistance);
        let l = (eye[0] * eye[0] + eye[1] * eye[1] + eye[2] * eye[2]).sqrt();
        let push = if l < 1.0e-5 {
            [0.0; 3]
        } else {
            eye.map(|d| d / l * power)
        };
        let p = self.player.as_mut()?;
        p.velocity = Vec3::new(
            p.velocity.x + push[0],
            p.velocity.y + push[1],
            p.velocity.z + push[2],
        );
        // ServerPlayer.onExplosionHit: a wind charge's burst cancels the
        // fall; any other burst ends the grace of the last impulse.
        let at = p.position;
        p.impulse.set_ignore_fall_damage(wind_charge, Some(at));
        self.adv.marks.blast = Some(([at.x, at.y, at.z], wind_charge));
        Some(push)
    }
    /// The burst: knockback, the blocks that react, the gust and the sound.
    pub(super) fn wind_burst(
        &mut self,
        center: [f64; 3],
        radius: f32,
        multiplier: f32,
        breeze: bool,
        wind_charge: bool,
    ) -> Result<()> {
        // In a guest's turn (their charge landing, their mace's burst) the
        // host holds the other players (`as_guest`): the burst waits for the
        // turn to end so that it reaches every player, the guest too.
        if self.lan_acting.is_some() && self.lan.is_none() {
            self.creatures.wind_bursts.push(Burst {
                center,
                radius,
                multiplier,
                breeze,
                wind_charge,
            });
            return Ok(());
        }
        // ServerExplosion.explode: the blast is heard first.
        let ctx = horizon_client::sculk::Context::default();
        self.post_game_event("minecraft:explode", center, ctx);
        let mut targets = self.wind_positions(center, radius);
        // ServerExplosion.shouldAffectBlocklikeEntities
        let blocklike = !breeze && !wind_charge && self.ui.rules.bool("mob_griefing");
        let pushed = self.wind_knockback(center, radius, multiplier, wind_charge, blocklike);
        let block_count = targets.len();
        // Explosion.canTriggerBlocks: a breeze's charge needs mob griefing.
        if !breeze || self.ui.rules.bool("mob_griefing") {
            // Util.shuffle(list, level.random)
            for i in (2..=targets.len()).rev() {
                let swap = self.world.random.next_int_bound(i as i32).unwrap_or(0) as usize;
                targets.swap(i - 1, swap);
            }
            for pos in targets {
                self.wind_trigger_block(pos)?;
            }
        }
        let sound = if breeze {
            "minecraft:entity.breeze.wind_burst"
        } else {
            "minecraft:entity.wind_charge.wind_burst"
        };
        let pitch = (1.0
            + (super::explosion::client_float() - super::explosion::client_float()) * 0.2)
            * 0.7;
        self.audio.play(sound, Some(center), 4.0, pitch);
        // Explosion.isSmall: below a radius of 2.
        let ty = if radius < 2.0 {
            ParticleType::GustEmitterSmall
        } else {
            ParticleType::GustEmitterLarge
        };
        self.particles
            .add(&ParticleOptions::simple(ty), center, [1.0, 0.0, 0.0]);
        self.lan_wind_burst(center, radius, sound, block_count, &pushed);
        Ok(())
    }
    /// The bursts guests' turns put off.
    pub(super) fn flush_wind_bursts(&mut self) -> Result<()> {
        self.flush_smash_pushes();
        for b in std::mem::take(&mut self.creatures.wind_bursts) {
            self.wind_burst(b.center, b.radius, b.multiplier, b.breeze, b.wind_charge)?;
        }
        Ok(())
    }
    /// `BlockBehaviour.onExplosionHit` for `TRIGGER_BLOCK`: doors, gates,
    /// buttons, levers and bells react as if used; candles go out.
    fn wind_trigger_block(&mut self, pos: [i32; 3]) -> Result<()> {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let name = self.res.facts().get(state).name.clone();
        let prop = |game: &Self, key: &str| {
            game.pack
                .assets
                .registry
                .state(state)
                .and_then(|s| s.properties.get(key).cloned())
        };
        if name.ends_with("candle") || name.ends_with("candle_cake") {
            if prop(self, "lit").as_deref() == Some("true") {
                let at = [
                    f64::from(pos[0]) + 0.5,
                    f64::from(pos[1]) + 0.5,
                    f64::from(pos[2]) + 0.5,
                ];
                self.audio.play(
                    "minecraft:entity.generic.extinguish_fire",
                    Some(at),
                    1.0,
                    1.0,
                );
                self.set_state_named(pos, &name, &["candles", "waterlogged"])?;
            }
            return Ok(());
        }
        if !triggers(&name) {
            return Ok(());
        }
        // Doors react in their lower half only; powered ones and pressed
        // buttons stay as they are.
        if name.ends_with("_door") && prop(self, "half").as_deref() != Some("lower") {
            return Ok(());
        }
        if prop(self, "powered").as_deref() == Some("true") && &*name != "minecraft:lever" {
            return Ok(());
        }
        let at = Vec3::new(
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        );
        let player = PlayerContext {
            yaw: self.yaw,
            pitch: self.pitch,
            may_build: true,
            can_eat: false,
            game_master: false,
        };
        self.with_level(|b, level, ticks| {
            b.use_without_item(
                level,
                ticks,
                BlockPos::new(pos[0], pos[1], pos[2]),
                Direction::Up,
                at,
                player,
            )
        });
        self.process_level_events()?;
        Ok(())
    }
    /// `WindChargeRenderer`: the spinning charge and its wind.
    pub(super) fn wind_charge_geometry(
        &self,
        p: &Projectile,
        at: glam::Vec3,
        light: u32,
        out: &mut Vec<horizon_client::mesher::Vertex>,
    ) {
        static MODEL: std::sync::OnceLock<Option<horizon_client::mobs::Part>> =
            std::sync::OnceLock::new();
        let model = MODEL.get_or_init(|| {
            horizon_client::mobs::load_model(&self.options.resources, "minecraft:wind_charge#main")
        });
        let Some(model) = model else { return };
        let Some(r) = self
            .gui
            .atlas
            .sprites
            .get("entity/projectiles/wind_charge")
            .copied()
        else {
            return;
        };
        let age = p.age as f32;
        let mut poses = horizon_client::mobs::anim::Poses::new(model);
        poses.yrot("wind_charge", -age * 16.0_f32.to_radians());
        poses.yrot("wind", age * 16.0_f32.to_radians());
        let base = glam::Mat4::from_translation(at)
            * glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0));
        horizon_client::mobs::render::draw_scrolled(
            model,
            base,
            &poses,
            [r.u0, r.v0, r.u1, r.v1],
            horizon_client::mobs::render::scroll_offset(
                age,
                horizon_client::mobs::render::WIND_CHARGE_SCROLL,
            ),
            0xFFFF_FFFF,
            light,
            0,
            out,
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_opens_wooden_doors_but_not_iron_ones() {
        assert!(triggers("minecraft:oak_door"));
        assert!(triggers("minecraft:copper_trapdoor"));
        assert!(!triggers("minecraft:iron_door"));
        assert!(!triggers("minecraft:iron_trapdoor"));
        assert!(triggers("minecraft:lever"));
        assert!(triggers("minecraft:stone_button"));
        assert!(triggers("minecraft:bell"));
        assert!(!triggers("minecraft:stone"));
    }
    #[test]
    fn a_deflected_projectile_turns_around_at_half_speed() {
        let mut p = Projectile::new(ProjectileKind::Snowball, Vec3::ZERO, None);
        p.velocity = Vec3::new(2.0, -1.0, 0.0);
        reverse(&mut p, 0.5);
        assert!((p.velocity.x + 1.0).abs() < 1e-9 && (p.velocity.y - 0.5).abs() < 1e-9);
        assert!((p.yaw - 180.0).abs() < 1e-4);
    }
}
