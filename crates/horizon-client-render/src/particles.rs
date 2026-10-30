//! Client particle engine (`ParticleEngine`, `Particle`, `SingleQuadParticle`)
//! and the vanilla particle types players see most: block crumbs, smoke,
//! flames, lava pops, drips, splashes and rain, bubbles, portal and enchant
//! glyphs, crits, hearts, villager particles, explosions, campfire smoke,
//! notes, redstone dust, totem, end rod, biome ash and spores, falling leaves,
//! spells and dragon breath. Constructors, ticks, sizes, colours and light
//! follow the vanilla classes; sprite sets come from `particles/*.json`.
use crate::{
    blocks::{BlockFacts, Fluid},
    collisions::SceneCollisions,
    hud::Rect,
    mesher::Vertex,
    world::World,
};
use glam::{DVec3, Quat, Vec3 as FVec3};
use horizon_core::{
    geometry::{Aabb, Axis, Vec3, trig},
    random::{LegacyRandom, RandomSource},
};
use horizon_world::block_collision::BlockCollisionRegistry;
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

pub use horizon_client_core::particle_type::ParticleType;
pub use horizon_items::firework::{DYE_FIREWORK_COLORS, FireworkExplosion, FireworkShape};

/// `ParticleOptions` with the parameters of the option classes used here.
#[derive(Clone, Debug)]
pub struct ParticleOptions {
    pub ty: ParticleType,
    /// `BlockParticleOption`: the block's particle sprite and terrain tint.
    pub block: Option<(String, u32)>,
    /// RGB(A) for dust, spells, entity effects and tinted leaves.
    pub color: [f32; 4],
    /// `DustParticleOptions.scale`, `PowerParticleOption.power`, `SpellParticleOption.power`,
    /// `SculkChargeParticleOptions.roll`, `ShriekParticleOption.delay`.
    pub scale: f32,
    /// `DustColorTransitionOptions.toColor` (`color` is the from colour).
    pub to_color: [f32; 3],
    /// `TrailParticleOption`: the point it flies to and how many ticks it takes.
    pub trail: Option<([f64; 3], i32)>,
    /// `ItemParticleOption`: the particle sprite of each layer of the item's
    /// model as resolved for the ground.
    pub item: Option<Vec<String>>,
    /// `GeyserParticleOptions.waterBlocks` and `GeyserBaseParticleOptions`'
    /// `waterBlocks` and `burstImpulseBase` (0 where the type has none).
    pub geyser: Option<(i32, f32)>,
}
impl ParticleOptions {
    pub fn simple(ty: ParticleType) -> Self {
        Self {
            ty,
            block: None,
            color: [1.0; 4],
            scale: 1.0,
            to_color: [1.0, 0.0, 0.0],
            trail: None,
            item: None,
            geyser: None,
        }
    }
    /// `GeyserParticleOptions(type, waterBlocks)`.
    pub fn geyser(ty: ParticleType, water_blocks: i32) -> Self {
        Self {
            geyser: Some((water_blocks, 0.0)),
            ..Self::simple(ty)
        }
    }
    /// `GeyserBaseParticleOptions(type, waterBlocks, burstImpulseBase)`.
    pub fn geyser_base(ty: ParticleType, water_blocks: i32, burst_impulse_base: f32) -> Self {
        Self {
            geyser: Some((water_blocks, burst_impulse_base)),
            ..Self::simple(ty)
        }
    }
    /// `ItemParticleOption`: the stack's layer particle sprites (`BreakingItemParticle`).
    pub fn item(layers: Vec<String>) -> Self {
        Self {
            item: Some(layers),
            ..Self::simple(ParticleType::Item)
        }
    }
    /// `DustColorTransitionOptions`.
    pub fn dust_transition(from: [f32; 3], to: [f32; 3], scale: f32) -> Self {
        Self {
            color: [from[0], from[1], from[2], 1.0],
            to_color: to,
            scale: scale.clamp(0.01, 4.0),
            ..Self::simple(ParticleType::DustColorTransition)
        }
    }
    /// `DustColorTransitionOptions.SCULK_TO_REDSTONE`.
    pub fn sculk_to_redstone() -> Self {
        let rgb = |c: u32| {
            [
                ((c >> 16) & 255) as f32 / 255.0,
                ((c >> 8) & 255) as f32 / 255.0,
                (c & 255) as f32 / 255.0,
            ]
        };
        Self::dust_transition(rgb(3_790_560), rgb(16_711_680), 1.0)
    }
    /// `SculkChargeParticleOptions(roll)`.
    pub fn sculk_charge(roll: f32) -> Self {
        Self {
            scale: roll,
            ..Self::simple(ParticleType::SculkCharge)
        }
    }
    /// `ShriekParticleOption(delay)`.
    pub fn shriek(delay: i32) -> Self {
        Self {
            scale: delay as f32,
            ..Self::simple(ParticleType::Shriek)
        }
    }
    /// `TrailParticleOption(target, color, duration)`.
    pub fn trail(target: [f64; 3], rgb: u32, duration: i32) -> Self {
        Self {
            color: [
                ((rgb >> 16) & 255) as f32 / 255.0,
                ((rgb >> 8) & 255) as f32 / 255.0,
                (rgb & 255) as f32 / 255.0,
                1.0,
            ],
            trail: Some((target, duration)),
            ..Self::simple(ParticleType::Trail)
        }
    }
    pub fn block(ty: ParticleType, sprite: String, tint: u32) -> Self {
        Self {
            block: Some((sprite, tint)),
            ..Self::simple(ty)
        }
    }
    /// `DustParticleOptions(color, scale)`.
    pub fn dust(rgb: [f32; 3], scale: f32) -> Self {
        Self {
            color: [rgb[0], rgb[1], rgb[2], 1.0],
            scale: scale.clamp(0.01, 4.0),
            ..Self::simple(ParticleType::Dust)
        }
    }
    pub fn colored(ty: ParticleType, rgba: [f32; 4]) -> Self {
        Self {
            color: rgba,
            ..Self::simple(ty)
        }
    }
}

#[derive(Clone, Debug)]
enum Sprite {
    /// Index into the engine's sprite table.
    Index(u32),
    /// An atlas sprite by name (block textures for terrain particles).
    Named(String),
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Size {
    Fixed,
    /// `quadSize * clamp((age + a) / lifetime * 32, 0, 1)`.
    GrowIn,
    Flame,
    Lava,
    Portal,
    ReversePortal,
    /// `ShriekParticle.getQuadSize`.
    Shriek,
    /// `FireworkParticles.OverlayParticle.getQuadSize`: a sine pulse.
    Flash,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Light {
    World,
    FullBright,
    /// `LightCoordsUtil.withBlock(light, 15)`.
    Block15,
    /// `addSmoothBlockEmission` with the age fraction.
    AgeEmission,
    /// `addSmoothBlockEmission` with the age fraction to the fourth power.
    AgeEmission4,
    /// `FireflyParticle.getLightCoords`: block light fading in and out, no sky.
    Firefly,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Behaviour {
    Base,
    /// BaseAshSmokeParticle / Explode / Dust / Snowflake / EmissiveRising: base tick, sprite from age.
    Animated,
    Lava,
    Drip {
        fluid: Fluid,
        stage: DripStage,
        next: Option<ParticleType>,
        sound: Option<&'static str>,
    },
    WaterDrop,
    Bubble,
    /// `SulfurBubbleParticle`: rises three blocks through still water,
    /// growing to 0.15.
    SulfurBubble {
        y_start: f64,
        y_end: f64,
        size_start: f32,
        y_prev: f64,
    },
    /// `NoxiousGasParticle`: fades out from `fade_start` ticks of age.
    NoxiousGas {
        fade_start: f32,
    },
    /// `NoxiousGasCloudParticle`: every second tick, a gas particle where the
    /// gas reaches.
    NoxiousGasCloud,
    /// `GeyserEruptionParticle`: the base puffs, the plume and the poofs of
    /// one eruption; `aux` is the speed its provider was given.
    GeyserEruption {
        water_blocks: i32,
        aux: [f64; 3],
    },
    /// `GeyserPlumeParticle`: shoots up `plumeHeight` blocks, then falls back.
    GeyserPlume {
        start_y: f64,
        max_y: f64,
        propulsion: f32,
        spray: [f32; 2],
        min_size: f32,
        max_size: f32,
        done: bool,
    },
    BubblePop,
    BubbleColumnUp,
    CurrentDown {
        angle: f32,
    },
    SuspendedTown,
    Portal {
        start: [f64; 3],
    },
    ReversePortal,
    FlyTowards {
        start: [f64; 3],
        alpha: [f32; 4],
    },
    Crit,
    HugeExplosion,
    ExplosionSeed,
    /// `GustSeedParticle`: three gusts around it every `delay + 1` ticks.
    GustSeed {
        scale: f64,
        delay: i32,
    },
    Campfire,
    SimpleAnimated {
        fade: Option<[f32; 3]>,
    },
    /// FireworkParticles.SparkParticle.
    Spark {
        fade: Option<[f32; 3]>,
        trail: bool,
        twinkle: bool,
    },
    Spell {
        original_alpha: f32,
    },
    Cloud,
    Snowflake,
    Leaves {
        rot_speed: f32,
        spin: f32,
        wind: f32,
        swirl: bool,
        flow: bool,
        flow_scale: [f64; 2],
        swirl_period: f64,
    },
    FallingDust {
        rot_speed: f32,
    },
    DragonBreath {
        hit_ground: bool,
    },
    /// `ShriekParticle`: waits `delay` ticks, then rises.
    Shriek {
        delay: i32,
    },
    /// `VibrationSignalParticle`: flies to a listener.
    Vibration {
        dest: [f64; 3],
        rot: f32,
        rot_o: f32,
        pitch: f32,
        pitch_o: f32,
    },
    /// `DustColorTransitionParticle`: from one colour to the other.
    Transition {
        from: [f32; 3],
        to: [f32; 3],
    },
    /// `TrailParticle`: eases towards its target over its lifetime.
    Trail {
        target: [f64; 3],
    },
    /// `SquidInkParticle`: `SimpleAnimatedParticle`, sinking through air.
    SquidInk,
    /// `FireflyParticle`: wanders, fading in and out, gone inside blocks.
    Firefly,
    /// `WakeParticle` (fishing): grows and cycles four sprites.
    Wake,
    /// `DustPlumeParticle`: gravity and friction decay every tick.
    DustPlume,
    /// `FlyStraightTowardsParticle`: from its start along its motion back to
    /// the start, the colour moving from one to the other.
    FlyStraight {
        start: [f64; 3],
        colors: [u32; 2],
    },
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum DripStage {
    Hang,
    CoolingHang,
    Fall,
    FallAndLand,
    Land,
}

/// One live particle (`Particle` + `SingleQuadParticle` state).
#[derive(Clone, Debug)]
pub struct Particle {
    pub ty: ParticleType,
    pub position: [f64; 3],
    pub previous: [f64; 3],
    pub velocity: [f64; 3],
    bb: Aabb,
    bb_width: f32,
    bb_height: f32,
    pub on_ground: bool,
    has_physics: bool,
    /// Particles that move without collision (`move` overridden).
    free_move: bool,
    stopped_by_collision: bool,
    removed: bool,
    pub age: i32,
    pub lifetime: i32,
    pub gravity: f32,
    friction: f32,
    speed_up_when_y_blocked: bool,
    /// `quadSize`.
    pub size: f32,
    pub rgba: [f32; 4],
    roll: f32,
    o_roll: f32,
    sprite: Sprite,
    /// Age-driven sprite set (`setSpriteFromAge`).
    set: Option<u32>,
    /// Sub-rectangle of the sprite (u0, v0, u1, v1) in 0..1.
    pub uv: [f32; 4],
    size_curve: Size,
    light: Light,
    behaviour: Behaviour,
    no_render: bool,
    /// Ticked once when created (`CritParticle` constructor).
    pre_tick: bool,
    /// `Particle.random`: the particle's own generator (`RandomSource.create()`).
    random: LegacyRandom,
}
fn aabb_at(p: [f64; 3], w: f32, h: f32) -> Aabb {
    let w = f64::from(w) / 2.0;
    Aabb::new(
        Vec3::new(p[0] - w, p[1], p[2] - w),
        Vec3::new(p[0] + w, p[1] + f64::from(h), p[2] + w),
    )
}
/// ClientLevel.addBreakingParticles placement, using local outline boxes.
pub fn breaking_position(
    random: &mut impl RandomSource,
    pos: [i32; 3],
    face: [i32; 3],
    boxes: &[[f64; 6]],
) -> Option<[f64; 3]> {
    let mut bounds = *boxes.first()?;
    for b in &boxes[1..] {
        for i in 0..3 {
            bounds[i] = bounds[i].min(b[i]);
            bounds[i + 3] = bounds[i + 3].max(b[i + 3]);
        }
    }
    // Vanilla consumes all three doubles before replacing the hit-face coordinate.
    let mut p = std::array::from_fn(|i| {
        f64::from(pos[i])
            + random.next_double() * (bounds[i + 3] - bounds[i] - f64::from(0.2f32))
            + f64::from(0.1f32)
            + bounds[i]
    });
    for i in 0..3 {
        match face[i] {
            -1 => p[i] = f64::from(pos[i]) + bounds[i] - f64::from(0.1f32),
            1 => p[i] = f64::from(pos[i]) + bounds[i + 3] + f64::from(0.1f32),
            _ => {}
        }
    }
    Some(p)
}

/// A burst of particles about an entity's box (`around_self`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Burst {
    pub count: u32,
    /// Scale of the Gaussian speeds; `None` spawns them at rest without drawing any.
    pub speed: Option<f64>,
    /// Added to `getRandomY()`.
    pub y: f64,
}
impl Burst {
    /// `AbstractVillager.addParticlesAroundSelf`.
    pub const VILLAGER: Self = Self {
        count: 5,
        speed: Some(0.02),
        y: 1.0,
    };
    /// `TamableAnimal`/`AbstractHorse.spawnTamingParticles`,
    /// `Ocelot.spawnTrustingParticles`, `Animal`'s entity event 18.
    pub const TAMING: Self = Self {
        count: 7,
        speed: Some(0.02),
        y: 0.5,
    };
    /// `Allay`'s entity event 18: three `spawnHeartParticle`.
    pub const ALLAY: Self = Self {
        count: 3,
        speed: Some(0.02),
        y: 0.5,
    };
    /// `Dolphin.addParticlesAroundSelf`.
    pub const DOLPHIN: Self = Self {
        count: 7,
        speed: Some(0.01),
        y: 0.2,
    };
    /// `AbstractHorse.handleEating`'s growth particle.
    pub const HORSE_EATING: Self = Self {
        count: 1,
        speed: None,
        y: 0.5,
    };
}
/// The particle bursts entity events show on the client: the particle and
/// burst of `handleEntityEvent(event)` for the entity type `entity`.
pub fn entity_event_burst(entity: &str, event: u8) -> Option<(&'static str, Burst)> {
    let entity = entity.strip_prefix("minecraft:").unwrap_or(entity);
    let tamable = matches!(
        entity,
        "wolf" | "cat" | "parrot" | "nautilus" | "zombie_nautilus"
    );
    let horse = matches!(
        entity,
        "horse"
            | "donkey"
            | "mule"
            | "llama"
            | "trader_llama"
            | "camel"
            | "camel_husk"
            | "skeleton_horse"
            | "zombie_horse"
    );
    Some(match (entity, event) {
        ("villager", 12) => ("heart", Burst::VILLAGER),
        ("villager", 13) => ("angry_villager", Burst::VILLAGER),
        ("villager", 14) => ("happy_villager", Burst::VILLAGER),
        ("villager", 42) => ("splash", Burst::VILLAGER),
        ("allay", 18) => ("heart", Burst::ALLAY),
        ("dolphin", 38) => ("happy_villager", Burst::DOLPHIN),
        ("ocelot", 41) => ("heart", Burst::TAMING),
        ("ocelot", 40) => ("smoke", Burst::TAMING),
        (_, 7) if tamable || horse => ("heart", Burst::TAMING),
        (_, 6) if tamable || horse => ("smoke", Burst::TAMING),
        (_, 18) if !matches!(entity, "villager" | "allay") => ("heart", Burst::TAMING),
        _ => return None,
    })
}

/// Particles about an entity's box, as `AbstractVillager.addParticlesAroundSelf`,
/// `Animal`'s love hearts (entity event 18), `TamableAnimal` and
/// `AbstractHorse.spawnTamingParticles`, `Ocelot.spawnTrustingParticles`,
/// `Allay`'s hearts and `Dolphin.addParticlesAroundSelf` place them: `count`
/// times three Gaussian speeds scaled by `speed` (none drawn without one), then
/// `getRandomX(1.0)`, `getRandomY() + y` and `getRandomZ(1.0)`.
pub fn around_self(
    random: &mut impl RandomSource,
    feet: [f64; 3],
    width: f64,
    height: f64,
    burst: Burst,
    mut emit: impl FnMut([f64; 3], [f64; 3]),
) {
    for _ in 0..burst.count {
        let v = burst.speed.map_or([0.0; 3], |s| {
            [
                random.next_gaussian() * s,
                random.next_gaussian() * s,
                random.next_gaussian() * s,
            ]
        });
        let p = [
            feet[0] + width * ((2.0 * random.next_double() - 1.0) * 1.0),
            feet[1] + height * random.next_double() + burst.y,
            feet[2] + width * ((2.0 * random.next_double() - 1.0) * 1.0),
        ];
        emit(p, v);
    }
}

/// ClientLevel.addDestroyBlockEffect placement and pre-constructor velocities.
pub fn destroy_positions(
    pos: [i32; 3],
    boxes: &[[f64; 6]],
    mut emit: impl FnMut([f64; 3], [f64; 3]),
) {
    for b in boxes {
        let widths: [f64; 3] = std::array::from_fn(|i| (b[i + 3] - b[i]).min(1.0));
        let counts: [usize; 3] = widths.map(|w| ((w / 0.25).ceil() as usize).max(2));
        for x in 0..counts[0] {
            for y in 0..counts[1] {
                for z in 0..counts[2] {
                    let cells = [x, y, z];
                    let relative: [f64; 3] =
                        std::array::from_fn(|i| (cells[i] as f64 + 0.5) / counts[i] as f64);
                    emit(
                        std::array::from_fn(|i| {
                            f64::from(pos[i]) + (relative[i] * widths[i] + b[i])
                        }),
                        relative.map(|v| v - 0.5),
                    );
                }
            }
        }
    }
}

/// `LivingEntity.spawnItemParticles(stack, count)` (eating, drinking, an item
/// breaking): `count` crumbs in front of the eyes, thrown up and forwards,
/// both turned by the pitch and yaw in degrees.
pub fn item_use_particles(
    random: &mut impl RandomSource,
    pitch: f32,
    yaw: f32,
    position: [f64; 3],
    eye_y: f64,
    count: u32,
    mut emit: impl FnMut([f64; 3], [f64; 3]),
) {
    let (x_rot, y_rot) = (-pitch * trig::DEG_TO_RAD, -yaw * trig::DEG_TO_RAD);
    for _ in 0..count {
        let d = Vec3::new(
            (f64::from(random.next_float()) - 0.5) * 0.1,
            f64::from(random.next_float()) * 0.1 + 0.1,
            0.0,
        )
        .rotate_x(x_rot)
        .rotate_y(y_rot);
        let y = f64::from(-random.next_float()) * 0.6 - 0.3;
        let p = Vec3::new((f64::from(random.next_float()) - 0.5) * 0.3, y, 0.6)
            .rotate_x(x_rot)
            .rotate_y(y_rot);
        emit(
            [p.x + position[0], p.y + eye_y, p.z + position[2]],
            [d.x, d.y + 0.05, d.z],
        );
    }
}

/// `Mth.nextDouble(random, min, max)`.
fn next_double_between(random: &mut impl RandomSource, min: f64, max: f64) -> f64 {
    if min >= max {
        min
    } else {
        random.next_double() * (max - min) + min
    }
}
/// `ParticleUtils.spawnParticlesOnBlockFaces(level, pos, particle, UniformInt.of(3, 5))`
/// (level events 3002 without an axis, 3003 wax on, 3004 wax off, 3005 scrape):
/// three to five particles on each face, in `Direction.values()` order, moving
/// along the face at up to half a block a tick.
pub fn on_block_faces(
    random: &mut impl RandomSource,
    pos: [i32; 3],
    emit: impl FnMut([f64; 3], [f64; 3]),
) {
    on_block_faces_between(random, pos, 3, 5, emit);
}
/// `ParticleUtils.spawnParticlesOnBlockFaces` with `UniformInt.of(min, max)`
/// particles per face (3009 sniffer egg cracks use 3 to 6).
pub fn on_block_faces_between(
    random: &mut impl RandomSource,
    pos: [i32; 3],
    min: i32,
    max: i32,
    mut emit: impl FnMut([f64; 3], [f64; 3]),
) {
    const FACES: [[i32; 3]; 6] = [
        [0, -1, 0],
        [0, 1, 0],
        [0, 0, -1],
        [0, 0, 1],
        [-1, 0, 0],
        [1, 0, 0],
    ];
    for face in FACES {
        let count = random.next_int_bound(max - min + 1).unwrap_or(0) + min;
        for _ in 0..count {
            // getRandomSpeedRanges, then spawnParticleOnFace with step factor 0.55.
            let speed: [f64; 3] = std::array::from_fn(|_| next_double_between(random, -0.5, 0.5));
            let mut p = [0.0; 3];
            let mut v = [0.0; 3];
            for i in 0..3 {
                let center = f64::from(pos[i]) + 0.5;
                if face[i] == 0 {
                    p[i] = center + next_double_between(random, -0.5, 0.5);
                    v[i] = speed[i];
                } else {
                    p[i] = center + f64::from(face[i]) * 0.55;
                }
            }
            emit(p, v);
        }
    }
}
/// `ParticleUtils.spawnParticlesAlongAxis(axis, level, pos, 0.125, particle,
/// UniformInt.of(min, max))` (`count` is the range; level event 3002 on a
/// lightning rod uses 10 to 19): sparks along the rod, moving along it.
/// `axis` is 0, 1, 2 for X, Y, Z.
pub fn along_axis(
    random: &mut impl RandomSource,
    axis: usize,
    pos: [i32; 3],
    (min, max): (i32, i32),
    mut emit: impl FnMut([f64; 3], [f64; 3]),
) {
    let count = random.next_int_bound(max - min + 1).unwrap_or(0) + min;
    for _ in 0..count {
        let p: [f64; 3] = std::array::from_fn(|i| {
            f64::from(pos[i])
                + 0.5
                + next_double_between(random, -1.0, 1.0) * if i == axis { 0.5 } else { 0.125 }
        });
        let v: [f64; 3] = std::array::from_fn(|i| {
            if i == axis {
                next_double_between(random, -1.0, 1.0)
            } else {
                0.0
            }
        });
        emit(p, v);
    }
}

impl Particle {
    /// `Particle(level, x, y, z)` + `SingleQuadParticle` (no initial velocity).
    fn at(random: &mut LegacyRandom, ty: ParticleType, p: [f64; 3]) -> Self {
        let mut particle = Self::base_at(random, ty, p);
        particle.size = 0.1 * (random.next_float() * 0.5 + 0.5) * 2.0;
        particle
    }
    /// `Particle(level, x, y, z)` without `SingleQuadParticle` (`NoRenderParticle`).
    fn base_at(random: &mut LegacyRandom, ty: ParticleType, p: [f64; 3]) -> Self {
        let mut particle = Self::blank(ty, p);
        particle.lifetime = (4.0 / (random.next_float() * 0.9 + 0.1)) as i32;
        particle
    }
    fn blank(ty: ParticleType, p: [f64; 3]) -> Self {
        Self {
            ty,
            position: p,
            previous: p,
            velocity: [0.0; 3],
            bb: aabb_at(p, 0.2, 0.2),
            bb_width: 0.2,
            bb_height: 0.2,
            on_ground: false,
            has_physics: true,
            free_move: false,
            stopped_by_collision: false,
            removed: false,
            age: 0,
            lifetime: 0,
            gravity: 0.0,
            friction: 0.98,
            speed_up_when_y_blocked: false,
            size: 0.0,
            rgba: [1.0; 4],
            roll: 0.0,
            o_roll: 0.0,
            sprite: Sprite::Index(u32::MAX),
            set: None,
            uv: [0.0, 0.0, 1.0, 1.0],
            size_curve: Size::Fixed,
            light: Light::World,
            behaviour: Behaviour::Base,
            no_render: false,
            pre_tick: false,
            random: LegacyRandom::new(0),
        }
    }
    /// `Particle(level, x, y, z, xa, ya, za)`: jittered, normalized initial velocity.
    fn moving(random: &mut LegacyRandom, ty: ParticleType, p: [f64; 3], v: [f64; 3]) -> Self {
        let mut particle = Self::base_moving(random, ty, p, v);
        particle.size = 0.1 * (random.next_float() * 0.5 + 0.5) * 2.0;
        particle
    }
    /// `Particle(level, x, y, z, xa, ya, za)` without `SingleQuadParticle`.
    fn base_moving(random: &mut LegacyRandom, ty: ParticleType, p: [f64; 3], v: [f64; 3]) -> Self {
        let lifetime = (4.0 / (random.next_float() * 0.9 + 0.1)) as i32;
        let mut d = [
            v[0] + f64::from((random.next_float() * 2.0 - 1.0) * 0.4),
            v[1] + f64::from((random.next_float() * 2.0 - 1.0) * 0.4),
            v[2] + f64::from((random.next_float() * 2.0 - 1.0) * 0.4),
        ];
        let speed = f64::from((random.next_float() + random.next_float() + 1.0) * 0.15);
        let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        for (i, c) in d.iter_mut().enumerate() {
            *c = *c / length * speed * f64::from(0.4f32);
            if i == 1 {
                *c += f64::from(0.1f32);
            }
        }
        let mut particle = Self::blank(ty, p);
        particle.lifetime = lifetime;
        particle.velocity = d;
        particle
    }
    /// `TerrainParticle(level, x, y, z, xa, ya, za, state)`: a crumb from a
    /// random quarter of the block's particle sprite, tinted like the block.
    pub fn terrain(
        random: &mut LegacyRandom,
        position: [f64; 3],
        velocity: [f64; 3],
        sprite: String,
        tint: u32,
    ) -> Self {
        let mut p = Self::moving(random, ParticleType::Block, position, velocity);
        p.sprite = Sprite::Named(sprite);
        p.gravity = 1.0;
        let c = |shift: u32| 0.6 * ((tint >> shift) & 255) as f32 / 255.0;
        p.rgba = [c(16), c(8), c(0), 1.0];
        p.size /= 2.0;
        let uo = random.next_float() * 3.0;
        let vo = random.next_float() * 3.0;
        // getU0 = (uo + 1) / 4, getU1 = uo / 4: the crumb is mirrored horizontally.
        p.uv = [(uo + 1.0) / 4.0, vo / 4.0, uo / 4.0, (vo + 1.0) / 4.0];
        p
    }
    /// What `CritParticle`'s providers do once its constructor has ticked it.
    fn after_constructor_tick(&mut self) {
        match self.ty {
            // MagicProvider.
            ParticleType::EnchantedHit => {
                self.rgba[0] *= 0.3;
                self.rgba[1] *= 0.8;
            }
            // DamageIndicatorProvider.
            ParticleType::DamageIndicator => self.lifetime = 20,
            _ => {}
        }
    }
    /// `Particle.setPower`.
    pub fn set_power(&mut self, power: f32) {
        let power = f64::from(power);
        self.velocity[0] *= power;
        self.velocity[1] = (self.velocity[1] - f64::from(0.1f32)) * power + f64::from(0.1f32);
        self.velocity[2] *= power;
    }
    /// `SingleQuadParticle.scale` (quad and collision box).
    pub fn scale(&mut self, scale: f32) {
        self.size *= scale;
        self.set_size(0.2 * scale, 0.2 * scale);
    }
    fn set_size(&mut self, w: f32, h: f32) {
        if w != self.bb_width || h != self.bb_height {
            self.bb_width = w;
            self.bb_height = h;
            let min = self.bb.min_position();
            let max = self.bb.max_position();
            let nx = (min.x + max.x - f64::from(w)) / 2.0;
            let nz = (min.z + max.z - f64::from(w)) / 2.0;
            self.bb = Aabb::new(
                Vec3::new(nx, min.y, nz),
                Vec3::new(nx + f64::from(w), min.y + f64::from(h), nz + f64::from(w)),
            );
        }
    }
    fn set_pos(&mut self, p: [f64; 3]) {
        self.position = p;
        self.bb = aabb_at(p, self.bb_width, self.bb_height);
    }
    fn set_location_from_bb(&mut self) {
        let (min, max) = (self.bb.min_position(), self.bb.max_position());
        self.position = [(min.x + max.x) / 2.0, min.y, (min.z + max.z) / 2.0];
    }
    fn set_color(&mut self, r: f32, g: f32, b: f32) {
        self.rgba[0] = r;
        self.rgba[1] = g;
        self.rgba[2] = b;
    }
    pub fn is_alive(&self) -> bool {
        !self.removed
    }
    /// `Particle.move` with `Entity.collideBoundingBox` against block collision shapes.
    fn move_by(&mut self, level: &ParticleLevel, d: [f64; 3]) {
        if self.free_move {
            self.bb = self.bb.move_by(Vec3::new(d[0], d[1], d[2]));
            self.set_location_from_bb();
            return;
        }
        if self.stopped_by_collision {
            return;
        }
        let original = d;
        let mut moved = d;
        let length2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
        if self.has_physics && length2 != 0.0 && length2 < 100.0 * 100.0 {
            moved = level.collide(self.bb, d);
        }
        if moved != [0.0; 3] {
            self.bb = self.bb.move_by(Vec3::new(moved[0], moved[1], moved[2]));
            self.set_location_from_bb();
        }
        if original[1].abs() >= f64::from(1.0e-5f32) && moved[1].abs() < f64::from(1.0e-5f32) {
            self.stopped_by_collision = true;
        }
        self.on_ground = original[1] != moved[1] && original[1] < 0.0;
        if original[0] != moved[0] {
            self.velocity[0] = 0.0;
        }
        if original[2] != moved[2] {
            self.velocity[2] = 0.0;
        }
    }
    /// `Particle.tick`.
    fn base_tick(&mut self, level: &ParticleLevel) {
        self.previous = self.position;
        let age = self.age;
        self.age += 1;
        if age >= self.lifetime {
            self.removed = true;
            return;
        }
        self.velocity[1] -= 0.04 * f64::from(self.gravity);
        self.move_by(level, self.velocity);
        if self.speed_up_when_y_blocked && self.position[1] == self.previous[1] {
            self.velocity[0] *= 1.1;
            self.velocity[2] *= 1.1;
        }
        let f = f64::from(self.friction);
        for c in &mut self.velocity {
            *c *= f;
        }
        if self.on_ground {
            self.velocity[0] *= f64::from(0.7f32);
            self.velocity[2] *= f64::from(0.7f32);
        }
    }
    /// `getLightCoords(a)` from the world's smooth sky and block light (0..=240).
    fn light_coords(&self, partial: f32, sky: u32, block: u32) -> (u32, u32) {
        let t = ((self.age as f32 + partial) / self.lifetime as f32).clamp(0.0, 1.0);
        match self.light {
            Light::World => (sky, block),
            Light::FullBright => (240, 240),
            Light::Block15 => (sky, 240),
            Light::AgeEmission => (sky, (block + (t * 240.0) as u32).min(240)),
            Light::AgeEmission4 => {
                let b = self.age as f32 / self.lifetime as f32;
                let b = (b * b) * (b * b);
                (sky, (block + (b.clamp(0.0, 1.0) * 240.0) as u32).min(240))
            }
            Light::Firefly => (
                0,
                (255.0 * fade_amount(self.lifetime_progress(self.age as f32 + partial), 0.1, 0.3))
                    as u32,
            ),
        }
    }
    fn quad_size(&self, a: f32) -> f32 {
        let t = (self.age as f32 + a) / self.lifetime as f32;
        match self.size_curve {
            Size::Fixed => self.size,
            Size::GrowIn => self.size * (t * 32.0).clamp(0.0, 1.0),
            Size::Flame => self.size * (1.0 - t * t * 0.5),
            Size::Lava => self.size * (1.0 - t * t),
            Size::Portal => {
                let s = 1.0 - t;
                self.size * (1.0 - s * s)
            }
            Size::ReversePortal => {
                self.size * (1.0 - (self.age as f32 + a) / (self.lifetime as f32 * 1.5))
            }
            Size::Shriek => self.size * (t * 0.75).clamp(0.0, 1.0),
            Size::Flash => {
                7.1 * trig::sin(f64::from(
                    (self.age as f32 + a - 1.0) * 0.25 * std::f64::consts::PI as f32,
                ))
            }
        }
    }
    /// `FireflyParticle.getLifetimeProgress`.
    fn lifetime_progress(&self, age: f32) -> f32 {
        (age / self.lifetime as f32).clamp(0.0, 1.0)
    }
}
/// `NoxiousGasCloudParticle.pickRandomParticleSpawnPoint`: a point up to three
/// blocks from the centre of `center_block` in a random horizontal direction,
/// a quarter block lower, from the level's random.
fn pick_random_particle_spawn_point(random: &mut LegacyRandom, center_block: [i32; 3]) -> [f64; 3] {
    let dx = f64::from(random.next_float() - 0.5);
    let dz = f64::from(random.next_float() - 0.5);
    // Vec3.normalize.
    let length = (dx * dx + dz * dz).sqrt();
    let (dx, dz) = if length < f64::from(1.0e-5f32) {
        (0.0, 0.0)
    } else {
        (dx / length, dz / length)
    };
    let distance = f64::from(random.next_float() * 3.0);
    [
        f64::from(center_block[0]) + 0.5 + dx * distance,
        f64::from(center_block[1]) + 0.5 - 0.25,
        f64::from(center_block[2]) + 0.5 + dz * distance,
    ]
}
/// `FireflyParticle.getFadeAmount`.
fn fade_amount(progress: f32, fade_in: f32, fade_out: f32) -> f32 {
    if progress >= 1.0 - fade_in {
        (1.0 - progress) / fade_in
    } else if progress <= fade_out {
        progress / fade_out
    } else {
        1.0
    }
}

/// A particle's fields as the parity probe reports them.
#[derive(Clone, Debug)]
pub struct ParticleSnapshot {
    pub age: i32,
    pub lifetime: i32,
    pub removed: bool,
    pub on_ground: bool,
    pub position: [f64; 3],
    pub velocity: [f64; 3],
    pub previous: [f64; 3],
    /// `SingleQuadParticle` state; `None` for `NoRenderParticle`s.
    pub quad: Option<QuadSnapshot>,
}
/// `SingleQuadParticle` state at a render.
#[derive(Clone, Debug)]
pub struct QuadSnapshot {
    /// `getQuadSize` at partial ticks 0 and 0.5.
    pub size: [f32; 2],
    pub rgba: [f32; 4],
    /// `roll`, `oRoll`.
    pub roll: [f32; 2],
    pub sprite: String,
    /// `getU0`, `getU1`, `getV0`, `getV1` within the sprite.
    pub uv: [f32; 4],
    /// `getLightCoords(0.5)` over full sky light and no block light.
    pub light: u32,
}

/// World access for particle physics.
pub struct ParticleLevel<'a> {
    pub world: &'a World,
    pub facts: &'a BlockFacts,
    pub collision: &'a BlockCollisionRegistry<'a>,
    /// Local player feet position and vertical velocity (`PlayerCloudParticle`).
    pub player: Option<([f64; 3], f64)>,
}
impl ParticleLevel<'_> {
    fn collide(&self, bb: Aabb, d: [f64; 3]) -> [f64; 3] {
        let mut scene = SceneCollisions {
            world: self.world,
            binding: self.collision,
            subject: None,
        };
        match scene.resolve(bb, Vec3::new(d[0], d[1], d[2]), 0.0, false) {
            Ok(v) => [v.x, v.y, v.z],
            Err(_) => d,
        }
    }
    pub(crate) fn block_at(p: [f64; 3]) -> [i32; 3] {
        [
            p[0].floor() as i32,
            p[1].floor() as i32,
            p[2].floor() as i32,
        ]
    }
    /// The fluid at a block and `FluidState.getHeight`.
    fn fluid(&self, b: [i32; 3]) -> (Fluid, f64) {
        let s = self.facts.get(self.world.block(b[0], b[1], b[2]));
        if s.fluid == Fluid::None {
            return (Fluid::None, 0.0);
        }
        let above = self.facts.get(self.world.block(b[0], b[1] + 1, b[2]));
        let height = if above.fluid == s.fluid {
            1.0
        } else {
            f64::from(s.fluid_level) / 9.0
        };
        (s.fluid, height)
    }
    /// The fluid at a block and whether it is a source (`FluidState.isSource`).
    pub(crate) fn fluid_source(&self, b: [i32; 3]) -> (Fluid, bool) {
        let s = self.facts.get(self.world.block(b[0], b[1], b[2]));
        (s.fluid, s.fluid_level == 8 && !s.fluid_falling)
    }
    /// `getCollisionShape(...).max(Y, x, z)` for cached shapes.
    fn collision_top(&self, b: [i32; 3], fx: f64, fz: f64) -> f64 {
        let state = self.world.block(b[0], b[1], b[2]);
        match self.collision.cached_shapes().cached_shape(state) {
            Ok(Some(shape)) if !shape.is_empty() => shape.max_at(Axis::Y, fx, fz),
            Ok(Some(_)) => 0.0,
            _ => {
                if self.facts.get(state).full_collision {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }
}

/// `ParticleEngine` plus the sprite sets of `ParticleResources`.
pub struct ParticleEngine {
    particles: Vec<Particle>,
    pending: Vec<Particle>,
    /// `ParticleEngine.random`, handed to every `ParticleProvider`.
    random: LegacyRandom,
    /// `ClientLevel.random` as particle constructors use it (`level.getRandom()`).
    level_random: LegacyRandom,
    /// `SpellParticle.RANDOM`.
    spell_random: LegacyRandom,
    /// Seeds for each particle's own random (`RandomSource.create()`).
    seeds: LegacyRandom,
    /// When set, particles ask the level for other particles here instead
    /// (type, position, velocity), as the parity probe's stand-in level does.
    pub requested: Option<Vec<(ParticleOptions, [f64; 3], [f64; 3])>>,
    names: Vec<String>,
    rects: Vec<Option<Rect>>,
    /// Each named sprite, and whether `SingleQuadParticle.Layer.bySprite` puts
    /// it on the translucent layer (a texel with 0 < alpha < 255).
    atlas: HashMap<String, (Rect, bool)>,
    sets: HashMap<ParticleType, u32>,
    set_sprites: Vec<Vec<u32>>,
    /// `TrackingEmitter`s: particle type, entity feet position, width, height, ticks left.
    emitters: Vec<(ParticleOptions, [f64; 3], f64, f64, u32)>,
    /// Camera position for the 32-block spawn limiter.
    pub camera: [f64; 3],
    /// The Particles option (`ParticleStatus`): 0 all, 1 decreased, 2 minimal.
    pub particle_level: u8,
    /// FireworkParticles.Starter instances.
    starters: Vec<Starter>,
    /// Local sounds requested by particles (dripstone drips): (event, position, volume, pitch).
    pub sounds: Vec<(&'static str, [f64; 3], f32, f32)>,
    /// How many of each type were made, for `horizon-ctl status`.
    made: std::collections::BTreeMap<&'static str, u64>,
    /// The item particles' sprites (`BreakingItemParticle`'s slime ball, snowball
    /// and cobweb models, resolved for the ground), one per model layer.
    pub item_sprites: HashMap<ParticleType, Vec<String>>,
}
/// `ParticleGroup.MAX_PARTICLES` (per render group; one group here).
const MAX_PARTICLES: usize = 16384;
impl ParticleEngine {
    pub fn new(root: &Path, atlas: &HashMap<String, Rect>, translucent: &HashSet<String>) -> Self {
        let mut engine = Self {
            particles: Vec::new(),
            pending: Vec::new(),
            random: LegacyRandom::new(0x7061_7274),
            level_random: LegacyRandom::new(0x6c65_7665),
            spell_random: LegacyRandom::new(0x7370_656c),
            seeds: LegacyRandom::new(0x7365_6564),
            requested: None,
            names: Vec::new(),
            rects: Vec::new(),
            atlas: HashMap::new(),
            sets: HashMap::new(),
            set_sprites: Vec::new(),
            emitters: Vec::new(),
            starters: Vec::new(),
            camera: [0.0; 3],
            particle_level: 0,
            sounds: Vec::new(),
            made: Default::default(),
            item_sprites: HashMap::new(),
        };
        engine.reload(root, atlas, translucent);
        engine
    }
    /// `onResourceManagerReload`: sprite rects and sets come from the
    /// reloaded atlas and pack; a live particle keeps drawing its sprite by
    /// name or index.
    pub fn reload(
        &mut self,
        root: &Path,
        atlas: &HashMap<String, Rect>,
        translucent: &HashSet<String>,
    ) {
        self.atlas = atlas
            .iter()
            .map(|(name, &rect)| (name.clone(), (rect, translucent.contains(name))))
            .collect();
        self.rects = self
            .names
            .iter()
            .map(|name| self.atlas.get(name).map(|&(rect, _)| rect))
            .collect();
        self.sets.clear();
        self.set_sprites.clear();
        self.item_sprites.clear();
        self.load_sprites(root);
    }
    /// The sprite sets of every `particles/*.json` under `root`.
    fn load_sprites(&mut self, root: &Path) {
        let dir = root.join("assets/minecraft/particles");
        for &ty in ParticleType::ALL {
            let Ok(bytes) = std::fs::read(dir.join(format!("{}.json", ty.name()))) else {
                continue;
            };
            let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                continue;
            };
            let sprites: Vec<u32> = json["textures"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|t| t.as_str())
                        .map(|t| {
                            self.intern(&format!("particle/{}", t.trim_start_matches("minecraft:")))
                        })
                        .collect()
                })
                .unwrap_or_default();
            if !sprites.is_empty() {
                self.sets.insert(ty, self.set_sprites.len() as u32);
                self.set_sprites.push(sprites);
            }
        }
    }
    fn intern(&mut self, name: &str) -> u32 {
        if let Some(i) = self.names.iter().position(|n| n == name) {
            return i as u32;
        }
        self.names.push(name.to_owned());
        self.rects.push(self.atlas.get(name).map(|&(rect, _)| rect));
        (self.names.len() - 1) as u32
    }
    pub fn len(&self) -> usize {
        self.particles.len()
    }
    pub fn is_empty(&self) -> bool {
        self.particles.is_empty()
    }
    pub fn clear(&mut self) {
        self.emitters.clear();
        self.particles.clear();
        self.pending.clear();
    }
    /// A vibration signal flying from `from` to `to` in `ticks`
    /// (`VibrationParticleOption`).
    pub fn vibration(&mut self, from: [f64; 3], to: [f64; 3], ticks: i32) {
        let mut q = Particle::blank(ParticleType::Vibration, from);
        q.size = 0.3;
        q.lifetime = ticks;
        q.has_physics = false;
        q.light = Light::Block15;
        q.set = self.set_of(ParticleType::Vibration);
        let d = [from[0] - to[0], from[1] - to[1], from[2] - to[2]];
        let rot = d[0].atan2(d[2]) as f32;
        let pitch = d[1].atan2((d[0] * d[0] + d[2] * d[2]).sqrt()) as f32;
        q.behaviour = Behaviour::Vibration {
            dest: to,
            rot,
            rot_o: rot,
            pitch,
            pitch_o: pitch,
        };
        self.pending.push(q);
    }
    /// `ParticleEngine.add` (via `particlesToAdd`, applied on the next tick).
    pub fn push(&mut self, particle: Particle) {
        self.pending.push(particle);
    }
    /// Random generator shared by particle constructors and spawners.
    pub fn random(&mut self) -> &mut LegacyRandom {
        &mut self.random
    }
    /// `ClientLevel.random` as client effects use it (`LevelEventHandler`).
    pub fn level_random(&mut self) -> &mut LegacyRandom {
        &mut self.level_random
    }
    /// `ClientLevel.addParticle`: dropped beyond 32 blocks of the camera unless
    /// the type overrides the limiter, and thinned by the Particles option.
    pub fn add(&mut self, options: &ParticleOptions, p: [f64; 3], v: [f64; 3]) {
        self.add_with(options, p, v, false, false);
    }
    /// `ClientLevel.addParticle(particle, overrideLimiter, alwaysShow, ...)`
    /// (`doAddParticle`). `always_show` is `addAlwaysVisibleParticle`'s flag.
    pub fn add_with(
        &mut self,
        options: &ParticleOptions,
        p: [f64; 3],
        v: [f64; 3],
        override_limiter: bool,
        always_show: bool,
    ) {
        let c = self.camera;
        let d2 = (p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2) + (p[2] - c[2]).powi(2);
        if let Some(requested) = &mut self.requested {
            requested.push((options.clone(), p, v));
            return;
        }
        let minimal = self.calculate_particle_level(always_show) == 2;
        if override_limiter || options.ty.override_limiter() || (d2 <= 1024.0 && !minimal) {
            self.create(options, p, v);
        }
    }
    /// `NoxiousGasCloudParticle.spawnNoxiousGasParticle`: the gas, as an
    /// always visible particle (`ClientLevel.addAlwaysVisibleParticle`).
    fn spawn_noxious_gas_particle(&mut self, pos: [f64; 3]) {
        self.add_with(
            &ParticleOptions::simple(ParticleType::NoxiousGas),
            pos,
            [0.0; 3],
            false,
            true,
        );
    }
    /// `ClientLevel.calculateParticleLevel`: minimal still shows one in ten
    /// always-visible particles as decreased, and decreased drops a third.
    fn calculate_particle_level(&mut self, always_show: bool) -> u8 {
        let mut level = self.particle_level.min(2);
        if always_show && level == 2 && self.level_random.next_int_bound(10).unwrap_or(0) == 0 {
            level = 1;
        }
        if level == 1 && self.level_random.next_int_bound(3).unwrap_or(0) == 0 {
            level = 2;
        }
        level
    }
    /// `LevelEventHandler.singleBlockTeleportationParticles` towards
    /// `BlockUtil.unpackDifferenceInPosition(from, data, radius)`: the dragon
    /// egg's event 2015 packs offsets within (16, 8, 16), a consumable's
    /// teleport (2017) within (127, 127, 127).
    pub fn block_teleportation(&mut self, from: [i32; 3], data: i32, radius: [i32; 3]) {
        let to = [
            from[0].wrapping_add(((data >> 16) & 255) - radius[0]),
            from[1].wrapping_add(((data >> 8) & 255) - radius[1]),
            from[2].wrapping_add((data & 255) - radius[2]),
        ]
        .map(f64::from);
        let from = from.map(f64::from);
        let options = ParticleOptions::simple(ParticleType::Portal);
        for _ in 0..128 {
            let r = &mut self.level_random;
            let t = r.next_double();
            let velocity = std::array::from_fn(|_| f64::from((r.next_float() - 0.5) * 0.2));
            let position = [
                to[0] + t * (from[0] - to[0]) + (r.next_double() - 0.5) + 0.5,
                to[1] + t * (from[1] - to[1]) + r.next_double() - 0.5,
                to[2] + t * (from[2] - to[2]) + (r.next_double() - 0.5) + 0.5,
            ];
            self.add(&options, position, velocity);
        }
    }
    /// `LevelEventHandler.levelEvent` 2018: an enderman's portal trail from
    /// where it was (`from`) to where it went, which `data` packs within
    /// (127, 127, 127). 128 portal particles along the way, spread about the
    /// width and height of an enderman.
    pub fn enderman_teleportation(&mut self, from: [i32; 3], data: i32, particle_level: u8) {
        // EntityTypes.ENDERMAN's dimensions.
        const WIDTH: f64 = 0.6_f32 as f64;
        const HEIGHT: f64 = 2.9_f32 as f64;
        let to = [
            from[0].wrapping_add(((data >> 16) & 255) - 127),
            from[1].wrapping_add(((data >> 8) & 255) - 127),
            from[2].wrapping_add((data & 255) - 127),
        ]
        .map(f64::from);
        let from = from.map(f64::from);
        let options = ParticleOptions::simple(ParticleType::Portal);
        for _ in 0..128 {
            let r = &mut self.level_random;
            let t = r.next_double();
            let velocity = std::array::from_fn(|_| f64::from((r.next_float() - 0.5) * 0.2));
            // Mth.lerp(t, to, from).
            let position = [
                to[0] + t * (from[0] - to[0]) + (r.next_double() - 0.5) * WIDTH * 2.0,
                to[1] + t * (from[1] - to[1]) + r.next_double() * HEIGHT,
                to[2] + t * (from[2] - to[2]) + (r.next_double() - 0.5) * WIDTH * 2.0,
            ];
            // The option roll shares the level random and precedes the distance check.
            let limited = match particle_level {
                1 => r.next_int_bound(3).unwrap_or(0) == 0,
                2 => true,
                _ => false,
            };
            if !limited {
                self.add(&options, position, velocity);
            }
        }
    }
    /// `ClientPacketListener.handleExplosion` + `ClientExplosionTracker`: the
    /// explosion particle (the emitter unless `ServerExplosion.isSmall`) and up
    /// to 512 poof/smoke particles thrown from air inside the radius.
    pub fn explosion(
        &mut self,
        center: [f64; 3],
        small: bool,
        block_count: usize,
        radius: f32,
        is_air: &dyn Fn([i32; 3]) -> bool,
    ) {
        let ty = if small {
            ParticleType::Explosion
        } else {
            ParticleType::ExplosionEmitter
        };
        self.add(&ParticleOptions::simple(ty), center, [1.0, 0.0, 0.0]);
        self.explosion_blocks(center, radius, block_count, is_air);
    }
    /// `LlamaSpit.recreateFromPacket`: seven spit particles thrown ahead of a
    /// new spit, each faster than the last along x and z.
    pub fn llama_spit_burst(&mut self, at: [f64; 3], movement: [f64; 3]) {
        for i in 0..7 {
            let k = 0.4 + 0.1 * f64::from(i);
            self.add(
                &ParticleOptions::simple(ParticleType::Spit),
                at,
                [movement[0] * k, movement[1], movement[2] * k],
            );
        }
    }
    /// `ClientExplosionTracker` with `Level.DEFAULT_EXPLOSION_BLOCK_PARTICLES`.
    pub fn explosion_blocks(
        &mut self,
        center: [f64; 3],
        radius: f32,
        block_count: usize,
        is_air: &dyn Fn([i32; 3]) -> bool,
    ) {
        // ClientExplosionTracker.tick: only the All option throws the debris.
        if self.particle_level != 0 {
            return;
        }
        for _ in 0..block_count.min(512) {
            let r = &mut self.random;
            let d = Vec3::new(
                f64::from(r.next_float() * 2.0 - 1.0),
                f64::from(r.next_float() * 2.0 - 1.0),
                f64::from(r.next_float() * 2.0 - 1.0),
            )
            .normalize();
            let distance = r.next_float().cbrt() * radius;
            let local = [
                d.x * f64::from(distance),
                d.y * f64::from(distance),
                d.z * f64::from(distance),
            ];
            let at = [
                center[0] + local[0],
                center[1] + local[1],
                center[2] + local[2],
            ];
            if !is_air(ParticleLevel::block_at(at)) {
                continue;
            }
            let speed = 0.5 / (distance / radius + 0.1) * r.next_float() * r.next_float() + 0.3;
            // Level.DEFAULT_EXPLOSION_BLOCK_PARTICLES: poof (scaling 0.5) or smoke, equally weighted.
            let (ty, scaling) = if r.next_int_bound(2).unwrap_or(0) == 0 {
                (ParticleType::Poof, 0.5)
            } else {
                (ParticleType::Smoke, 1.0)
            };
            let p = [
                center[0] + local[0] * scaling,
                center[1] + local[1] * scaling,
                center[2] + local[2] * scaling,
            ];
            let v = [
                d.x * f64::from(speed),
                d.y * f64::from(speed),
                d.z * f64::from(speed),
            ];
            self.add(&ParticleOptions::simple(ty), p, v);
        }
    }
    /// `ParticleEngine.createTrackingEmitter` (crits): 16 tries per tick for
    /// three ticks inside a quarter of the entity's box, ticked once at creation.
    pub fn tracking_emitter(
        &mut self,
        options: ParticleOptions,
        feet: [f64; 3],
        width: f64,
        height: f64,
    ) {
        self.tracking_emitter_for(options, feet, width, height, 3);
    }
    /// `createTrackingEmitter(entity, type, lifetime)`.
    pub fn tracking_emitter_for(
        &mut self,
        options: ParticleOptions,
        feet: [f64; 3],
        width: f64,
        height: f64,
        life: u32,
    ) {
        self.emit(&options, feet, width, height);
        if life > 1 {
            self.emitters.push((options, feet, width, height, life - 1));
        }
    }
    fn emit(&mut self, options: &ParticleOptions, feet: [f64; 3], width: f64, height: f64) {
        for _ in 0..16 {
            let r = &mut self.random;
            let xa = f64::from(r.next_float() * 2.0 - 1.0);
            let ya = f64::from(r.next_float() * 2.0 - 1.0);
            let za = f64::from(r.next_float() * 2.0 - 1.0);
            if xa * xa + ya * ya + za * za <= 1.0 {
                let p = [
                    feet[0] + width * xa / 4.0,
                    feet[1] + height * (0.5 + ya / 4.0),
                    feet[2] + width * za / 4.0,
                ];
                self.add(options, p, [xa, ya + 0.2, za]);
            }
        }
    }
    /// `ClientPacketListener.handleParticleEvent` for `ServerLevel.sendParticles`.
    pub fn send_particles(
        &mut self,
        options: &ParticleOptions,
        at: [f64; 3],
        spread: [f64; 3],
        speed: f64,
        count: u32,
    ) {
        if count == 0 {
            let v = [speed * spread[0], speed * spread[1], speed * spread[2]];
            self.add(options, at, v);
            return;
        }
        for _ in 0..count {
            let r = &mut self.random;
            let p = [
                at[0] + r.next_gaussian() * spread[0],
                at[1] + r.next_gaussian() * spread[1],
                at[2] + r.next_gaussian() * spread[2],
            ];
            let v = [
                r.next_gaussian() * speed,
                r.next_gaussian() * speed,
                r.next_gaussian() * speed,
            ];
            self.add(options, p, v);
        }
    }
    /// `ParticleEngine.createParticle`: always spawned.
    pub fn create(&mut self, options: &ParticleOptions, p: [f64; 3], v: [f64; 3]) {
        let seed = self.seeds.next_long();
        if let Some(particle) = self.make(options, p, v, seed) {
            *self.made.entry(options.ty.name()).or_default() += 1;
            self.pending.push(particle);
        }
    }
    /// How many particles of each type were made so far.
    pub fn made(&self) -> &std::collections::BTreeMap<&'static str, u64> {
        &self.made
    }
    /// Parity probes: seed the provider (`ParticleEngine.random`), level and
    /// `SpellParticle.RANDOM` generators.
    pub fn seed_randoms(&mut self, provider: i64, level: i64, spell: i64) {
        self.random = LegacyRandom::new(provider);
        self.level_random = LegacyRandom::new(level);
        self.spell_random = LegacyRandom::new(spell);
    }
    /// Parity probes: the next long of the provider, level and spell generators.
    pub fn next_randoms(&mut self) -> [i64; 3] {
        [
            self.random.next_long(),
            self.level_random.next_long(),
            self.spell_random.next_long(),
        ]
    }
    /// Parity probes: `ParticleProvider.createParticle` with the particle's own
    /// random seeded `seed`, ticked in its constructor where vanilla does.
    pub fn probe_create(
        &mut self,
        options: &ParticleOptions,
        p: [f64; 3],
        v: [f64; 3],
        seed: i64,
        level: &ParticleLevel,
    ) -> Option<Particle> {
        let mut particle = self.make(options, p, v, seed)?;
        if particle.pre_tick {
            particle.pre_tick = false;
            self.tick_particle(&mut particle, level);
            particle.after_constructor_tick();
        }
        Some(particle)
    }
    /// Parity probes: `Particle.tick`.
    pub fn probe_tick(&mut self, particle: &mut Particle, level: &ParticleLevel) {
        self.tick_particle(particle, level);
    }
    /// Parity probes: the state vanilla's particle fields hold.
    pub fn snapshot(&self, p: &Particle) -> ParticleSnapshot {
        let sprite = match &p.sprite {
            Sprite::Index(i) => self.names.get(*i as usize).map_or_else(
                || "-".to_owned(),
                |n| n.strip_prefix("particle/").unwrap_or(n).to_owned(),
            ),
            Sprite::Named(n) => n.clone(),
        };
        let (sky, block) = p.light_coords(0.5, 240, 0);
        ParticleSnapshot {
            age: p.age,
            lifetime: p.lifetime,
            removed: p.removed,
            on_ground: p.on_ground,
            position: p.position,
            velocity: p.velocity,
            previous: p.previous,
            quad: (!p.no_render).then(|| QuadSnapshot {
                size: [p.quad_size(0.0), p.quad_size(0.5)],
                rgba: p.rgba,
                roll: [p.roll, p.o_roll],
                sprite,
                uv: [p.uv[0], p.uv[2], p.uv[1], p.uv[3]],
                light: block | sky << 16,
            }),
        }
    }
    /// `SpriteSet.get(random)`.
    fn sprite_from_set(&mut self, ty: ParticleType) -> Sprite {
        match self.sets.get(&ty) {
            Some(&set) => {
                let sprites = &self.set_sprites[set as usize];
                Sprite::Index(
                    sprites[self
                        .random
                        .next_int_bound(sprites.len() as i32)
                        .unwrap_or(0) as usize],
                )
            }
            None => Sprite::Index(u32::MAX),
        }
    }
    fn set_of(&self, ty: ParticleType) -> Option<u32> {
        self.sets.get(&ty).copied()
    }
    fn first(&self, ty: ParticleType) -> Sprite {
        self.set_of(ty).map_or(Sprite::Index(u32::MAX), |s| {
            Sprite::Index(self.set_sprites[s as usize][0])
        })
    }
    /// `SingleQuadParticle.setSpriteFromAge`.
    fn sprite_from_age(&self, p: &mut Particle) {
        if p.removed {
            return;
        }
        if let Some(set) = p.set {
            // A sprite set's index can outlive the reload that rebuilt them.
            let Some(sprites) = self.set_sprites.get(set as usize) else {
                return;
            };
            let lifetime = p.lifetime.max(1);
            let index = (p.age.max(0) as i64 * (sprites.len() as i64 - 1) / i64::from(lifetime))
                .clamp(0, sprites.len() as i64 - 1);
            p.sprite = Sprite::Index(sprites[index as usize]);
        }
    }
    #[allow(clippy::too_many_lines)]
    /// `ParticleProvider.createParticle`: provider draws use the engine's
    /// random, constructor and tick draws the particle's own (seeded `seed`).
    fn make(
        &mut self,
        o: &ParticleOptions,
        p: [f64; 3],
        v: [f64; 3],
        seed: i64,
    ) -> Option<Particle> {
        use ParticleType as T;
        let ty = o.ty;
        let set = self.set_of(ty);
        let mut local = LegacyRandom::new(seed);
        let r = &mut local;
        let mut particle = match ty {
            T::Block | T::BlockCrumble | T::DustPillar => {
                let (sprite, tint) = o.block.clone()?;
                let mut q = Particle::terrain(r, p, v, sprite, tint);
                q.ty = ty;
                let pr = &mut self.random;
                if ty == T::BlockCrumble {
                    q.velocity = [0.0; 3];
                    q.lifetime = pr.next_int_bound(10).unwrap_or(0) + 1;
                } else if ty == T::DustPillar {
                    q.velocity = [
                        pr.next_gaussian() / 30.0,
                        v[1] + pr.next_gaussian() / 2.0,
                        pr.next_gaussian() / 30.0,
                    ];
                    q.lifetime = pr.next_int_bound(20).unwrap_or(0) + 20;
                }
                q
            }
            T::FallingDust => {
                let (_, tint) = o.block.clone()?;
                let mut q = Particle::at(r, ty, p);
                q.set_color(
                    ((tint >> 16) & 255) as f32 / 255.0,
                    ((tint >> 8) & 255) as f32 / 255.0,
                    (tint & 255) as f32 / 255.0,
                );
                q.size *= 0.674_999_95;
                let base = (32.0 / (r.next_float() * 0.8 + 0.2)) as i32;
                q.lifetime = ((base as f32 * 0.9) as i32).max(1);
                let rot_speed = (r.next_float() - 0.5) * 0.1;
                q.roll = r.next_float() * std::f32::consts::TAU;
                q.size_curve = Size::GrowIn;
                q.behaviour = Behaviour::FallingDust { rot_speed };
                q.set = set;
                q
            }
            T::Smoke
            | T::LargeSmoke
            | T::WhiteSmoke
            | T::Ash
            | T::WhiteAsh
            | T::DustPlume
            | T::NoxiousGas
            | T::GeyserBase
            | T::GeyserPoof => {
                // BaseAshSmokeParticle(dir, xa, scale, colorRandom, maxLifetime, gravity, hasPhysics).
                let (dir, scale, color_random, max_lifetime, gravity, physics, vel) = match ty {
                    T::Smoke | T::WhiteSmoke => ([0.1, 0.1, 0.1], 1.0, 0.3, 8, -0.1, true, v),
                    T::LargeSmoke => ([0.1, 0.1, 0.1], 2.5, 0.3, 8, -0.1, true, v),
                    T::Ash => ([0.1, -0.1, 0.1], 1.0, 0.5, 20, 0.1, false, [0.0; 3]),
                    // DustPlumeParticle: ya + 0.15F.
                    T::DustPlume => (
                        [0.7, 0.6, 0.7],
                        1.0,
                        0.5,
                        7,
                        0.5,
                        false,
                        [v[0], v[1] + f64::from(0.15f32), v[2]],
                    ),
                    // NoxiousGasParticle.Provider: scale 3.
                    T::NoxiousGas => ([0.1, 0.1, 0.1], 3.0, 0.3, 5, -0.02, true, v),
                    // GeyserBaseParticle: the burst impulse scales every axis.
                    T::GeyserBase | T::GeyserPoof => {
                        let (water_blocks, burst_impulse_base) = o.geyser?;
                        let burst_impulse = burst_impulse_base + 0.25 * water_blocks as f32;
                        (
                            [f64::from(burst_impulse); 3],
                            3.0 + 0.125 * water_blocks as f32,
                            0.0,
                            0,
                            0.0,
                            true,
                            v,
                        )
                    }
                    _ => {
                        // WhiteAshParticle.Provider draws the motion.
                        let pr = &mut self.random;
                        let xa =
                            f64::from(pr.next_float()) * -1.9 * f64::from(pr.next_float()) * 0.1;
                        let ya = f64::from(pr.next_float())
                            * -0.5
                            * f64::from(pr.next_float())
                            * 0.1
                            * 5.0;
                        let za =
                            f64::from(pr.next_float()) * -1.9 * f64::from(pr.next_float()) * 0.1;
                        ([0.1, -0.1, 0.1], 1.0, 0.0, 20, 0.0125, false, [xa, ya, za])
                    }
                };
                // GeyserBaseParticle.Provider: a random spot half a block
                // around the point, a fifth of a block up (the provider's
                // random, as float arithmetic added to the double position).
                let p = if matches!(ty, T::GeyserBase | T::GeyserPoof) {
                    let pr = &mut self.random;
                    let x = p[0] + f64::from((pr.next_float() - 0.5) * 0.5);
                    let y = p[1] + f64::from((pr.next_float() - 0.5) * 0.5) + f64::from(0.2f32);
                    let z = p[2] + f64::from((pr.next_float() - 0.5) * 0.5);
                    [x, y, z]
                } else {
                    p
                };
                let mut q = Particle::moving(r, ty, p, [0.0; 3]);
                q.friction = 0.96;
                q.gravity = gravity;
                q.speed_up_when_y_blocked = true;
                for i in 0..3 {
                    q.velocity[i] = q.velocity[i] * f64::from(dir[i] as f32) + vel[i];
                }
                let col = r.next_float() * color_random;
                q.set_color(col, col, col);
                q.size *= 0.75 * scale;
                q.lifetime = ((f64::from(max_lifetime) / (f64::from(r.next_float()) * 0.8 + 0.2)
                    * f64::from(scale)) as i32)
                    .max(1);
                q.has_physics = physics;
                if matches!(ty, T::WhiteSmoke | T::WhiteAsh) {
                    q.set_color(186.0 / 255.0, 177.0 / 255.0, 194.0 / 255.0);
                }
                q.size_curve = Size::GrowIn;
                q.behaviour = Behaviour::Animated;
                if ty == T::DustPlume {
                    let shift = r.next_float() * 0.2;
                    q.set_color(
                        186.0 / 255.0 - shift,
                        177.0 / 255.0 - shift,
                        194.0 / 255.0 - shift,
                    );
                    q.behaviour = Behaviour::DustPlume;
                }
                match ty {
                    // NoxiousGasParticle: white, living `6 / (0.5 to 1) * 3`
                    // ticks, fading out over its second half.
                    T::NoxiousGas => {
                        q.set_color(1.0, 1.0, 1.0);
                        q.lifetime = (6.0 / (f64::from(r.next_float()) * 0.5 + 0.5)
                            * f64::from(3.0f32)) as i32;
                        q.behaviour = Behaviour::NoxiousGas {
                            fade_start: q.lifetime as f32 / 2.0,
                        };
                    }
                    // GeyserBaseParticle: white, only ever rising, 20 to 25 ticks.
                    T::GeyserBase | T::GeyserPoof => {
                        q.friction = 0.725;
                        q.set_color(1.0, 1.0, 1.0);
                        q.velocity[1] = q.velocity[1].abs();
                        let lifetime_factor = 0.8 + 0.2 * self.level_random.next_float();
                        q.lifetime = (25.0 * lifetime_factor) as i32;
                    }
                    _ => {}
                }
                q.set = set;
                q
            }
            // NoxiousGasCloudParticle: lives 20 ticks, draws nothing itself.
            T::NoxiousGasCloud => {
                let mut q = Particle::base_at(r, ty, p);
                q.lifetime = 20;
                q.no_render = true;
                q.behaviour = Behaviour::NoxiousGasCloud;
                q
            }
            // GeyserEruptionParticle: lives 20 ticks, draws nothing itself.
            T::Geyser => {
                let (water_blocks, _) = o.geyser?;
                let mut q = Particle::base_at(r, ty, p);
                q.lifetime = 20;
                q.no_render = true;
                q.behaviour = Behaviour::GeyserEruption {
                    water_blocks,
                    aux: v,
                };
                q
            }
            // GeyserPlumeParticle, after its provider's random placement.
            T::GeyserPlume => {
                let (water_blocks, _) = o.geyser?;
                let pr = &mut self.random;
                let at = [
                    p[0] + f64::from((pr.next_float() - 0.5) * 0.2),
                    p[1] + f64::from(pr.next_float()),
                    p[2] + f64::from((pr.next_float() - 0.5) * 0.2),
                ];
                let plume_height = 5 * water_blocks.max(1);
                let mut q = Particle::moving(r, ty, at, v);
                q.has_physics = true;
                q.speed_up_when_y_blocked = true;
                q.lifetime = plume_height * 5;
                q.velocity[1] = 0.0;
                let level = &mut self.level_random;
                let spray = [
                    (level.next_float() - 0.5) * 0.2,
                    (level.next_float() - 0.5) * 0.2,
                ];
                q.friction = 1.0;
                let propulsion =
                    (if water_blocks == 1 { 1.5 } else { 1.0 }) * plume_height as f32 * 1.45;
                q.gravity = -propulsion;
                let initial_size = q.size * 0.75;
                let min_size = initial_size * (2.0 + plume_height as f32 / 8.0);
                let max_size = initial_size * (3.0 + plume_height as f32 / 8.0);
                q.size = min_size;
                q.behaviour = Behaviour::GeyserPlume {
                    start_y: at[1],
                    max_y: at[1] + f64::from(plume_height) - 1.0,
                    propulsion,
                    spray,
                    min_size,
                    max_size,
                    done: false,
                };
                q.set = set;
                q
            }
            T::Flame
            | T::SoulFireFlame
            | T::CopperFireFlame
            | T::SmallFlame
            | T::Soul
            | T::SculkSoul => {
                // RisingParticle.
                let mut q = Particle::moving(r, ty, p, v);
                q.friction = 0.96;
                for i in 0..3 {
                    q.velocity[i] = q.velocity[i] * f64::from(0.01f32) + v[i];
                }
                for i in 0..3 {
                    q.position[i] += f64::from((r.next_float() - r.next_float()) * 0.05);
                }
                // RisingParticle moves x/y/z but not the bounding box.
                q.previous = p;
                q.lifetime = (8.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32 + 4;
                if matches!(ty, T::Soul | T::SculkSoul) {
                    q.scale(1.5);
                    q.behaviour = Behaviour::Animated;
                    q.set = set;
                    if ty == T::SculkSoul {
                        q.light = Light::Block15;
                    }
                } else {
                    q.free_move = true;
                    q.size_curve = Size::Flame;
                    q.light = Light::AgeEmission;
                    if ty == T::SmallFlame {
                        q.scale(0.5);
                    }
                }
                q
            }
            T::Lava => {
                let mut q = Particle::moving(r, ty, p, [0.0; 3]);
                q.gravity = 0.75;
                q.friction = 0.999;
                q.velocity[0] *= f64::from(0.8f32);
                q.velocity[1] *= f64::from(0.8f32);
                q.velocity[2] *= f64::from(0.8f32);
                q.velocity[1] = f64::from(r.next_float() * 0.4 + 0.05);
                q.size *= r.next_float() * 2.0 + 0.2;
                q.lifetime = (16.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32;
                q.size_curve = Size::Lava;
                q.light = Light::Block15;
                q.behaviour = Behaviour::Lava;
                q
            }
            T::DrippingWater
            | T::FallingWater
            | T::DrippingLava
            | T::FallingLava
            | T::LandingLava
            | T::DrippingHoney
            | T::FallingHoney
            | T::LandingHoney
            | T::FallingNectar
            | T::FallingSporeBlossom
            | T::DrippingObsidianTear
            | T::FallingObsidianTear
            | T::LandingObsidianTear
            | T::DrippingDripstoneWater
            | T::FallingDripstoneWater
            | T::DrippingDripstoneLava
            | T::FallingDripstoneLava => {
                // The provider picks the sprite first (`sprite.get(random)`).
                let sprite = self.sprite_from_set(ty);
                let mut q = Particle::at(r, ty, p);
                q.sprite = sprite;
                q.set_size(0.01, 0.01);
                q.gravity = 0.06;
                let water = [0.2, 0.3, 1.0];
                let lava = [1.0, 0.285_714_3, 0.083_333_336];
                let tear = [0.511_718_75, 0.031_25, 0.890_625];
                let (fluid, stage, next, color, sound) = match ty {
                    T::DrippingWater => (
                        Fluid::Water,
                        DripStage::Hang,
                        Some(T::FallingWater),
                        Some(water),
                        None,
                    ),
                    T::FallingWater => (
                        Fluid::Water,
                        DripStage::FallAndLand,
                        Some(T::Splash),
                        Some(water),
                        None,
                    ),
                    T::DrippingLava => (
                        Fluid::Lava,
                        DripStage::CoolingHang,
                        Some(T::FallingLava),
                        None,
                        None,
                    ),
                    T::FallingLava => (
                        Fluid::Lava,
                        DripStage::FallAndLand,
                        Some(T::LandingLava),
                        Some(lava),
                        None,
                    ),
                    T::LandingLava => (Fluid::Lava, DripStage::Land, None, Some(lava), None),
                    T::DrippingHoney => (
                        Fluid::None,
                        DripStage::Hang,
                        Some(T::FallingHoney),
                        Some([0.622, 0.508, 0.082]),
                        None,
                    ),
                    T::FallingHoney => (
                        Fluid::None,
                        DripStage::FallAndLand,
                        Some(T::LandingHoney),
                        Some([0.582, 0.448, 0.082]),
                        Some("minecraft:block.beehive.drip"),
                    ),
                    T::LandingHoney => (
                        Fluid::None,
                        DripStage::Land,
                        None,
                        Some([0.522, 0.408, 0.082]),
                        None,
                    ),
                    T::FallingNectar => (
                        Fluid::None,
                        DripStage::Fall,
                        None,
                        Some([0.92, 0.782, 0.72]),
                        None,
                    ),
                    T::FallingSporeBlossom => (
                        Fluid::None,
                        DripStage::Fall,
                        None,
                        Some([0.32, 0.5, 0.22]),
                        None,
                    ),
                    T::DrippingObsidianTear => (
                        Fluid::None,
                        DripStage::Hang,
                        Some(T::FallingObsidianTear),
                        Some(tear),
                        None,
                    ),
                    T::FallingObsidianTear => (
                        Fluid::None,
                        DripStage::FallAndLand,
                        Some(T::LandingObsidianTear),
                        Some(tear),
                        None,
                    ),
                    T::LandingObsidianTear => {
                        (Fluid::None, DripStage::Land, None, Some(tear), None)
                    }
                    T::DrippingDripstoneWater => (
                        Fluid::Water,
                        DripStage::Hang,
                        Some(T::FallingDripstoneWater),
                        Some(water),
                        None,
                    ),
                    T::FallingDripstoneWater => (
                        Fluid::Water,
                        DripStage::FallAndLand,
                        Some(T::Splash),
                        Some(water),
                        Some("minecraft:block.pointed_dripstone.drip_water"),
                    ),
                    T::DrippingDripstoneLava => (
                        Fluid::Lava,
                        DripStage::CoolingHang,
                        Some(T::FallingDripstoneLava),
                        None,
                        None,
                    ),
                    _ => (
                        Fluid::Lava,
                        DripStage::FallAndLand,
                        Some(T::LandingLava),
                        Some(lava),
                        Some("minecraft:block.pointed_dripstone.drip_lava"),
                    ),
                };
                match stage {
                    DripStage::Hang | DripStage::CoolingHang => {
                        q.gravity *= 0.02;
                        q.lifetime = 40;
                    }
                    DripStage::FallAndLand => {
                        q.lifetime = (64.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32
                    }
                    DripStage::Land => {
                        q.lifetime = (16.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32
                    }
                    DripStage::Fall => {}
                }
                match ty {
                    T::FallingHoney | T::FallingObsidianTear => q.gravity = 0.01,
                    T::DrippingHoney | T::DrippingObsidianTear => {
                        q.gravity *= 0.01;
                        q.lifetime = 100;
                    }
                    // Lifetimes the providers draw from their own random.
                    T::LandingHoney => {
                        q.lifetime =
                            (128.0 / (f64::from(self.random.next_float()) * 0.8 + 0.2)) as i32
                    }
                    T::LandingObsidianTear => {
                        q.lifetime =
                            (28.0 / (f64::from(self.random.next_float()) * 0.8 + 0.2)) as i32
                    }
                    T::FallingNectar => {
                        q.lifetime =
                            (16.0 / (f64::from(self.random.next_float()) * 0.8 + 0.2)) as i32;
                        q.gravity = 0.007;
                    }
                    T::FallingSporeBlossom => {
                        q.lifetime = (64.0 / (r.next_float() * (0.9 - 0.1) + 0.1)) as i32;
                        q.gravity = 0.005;
                    }
                    _ => {}
                }
                if matches!(
                    ty,
                    T::DrippingObsidianTear | T::FallingObsidianTear | T::LandingObsidianTear
                ) {
                    q.light = Light::Block15;
                }
                if let Some(c) = color {
                    q.set_color(c[0], c[1], c[2]);
                }
                q.behaviour = Behaviour::Drip {
                    fluid,
                    stage,
                    next,
                    sound,
                };
                q
            }
            T::Rain | T::Splash => {
                // WaterDropParticle.
                let mut q = Particle::moving(r, ty, p, [0.0; 3]);
                q.velocity[0] *= f64::from(0.3f32);
                q.velocity[1] = f64::from(r.next_float() * 0.2 + 0.1);
                q.velocity[2] *= f64::from(0.3f32);
                q.set_size(0.01, 0.01);
                q.gravity = 0.06;
                q.lifetime = (8.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32;
                if ty == T::Splash {
                    q.gravity = 0.04;
                    if v[1] == 0.0 && (v[0] != 0.0 || v[2] != 0.0) {
                        q.velocity = [v[0], 0.1, v[2]];
                    }
                }
                q.behaviour = Behaviour::WaterDrop;
                q
            }
            T::Bubble | T::BubbleColumnUp => {
                let mut q = Particle::at(r, ty, p);
                q.set_size(0.02, 0.02);
                q.size *= r.next_float() * 0.6 + 0.2;
                for i in 0..3 {
                    q.velocity[i] =
                        v[i] * f64::from(0.2f32) + f64::from((r.next_float() * 2.0 - 1.0) * 0.02);
                }
                if ty == T::Bubble {
                    q.lifetime = (8.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32;
                    q.behaviour = Behaviour::Bubble;
                } else {
                    q.gravity = -0.125;
                    q.friction = 0.85;
                    q.lifetime = (40.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32;
                    q.behaviour = Behaviour::BubbleColumnUp;
                }
                q
            }
            T::SulfurBubbles => {
                let mut q = Particle::at(r, ty, p);
                q.gravity = -0.04;
                q.friction = 0.85;
                q.set_size(0.02, 0.02);
                q.velocity[0] =
                    v[0] * f64::from(0.2f32) + f64::from((r.next_float() * 2.0 - 1.0) * 0.02);
                // The provider passes `yAux` as `za`, so the Z speed comes from Y.
                q.velocity[2] =
                    v[1] * f64::from(0.2f32) + f64::from((r.next_float() * 2.0 - 1.0) * 0.02);
                let size_start = 0.02 + 0.02 * r.next_float();
                q.size = size_start;
                q.lifetime = i32::MAX;
                q.behaviour = Behaviour::SulfurBubble {
                    y_start: p[1],
                    y_end: p[1] + 4.0 - 1.0,
                    size_start,
                    y_prev: p[1],
                };
                q
            }
            T::BubblePop => {
                let mut q = Particle::at(r, ty, p);
                q.lifetime = 4;
                q.gravity = 0.008;
                q.velocity = v;
                q.behaviour = Behaviour::BubblePop;
                q.set = set;
                q
            }
            T::CurrentDown => {
                let mut q = Particle::at(r, ty, p);
                q.lifetime = (r.next_float() * 60.0) as i32 + 30;
                q.has_physics = false;
                q.velocity = [0.0, -0.05, 0.0];
                q.set_size(0.02, 0.02);
                q.size *= r.next_float() * 0.6 + 0.2;
                q.gravity = 0.002;
                q.behaviour = Behaviour::CurrentDown { angle: 0.0 };
                q
            }
            T::Underwater | T::CrimsonSpore | T::WarpedSpore | T::SporeBlossomAir => {
                // SuspendedParticle; the providers draw motion, then the sprite.
                let below = [p[0], p[1] - 0.125, p[2]];
                let pr = &mut self.random;
                let vel = match ty {
                    T::CrimsonSpore => [
                        pr.next_gaussian() * f64::from(1.0e-6f32),
                        pr.next_gaussian() * f64::from(1.0e-4f32),
                        pr.next_gaussian() * f64::from(1.0e-6f32),
                    ],
                    T::WarpedSpore => [
                        0.0,
                        f64::from(pr.next_float()) * -1.9 * f64::from(pr.next_float()) * 0.1,
                        0.0,
                    ],
                    _ => [0.0, f64::from(-0.8f32), 0.0],
                };
                let sprite = self.sprite_from_set(ty);
                let mut q = if ty == T::Underwater {
                    let mut q = Particle::at(r, ty, below);
                    q.set_size(0.01, 0.01);
                    q.size *= r.next_float() * 0.6 + 0.2;
                    q
                } else {
                    let mut q = Particle::moving(r, ty, below, vel);
                    q.set_size(0.01, 0.01);
                    q.size *= r.next_float() * 0.6 + 0.6;
                    q
                };
                q.lifetime = (16.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32;
                q.has_physics = false;
                q.friction = 1.0;
                q.gravity = 0.0;
                match ty {
                    T::Underwater => q.set_color(0.4, 0.4, 0.7),
                    T::CrimsonSpore => q.set_color(0.9, 0.4, 0.5),
                    T::WarpedSpore => {
                        q.set_color(0.1, 0.1, 0.3);
                        q.set_size(0.001, 0.001);
                    }
                    _ => {
                        q.lifetime = self.random.next_int_bound(501).unwrap_or(0) + 500;
                        q.gravity = 0.01;
                        q.set_color(0.32, 0.5, 0.22);
                    }
                }
                q.sprite = sprite;
                q
            }
            T::Portal | T::ReversePortal => {
                let mut q = Particle::at(r, ty, p);
                q.velocity = v;
                q.size = 0.1 * (r.next_float() * 0.2 + 0.5);
                let br = r.next_float() * 0.6 + 0.4;
                q.set_color(br * 0.9, br * 0.3, br);
                q.lifetime = (r.next_float() * 10.0) as i32 + 40;
                q.free_move = true;
                q.light = Light::AgeEmission4;
                if ty == T::Portal {
                    q.size_curve = Size::Portal;
                    q.behaviour = Behaviour::Portal { start: p };
                } else {
                    q.size *= 1.5;
                    q.lifetime = (r.next_float() * 2.0) as i32 + 60;
                    q.size_curve = Size::ReversePortal;
                    q.behaviour = Behaviour::ReversePortal;
                }
                q
            }
            T::Enchant | T::Nautilus | T::VaultConnection => {
                let mut q = Particle::at(r, ty, p);
                let alpha = if ty == T::VaultConnection {
                    [0.0, 0.6, 0.25, 1.0]
                } else {
                    [1.0, 1.0, 0.0, 1.0]
                };
                q.rgba[3] = alpha[0];
                q.velocity = v;
                let start = [p[0] + v[0], p[1] + v[1], p[2] + v[2]];
                q.previous = start;
                q.set_pos(start);
                q.position = start;
                q.size = 0.1 * (r.next_float() * 0.5 + 0.2);
                let br = r.next_float() * 0.6 + 0.4;
                q.set_color(0.9 * br, 0.9 * br, br);
                q.has_physics = false;
                q.lifetime = (r.next_float() * 10.0) as i32 + 30;
                q.free_move = true;
                q.light = if ty == T::VaultConnection {
                    Light::Block15
                } else {
                    Light::AgeEmission4
                };
                q.behaviour = Behaviour::FlyTowards { start: p, alpha };
                if ty == T::VaultConnection {
                    q.scale(1.5);
                }
                q
            }
            T::Crit | T::EnchantedHit | T::DamageIndicator => {
                let vel = if ty == T::DamageIndicator {
                    [v[0], v[1] + 1.0, v[2]]
                } else {
                    v
                };
                let mut q = Particle::moving(r, ty, p, [0.0; 3]);
                q.friction = 0.7;
                q.gravity = 0.5;
                for i in 0..3 {
                    q.velocity[i] = q.velocity[i] * f64::from(0.1f32) + vel[i] * 0.4;
                }
                let col = r.next_float() * 0.3 + 0.6;
                q.set_color(col, col, col);
                q.size *= 0.75;
                q.lifetime = ((6.0 / (f64::from(r.next_float()) * 0.8 + 0.6)) as i32).max(1);
                q.has_physics = false;
                q.size_curve = Size::GrowIn;
                q.behaviour = Behaviour::Crit;
                q.pre_tick = true;
                q
            }
            T::Heart | T::AngryVillager => {
                let at = if ty == T::AngryVillager {
                    [p[0], p[1] + 0.5, p[2]]
                } else {
                    p
                };
                let mut q = Particle::moving(r, ty, at, [0.0; 3]);
                q.speed_up_when_y_blocked = true;
                q.friction = 0.86;
                for c in &mut q.velocity {
                    *c *= f64::from(0.01f32);
                }
                q.velocity[1] += 0.1;
                q.size *= 1.5;
                q.lifetime = 16;
                q.has_physics = false;
                q.size_curve = Size::GrowIn;
                q
            }
            T::HappyVillager | T::Composter | T::Mycelium | T::Dolphin | T::EggCrack => {
                // SuspendedTownParticle; the provider picks the sprite first.
                let sprite = self.sprite_from_set(ty);
                let mut q = Particle::moving(r, ty, p, v);
                q.sprite = sprite;
                let br = r.next_float() * 0.1 + 0.2;
                q.set_color(br, br, br);
                q.set_size(0.02, 0.02);
                q.size *= r.next_float() * 0.6 + 0.5;
                for c in &mut q.velocity {
                    *c *= f64::from(0.02f32);
                }
                q.lifetime = (20.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32;
                q.free_move = true;
                q.behaviour = Behaviour::SuspendedTown;
                match ty {
                    T::HappyVillager | T::EggCrack => q.set_color(1.0, 1.0, 1.0),
                    T::Composter => {
                        q.set_color(1.0, 1.0, 1.0);
                        q.lifetime = 3 + self.level_random.next_int_bound(5).unwrap_or(0);
                    }
                    T::Dolphin => {
                        q.set_color(0.3, 0.5, 1.0);
                        q.rgba[3] = 1.0 - self.random.next_float() * 0.7;
                        q.lifetime /= 2;
                    }
                    _ => {}
                }
                q
            }
            T::Poof | T::Snowflake | T::Spit => {
                let mut q = Particle::at(r, ty, p);
                if ty != T::Snowflake {
                    q.gravity = -0.1;
                    q.friction = 0.9;
                } else {
                    q.gravity = 0.225;
                    q.friction = 1.0;
                }
                for i in 0..3 {
                    q.velocity[i] = v[i] + f64::from((r.next_float() * 2.0 - 1.0) * 0.05);
                }
                if ty != T::Snowflake {
                    let col = r.next_float() * 0.3 + 0.7;
                    q.set_color(col, col, col);
                    q.size = 0.1 * (r.next_float() * r.next_float() * 6.0 + 1.0);
                } else {
                    q.size = 0.1 * (r.next_float() * r.next_float() + 1.0);
                }
                q.lifetime = (16.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32 + 2;
                q.behaviour = if ty == T::Snowflake {
                    Behaviour::Snowflake
                } else {
                    Behaviour::Animated
                };
                // SpitParticle: ExplodeParticle falling at gravity 0.5.
                if ty == T::Spit {
                    q.gravity = 0.5;
                }
                if ty == T::Snowflake {
                    q.set_color(0.923, 0.964, 0.999);
                }
                q.set = set;
                q
            }
            T::Explosion | T::SonicBoom => {
                let mut q = Particle::moving(r, ty, p, [0.0; 3]);
                q.lifetime = 6 + r.next_int_bound(4).unwrap_or(0);
                let col = r.next_float() * 0.6 + 0.4;
                q.set_color(col, col, col);
                q.size = 2.0 * (1.0 - v[0] as f32 * 0.5);
                if ty == T::SonicBoom {
                    // SonicBoomParticle.
                    q.lifetime = 16;
                    q.size = 1.5;
                }
                q.light = Light::FullBright;
                q.behaviour = Behaviour::HugeExplosion;
                q.set = set;
                q
            }
            T::ExplosionEmitter => {
                let mut q = Particle::base_moving(r, ty, p, [0.0; 3]);
                q.lifetime = 8;
                q.no_render = true;
                q.behaviour = Behaviour::ExplosionSeed;
                q
            }
            // GustParticle: a still, bright, opaque quad that animates over
            // its 12 to 15 ticks; the small one is scaled to 15%.
            T::Gust | T::SmallGust => {
                let mut q = Particle::at(r, ty, p);
                q.lifetime = 12 + r.next_int_bound(4).unwrap_or(0);
                q.size = 1.0;
                q.set_size(1.0, 1.0);
                if ty == T::SmallGust {
                    q.scale(0.15);
                }
                q.light = Light::FullBright;
                q.behaviour = Behaviour::HugeExplosion;
                q.set = set;
                q
            }
            T::GustEmitterLarge | T::GustEmitterSmall => {
                let mut q = Particle::base_moving(r, ty, p, [0.0; 3]);
                let (scale, lifetime, delay) = if ty == T::GustEmitterLarge {
                    (3.0, 7, 0)
                } else {
                    (1.0, 3, 2)
                };
                q.lifetime = lifetime;
                q.no_render = true;
                q.behaviour = Behaviour::GustSeed { scale, delay };
                q
            }
            T::CampfireCosySmoke | T::CampfireSignalSmoke => {
                let mut q = Particle::at(r, ty, p);
                q.scale(3.0);
                q.set_size(0.25, 0.25);
                q.lifetime = r.next_int_bound(50).unwrap_or(0)
                    + if ty == T::CampfireSignalSmoke {
                        280
                    } else {
                        80
                    };
                q.gravity = 3.0e-6;
                q.velocity = [v[0], v[1] + f64::from(r.next_float() / 500.0), v[2]];
                q.rgba[3] = if ty == T::CampfireSignalSmoke {
                    0.95
                } else {
                    0.9
                };
                q.behaviour = Behaviour::Campfire;
                q
            }
            T::Note => {
                let mut q = Particle::moving(r, ty, p, [0.0; 3]);
                q.friction = 0.66;
                q.speed_up_when_y_blocked = true;
                for c in &mut q.velocity {
                    *c *= f64::from(0.01f32);
                }
                q.velocity[1] += 0.2;
                let color = v[0] as f32;
                // `Mth.sin` (the quantized table) of a float angle.
                let channel = |offset: f32| {
                    (trig::sin(f64::from((color + offset) * std::f64::consts::TAU as f32)) * 0.65
                        + 0.35)
                        .max(0.0)
                };
                q.set_color(channel(0.0), channel(0.333_333_34), channel(0.666_666_7));
                q.size *= 1.5;
                q.lifetime = 6;
                q.size_curve = Size::GrowIn;
                q
            }
            T::DustColorTransition => {
                let mut q = Particle::moving(r, ty, p, v);
                q.friction = 0.96;
                q.speed_up_when_y_blocked = true;
                for c in &mut q.velocity {
                    *c *= f64::from(0.1f32);
                }
                q.size *= 0.75 * o.scale;
                let base = (8.0 / (r.next_double() * 0.8 + 0.2)) as i32;
                q.lifetime = ((base as f32 * o.scale).max(1.0)) as i32;
                let factor = r.next_float() * 0.4 + 0.6;
                let mut randomize = |c: f32| (r.next_float() * 0.2 + 0.8) * c * factor;
                let from = [
                    randomize(o.color[0]),
                    randomize(o.color[1]),
                    randomize(o.color[2]),
                ];
                let to = [
                    randomize(o.to_color[0]),
                    randomize(o.to_color[1]),
                    randomize(o.to_color[2]),
                ];
                q.size_curve = Size::GrowIn;
                q.behaviour = Behaviour::Transition { from, to };
                q.set = set;
                q
            }
            // SculkChargeParticle / SculkChargePopParticle.
            T::SculkCharge | T::SculkChargePop => {
                let mut q = Particle::moving(r, ty, p, v);
                q.friction = 0.96;
                q.scale(if ty == T::SculkCharge { 1.5 } else { 1.0 });
                q.has_physics = false;
                q.velocity = v;
                q.light = Light::Block15;
                q.behaviour = Behaviour::Animated;
                q.set = set;
                if ty == T::SculkCharge {
                    q.roll = o.scale;
                    q.o_roll = o.scale;
                    q.lifetime = self.random.next_int_bound(12).unwrap_or(0) + 8;
                } else {
                    q.lifetime = self.random.next_int_bound(4).unwrap_or(0) + 6;
                }
                q
            }
            // Vibration signals are created by `vibration`, aimed at a listener.
            T::Vibration => return None,
            // ShriekParticle.
            T::Shriek => {
                let mut q = Particle::moving(r, ty, p, [0.0; 3]);
                q.size = 0.85;
                q.lifetime = 30;
                q.gravity = 0.0;
                q.velocity = [0.0, 0.1, 0.0];
                q.light = Light::Block15;
                q.size_curve = Size::Shriek;
                q.behaviour = Behaviour::Shriek {
                    delay: o.scale as i32,
                };
                q
            }
            T::Dust => {
                let mut q = Particle::moving(r, ty, p, v);
                q.friction = 0.96;
                q.speed_up_when_y_blocked = true;
                for c in &mut q.velocity {
                    *c *= f64::from(0.1f32);
                }
                q.size *= 0.75 * o.scale;
                let base = (8.0 / (r.next_double() * 0.8 + 0.2)) as i32;
                q.lifetime = ((base as f32 * o.scale).max(1.0)) as i32;
                let factor = r.next_float() * 0.4 + 0.6;
                let mut randomize = |c: f32| (r.next_float() * 0.2 + 0.8) * c * factor;
                let rgb = [
                    randomize(o.color[0]),
                    randomize(o.color[1]),
                    randomize(o.color[2]),
                ];
                q.set_color(rgb[0], rgb[1], rgb[2]);
                q.size_curve = Size::GrowIn;
                q.behaviour = Behaviour::Animated;
                q.set = set;
                q
            }
            // FireworkParticles.SparkProvider (a rocket's trail).
            T::Firework => {
                let mut q = Particle::at(r, ty, p);
                q.velocity = v;
                q.gravity = 0.1;
                q.friction = 0.91;
                q.size *= 0.75;
                q.lifetime = 48 + r.next_int_bound(12).unwrap_or(0);
                q.rgba[3] = 0.99;
                q.light = Light::FullBright;
                q.behaviour = Behaviour::Spark {
                    fade: None,
                    trail: false,
                    twinkle: false,
                };
                q.set = set;
                q
            }
            T::TotemOfUndying | T::EndRod => {
                let mut q = Particle::at(r, ty, p);
                q.friction = 0.91;
                q.gravity = if ty == T::TotemOfUndying {
                    1.25
                } else {
                    0.0125
                };
                q.velocity = v;
                q.size *= 0.75;
                q.lifetime = 60 + r.next_int_bound(12).unwrap_or(0);
                let mut fade = None;
                if ty == T::TotemOfUndying {
                    q.friction = 0.6;
                    if r.next_int_bound(4).unwrap_or(1) == 0 {
                        q.set_color(
                            0.6 + r.next_float() * 0.2,
                            0.6 + r.next_float() * 0.3,
                            r.next_float() * 0.2,
                        );
                    } else {
                        q.set_color(
                            0.1 + r.next_float() * 0.2,
                            0.4 + r.next_float() * 0.3,
                            r.next_float() * 0.2,
                        );
                    }
                } else {
                    fade = Some([
                        0xF2 as f32 / 255.0,
                        0xDE as f32 / 255.0,
                        0xC9 as f32 / 255.0,
                    ]);
                    q.free_move = true;
                }
                q.light = Light::FullBright;
                q.behaviour = Behaviour::SimpleAnimated { fade };
                q.set = set;
                q
            }
            T::CherryLeaves
            | T::PaleOakLeaves
            | T::TintedLeaves
            | T::RedPoplarLeaves
            | T::OrangePoplarLeaves
            | T::YellowPoplarLeaves => {
                let (fall, side, swirl, flow, scale, start) = if ty == T::CherryLeaves {
                    (0.25f32, 2.0f32, false, true, 1.0f32, 0.0f32)
                } else {
                    (0.07, 10.0, true, false, 2.0, 0.021)
                };
                let mut q = Particle::at(r, ty, p);
                let rot_speed = (if r.next_boolean() { -30.0f32 } else { 30.0 }).to_radians();
                let spin = (if r.next_boolean() { -5.0f32 } else { 5.0 }).to_radians();
                q.lifetime = 300;
                q.gravity = fall * 1.2 * 0.0025;
                let s = scale * if r.next_boolean() { 0.05 } else { 0.075 };
                q.size = s;
                q.set_size(s, s);
                q.friction = 1.0;
                q.velocity[1] = -f64::from(start);
                let pr = r.next_float();
                let flow_scale = [
                    f64::from(pr * 60.0).to_radians().cos() * f64::from(side),
                    f64::from(pr * 60.0).to_radians().sin() * f64::from(side),
                ];
                let swirl_period = f64::from(1000.0 + pr * 3000.0).to_radians();
                if ty == T::TintedLeaves {
                    q.set_color(o.color[0], o.color[1], o.color[2]);
                }
                q.behaviour = Behaviour::Leaves {
                    rot_speed,
                    spin,
                    wind: side,
                    swirl,
                    flow,
                    flow_scale,
                    swirl_period,
                };
                q
            }
            T::Effect
            | T::InstantEffect
            | T::EntityEffect
            | T::Witch
            | T::Infested
            | T::TrialOmen
            | T::RaidOmen => {
                // SpellParticle: x/z velocity from `SpellParticle.RANDOM`, y from the option.
                let xa = 0.5 - self.spell_random.next_double();
                let za = 0.5 - self.spell_random.next_double();
                let mut q = Particle::moving(r, ty, p, [xa, v[1], za]);
                q.friction = 0.96;
                q.gravity = -0.1;
                q.speed_up_when_y_blocked = true;
                q.velocity[1] *= f64::from(0.2f32);
                if v[0] == 0.0 && v[2] == 0.0 {
                    q.velocity[0] *= f64::from(0.1f32);
                    q.velocity[2] *= f64::from(0.1f32);
                }
                q.size *= 0.75;
                q.lifetime = (8.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32;
                q.has_physics = false;
                match ty {
                    T::Witch => {
                        let b = self.random.next_float() * 0.5 + 0.35;
                        q.set_color(b, 0.0, b);
                    }
                    T::EntityEffect => {
                        q.set_color(o.color[0], o.color[1], o.color[2]);
                        q.rgba[3] = o.color[3];
                    }
                    T::Effect | T::InstantEffect => {
                        q.set_color(o.color[0], o.color[1], o.color[2]);
                        q.set_power(o.scale);
                    }
                    _ => {}
                }
                q.behaviour = Behaviour::Spell {
                    original_alpha: q.rgba[3],
                };
                q.set = set;
                q
            }
            T::Cloud | T::Sneeze => {
                let mut q = Particle::moving(r, ty, p, [0.0; 3]);
                q.friction = 0.96;
                for i in 0..3 {
                    q.velocity[i] = q.velocity[i] * f64::from(0.1f32) + v[i];
                }
                let col = 1.0 - r.next_float() * 0.3;
                q.set_color(col, col, col);
                q.size *= 1.875;
                let base = (8.0 / (f64::from(r.next_float()) * 0.8 + 0.3)) as i32;
                q.lifetime = ((base as f32 * 2.5).max(1.0)) as i32;
                q.has_physics = false;
                if ty == T::Sneeze {
                    q.set_color(0.22, 1.0, 0.53);
                    q.rgba[3] = 0.4;
                }
                q.size_curve = Size::GrowIn;
                q.behaviour = Behaviour::Cloud;
                q.set = set;
                q
            }
            T::DragonBreath => {
                let mut q = Particle::at(r, ty, p);
                q.friction = 0.96;
                q.velocity = v;
                q.set_color(
                    r.next_float() * (0.874_509_8 - 0.717_647_1) + 0.717_647_1,
                    0.0,
                    r.next_float() * (0.976_470_6 - 0.823_529_4) + 0.823_529_4,
                );
                q.size *= 0.75;
                q.lifetime = (20.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32;
                q.has_physics = false;
                q.size_curve = Size::GrowIn;
                q.behaviour = Behaviour::DragonBreath { hit_ground: false };
                q.set = set;
                q.set_power(o.scale);
                q
            }
            // AttackSweepParticle: `xAux` shrinks it; still, bright, animated over 4 ticks.
            T::SweepAttack => {
                let mut q = Particle::moving(r, ty, p, [0.0; 3]);
                q.lifetime = 4;
                let col = r.next_float() * 0.6 + 0.4;
                q.set_color(col, col, col);
                q.size = 1.0 - v[0] as f32 * 0.5;
                q.light = Light::FullBright;
                q.behaviour = Behaviour::HugeExplosion;
                q.set = set;
                q
            }
            // GlowParticle and its providers.
            T::Glow | T::WaxOn | T::WaxOff | T::ElectricSpark | T::Scrape => {
                let pr = &mut self.random;
                let start = if ty == T::Glow {
                    let xa = 0.5 - pr.next_double();
                    [xa, v[1], 0.5 - pr.next_double()]
                } else {
                    [0.0; 3]
                };
                let mut q = Particle::moving(r, ty, p, start);
                q.friction = 0.96;
                q.speed_up_when_y_blocked = true;
                q.size *= 0.75;
                q.has_physics = false;
                q.light = Light::AgeEmission;
                q.behaviour = Behaviour::Animated;
                q.set = set;
                match ty {
                    T::ElectricSpark => {
                        q.set_color(1.0, 0.9, 1.0);
                        q.velocity = [v[0] * 0.25, v[1] * 0.25, v[2] * 0.25];
                        q.lifetime = pr.next_int_bound(2).unwrap_or(0) + 2;
                    }
                    T::Glow => {
                        if pr.next_boolean() {
                            q.set_color(0.6, 1.0, 0.8);
                        } else {
                            q.set_color(0.08, 0.4, 0.4);
                        }
                        q.velocity[1] *= f64::from(0.2f32);
                        if v[0] == 0.0 && v[2] == 0.0 {
                            q.velocity[0] *= f64::from(0.1f32);
                            q.velocity[2] *= f64::from(0.1f32);
                        }
                        q.lifetime = (8.0 / (pr.next_double() * 0.8 + 0.2)) as i32;
                    }
                    T::Scrape => {
                        if pr.next_boolean() {
                            q.set_color(0.29, 0.58, 0.51);
                        } else {
                            q.set_color(0.43, 0.77, 0.62);
                        }
                        q.velocity = [v[0] * 0.01, v[1] * 0.01, v[2] * 0.01];
                        q.lifetime = pr.next_int_bound(30).unwrap_or(0) + 10;
                    }
                    _ => {
                        if ty == T::WaxOn {
                            q.set_color(0.91, 0.55, 0.08);
                        } else {
                            q.set_color(1.0, 0.9, 1.0);
                        }
                        q.velocity = [v[0] * 0.01 / 2.0, v[1] * 0.01, v[2] * 0.01 / 2.0];
                        q.lifetime = pr.next_int_bound(30).unwrap_or(0) + 10;
                    }
                }
                q
            }
            // SquidInkParticle (SimpleAnimatedParticle, gravity 0).
            T::SquidInk | T::GlowSquidInk => {
                let mut q = Particle::at(r, ty, p);
                q.friction = 0.92;
                q.size = 0.5;
                if ty == T::GlowSquidInk {
                    // ARGB.colorFromFloat(1, 0.2, 0.8, 0.6), back to floats.
                    q.set_color(51.0 / 255.0, 204.0 / 255.0, 153.0 / 255.0);
                } else {
                    q.set_color(0.0, 0.0, 0.0);
                }
                q.lifetime = (q.size * 12.0 / (r.next_float() * 0.8 + 0.2)) as i32;
                q.has_physics = false;
                q.velocity = v;
                q.light = Light::FullBright;
                q.behaviour = Behaviour::SquidInk;
                q.set = set;
                q
            }
            // BreakingItemParticle: a crumb of the item's sprite.
            T::Item | T::ItemSlime | T::ItemSnowball | T::ItemCobweb | T::SulfurCubeGoo => {
                let sprite = if ty == T::SulfurCubeGoo {
                    self.first(ty)
                } else {
                    // ItemStackRenderState.pickParticleMaterial.
                    let layers = match ty {
                        T::Item => o.item.clone().unwrap_or_default(),
                        _ => self.item_sprites.get(&ty).cloned().unwrap_or_default(),
                    };
                    if layers.is_empty() {
                        Sprite::Index(u32::MAX)
                    } else {
                        let i = self.random.next_int_bound(layers.len() as i32).unwrap_or(0);
                        Sprite::Named(layers[i as usize].clone())
                    }
                };
                let mut q = Particle::moving(r, ty, p, [0.0; 3]);
                q.sprite = sprite;
                q.gravity = 1.0;
                q.size /= 2.0;
                let uo = r.next_float() * 3.0;
                let vo = r.next_float() * 3.0;
                q.uv = [(uo + 1.0) / 4.0, vo / 4.0, uo / 4.0, (vo + 1.0) / 4.0];
                if ty == T::Item {
                    for i in 0..3 {
                        q.velocity[i] = q.velocity[i] * f64::from(0.1f32) + v[i];
                    }
                }
                q
            }
            // FireworkParticles.OverlayParticle: the flash of an explosion.
            T::Flash => {
                let mut q = Particle::at(r, ty, p);
                q.lifetime = 4;
                q.set_color(o.color[0], o.color[1], o.color[2]);
                q.rgba[3] = o.color[3];
                q.size_curve = Size::Flash;
                q
            }
            // FireflyParticle; the provider draws its motion, sprite and lifetime.
            T::Firefly => {
                let pr = &mut self.random;
                let xa = 0.5 - pr.next_double();
                let ya = if pr.next_boolean() { v[1] } else { -v[1] };
                let za = 0.5 - pr.next_double();
                let sprite = self.sprite_from_set(ty);
                let mut q = Particle::moving(r, ty, p, [xa, ya, za]);
                q.sprite = sprite;
                q.speed_up_when_y_blocked = true;
                q.friction = 0.96;
                q.size *= 0.75;
                q.velocity[1] *= f64::from(0.8f32);
                q.velocity[0] *= f64::from(0.8f32);
                q.velocity[2] *= f64::from(0.8f32);
                q.lifetime = self
                    .random
                    .next_int_between_inclusive(200, 300)
                    .unwrap_or(200);
                q.scale(1.5);
                q.rgba[3] = 0.0;
                q.light = Light::Firefly;
                q.behaviour = Behaviour::Firefly;
                q
            }
            // TrialSpawnerDetectionParticle (scale 1.5 from the provider).
            T::TrialSpawnerDetection | T::TrialSpawnerDetectionOminous => {
                let scale = 1.5f32;
                let mut q = Particle::moving(r, ty, p, [0.0; 3]);
                q.friction = 0.96;
                q.gravity = -0.1;
                q.speed_up_when_y_blocked = true;
                q.velocity[0] *= 0.0;
                q.velocity[1] *= 0.9;
                q.velocity[2] *= 0.0;
                for i in 0..3 {
                    q.velocity[i] += v[i];
                }
                q.size *= 0.75 * scale;
                // Mth.randomBetween(random, 0.5, 1.0).
                q.lifetime = ((8.0 / (r.next_float() * (1.0 - 0.5) + 0.5) * scale) as i32).max(1);
                q.light = Light::Block15;
                q.size_curve = Size::GrowIn;
                q.behaviour = Behaviour::Animated;
                q.set = set;
                q
            }
            // FlyStraightTowardsParticle.OminousSpawnProvider.
            T::OminousSpawning => {
                let sprite = self.sprite_from_set(ty);
                let mut q = Particle::at(r, ty, p);
                q.sprite = sprite;
                q.velocity = v;
                let start = [p[0] + v[0], p[1] + v[1], p[2] + v[2]];
                q.previous = start;
                q.position = start;
                q.size = 0.1 * (r.next_float() * 0.5 + 0.2);
                q.has_physics = false;
                q.free_move = true;
                q.lifetime = (r.next_float() * 5.0) as i32 + 25;
                q.light = Light::Block15;
                q.behaviour = Behaviour::FlyStraight {
                    start: p,
                    colors: [0xFF45_AEFE, 0xFFFF_FFFF],
                };
                // Mth.randomBetween(level.getRandom(), 3, 5).
                let s = self.level_random.next_float() * (5.0 - 3.0) + 3.0;
                q.scale(s);
                q
            }
            // WakeParticle.
            T::Fishing => {
                let mut q = Particle::moving(r, ty, p, [0.0; 3]);
                q.velocity[0] *= f64::from(0.3f32);
                q.velocity[1] = f64::from(r.next_float() * 0.2 + 0.1);
                q.velocity[2] *= f64::from(0.3f32);
                q.set_size(0.01, 0.01);
                q.lifetime = (8.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32;
                q.gravity = 0.0;
                q.velocity = v;
                q.behaviour = Behaviour::Wake;
                q.set = set;
                q
            }
            // SimpleVerticalParticle.
            T::PauseMobGrowth | T::ResetMobGrowth => {
                let mut q = Particle::moving(r, ty, p, v);
                q.velocity = v;
                q.gravity = 0.0;
                q.velocity[1] += if ty == T::ResetMobGrowth { 0.03 } else { -0.03 };
                q.size *= r.next_float() * 0.6 + 0.5;
                q.lifetime = 8;
                q
            }
            // BlockMarker: the block's particle sprite, half a block wide, 80 ticks.
            T::BlockMarker => {
                let (sprite, _) = o.block.clone()?;
                let mut q = Particle::at(r, ty, p);
                q.sprite = Sprite::Named(sprite);
                q.gravity = 0.0;
                q.lifetime = 80;
                q.has_physics = false;
                q.size = 0.5;
                q
            }
            T::Trail => {
                let (target, duration) = o.trail?;
                let mut q = Particle::moving(r, ty, p, v);
                let jitter = |r: &mut LegacyRandom| 0.875 + r.next_float() * 0.25;
                let (jr, jg, jb) = (jitter(r), jitter(r), jitter(r));
                // `ARGB.scaleRGB` on the 8-bit channels.
                let scaled = |c: f32, s: f32| {
                    ((((c * 255.0).round() as i32) as f32 * s) as i32).clamp(0, 255) as f32 / 255.0
                };
                q.set_color(
                    scaled(o.color[0], jr),
                    scaled(o.color[1], jg),
                    scaled(o.color[2], jb),
                );
                q.size = 0.26;
                q.lifetime = duration;
                q.has_physics = false;
                q.free_move = true;
                q.light = Light::FullBright;
                q.behaviour = Behaviour::Trail { target };
                q
            }
        };
        if particle.set.is_some() {
            particle.sprite = self.first(ty);
            self.sprite_from_age(&mut particle);
        } else if matches!(particle.sprite, Sprite::Index(u32::MAX)) {
            particle.sprite = self.sprite_from_set(ty);
        }
        particle.random = local;
        Some(particle)
    }

    /// `ParticleEngine.tick`: tick live particles, then admit queued ones.
    /// A firework spark (`SparkParticle`: gravity 0.1, friction 0.91, 3/4 size,
    /// 48..59 ticks).
    fn spark(
        &mut self,
        at: [f64; 3],
        v: [f64; 3],
        rgba: [f32; 4],
        fade: Option<[f32; 3]>,
        trail: bool,
        twinkle: bool,
    ) -> Particle {
        let mut local = LegacyRandom::new(self.seeds.next_long());
        let mut q = Particle::at(&mut local, ParticleType::Firework, at);
        q.velocity = v;
        q.gravity = 0.1;
        q.friction = 0.91;
        q.size *= 0.75;
        q.lifetime = 48 + local.next_int_bound(12).unwrap_or(0);
        q.random = local;
        q.rgba = [rgba[0], rgba[1], rgba[2], 0.99];
        q.light = Light::FullBright;
        q.behaviour = Behaviour::Spark {
            fade,
            trail,
            twinkle,
        };
        q.set = self.set_of(ParticleType::Firework);
        self.sprite_from_age(&mut q);
        q
    }
    /// ClientLevel.createFireworks: a Starter (or poofs without explosions).
    pub fn fireworks(
        &mut self,
        at: [f64; 3],
        v: [f64; 3],
        explosions: Vec<FireworkExplosion>,
        camera: [f64; 3],
    ) {
        if explosions.is_empty() {
            for _ in 0..self.random.next_int_bound(3).unwrap_or(0) + 2 {
                let g = |r: &mut LegacyRandom| r.next_gaussian() * 0.05;
                let (vx, vz) = (g(&mut self.random), g(&mut self.random));
                self.add(
                    &ParticleOptions::simple(ParticleType::Poof),
                    at,
                    [vx, 0.005, vz],
                );
            }
            return;
        }
        let twinkle = explosions.iter().any(|e| e.twinkle);
        let lifetime = explosions.len() as i32 * 2 - 1 + if twinkle { 15 } else { 0 };
        let far = (0..3).map(|i| (at[i] - camera[i]).powi(2)).sum::<f64>() >= 256.0;
        self.starters.push(Starter {
            at,
            v,
            explosions,
            life: 0,
            lifetime,
            twinkle_delay: twinkle,
            far,
        });
    }
    fn tick_starters(&mut self) {
        for mut s in std::mem::take(&mut self.starters) {
            if s.life == 0 {
                let large = s.explosions.len() >= 3
                    || s.explosions
                        .iter()
                        .any(|e| e.shape == FireworkShape::LargeBall);
                let sound = match (large, s.far) {
                    (true, true) => "minecraft:entity.firework_rocket.large_blast_far",
                    (true, false) => "minecraft:entity.firework_rocket.large_blast",
                    (false, true) => "minecraft:entity.firework_rocket.blast_far",
                    (false, false) => "minecraft:entity.firework_rocket.blast",
                };
                let pitch = 0.95 + self.random.next_float() * 0.1;
                self.sounds.push((sound, s.at, 20.0, pitch));
            }
            if s.life % 2 == 0 && ((s.life / 2) as usize) < s.explosions.len() {
                let e = s.explosions[(s.life / 2) as usize].clone();
                // DyeColor.BLACK.getFireworkColor without colours.
                let colors = if e.colors.is_empty() {
                    vec![0x1E1B1B]
                } else {
                    e.colors.clone()
                };
                match e.shape {
                    FireworkShape::SmallBall => self.spark_ball(&s, 0.25, 2, &colors, &e),
                    FireworkShape::LargeBall => self.spark_ball(&s, 0.5, 4, &colors, &e),
                    FireworkShape::Star => self.spark_shape(&s, 0.5, &STAR, &colors, &e, false),
                    FireworkShape::Creeper => {
                        self.spark_shape(&s, 0.5, &CREEPER, &colors, &e, true)
                    }
                    FireworkShape::Burst => {
                        let g = |r: &mut LegacyRandom| r.next_gaussian();
                        let (bx, bz) = (g(&mut self.random) * 0.05, g(&mut self.random) * 0.05);
                        for _ in 0..70 {
                            let xa = s.v[0] * 0.5 + g(&mut self.random) * 0.15 + bx;
                            let za = s.v[2] * 0.5 + g(&mut self.random) * 0.15 + bz;
                            let ya = s.v[1] * 0.5 + self.random.next_double() * 0.5;
                            self.spark_from(&s, [xa, ya, za], &colors, &e);
                        }
                    }
                }
                // The flash: ColorParticleOption.create(FLASH, first colour), alpha 0.
                let c = colors[0];
                let rgba = [
                    ((c >> 16) & 255) as f32 / 255.0,
                    ((c >> 8) & 255) as f32 / 255.0,
                    (c & 255) as f32 / 255.0,
                    ((c >> 24) & 255) as f32 / 255.0,
                ];
                self.create(
                    &ParticleOptions::colored(ParticleType::Flash, rgba),
                    s.at,
                    [0.0; 3],
                );
            }
            s.life += 1;
            if s.life > s.lifetime {
                if s.twinkle_delay {
                    let sound = if s.far {
                        "minecraft:entity.firework_rocket.twinkle_far"
                    } else {
                        "minecraft:entity.firework_rocket.twinkle"
                    };
                    let pitch = 0.9 + self.random.next_float() * 0.15;
                    self.sounds.push((sound, s.at, 20.0, pitch));
                }
                continue;
            }
            self.starters.push(s);
        }
    }
    /// Starter.createParticle: a random colour and fade colour.
    fn spark_from(&mut self, s: &Starter, v: [f64; 3], colors: &[u32], e: &FireworkExplosion) {
        let rgb = |c: u32| {
            [
                ((c >> 16) & 255) as f32 / 255.0,
                ((c >> 8) & 255) as f32 / 255.0,
                (c & 255) as f32 / 255.0,
            ]
        };
        let c = colors[self.random.next_int_bound(colors.len() as i32).unwrap_or(0) as usize];
        let fade = (!e.fade.is_empty()).then(|| {
            rgb(e.fade[self.random.next_int_bound(e.fade.len() as i32).unwrap_or(0) as usize])
        });
        let [r, g, b] = rgb(c);
        let p = self.spark(s.at, v, [r, g, b, 0.99], fade, e.trail, e.twinkle);
        self.pending.push(p);
    }
    fn spark_ball(
        &mut self,
        s: &Starter,
        speed: f64,
        steps: i32,
        colors: &[u32],
        e: &FireworkExplosion,
    ) {
        for y in -steps..=steps {
            for x in -steps..=steps {
                let mut z = -steps;
                while z <= steps {
                    let d = |r: &mut LegacyRandom| (r.next_double() - r.next_double()) * 0.5;
                    let xa = f64::from(x) + d(&mut self.random);
                    let ya = f64::from(y) + d(&mut self.random);
                    let za = f64::from(z) + d(&mut self.random);
                    let len = (xa * xa + ya * ya + za * za).sqrt() / speed
                        + self.random.next_gaussian() * 0.05;
                    self.spark_from(s, [xa / len, ya / len, za / len], colors, e);
                    if y != -steps && y != steps && x != -steps && x != steps {
                        z += steps * 2 - 1;
                    }
                    z += 1;
                }
            }
        }
    }
    fn spark_shape(
        &mut self,
        s: &Starter,
        speed: f64,
        coords: &[[f64; 2]],
        colors: &[u32],
        e: &FireworkExplosion,
        flat: bool,
    ) {
        let (sx, sy) = (coords[0][0], coords[0][1]);
        self.spark_from(s, [sx * speed, sy * speed, 0.0], colors, e);
        let base = f64::from(self.random.next_float() * std::f32::consts::PI);
        let modifier = if flat { 0.034 } else { 0.34 };
        for step in 0..3 {
            let angle = base + f64::from(step as f32 * std::f32::consts::PI) * modifier;
            let (mut ox, mut oy) = (sx, sy);
            for c in &coords[1..] {
                let (tx, ty) = (c[0], c[1]);
                let mut sub = 0.25;
                while sub <= 1.0 {
                    let mut xa = (ox + (tx - ox) * sub) * speed;
                    let ya = (oy + (ty - oy) * sub) * speed;
                    let za = xa * angle.sin();
                    xa *= angle.cos();
                    for flip in [-1.0, 1.0] {
                        self.spark_from(s, [xa * flip, ya, za * flip], colors, e);
                    }
                    sub += 0.25;
                }
                (ox, oy) = (tx, ty);
            }
        }
    }
    pub fn tick(&mut self, level: &ParticleLevel) {
        self.tick_starters();
        for (options, feet, width, height, life) in std::mem::take(&mut self.emitters) {
            self.emit(&options, feet, width, height);
            if life > 1 {
                self.emitters.push((options, feet, width, height, life - 1));
            }
        }
        let mut particles = std::mem::take(&mut self.particles);
        for p in &mut particles {
            self.tick_particle(p, level);
        }
        particles.retain(|p| !p.removed);
        self.particles = particles;
        let pending = std::mem::take(&mut self.pending);
        for mut p in pending {
            if p.pre_tick {
                p.pre_tick = false;
                self.tick_particle(&mut p, level);
                p.after_constructor_tick();
            }
            // ParticleGroup.add: the last quarter fills with decreasing probability.
            let size = self.particles.len();
            if size >= MAX_PARTICLES {
                continue;
            }
            if size >= 12288 {
                let free = (MAX_PARTICLES - size) as f32 / 4096.0;
                if self.random.next_float() >= free * free {
                    continue;
                }
            }
            self.particles.push(p);
        }
    }
    #[allow(clippy::too_many_lines)]
    fn tick_particle(&mut self, p: &mut Particle, level: &ParticleLevel) {
        match p.behaviour {
            Behaviour::Base => p.base_tick(level),
            Behaviour::Animated => {
                p.base_tick(level);
                self.sprite_from_age(p);
            }
            Behaviour::Snowflake => {
                p.base_tick(level);
                self.sprite_from_age(p);
                p.velocity[0] *= f64::from(0.95f32);
                p.velocity[1] *= f64::from(0.9f32);
                p.velocity[2] *= f64::from(0.95f32);
            }
            Behaviour::Lava => {
                p.base_tick(level);
                if !p.removed {
                    let odds = p.age as f32 / p.lifetime as f32;
                    if p.random.next_float() > odds {
                        self.add(
                            &ParticleOptions::simple(ParticleType::Smoke),
                            p.position,
                            p.velocity,
                        );
                    }
                }
            }
            Behaviour::Crit => {
                p.base_tick(level);
                p.rgba[1] *= 0.96;
                p.rgba[2] *= 0.9;
            }
            Behaviour::Drip {
                fluid,
                stage,
                next,
                sound,
            } => {
                p.previous = p.position;
                // preMoveUpdate
                if stage == DripStage::CoolingHang {
                    p.rgba[0] = 1.0;
                    p.rgba[1] = 16.0 / (40 - p.lifetime + 16) as f32;
                    p.rgba[2] = 4.0 / (40 - p.lifetime + 8) as f32;
                }
                let lifetime = p.lifetime;
                p.lifetime -= 1;
                if lifetime <= 0 {
                    p.removed = true;
                    if matches!(stage, DripStage::Hang | DripStage::CoolingHang)
                        && let Some(next) = next
                    {
                        self.add(&ParticleOptions::simple(next), p.position, p.velocity);
                    }
                }
                if p.removed {
                    return;
                }
                p.velocity[1] -= f64::from(p.gravity);
                p.move_by(level, p.velocity);
                // postMoveUpdate
                match stage {
                    DripStage::Hang | DripStage::CoolingHang => {
                        for c in &mut p.velocity {
                            *c *= 0.02;
                        }
                    }
                    DripStage::Fall if p.on_ground => p.removed = true,
                    DripStage::FallAndLand if p.on_ground => {
                        p.removed = true;
                        if let Some(next) = next {
                            self.add(&ParticleOptions::simple(next), p.position, [0.0; 3]);
                        }
                        if let Some(sound) = sound {
                            let volume = p.random.next_float() * 0.7 + 0.3;
                            self.sounds.push((sound, p.position, volume, 1.0));
                        }
                    }
                    _ => {}
                }
                if p.removed {
                    return;
                }
                for c in &mut p.velocity {
                    *c *= f64::from(0.98f32);
                }
                if fluid != Fluid::None {
                    let b = ParticleLevel::block_at(p.position);
                    let (f, height) = level.fluid(b);
                    if f == fluid && p.position[1] < f64::from(b[1]) + height {
                        p.removed = true;
                    }
                }
            }
            Behaviour::WaterDrop => {
                p.previous = p.position;
                let lifetime = p.lifetime;
                p.lifetime -= 1;
                if lifetime <= 0 {
                    p.removed = true;
                    return;
                }
                p.velocity[1] -= f64::from(p.gravity);
                p.move_by(level, p.velocity);
                for c in &mut p.velocity {
                    *c *= f64::from(0.98f32);
                }
                if p.on_ground {
                    if p.random.next_float() < 0.5 {
                        p.removed = true;
                    }
                    p.velocity[0] *= f64::from(0.7f32);
                    p.velocity[2] *= f64::from(0.7f32);
                }
                let b = ParticleLevel::block_at(p.position);
                let top = level.collision_top(
                    b,
                    p.position[0] - f64::from(b[0]),
                    p.position[2] - f64::from(b[2]),
                );
                let offset = top.max(level.fluid(b).1);
                if offset > 0.0 && p.position[1] < f64::from(b[1]) + offset {
                    p.removed = true;
                }
            }
            Behaviour::Bubble => {
                p.previous = p.position;
                let lifetime = p.lifetime;
                p.lifetime -= 1;
                if lifetime <= 0 {
                    p.removed = true;
                    return;
                }
                p.velocity[1] += 0.002;
                p.move_by(level, p.velocity);
                for c in &mut p.velocity {
                    *c *= f64::from(0.85f32);
                }
                if level.fluid(ParticleLevel::block_at(p.position)).0 != Fluid::Water {
                    p.removed = true;
                }
            }
            Behaviour::SulfurBubble {
                y_start,
                y_end,
                size_start,
                y_prev,
            } => {
                p.base_tick(level);
                let (fluid, source) = level.fluid_source(ParticleLevel::block_at(p.position));
                if !(p.removed || (fluid == Fluid::Water && source)) {
                    p.removed = true;
                }
                if !p.removed && p.position[1] >= y_end {
                    p.removed = true;
                }
                if !p.removed && p.position[1] <= y_prev {
                    p.removed = true;
                }
                // randomHorizontalWiggling, then a second, horizontal move.
                let wiggle = |r: &mut LegacyRandom| {
                    let f = r.next_float() * 0.003;
                    f64::from(if r.next_boolean() { f } else { -f }) * 0.5
                };
                p.velocity[0] += wiggle(&mut p.random);
                p.velocity[2] += wiggle(&mut p.random);
                p.move_by(level, [p.velocity[0], 0.0, p.velocity[2]]);
                let progress = ((p.position[1] - y_start) / (y_end - y_start)) as f32;
                p.size = size_start + progress * (0.15 - size_start);
                p.behaviour = Behaviour::SulfurBubble {
                    y_start,
                    y_end,
                    size_start,
                    y_prev: p.position[1],
                };
            }
            Behaviour::NoxiousGas { fade_start } => {
                p.base_tick(level);
                self.sprite_from_age(p);
                if p.age as f32 > fade_start {
                    let frames_since_fade_out_start = p.age as f32 - fade_start;
                    p.rgba[3] =
                        (p.lifetime as f32 - frames_since_fade_out_start) / p.lifetime as f32;
                }
            }
            Behaviour::NoxiousGasCloud => {
                p.base_tick(level);
                if p.age % 2 == 0 {
                    let source = ParticleLevel::block_at(p.position);
                    let spawn = pick_random_particle_spawn_point(&mut self.level_random, source);
                    if crate::potent_sulfur::can_be_reached_by_noxious_gas(level, source, spawn) {
                        self.spawn_noxious_gas_particle(spawn);
                    }
                }
            }
            Behaviour::GeyserEruption { water_blocks, aux } => {
                p.base_tick(level);
                if p.age % 2 == 0 {
                    for _ in 0..2 {
                        self.add(
                            &ParticleOptions::geyser_base(
                                ParticleType::GeyserBase,
                                water_blocks,
                                1.5,
                            ),
                            p.position,
                            aux,
                        );
                    }
                }
                for _ in 0..water_blocks + 2 {
                    self.add(
                        &ParticleOptions::geyser(ParticleType::GeyserPlume, water_blocks),
                        p.position,
                        aux,
                    );
                }
                if p.age % 10 == 0 {
                    for _ in 0..20 {
                        self.add(
                            &ParticleOptions::geyser_base(
                                ParticleType::GeyserPoof,
                                water_blocks,
                                2.0,
                            ),
                            p.position,
                            aux,
                        );
                    }
                }
            }
            Behaviour::GeyserPlume {
                start_y,
                max_y,
                propulsion,
                spray,
                min_size,
                max_size,
                mut done,
            } => {
                p.base_tick(level);
                if !done
                    && (p.velocity[1] < 0.0
                        || p.position[1] > max_y
                        || p.position[1] == p.previous[1])
                {
                    p.lifetime = p.lifetime.min(p.age + 5);
                    p.friction = 0.0;
                    done = true;
                }
                let y_progress_linear =
                    ((p.position[1] - start_y) / (max_y - start_y)).clamp(0.0, 1.0);
                let y_progress_exponential = horizon_core::jmath::pow(y_progress_linear, 3.0);
                p.gravity = propulsion * y_progress_exponential as f32 * 0.12;
                p.velocity[0] = y_progress_linear * f64::from(spray[0]);
                p.velocity[2] = y_progress_linear * f64::from(spray[1]);
                self.sprite_from_age(p);
                p.size = min_size + (y_progress_linear * f64::from(max_size - min_size)) as f32;
                p.behaviour = Behaviour::GeyserPlume {
                    start_y,
                    max_y,
                    propulsion,
                    spray,
                    min_size,
                    max_size,
                    done,
                };
            }
            Behaviour::BubbleColumnUp => {
                p.base_tick(level);
                if !p.removed && level.fluid(ParticleLevel::block_at(p.position)).0 != Fluid::Water
                {
                    p.removed = true;
                }
            }
            Behaviour::BubblePop => {
                p.previous = p.position;
                let age = p.age;
                p.age += 1;
                if age >= p.lifetime {
                    p.removed = true;
                    return;
                }
                p.velocity[1] -= f64::from(p.gravity);
                p.move_by(level, p.velocity);
                self.sprite_from_age(p);
            }
            Behaviour::CurrentDown { angle } => {
                p.previous = p.position;
                let age = p.age;
                p.age += 1;
                if age >= p.lifetime {
                    p.removed = true;
                    return;
                }
                p.velocity[0] += f64::from(0.6 * angle.cos());
                p.velocity[2] += f64::from(0.6 * angle.sin());
                p.velocity[0] *= 0.07;
                p.velocity[2] *= 0.07;
                p.move_by(level, p.velocity);
                if level.fluid(ParticleLevel::block_at(p.position)).0 != Fluid::Water || p.on_ground
                {
                    p.removed = true;
                }
                p.behaviour = Behaviour::CurrentDown {
                    angle: angle + 0.08,
                };
            }
            Behaviour::SuspendedTown => {
                p.previous = p.position;
                let lifetime = p.lifetime;
                p.lifetime -= 1;
                if lifetime <= 0 {
                    p.removed = true;
                    return;
                }
                p.move_by(level, p.velocity);
                for c in &mut p.velocity {
                    *c *= 0.99;
                }
            }
            Behaviour::Portal { start } => {
                p.previous = p.position;
                let age = p.age;
                p.age += 1;
                if age >= p.lifetime {
                    p.removed = true;
                    return;
                }
                let a = p.age as f32 / p.lifetime as f32;
                let pos = 1.0 - (-a + a * a * 2.0);
                p.position = [
                    start[0] + p.velocity[0] * f64::from(pos),
                    start[1] + p.velocity[1] * f64::from(pos) + f64::from(1.0 - a),
                    start[2] + p.velocity[2] * f64::from(pos),
                ];
            }
            Behaviour::ReversePortal => {
                p.previous = p.position;
                let age = p.age;
                p.age += 1;
                if age >= p.lifetime {
                    p.removed = true;
                    return;
                }
                let m = f64::from(p.age as f32 / p.lifetime as f32);
                for i in 0..3 {
                    p.position[i] += p.velocity[i] * m;
                }
            }
            Behaviour::FlyTowards { start, .. } => {
                p.previous = p.position;
                let age = p.age;
                p.age += 1;
                if age >= p.lifetime {
                    p.removed = true;
                    return;
                }
                let pos = 1.0 - p.age as f32 / p.lifetime as f32;
                let mut pp = 1.0 - pos;
                pp *= pp;
                pp *= pp;
                p.position = [
                    start[0] + p.velocity[0] * f64::from(pos),
                    start[1] + p.velocity[1] * f64::from(pos) - f64::from(pp * 1.2),
                    start[2] + p.velocity[2] * f64::from(pos),
                ];
            }
            Behaviour::Transition { .. } => {
                p.base_tick(level);
                self.sprite_from_age(p);
            }
            Behaviour::Shriek { delay } => {
                if delay > 0 {
                    p.behaviour = Behaviour::Shriek { delay: delay - 1 };
                } else {
                    p.base_tick(level);
                }
            }
            Behaviour::Vibration {
                dest, rot, pitch, ..
            } => {
                p.previous = p.position;
                let age = p.age;
                p.age += 1;
                if age >= p.lifetime {
                    p.removed = true;
                    return;
                }
                let remaining = p.lifetime - p.age;
                if remaining > 0 {
                    let alpha = 1.0 / f64::from(remaining);
                    for i in 0..3 {
                        p.position[i] += (dest[i] - p.position[i]) * alpha;
                    }
                }
                let d = [
                    p.position[0] - dest[0],
                    p.position[1] - dest[1],
                    p.position[2] - dest[2],
                ];
                p.behaviour = Behaviour::Vibration {
                    dest,
                    rot: d[0].atan2(d[2]) as f32,
                    rot_o: rot,
                    pitch: d[1].atan2((d[0] * d[0] + d[2] * d[2]).sqrt()) as f32,
                    pitch_o: pitch,
                };
                self.sprite_from_age(p);
            }
            Behaviour::Trail { target } => {
                p.previous = p.position;
                let age = p.age;
                p.age += 1;
                if age >= p.lifetime {
                    p.removed = true;
                    return;
                }
                // `Mth.lerp(1.0 / ticksRemaining, x, target)`, infinite on the last tick.
                let alpha = 1.0 / f64::from(p.lifetime - p.age);
                for i in 0..3 {
                    p.position[i] += alpha * (target[i] - p.position[i]);
                }
            }
            Behaviour::HugeExplosion => {
                p.previous = p.position;
                let age = p.age;
                p.age += 1;
                if age >= p.lifetime {
                    p.removed = true;
                    return;
                }
                self.sprite_from_age(p);
            }
            Behaviour::GustSeed { scale, delay } => {
                if p.age % (delay + 1) == 0 {
                    for _ in 0..3 {
                        let r = &mut p.random;
                        let at = [
                            p.position[0] + (r.next_double() - r.next_double()) * scale,
                            p.position[1] + (r.next_double() - r.next_double()) * scale,
                            p.position[2] + (r.next_double() - r.next_double()) * scale,
                        ];
                        let f = f64::from(p.age as f32 / p.lifetime as f32);
                        self.add(
                            &ParticleOptions::simple(ParticleType::Gust),
                            at,
                            [f, 0.0, 0.0],
                        );
                    }
                }
                let age = p.age;
                p.age += 1;
                if age == p.lifetime {
                    p.removed = true;
                }
            }
            Behaviour::ExplosionSeed => {
                for _ in 0..6 {
                    let r = &mut p.random;
                    let at = [
                        p.position[0] + (r.next_double() - r.next_double()) * 4.0,
                        p.position[1] + (r.next_double() - r.next_double()) * 4.0,
                        p.position[2] + (r.next_double() - r.next_double()) * 4.0,
                    ];
                    let f = f64::from(p.age as f32 / p.lifetime as f32);
                    self.add(
                        &ParticleOptions::simple(ParticleType::Explosion),
                        at,
                        [f, 0.0, 0.0],
                    );
                }
                p.age += 1;
                if p.age == p.lifetime {
                    p.removed = true;
                }
            }
            Behaviour::Campfire => {
                p.previous = p.position;
                let age = p.age;
                p.age += 1;
                if age < p.lifetime && p.rgba[3] > 0.0 {
                    let r = &mut p.random;
                    let dx = r.next_float() / 5000.0 * if r.next_boolean() { 1.0 } else { -1.0 };
                    p.velocity[0] += f64::from(dx);
                    let dz = r.next_float() / 5000.0 * if r.next_boolean() { 1.0 } else { -1.0 };
                    p.velocity[2] += f64::from(dz);
                    p.velocity[1] -= f64::from(p.gravity);
                    p.move_by(level, p.velocity);
                    if p.age >= p.lifetime - 60 && p.rgba[3] > 0.01 {
                        p.rgba[3] -= 0.015;
                    }
                } else {
                    p.removed = true;
                }
            }
            Behaviour::SimpleAnimated { fade } => {
                p.base_tick(level);
                self.sprite_from_age(p);
                if p.age > p.lifetime / 2 {
                    p.rgba[3] = 1.0 - (p.age as f32 - (p.lifetime / 2) as f32) / p.lifetime as f32;
                    if let Some(f) = fade {
                        for i in 0..3 {
                            p.rgba[i] += (f[i] - p.rgba[i]) * 0.2;
                        }
                    }
                }
            }
            Behaviour::Spark {
                fade,
                trail,
                twinkle,
            } => {
                p.base_tick(level);
                self.sprite_from_age(p);
                if p.age > p.lifetime / 2 {
                    p.rgba[3] = 1.0 - (p.age as f32 - (p.lifetime / 2) as f32) / p.lifetime as f32;
                    if let Some(f) = fade {
                        for i in 0..3 {
                            p.rgba[i] += (f[i] - p.rgba[i]) * 0.2;
                        }
                    }
                }
                if trail && p.age < p.lifetime / 2 && (p.age + p.lifetime) % 2 == 0 {
                    let mut child = self.spark(p.position, [0.0; 3], p.rgba, fade, false, twinkle);
                    child.age = child.lifetime / 2;
                    self.pending.push(child);
                }
            }
            Behaviour::Spell { original_alpha } => {
                p.base_tick(level);
                self.sprite_from_age(p);
                p.rgba[3] += (original_alpha - p.rgba[3]) * 0.05;
            }
            Behaviour::Cloud => {
                p.base_tick(level);
                if !p.removed {
                    self.sprite_from_age(p);
                    if let Some((player, vy)) = level.player {
                        let d2 = (player[0] - p.position[0]).powi(2)
                            + (player[1] - p.position[1]).powi(2)
                            + (player[2] - p.position[2]).powi(2);
                        if d2 < 4.0 && p.position[1] > player[1] {
                            let y = p.position[1] + (player[1] - p.position[1]) * 0.2;
                            p.velocity[1] += (vy - p.velocity[1]) * 0.2;
                            p.set_pos([p.position[0], y, p.position[2]]);
                        }
                    }
                }
            }
            Behaviour::Leaves {
                mut rot_speed,
                spin,
                wind,
                swirl,
                flow,
                flow_scale,
                swirl_period,
            } => {
                p.previous = p.position;
                let lifetime = p.lifetime;
                p.lifetime -= 1;
                if lifetime <= 0 {
                    p.removed = true;
                }
                if !p.removed {
                    let alive = (300 - p.lifetime) as f32;
                    let relative = (alive / 300.0).min(1.0);
                    let (mut xa, mut za) = (0.0f64, 0.0f64);
                    if flow {
                        xa += flow_scale[0] * f64::from(relative).powf(1.25);
                        za += flow_scale[1] * f64::from(relative).powf(1.25);
                    }
                    if swirl {
                        let rel = f64::from(relative);
                        xa += rel * (rel * swirl_period).cos() * f64::from(wind);
                        za += rel * (rel * swirl_period).sin() * f64::from(wind);
                    }
                    p.velocity[0] += xa * f64::from(0.0025f32);
                    p.velocity[2] += za * f64::from(0.0025f32);
                    p.velocity[1] -= f64::from(p.gravity);
                    rot_speed += spin / 20.0;
                    p.o_roll = p.roll;
                    p.roll += rot_speed / 20.0;
                    p.move_by(level, p.velocity);
                    if p.on_ground
                        || p.lifetime < 299 && (p.velocity[0] == 0.0 || p.velocity[2] == 0.0)
                    {
                        p.removed = true;
                    }
                    if !p.removed {
                        let f = f64::from(p.friction);
                        for c in &mut p.velocity {
                            *c *= f;
                        }
                    }
                }
                p.behaviour = Behaviour::Leaves {
                    rot_speed,
                    spin,
                    wind,
                    swirl,
                    flow,
                    flow_scale,
                    swirl_period,
                };
            }
            Behaviour::FallingDust { rot_speed } => {
                p.previous = p.position;
                let age = p.age;
                p.age += 1;
                if age >= p.lifetime {
                    p.removed = true;
                    return;
                }
                self.sprite_from_age(p);
                p.o_roll = p.roll;
                p.roll += std::f32::consts::PI * rot_speed * 2.0;
                if p.on_ground {
                    p.o_roll = 0.0;
                    p.roll = 0.0;
                }
                p.move_by(level, p.velocity);
                p.velocity[1] -= f64::from(0.003f32);
                p.velocity[1] = p.velocity[1].max(f64::from(-0.14f32));
            }
            Behaviour::SquidInk => {
                // SimpleAnimatedParticle.tick, then SquidInkParticle's.
                p.base_tick(level);
                self.sprite_from_age(p);
                let half = |p: &mut Particle| {
                    if p.age > p.lifetime / 2 {
                        p.rgba[3] =
                            1.0 - (p.age as f32 - (p.lifetime / 2) as f32) / p.lifetime as f32;
                    }
                };
                half(p);
                if !p.removed {
                    self.sprite_from_age(p);
                    half(p);
                    let b = ParticleLevel::block_at(p.position);
                    if level.facts.is_air(level.world.block(b[0], b[1], b[2])) {
                        p.velocity[1] -= f64::from(0.0074f32);
                    }
                }
            }
            Behaviour::Firefly => {
                p.base_tick(level);
                let b = ParticleLevel::block_at(p.position);
                if !level.facts.is_air(level.world.block(b[0], b[1], b[2])) {
                    p.removed = true;
                } else {
                    p.rgba[3] = fade_amount(p.lifetime_progress(p.age as f32), 0.3, 0.5);
                    if p.random.next_float() > 0.95 || p.age == 1 {
                        let r = &mut p.random;
                        let mut speed = || f64::from(-0.05f32 + 0.1 * r.next_float());
                        p.velocity = [speed(), speed(), speed()];
                    }
                }
            }
            Behaviour::Wake => {
                p.previous = p.position;
                let life = 60 - p.lifetime;
                let lifetime = p.lifetime;
                p.lifetime -= 1;
                if lifetime <= 0 {
                    p.removed = true;
                    return;
                }
                p.velocity[1] -= f64::from(p.gravity);
                p.move_by(level, p.velocity);
                for c in &mut p.velocity {
                    *c *= f64::from(0.98f32);
                }
                let size = life as f32 * 0.001;
                p.set_size(size, size);
                // SpriteSet.get(life % 4, 4).
                if let Some(set) = p.set {
                    let sprites = &self.set_sprites[set as usize];
                    let index = (life % 4) as usize * (sprites.len() - 1) / 4;
                    p.sprite = Sprite::Index(sprites[index]);
                }
            }
            Behaviour::DustPlume => {
                p.gravity *= 0.88;
                p.friction *= 0.92;
                p.base_tick(level);
                self.sprite_from_age(p);
            }
            Behaviour::FlyStraight { start, colors } => {
                p.previous = p.position;
                let age = p.age;
                p.age += 1;
                if age >= p.lifetime {
                    p.removed = true;
                    return;
                }
                let normalized = p.age as f32 / p.lifetime as f32;
                let along = 1.0 - normalized;
                for i in 0..3 {
                    p.position[i] = start[i] + p.velocity[i] * f64::from(along);
                }
                // ARGB.srgbLerp: Mth.lerpInt on each 8-bit channel.
                let lerp = |shift: u32| {
                    let (a, b) = (
                        (colors[0] >> shift & 255) as i32,
                        (colors[1] >> shift & 255) as i32,
                    );
                    (a + (normalized * (b - a) as f32).floor() as i32) as f32 / 255.0
                };
                p.set_color(lerp(16), lerp(8), lerp(0));
                p.rgba[3] = lerp(24);
            }
            Behaviour::DragonBreath { mut hit_ground } => {
                p.previous = p.position;
                let age = p.age;
                p.age += 1;
                if age >= p.lifetime {
                    p.removed = true;
                    return;
                }
                self.sprite_from_age(p);
                if p.on_ground {
                    p.velocity[1] = 0.0;
                    hit_ground = true;
                }
                if hit_ground {
                    p.velocity[1] += 0.002;
                }
                p.move_by(level, p.velocity);
                if p.position[1] == p.previous[1] {
                    p.velocity[0] *= 1.1;
                    p.velocity[2] *= 1.1;
                }
                let f = f64::from(p.friction);
                p.velocity[0] *= f;
                p.velocity[2] *= f;
                if hit_ground {
                    p.velocity[1] *= f;
                }
                p.behaviour = Behaviour::DragonBreath { hit_ground };
            }
        }
    }

    /// Camera-facing quads (`QuadParticleRenderState.renderRotatedQuad`).
    pub fn render(
        &self,
        camera: DVec3,
        yaw: f32,
        pitch: f32,
        partial: f32,
        light_at: &dyn Fn([i32; 3]) -> u8,
        out: &mut Vec<Vertex>,
    ) {
        // Camera.setRotation: rotationYXZ(PI - yRot, -xRot, 0).
        let base = Quat::from_rotation_y(std::f32::consts::PI - yaw.to_radians())
            * Quat::from_rotation_x(-pitch.to_radians());
        for p in &self.particles {
            if p.no_render {
                continue;
            }
            // SparkParticle.extract: twinkling sparks blink after a third of their life.
            if let Behaviour::Spark { twinkle: true, .. } = p.behaviour
                && p.age >= p.lifetime / 3
                && ((p.age + p.lifetime) / 3) % 2 != 0
            {
                continue;
            }
            // `SubmitNodeCollection.submitQuadParticleGroup`: the translucent
            // layer draws after translucent terrain, so a crumb of stained
            // glass cannot hide the glass behind it (`getLayer`).
            let (rect, layer) = match &p.sprite {
                Sprite::Index(i) => (
                    self.rects.get(*i as usize).copied().flatten(),
                    p.ty.translucent_layer(),
                ),
                Sprite::Named(name) => match self.atlas.get(name) {
                    Some(&(rect, translucent)) => (Some(rect), translucent),
                    None => (None, false),
                },
            };
            let Some(r) = rect else { continue };
            let a = f64::from(partial);
            let pos = DVec3::new(
                p.previous[0] + (p.position[0] - p.previous[0]) * a,
                p.previous[1] + (p.position[1] - p.previous[1]) * a,
                p.previous[2] + (p.position[2] - p.previous[2]) * a,
            ) - camera;
            let center = pos.as_vec3();
            let rotation = if p.roll != 0.0 {
                base * Quat::from_rotation_z(p.o_roll + (p.roll - p.o_roll) * partial)
            } else {
                base
            };
            let size = p.quad_size(partial);
            let mut alpha = p.rgba[3];
            // OverlayParticle.extract: the flash fades as it pulses.
            if p.ty == ParticleType::Flash {
                alpha = 0.6 - (p.age as f32 + partial - 1.0) * 0.25 * 0.5;
            }
            if let Behaviour::FlyTowards { alpha: la, .. } = p.behaviour {
                alpha = lifetime_alpha(la, p.age, p.lifetime, partial);
            }
            let mut rotations = [Some(rotation), None];
            let mut rgb = [p.rgba[0], p.rgba[1], p.rgba[2]];
            match p.behaviour {
                // ShriekParticle.extract: two tilted quads, fading out.
                Behaviour::Shriek { delay } => {
                    if delay > 0 {
                        continue;
                    }
                    alpha = 1.0 - ((p.age as f32 + partial) / p.lifetime as f32).clamp(0.0, 1.0);
                    rotations = [
                        Some(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_3)),
                        Some(Quat::from_euler(
                            glam::EulerRot::YXZ,
                            -std::f32::consts::PI,
                            std::f32::consts::FRAC_PI_3,
                            0.0,
                        )),
                    ];
                }
                // VibrationSignalParticle.extract: two quads along the flight.
                Behaviour::Vibration {
                    rot,
                    rot_o,
                    pitch,
                    pitch_o,
                    ..
                } => {
                    let sway =
                        ((p.age as f32 + partial - std::f32::consts::TAU) * 0.05).sin() * 2.0;
                    let lerped_rot = rot_o + (rot - rot_o) * partial;
                    let lerped_pitch =
                        pitch_o + (pitch - pitch_o) * partial + std::f32::consts::FRAC_PI_2;
                    rotations = [
                        Some(
                            Quat::from_rotation_y(lerped_rot)
                                * Quat::from_rotation_x(-lerped_pitch)
                                * Quat::from_rotation_y(sway),
                        ),
                        Some(
                            Quat::from_rotation_y(-std::f32::consts::PI + lerped_rot)
                                * Quat::from_rotation_x(lerped_pitch)
                                * Quat::from_rotation_y(sway),
                        ),
                    ];
                }
                // DustColorTransitionParticle.lerpColors.
                Behaviour::Transition { from, to } => {
                    let t = (p.age as f32 + partial) / (p.lifetime as f32 + 1.0);
                    rgb = std::array::from_fn(|i| from[i] + (to[i] - from[i]) * t);
                }
                _ => {}
            }
            let block = ParticleLevel::block_at(p.position);
            let packed = light_at(block);
            let (sky_light, block_light) = p.light_coords(
                partial,
                u32::from(packed >> 4) * 16,
                u32::from(packed & 15) * 16,
            );
            let light = sky_light | block_light << 8;
            let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as u32;
            let color = c(alpha) << 24 | c(rgb[2]) << 16 | c(rgb[1]) << 8 | c(rgb[0]);
            let (du, dv) = (r.u1 - r.u0, r.v1 - r.v0);
            let (u0, v0, u1, v1) = (
                r.u0 + du * p.uv[0],
                r.v0 + dv * p.uv[1],
                r.u0 + du * p.uv[2],
                r.v0 + dv * p.uv[3],
            );
            for rotation in rotations.into_iter().flatten() {
                for (n, uv) in [
                    ((1.0, -1.0), [u1, v1]),
                    ((1.0, 1.0), [u1, v0]),
                    ((-1.0, 1.0), [u0, v0]),
                    ((-1.0, -1.0), [u0, v1]),
                ] {
                    let v = center + rotation * FVec3::new(n.0, n.1, 0.0) * size;
                    out.push(Vertex {
                        normal: [0; 4],
                        pos: v.to_array(),
                        uv,
                        layer: if layer {
                            crate::gfx::TRANSLUCENT_PARTICLE_OVERLAY
                        } else {
                            u32::from(hdr_tier(p.ty)) << 16
                        },
                        color,
                        light,

                        glint_uv: [0.0, 0.0],
                    });
                }
            }
        }
    }
}
/// How brightly particle `ty` shines in HDR output, as `mesher::hdr_tier`: flames and
/// glowing drops, not their smoke. Only opaque-layer particles are marked.
fn hdr_tier(ty: ParticleType) -> u8 {
    use ParticleType as P;
    match ty {
        P::Flame
        | P::SoulFireFlame
        | P::CopperFireFlame
        | P::SmallFlame
        | P::Lava
        | P::DrippingLava
        | P::FallingLava
        | P::LandingLava
        | P::DrippingDripstoneLava
        | P::FallingDripstoneLava => crate::mesher::INCANDESCENT,
        P::DrippingObsidianTear | P::FallingObsidianTear | P::LandingObsidianTear => {
            crate::mesher::LOW
        }
        _ => 0,
    }
}
/// `Particle.LifetimeAlpha.currentAlphaForAge`.
fn lifetime_alpha(a: [f32; 4], age: i32, lifetime: i32, partial: f32) -> f32 {
    if (a[0] - a[1]).abs() < 1.0e-5 {
        return a[0];
    }
    let t = ((age as f32 + partial) / lifetime as f32 - a[2]) / (a[3] - a[2]);
    a[0] + (a[1] - a[0]) * t.clamp(0.0, 1.0)
}

struct Starter {
    at: [f64; 3],
    v: [f64; 3],
    explosions: Vec<FireworkExplosion>,
    life: i32,
    lifetime: i32,
    twinkle_delay: bool,
    far: bool,
}
const STAR: [[f64; 2]; 6] = [
    [0.0, 1.0],
    [0.3455, 0.309],
    [0.9511, 0.309],
    [0.379_591_836_734_693_9, -0.126_530_612_244_897_95],
    [0.612_244_897_959_183_7, -0.804_081_632_653_061_2],
    [0.0, -0.359_183_673_469_387_73],
];
const CREEPER: [[f64; 2]; 12] = [
    [0.0, 0.2],
    [0.2, 0.2],
    [0.2, 0.6],
    [0.6, 0.6],
    [0.6, 0.2],
    [0.2, 0.2],
    [0.2, 0.0],
    [0.4, 0.0],
    [0.4, -0.6],
    [0.2, -0.6],
    [0.2, -0.4],
    [0.0, -0.4],
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn egg_teleport_unpacks_offsets_and_preserves_seeded_draw_order() {
        let mut engine = ParticleEngine::new(Path::new(""), &HashMap::new(), &HashSet::new());
        engine.seed_randoms(0, 0, 0);
        // Destination offset (-15, 7, 15); the unused top byte is ignored.
        engine.block_teleportation([4, 12, -5], 0xff01_0f1fu32 as i32, [16, 8, 16]);
        assert_eq!(engine.pending.len(), 128);
        assert!(engine.pending.iter().all(|p| p.ty == ParticleType::Portal));
        let first = &engine.pending[0];
        assert_eq!(
            first.position.map(f64::to_bits),
            [0x3fd18220c05a530a, 0x402b001e6be383c1, 0xbfc76bf5c46dbe5c]
        );
        assert_eq!(
            first.velocity.map(f64::to_bits),
            [0xbfaa91aee0000000, 0x3f95c78ce0000000, 0x3f9c24a0e0000000]
        );
        assert_eq!(
            engine.pending[127].position.map(f64::to_bits),
            [0xbfd728b135cb9ef6, 0x402ddd72dc6d0019, 0x3fe0b5775a29f186]
        );
        assert_eq!(engine.level_random.state(), 99_413_408_083_437);
        assert!(engine.sounds.is_empty());
    }

    #[test]
    fn consume_teleport_unpacks_offsets_within_127() {
        let mut engine = ParticleEngine::new(Path::new(""), &HashMap::new(), &HashSet::new());
        engine.seed_randoms(0, 0, 0);
        // TeleportRandomlyConsumeEffect: BlockUtil.clampedPackDifferenceInPosition
        // of the move (-7, -3, 9) with radii 127, from (4, 12, -5) to (-3, 9, 4).
        let data = (-7 + 127) << 16 | (-3 + 127) << 8 | (9 + 127);
        engine.block_teleportation([4, 12, -5], data, [127, 127, 127]);
        assert_eq!(engine.pending.len(), 128);
        for p in &engine.pending {
            let [x, y, z] = p.position;
            assert!((-3.0..5.0).contains(&x), "{x}");
            assert!((8.5..12.5).contains(&y), "{y}");
            assert!((-5.0..5.0).contains(&z), "{z}");
        }
        // The same draws as the egg's: only the unpacking differs.
        assert_eq!(engine.level_random.state(), 99_413_408_083_437);
    }

    #[test]
    fn egg_teleport_options_roll_before_distance_rejection() {
        for (option, count, state) in [
            (0, 128, 99_413_408_083_437),
            (1, 89, 244_221_347_771_501),
            (2, 0, 99_413_408_083_437),
        ] {
            for distant in [false, true] {
                let mut engine =
                    ParticleEngine::new(Path::new(""), &HashMap::new(), &HashSet::new());
                engine.seed_randoms(0, 0, 0);
                if distant {
                    engine.camera = [1000.0; 3];
                }
                engine.particle_level = option;
                engine.block_teleportation([4, 12, -5], 0x010f1f, [16, 8, 16]);
                assert_eq!(engine.pending.len(), if distant { 0 } else { count });
                assert_eq!(engine.level_random.state(), state);
            }
        }
    }

    #[test]
    fn mining_particles_follow_slab_and_trapdoor_height() {
        for height in [0.5, 3.0 / 16.0] {
            let boxes = [[0.0, 0.0, 0.0, 1.0, height, 1.0]];
            let p = breaking_position(&mut LegacyRandom::new(1), [2, 97, 3], [0, 1, 0], &boxes)
                .unwrap();
            assert_eq!(p[1], 97.0 + height + f64::from(0.1f32));
            for seed in 0..128 {
                let p = breaking_position(&mut LegacyRandom::new(seed), [0; 3], [0, 0, -1], &boxes)
                    .unwrap();
                assert!(p[1] <= height, "crumb above shape: {p:?}");
            }
        }
    }
    #[test]
    fn destroyed_slab_uses_its_shape_and_particle_density() {
        let mut burst = Vec::new();
        destroy_positions([0; 3], &[[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]], |p, v| {
            burst.push((p, v))
        });
        assert_eq!(burst.len(), 32);
        assert!(burst.iter().all(|(p, _)| p[1] < 0.5));
        assert_eq!(burst[0], ([0.125, 0.125, 0.125], [-0.375, -0.25, -0.375]));
    }
    #[test]
    fn empty_outline_emits_no_terrain_particles_or_random_draws() {
        let mut r = LegacyRandom::new(1);
        assert_eq!(breaking_position(&mut r, [0; 3], [0, 1, 0], &[]), None);
        assert_eq!(r.next_long(), LegacyRandom::new(1).next_long());
        destroy_positions([0; 3], &[], |_, _| panic!("empty shape emitted a particle"));
    }
    #[test]
    fn every_hit_face_consumes_three_position_draws() {
        let boxes = [[0.25, 0.0, 0.25, 0.75, 0.5, 0.75]];
        for face in [
            [-1, 0, 0],
            [1, 0, 0],
            [0, -1, 0],
            [0, 1, 0],
            [0, 0, -1],
            [0, 0, 1],
        ] {
            let mut random = LegacyRandom::new(42);
            let p = breaking_position(&mut random, [0; 3], face, &boxes).unwrap();
            let i = face.iter().position(|v| *v != 0).unwrap();
            let expected = if face[i] < 0 {
                boxes[0][i] - f64::from(0.1f32)
            } else {
                boxes[0][i + 3] + f64::from(0.1f32)
            };
            assert_eq!(p[i], expected);
            let mut reference = LegacyRandom::new(42);
            for _ in 0..3 {
                reference.next_double();
            }
            assert_eq!(random.next_long(), reference.next_long());
        }
    }
    #[test]
    fn multipart_destroy_burst_uses_each_box_and_caps_widths() {
        let boxes = [
            [0.0, 0.0, 0.0, 1.0, 0.5, 1.0],
            [0.0, 0.5, 0.5, 1.0, 1.0, 1.0],
        ];
        let mut burst = Vec::new();
        destroy_positions([0; 3], &boxes, |p, v| burst.push((p, v)));
        assert_eq!(burst.len(), 48);
        assert_eq!(burst[32].0, [0.125, 0.625, 0.625]);
        let mut tall = Vec::new();
        destroy_positions([0; 3], &[[0.0, 0.0, 0.0, 1.0, 1.5, 1.0]], |p, _| {
            tall.push(p)
        });
        assert_eq!(tall.len(), 64);
        assert!(tall.iter().all(|p| p[1] < 1.0));
    }
    #[test]
    fn particle_names_round_trip() {
        for &ty in ParticleType::ALL {
            assert_eq!(
                ParticleType::from_name(&format!("minecraft:{}", ty.name())),
                Some(ty)
            );
        }
        assert_eq!(ParticleType::from_name("minecraft:not_a_particle"), None);
    }
    #[test]
    fn terrain_crumb_samples_a_mirrored_quarter() {
        let mut r = LegacyRandom::new(1);
        let p = Particle::terrain(&mut r, [0.5; 3], [0.0; 3], "block/stone".into(), 0xFFFFFF);
        assert!((p.uv[0] - p.uv[2] - 0.25).abs() < 1e-6);
        assert!((p.uv[3] - p.uv[1] - 0.25).abs() < 1e-6);
        assert!((p.rgba[0] - 0.6).abs() < 1e-6);
        // The initial speed is normalized to 0.15..0.45 * 0.4 plus 0.1 upwards.
        let s =
            (p.velocity[0].powi(2) + (p.velocity[1] - 0.1).powi(2) + p.velocity[2].powi(2)).sqrt();
        assert!((0.15 * 0.4 - 1e-6..=0.45 * 0.4 + 1e-6).contains(&s));
    }
    #[test]
    fn sizes_follow_vanilla_curves() {
        let mut r = LegacyRandom::new(2);
        let mut p = Particle::at(&mut r, ParticleType::Flame, [0.0; 3]);
        p.size = 1.0;
        p.lifetime = 10;
        p.age = 5;
        p.size_curve = Size::Flame;
        assert!((p.quad_size(0.0) - (1.0 - 0.25 * 0.5)).abs() < 1e-6);
        p.size_curve = Size::GrowIn;
        p.age = 0;
        assert!((p.quad_size(0.1) - (0.1f32 / 10.0 * 32.0)).abs() < 1e-6);
    }
    #[test]
    fn lifetime_alpha_interpolates() {
        let a = [0.0, 0.6, 0.25, 1.0];
        assert_eq!(lifetime_alpha(a, 0, 40, 0.0), 0.0);
        assert!((lifetime_alpha(a, 40, 40, 0.0) - 0.6).abs() < 1e-6);
    }
}
