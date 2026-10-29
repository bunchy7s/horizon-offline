//! Mobs: entity types (dimensions and default attributes exported from
//! vanilla), models (vanilla `LayerDefinitions` baked part trees) and the
//! `Mob` entity with vanilla's AI framework: goal and target selectors with
//! priorities and control flags (`goals`), A* pathfinding with walk, swim,
//! fly and amphibious node evaluators (`path`, `nav`), move/look/jump/body
//! controls (`control`), `LivingEntity` physics (`living`) and per-species
//! behaviour, models, animations and sounds.
pub mod ai;
pub mod allay;
pub mod ambient;
pub mod anim;
pub mod animal_brain;
pub mod aquatic;
pub mod armadillo;
pub mod axolotl;
pub mod bat;
pub mod bee;
pub mod body_anim;
pub mod brain;
pub mod breeze;
pub mod camel;
pub mod caravan;
pub mod control;
pub mod copper_golem;
pub mod creaking;
pub mod creaking_heart;
pub mod cube;
pub mod data;
pub mod dolphin;
pub mod doors;
pub mod drops;
pub mod enderman;
pub mod equine;
pub mod farm;
pub mod finalize;
pub mod fox;
pub mod frog;
pub mod gear;
pub mod goals;
pub mod goat;
pub mod golem;
pub mod guardian;
pub mod happy_ghast;
pub mod hostile;
pub mod illager;
pub mod illusioner;
pub mod keyframes;
pub mod leash;
pub mod living;
pub mod llama;
pub mod monster_brain;
pub mod nautilus;
pub mod nav;
pub mod nether;
pub mod panda;
pub mod parrot;
pub mod path;
pub mod phantom;
pub mod piglin;
pub mod polar_bear;
pub mod rabbit;
pub mod raider;
pub mod ravager;
pub mod render;
pub mod riders;
pub mod seen_grid;
pub mod shulker;
pub mod sniffer;
pub mod spawn_rules;
pub mod spawn_selection;
pub mod spawner;
pub mod spawning;
pub mod species;
pub mod strider;
pub mod sulfur_cube;
pub mod sync;
pub mod tame;
pub mod trader;
pub mod transport_items;
pub mod trial_spawner;
pub mod turtle;
pub mod undead;
pub mod vex;
pub mod villager;
pub mod villager_brain;
pub mod villager_hats;
pub mod warden;
pub mod warden_brain;
pub mod witch;
pub mod wither;
mod world;
pub use world::visual_clip;
pub mod zombie_villager;

use crate::{blocks::BlockFacts, mesher::Vertex, world::World};
pub use data::MobData;
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::{LegacyRandom, RandomSource},
};
use horizon_world::block_collision::BlockCollisionRegistry;
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path, sync::Arc};

#[derive(Clone, Debug)]
pub struct Polygon {
    pub normal: [f32; 3],
    pub vertices: [[f32; 5]; 4],
}
impl Polygon {
    fn vertex_normal(&self, pose: ModelNormals) -> [i8; 4] {
        let n = pose.matrix * glam::Vec3::from_array(self.normal);
        let n = if pose.trusted {
            n
        } else {
            n.normalize_or_zero()
        };
        let packed = n.to_array().map(|v| (v.clamp(-1.0, 1.0) * 127.0) as i8);
        [packed[0], packed[1], packed[2], -127]
    }
}

/// ModelPart uses a separate normal stack: uniform scaling keeps normal lengths,
/// while nonuniform scaling requires normalization after the complete part transform.
#[derive(Clone, Copy)]
struct ModelNormals {
    matrix: glam::Mat3,
    trusted: bool,
}
impl ModelNormals {
    // shortcut: infer the root normal pose from its geometry matrix; pass an explicit
    // root normal stack when root-transform rounding is ported as well.
    fn new(base: glam::Mat4) -> Self {
        let m = glam::Mat3::from_mat4(base);
        let lengths = glam::Vec3::new(m.x_axis.length(), m.y_axis.length(), m.z_axis.length());
        let uniform =
            lengths.min_element() > 0.0 && lengths.max_element() - lengths.min_element() < 1.0e-6;
        Self {
            matrix: if uniform {
                m / lengths.x
            } else {
                m.inverse().transpose()
            },
            trusted: uniform,
        }
    }
    fn rotation(x: f32, y: f32, z: f32) -> glam::Mat3 {
        // JOML derives cosine from the rounded sine, then retains quaternion length.
        // Assuming an exactly unit quaternion changes a packed normal by one byte.
        let sin_cos = |angle: f32| {
            let angle = angle * 0.5;
            let sin = f64::from(angle).sin() as f32;
            let cos = (1.0 - sin * sin).sqrt();
            let a = angle + std::f32::consts::FRAC_PI_2;
            let tau = std::f32::consts::TAU;
            let mut quadrant = a - (a / tau) as i32 as f32 * tau;
            if quadrant < 0.0 {
                quadrant += tau;
            }
            (
                sin,
                if quadrant >= std::f32::consts::PI {
                    -cos
                } else {
                    cos
                },
            )
        };
        let (sx, cx) = sin_cos(x);
        let (sy, cy) = sin_cos(y);
        let (sz, cz) = sin_cos(z);
        let (cycz, sysz, sycz, cysz) = (cy * cz, sy * sz, sy * cz, cy * sz);
        let (w, x, y, z) = (
            cx * cycz + sx * sysz,
            sx * cycz - cx * sysz,
            cx * sycz + sx * cysz,
            cx * cysz - sx * sycz,
        );
        let (ww, xx, yy, zz) = (w * w, x * x, y * y, z * z);
        let (xy, xz, xw, yz, yw, zw) = (x * y, x * z, x * w, y * z, y * w, z * w);
        glam::Mat3::from_cols_array(&[
            ww + xx - zz - yy,
            (xy + xy) + (zw + zw),
            (xz + xz) - (yw + yw),
            (xy + xy) - (zw + zw),
            yy - zz + ww - xx,
            (yz + yz) + (xw + xw),
            (yw + yw) + (xz + xz),
            (yz + yz) - (xw + xw),
            zz - yy - xx + ww,
        ])
    }
    fn part(mut self, pose: [f32; 9]) -> Self {
        self.matrix *= Self::rotation(pose[3], pose[4], pose[5]);
        let scale = glam::Vec3::new(pose[6], pose[7], pose[8]);
        if scale.x.abs() == scale.y.abs() && scale.x.abs() == scale.z.abs() {
            if scale.min_element() < 0.0 {
                self.matrix *= glam::Mat3::from_diagonal(scale.signum());
            }
        } else {
            self.matrix *= glam::Mat3::from_diagonal(scale.recip());
            self.trusted = false;
        }
        self
    }
}

#[derive(Clone, Debug)]
pub struct Part {
    pub name: String,
    /// x, y, z, xRot, yRot, zRot, xScale, yScale, zScale
    pub pose: [f32; 9],
    pub polygons: Vec<Polygon>,
    pub children: Vec<Part>,
}
impl Part {
    fn parse(name: &str, v: &Value) -> Self {
        let pose: Vec<f32> = v["pose"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|x| x.as_f64().unwrap_or(0.0) as f32)
            .collect();
        let mut p = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        for (i, x) in pose.iter().enumerate().take(9) {
            p[i] = *x;
        }
        let polygons = v["polygons"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(index, poly)| {
                let verts: Vec<[f32; 5]> = poly
                    .as_array()?
                    .iter()
                    .map(|vx| {
                        let a: Vec<f32> = vx
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|x| x.as_f64().unwrap_or(0.0) as f32)
                            .collect();
                        [a[0], a[1], a[2], a[3], a[4]]
                    })
                    .collect();
                (verts.len() == 4).then(|| Polygon {
                    normal: v["normals"][index].as_array().map_or_else(
                        || {
                            // shortcut: collapsed legacy faces have no recoverable normal; re-prepare resources.
                            let p = verts
                                .iter()
                                .map(|v| glam::Vec3::new(v[0], v[1], v[2]))
                                .collect::<Vec<_>>();
                            (p[1] - p[0])
                                .cross(p[2] - p[0])
                                .normalize_or_zero()
                                .to_array()
                        },
                        |n| std::array::from_fn(|i| n[i].as_f64().unwrap_or(0.0) as f32),
                    ),
                    vertices: [verts[0], verts[1], verts[2], verts[3]],
                })
            })
            .collect();
        let mut children: Vec<Part> = v["children"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, c)| Part::parse(k, c))
            .collect();
        children.sort_by(|a, b| a.name.cmp(&b.name));
        // A model whose root holds a part also named "root" (allay, vex,
        // frog...): that inner part is what `root.getChild("root")` and the
        // keyframe bones mean, so the outer one gives up the name.
        let name = if name == "root" && children.iter().any(|c| c.name == "root") {
            ""
        } else {
            name
        };
        Self {
            name: name.to_owned(),
            pose: p,
            polygons,
            children,
        }
    }
}
/// Kept for callers of the earlier simple AI; species now select behaviour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Brain {
    Passive,
    Chicken,
    Zombie,
    Skeleton,
    Spider,
    Creeper,
}
/// Model animation family (vanilla `EntityModel.setupAnim` implementations).
pub use anim::Anim;
pub use species::Species;

/// Per-type behaviour switches derived from vanilla classes and tags.
#[derive(Clone, Copy, Debug, Default)]
pub struct Flags {
    /// WaterAnimal / fish: not affected by fluid physics (swims with its own control).
    pub aquatic_swimmer: bool,
    pub fall_immune: bool,
    /// FlyingMob-style travel (ghast, phantom): travelFlying without gravity.
    pub flying: bool,
    /// Spiders: onClimbable while colliding horizontally.
    pub climber: bool,
    /// Striders stand on lava.
    pub lava_walker: bool,
    /// LivingEntity.omnidirectionalAirMover (bees and parrots).
    pub omnidirectional: bool,
    pub fire_immune: bool,
    /// EntityTypeTags.BURN_IN_DAYLIGHT
    pub burns_in_day: bool,
    pub undead: bool,
    pub breathes_water: bool,
    /// Drowns out of water (fish, squid, dolphins after their air runs out).
    pub needs_water: bool,
    /// Hurt by water (endermen, blazes, striders out of lava, snow golems).
    pub water_sensitive: bool,
    pub animal: bool,
}
#[derive(Clone, Debug)]
pub struct MobType {
    pub id: String,
    pub species: Species,
    pub width: f64,
    pub height: f64,
    pub eye_height: f64,
    pub category: String,
    pub max_health: f32,
    pub speed: f64,
    pub flying_speed: f64,
    pub attack_damage: f32,
    pub attack_knockback: f64,
    pub follow_range: f64,
    pub step_height: f64,
    pub armor: f32,
    pub armor_toughness: f32,
    pub knockback_resistance: f64,
    pub jump_strength: f64,
    /// Every default attribute value of the type (`entity-types.json`).
    pub attributes: Arc<HashMap<String, f64>>,
    pub gravity: f64,
    pub safe_fall_distance: f64,
    pub fall_damage_multiplier: f64,
    pub water_efficiency: f64,
    pub water_slowdown: f32,
    /// Air speed for travelFlying mobs (ghast 0.02, phantom 0.2).
    pub air_speed: f32,
    pub baby_scale: f64,
    pub head_rot_speed: f32,
    pub max_head_x_rot: f32,
    pub max_head_y_rot: f32,
    pub ambient_interval: i32,
    pub flags: Flags,
    pub model: Arc<Part>,
    /// Every baked layer of this entity's models keyed by `model#layer`
    /// (e.g. `sheep#wool`, `zombie_baby#main`).
    pub layers: HashMap<String, Arc<Part>>,
    pub texture: String,
    pub brain: Brain,
    pub anim: Anim,
    /// Ambient, hurt and death sound events (without the namespace).
    pub sounds: [&'static str; 3],
    pub step_sound: Option<&'static str>,
    /// Animal.isFood items (resolved from the item tag).
    pub food: Arc<[String]>,
    /// TemptGoal items when they differ from the food.
    pub tempt: Arc<[String]>,
}
impl MobType {
    pub fn layer(&self, key: &str) -> Option<&Arc<Part>> {
        self.layers.get(key)
    }
}
/// Each type's default attribute values, parsed exactly (serde_json's
/// default float parsing can miss the nearest double by one ulp).
fn exact_attributes(bytes: &[u8]) -> HashMap<String, HashMap<String, f64>> {
    use serde_json::value::RawValue;
    type Raw = HashMap<String, Box<RawValue>>;
    let Ok(types) = serde_json::from_slice::<Raw>(bytes) else {
        return HashMap::new();
    };
    types
        .into_iter()
        .filter_map(|(id, t)| {
            let t: Raw = serde_json::from_str(t.get()).ok()?;
            let attrs: Raw = serde_json::from_str(t.get("attributes")?.get()).ok()?;
            let values = attrs
                .into_iter()
                .filter_map(|(k, v)| v.get().trim().parse::<f64>().ok().map(|v| (k, v)))
                .collect();
            Some((id, values))
        })
        .collect()
}
pub struct MobTypes {
    pub types: HashMap<String, MobType>,
}
impl MobTypes {
    pub fn load(root: &Path) -> crate::Result<Self> {
        let bytes = fs::read(root.join("entity-types.json"))?;
        let types: Value = serde_json::from_slice(&bytes)?;
        let exact = exact_attributes(&bytes);
        let models: Value = serde_json::from_slice(&fs::read(root.join("entity-models.json"))?)?;
        let item_tags = crate::items::load_tags(root, "item").unwrap_or_default();
        sulfur_cube::load_archetype_items(&item_tags);
        let tag = |name: &str| -> Arc<[String]> {
            let mut v: Vec<String> = item_tags
                .get(&format!("minecraft:{name}"))
                .into_iter()
                .flatten()
                .cloned()
                .collect();
            v.sort();
            v.into()
        };
        let mut out = HashMap::new();
        for def in species::ALL {
            let id = format!("minecraft:{}", def.name);
            let t = &types[&id];
            if t.is_null() {
                continue;
            }
            let model_key = format!("minecraft:{}", def.model);
            let Some(model) = models.get(&model_key) else {
                continue;
            };
            let defaults = exact.get(&id).cloned().unwrap_or_default();
            let attr = |k: &str| defaults.get(&format!("minecraft:{k}")).copied();
            let mut layers = HashMap::new();
            if let Some(all) = models.as_object() {
                for prefix in def.layer_prefixes() {
                    let start = format!("minecraft:{prefix}#");
                    for (k, v) in all.iter().filter(|(k, _)| k.starts_with(&start)) {
                        layers.insert(
                            k.trim_start_matches("minecraft:").to_owned(),
                            Arc::new(Part::parse("root", v)),
                        );
                    }
                }
            }
            let category = t["category"].as_str().unwrap_or("creature").to_owned();
            let speed = attr("movement_speed").unwrap_or(0.25);
            let flags = def.flags(t["fire_immune"].as_bool().unwrap_or(false));
            out.insert(
                id.clone(),
                MobType {
                    species: def.species,
                    width: t["width"].as_f64().unwrap_or(0.6),
                    height: t["height"].as_f64().unwrap_or(1.8),
                    eye_height: t["eye_height"].as_f64().unwrap_or(1.6),
                    max_health: attr("max_health").unwrap_or(10.0) as f32,
                    speed,
                    flying_speed: attr("flying_speed").unwrap_or(0.4),
                    attack_damage: attr("attack_damage").unwrap_or(0.0) as f32,
                    attack_knockback: attr("attack_knockback").unwrap_or(0.0),
                    follow_range: attr("follow_range").unwrap_or(16.0),
                    step_height: attr("step_height").unwrap_or(0.6),
                    armor: attr("armor").unwrap_or(0.0) as f32,
                    armor_toughness: attr("armor_toughness").unwrap_or(0.0) as f32,
                    knockback_resistance: attr("knockback_resistance").unwrap_or(0.0),
                    jump_strength: attr("jump_strength").unwrap_or(0.42),
                    attributes: Arc::new(defaults.clone()),
                    gravity: attr("gravity").unwrap_or(0.08),
                    safe_fall_distance: attr("safe_fall_distance").unwrap_or(3.0),
                    fall_damage_multiplier: attr("fall_damage_multiplier").unwrap_or(1.0),
                    water_efficiency: attr("water_movement_efficiency").unwrap_or(0.0),
                    water_slowdown: def.water_slowdown(),
                    air_speed: def.air_speed(),
                    baby_scale: def.baby_scale(),
                    head_rot_speed: def.head_rot_speed(),
                    max_head_x_rot: def.max_head_x_rot(),
                    max_head_y_rot: def.max_head_y_rot(),
                    ambient_interval: def.ambient_interval(),
                    flags,
                    model: Arc::new(Part::parse("root", model)),
                    layers,
                    texture: def.texture.to_owned(),
                    brain: def.brain(),
                    anim: def.anim,
                    sounds: def.sounds,
                    step_sound: def.step_sound,
                    food: def
                        .food_tag()
                        .map(tag)
                        .unwrap_or_else(|| Arc::from(Vec::new())),
                    tempt: def
                        .tempt_tag()
                        .or(def.food_tag())
                        .map(tag)
                        .unwrap_or_else(|| Arc::from(Vec::new())),
                    category,
                    id,
                },
            );
        }
        Ok(Self { types: out })
    }
}
pub use horizon_client_core::who::Who;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DamageKind {
    PlayerAttack,
    /// `mace_smash`: a mace hit after a fall (the player attack tags too).
    MaceSmash,
    /// `spear`: a stab of a spear (the player attack tags too).
    Spear,
    MobAttack,
    Arrow,
    Thrown,
    Fireball,
    Explosion,
    Fall,
    FallingBlock,
    FallingStalactite,
    OnFire,
    InFire,
    Lava,
    Drown,
    /// `dry_out`: an axolotl out of water for too long.
    DryOut,
    Magic,
    Sting,
    Cactus,
    /// `sweet_berry_bush`.
    SweetBerryBush,
    Generic,
    InWall,
    Thorns,
    SonicBoom,
    /// `freeze`: a fully frozen mob in powder snow.
    Freeze,
    /// `fell_out_of_world`: below the world's floor.
    OutOfWorld,
    /// `cramming`: crushed by the mobs around it.
    Cramming,
    /// `falling_anvil`: an anvil landing on it (it damages a helmet).
    FallingAnvil,
    /// `fireworks`: a firework rocket's burst (an explosion for Blast Protection).
    Fireworks,
    /// `generic_kill`: a kill that bypasses invulnerability.
    GenericKill,
    /// `lightning_bolt`.
    LightningBolt,
    /// `outside_border`: past the world border.
    OutsideBorder,
    /// `spit`: a llama's spit.
    Spit,
    /// `stalagmite`: a fall onto a stalagmite (a fall for Feather Falling).
    Stalagmite,
    /// `starve`: no food left.
    Starve,
    /// `sulfur_cube_hot`: a sulfur cube's hot floor (a fire).
    SulfurCubeHot,
    /// `wind_charge`: a wind charge's burst (a projectile).
    WindCharge,
    /// `wither`: the wither effect.
    Wither,
}
impl DamageKind {
    /// DamageTypeTags.BYPASSES_INVULNERABILITY
    pub fn bypasses_invulnerability(self) -> bool {
        matches!(self, DamageKind::OutOfWorld | DamageKind::GenericKill)
    }
}
/// A damage source: its type, the attacking entity and where it came from.
#[derive(Clone, Copy, Debug)]
pub struct Damage {
    pub kind: DamageKind,
    pub attacker: Option<Who>,
    pub from: Option<[f64; 3]>,
}
impl Damage {
    pub fn fall() -> Self {
        Self {
            kind: DamageKind::Fall,
            attacker: None,
            from: None,
        }
    }
    pub fn of(kind: DamageKind) -> Self {
        Self {
            kind,
            attacker: None,
            from: None,
        }
    }
    pub fn player(from: [f64; 3]) -> Self {
        Self {
            kind: DamageKind::PlayerAttack,
            attacker: Some(Who::Player),
            from: Some(from),
        }
    }
    /// `DamageSources.mace`: a player's smash attack.
    pub fn mace(from: [f64; 3]) -> Self {
        Self {
            kind: DamageKind::MaceSmash,
            attacker: Some(Who::Player),
            from: Some(from),
        }
    }
    /// `DamageSources.spear`: a player's stab with a spear.
    pub fn spear(from: [f64; 3]) -> Self {
        Self {
            kind: DamageKind::Spear,
            attacker: Some(Who::Player),
            from: Some(from),
        }
    }
    pub fn mob(id: u32, from: [f64; 3]) -> Self {
        Self {
            kind: DamageKind::MobAttack,
            attacker: Some(Who::Mob(id)),
            from: Some(from),
        }
    }
    /// DamageTypeTags.PANIC_CAUSES
    pub fn causes_panic(&self) -> bool {
        !matches!(
            self.kind,
            DamageKind::Fall
                | DamageKind::Drown
                | DamageKind::DryOut
                | DamageKind::Generic
                | DamageKind::InWall
                | DamageKind::SweetBerryBush
                | DamageKind::OutOfWorld
                | DamageKind::FallingBlock
                | DamageKind::FallingAnvil
                | DamageKind::Cramming
                | DamageKind::GenericKill
                | DamageKind::OutsideBorder
                | DamageKind::Spit
                | DamageKind::Stalagmite
                | DamageKind::Starve
        )
    }
    /// DamageTypeTags.BYPASSES_ARMOR
    pub fn bypasses_armor(&self) -> bool {
        matches!(
            self.kind,
            DamageKind::Fall
                | DamageKind::OnFire
                | DamageKind::Drown
                | DamageKind::Magic
                | DamageKind::InWall
                | DamageKind::Generic
                | DamageKind::SonicBoom
                | DamageKind::Freeze
                | DamageKind::OutOfWorld
                | DamageKind::Cramming
                | DamageKind::GenericKill
                | DamageKind::OutsideBorder
                | DamageKind::Stalagmite
                | DamageKind::Starve
                | DamageKind::Wither
        )
    }
    pub fn is_fire(&self) -> bool {
        matches!(
            self.kind,
            DamageKind::OnFire
                | DamageKind::InFire
                | DamageKind::Lava
                | DamageKind::Fireball
                | DamageKind::SulfurCubeHot
        )
    }
    /// DamageTypeTags.NO_KNOCKBACK
    pub fn no_knockback(&self) -> bool {
        matches!(
            self.kind,
            DamageKind::Explosion
                | DamageKind::Fall
                | DamageKind::OnFire
                | DamageKind::InFire
                | DamageKind::Lava
                | DamageKind::Drown
                | DamageKind::DryOut
                | DamageKind::Magic
                | DamageKind::Cactus
                | DamageKind::SweetBerryBush
                | DamageKind::Generic
                | DamageKind::InWall
                | DamageKind::Freeze
                | DamageKind::OutOfWorld
                | DamageKind::LightningBolt
                | DamageKind::Cramming
                | DamageKind::GenericKill
                | DamageKind::OutsideBorder
                | DamageKind::Stalagmite
                | DamageKind::Starve
                | DamageKind::SulfurCubeHot
                | DamageKind::Wither
        )
    }
    pub fn is_projectile(&self) -> bool {
        matches!(
            self.kind,
            DamageKind::Arrow | DamageKind::Thrown | DamageKind::Fireball | DamageKind::WindCharge
        )
    }
}
/// Per-mob attribute base values (the type's defaults unless changed).
#[derive(Clone, Copy, Debug)]
pub struct Attrs {
    pub speed: f64,
    pub attack_damage: f32,
    pub max_health: f32,
    pub follow_range: f64,
    pub knockback_resistance: f64,
    pub spawn_reinforcements: f64,
    pub jump_strength: f64,
}
/// `minecraft:wolf_armor`'s maximum damage.
pub const WOLF_ARMOR_DURABILITY: i32 = 64;
/// `NeutralMob.readPersistentAngerSaveData`: the anger runs until its end
/// time, so a mob that is loaded again is angry for what is left of it.
pub fn anger_left(anger_end: i64, game_time: i64) -> i32 {
    if anger_end <= 0 {
        return 0;
    }
    (anger_end - game_time).clamp(0, i64::from(i32::MAX)) as i32
}

/// Species-specific persistent state (variants, colours, taming, ageing...).
#[derive(Clone, Debug, Default)]
pub struct Extra {
    /// The local player's `minecraft:scale`, which scales the whole model
    /// (`LivingEntityRenderState.scale`); 0 is the default, 1.
    pub model_scale: f32,
    /// Players: crouching and riding (HumanoidModel's poses).
    pub crouching: bool,
    /// A player's or mannequin's synced pose (`Entity.DATA_POSE`), which
    /// sets its box (`Avatar.POSES`).
    pub avatar_pose: crate::pose::Pose,
    /// `Entity.isSprinting` (`Entity.DATA_SHARED_FLAGS_ID`'s flag 3): the dust a
    /// player raises; a cat or ocelot dashing (`Cat/Ocelot.customServerAiStep`),
    /// which `LivingEntity.setSprinting`'s speed modifier makes faster.
    pub sprinting: bool,
    pub passenger: bool,
    /// AgeableMob.age: negative while a baby (a tadpole counts up to 24000).
    pub age: i32,
    /// `AgeLocked` (tadpoles fed a slime ball... and ageable mobs).
    pub age_locked: bool,
    /// AgeableMob.forcedAge: the age fed food added, restored as the age
    /// when it reaches 0 (`ForcedAge`).
    pub forced_age: i32,
    /// AgeableMob.forcedAgeTimer: ticks of happy villager particles left.
    pub forced_age_timer: i32,
    pub in_love: i32,
    pub love_player: bool,
    /// Texture/model variant index (cat type, horse colour, rabbit type...).
    pub variant: u8,
    /// Secondary variant (horse markings, tropical fish pattern colours, panda hidden gene).
    pub variant2: u8,
    pub variant3: u8,
    /// Dye colour id (sheep wool, collars).
    pub color: u8,
    pub sheared: bool,
    /// `Mob.canPickUpLoot` (also lets dispensers equip it).
    pub pick_up_loot: bool,
    pub saddled: bool,
    /// The durability a wolf's body armor (`extra.head`) has lost.
    pub body_damage: i32,
    /// The components (dye, name...) of that armor stack.
    pub body_extra: Option<Box<crate::item_data::StackData>>,
    /// `Cat.RELAX_STATE_ONE`: watching a sleeping owner before lying down.
    pub relax_one: bool,
    /// The saddle's components (name, enchantments...) when it is not a bare
    /// one; `saddled` stays the gameplay flag.
    pub saddle_stack: Option<crate::items::ItemStack>,
    pub chested: bool,
    /// The chest contents of a donkey or mule (15 slots).
    pub items: Vec<Option<crate::items::ItemStack>>,
    /// `Piglin.inventory` (`SimpleContainer(8)`): what it picked up and kept.
    pub inventory: Vec<crate::items::ItemStack>,
    pub tame: bool,
    pub sitting: bool,
    /// `Parrot.partyParrot` and `jukebox`: the playing jukebox it dances to.
    pub party_jukebox: Option<[i32; 3]>,
    /// The owner's UUID (tamed animals).
    pub owner: Option<[i32; 4]>,
    /// Slime/magma cube size (0 = not sized).
    pub size: i32,
    /// Sheep eat-grass / iron golem attack / ravager animation ticks.
    pub anim_ticks: i32,
    pub egg_time: i32,
    pub anger: i32,
    /// `NeutralMob.getPersistentAngerEndTime`: the absolute game time the
    /// anger ends at, -1 when the mob is calm. The save keeps it, so the
    /// anger goes on while the chunk is unloaded.
    pub anger_end: i64,
    /// `NeutralMob.getPersistentAngerTarget`: the entity the mob is angry
    /// at, which the save keeps with it.
    pub angry_at: Option<Who>,
    /// `IronGolem.isPlayerCreated` (`PlayerCreated`): built from blocks.
    pub player_created: bool,
    pub carried: Option<horizon_world::registry::BlockStateId>,
    pub charged: bool,
    /// Held item ids (main hand, off hand) and head item.
    pub main_hand: Option<Arc<str>>,
    pub off_hand: Option<Arc<str>>,
    pub head: Option<Arc<str>>,
    /// Worn armour below the head (humanoid mobs).
    pub chest: Option<Arc<str>>,
    pub legs: Option<Arc<str>>,
    pub feet: Option<Arc<str>>,
    /// The worn armour stacks (head to feet): the dyed colour and the
    /// trim of a piece are on the stack, not on its id.
    pub armor: [Option<crate::items::ItemStack>; 4],
    pub using_item: i32,
    pub using_off_hand: bool,
    pub blocking: bool,
    pub fall_flying: bool,
    pub fall_fly_ticks: i32,
    /// `Entity.DATA_POSE` of another player, as the server sent it.
    pub synced_pose: u8,
    /// `LivingEntity.isAutoSpinAttack` (Riptide).
    pub auto_spin_attack: bool,
    pub main_hand_left: bool,
    /// The skin layers a player hides (`PlayerModelPart` bits, set to
    /// `Avatar.DATA_PLAYER_MODE_CUSTOMISATION` inverted); 0 shows all.
    pub hidden_model_parts: u8,
    pub elytra: ElytraAnimation,
    pub swim: SwimAnimation,
    /// `ClientAvatarState` of a player: the cloak, bob and walked distance its cape swings with.
    pub avatar: crate::camera::AvatarBob,
    /// The arm the current swing moves (`currentSwing.hand().asArm`): `Some(true)`
    /// for the right arm.
    pub swing_right_arm: Option<bool>,
    /// The current swing is a STAB (`SwingAnimationType` 2): a spear's jab.
    pub swing_stab: bool,
    pub flag: bool,
    pub flag2: bool,
    /// Species scratch values (the wither's head targets and clocks).
    pub ints: [i32; 6],
    pub counter: i32,
    pub counter2: i32,
    pub home: Option<[i32; 3]>,
    /// A copper golem's `openedChestPos`.
    pub opened_chest: Option<[i32; 3]>,
    pub point: Option<[f64; 3]>,
    /// `NEAREST_REPELLENT`: the nearest repelling block a hoglin or piglin
    /// sensed, looked up every 20 ticks.
    pub repellent: Option<[i32; 3]>,
    pub vector: [f64; 3],
    /// Active status effects: (effect id, ticks left, amplifier).
    pub effects: Vec<(&'static str, i32, i32)>,
    /// The effects above given with `hideParticles`: no swirls for them.
    pub hidden_effects: Vec<&'static str>,
    /// `LivingEntity.DATA_EFFECT_PARTICLES` and `DATA_EFFECT_AMBIENCE_ID` as a
    /// server gave them: the swirls around it, and whether they are all faint.
    pub effect_particles: Vec<crate::particles::ParticleOptions>,
    pub effect_ambient: bool,
    /// An invisible illusioner's mirror images.
    pub illusions: Option<illusioner::Illusions>,
    /// A villager's claimed bed head and meeting bell.
    pub bed: Option<[i32; 3]>,
    pub meeting: Option<[i32; 3]>,
    /// Villager and wandering trader offers and experience.
    pub trade: Option<Box<crate::merchant::Trader>>,
    /// Who holds this mob's lead, a holder not yet found after loading, and
    /// the spin the rope's torque gives it.
    pub leash: Option<leash::Holder>,
    pub leash_pending: Option<leash::Pending>,
    /// The UUID of the player behind `Holder::Player` (host or guest).
    pub leash_player: Option<[i32; 4]>,
    pub leash_spin: f64,
    /// More species scratch values.
    pub scratch: [i32; 4],
    /// Block positions a species remembers (a sniffer's explored ground).
    pub positions: Vec<[i32; 3]>,
    /// A warden's brain, anger and vibrations.
    pub warden: Option<Box<warden::Warden>>,
    /// A raider's patrol and raid state.
    pub raider: Option<Box<raider::State>>,
    /// A villager's raid activity and gossip.
    pub villager: Option<Box<villager::Ext>>,
    /// The sniffer's and the ghastling's brain.
    pub animal_brain: Option<Box<animal_brain::Brain>>,
    pub axolotl: axolotl::Animation,
    /// The mob or boat this one rides, and the mob riding it (`getVehicle`,
    /// `getFirstPassenger`).
    pub mount: Option<u32>,
    pub rider: Option<u32>,
    /// `Entity.boardingCooldown`: ticks after leaving a boat before a boat
    /// can pick this mob up again.
    pub boarding_cooldown: i32,
    /// A chicken carrying a zombie (`IsChickenJockey`).
    pub jockey: bool,
    /// A skeleton horse waiting to spring its trap, and how long it has waited.
    pub trap: bool,
    pub trap_time: i32,
    /// Enchantments on the equipment above.
    pub gear: Option<Box<gear::Gear>>,
    /// `Mob.dropChances` by `EquipmentSlot` (`drops::SLOTS` order) when set
    /// explicitly; None uses the defaults of `drops::drop_chance`.
    pub drop_chances: Option<Box<[f32; 8]>>,
    /// `Mob.leftHanded`.
    pub left_handed: bool,
    /// The sound variant: index in the species' `*_sound_variant` registry
    /// (`finalize::sound_variants`); None is the default, `classic`.
    pub sound_variant: Option<u8>,
    /// `Zombie.canBreakDoors`.
    pub can_break_doors: bool,
    /// A goat's `IsScreamingGoat` and the horns it lost (left, right).
    pub screaming: bool,
    pub lost_horns: [bool; 2],
    /// A goat's lowered head and ram behaviours.
    pub goat: goat::State,
    /// Attribute bases and permanent modifiers set by `finalizeSpawn`.
    pub attributes: Option<Box<finalize::Attributes>>,
    /// Brain memories `finalizeSpawn` sets: (memory, value, ticks to expiry).
    pub memories: Vec<(&'static str, i64, i64)>,
    /// A camel's `LAST_POSE_CHANGE_TICK`.
    pub pose_change_tick: i64,
    /// `Entity.DATA_POSE` as a mob of a server has it (`sync::pose_id`).
    pub pose: u8,
    /// The school leader a fish follows, and a tropical fish that swims alone.
    pub leader: Option<u32>,
    pub lone: bool,
    /// A breeze's or creaking's brain (`monster_brain`).
    pub brain: Option<Box<monster_brain::Brain>>,
    /// A bee's nectar, anger, flower and hive state.
    pub bee: Option<Box<bee::Bee>>,
    /// An armadillo's state and scare brain.
    pub armadillo: armadillo::State,
    /// A frog's brain memories and tongue target.
    pub frog: frog::State,
    /// A panda's flags, counters and amounts.
    pub panda: Option<Box<panda::State>>,
}
#[derive(Clone, Debug)]
pub struct Mob {
    pub id: u32,
    pub kind: Arc<MobType>,
    pub position: Vec3,
    pub previous: Vec3,
    pub velocity: Vec3,
    /// Entity.yRot (movement facing).
    pub yaw: f32,
    pub previous_yaw: f32,
    pub body_yaw: f32,
    pub previous_body_yaw: f32,
    pub head_yaw: f32,
    pub previous_head_yaw: f32,
    /// Entity.xRot
    pub head_pitch: f32,
    pub previous_head_pitch: f32,
    pub on_ground: bool,
    pub horizontal_collision: bool,
    pub vertical_collision_below: bool,
    /// `Entity.needsSync`: knocked back, pushed or jumped since its server
    /// last sent it, so the next update goes out at once.
    pub needs_sync: bool,
    pub health: f32,
    pub hurt_time: i32,
    pub invulnerable: i32,
    pub death_time: i32,
    pub age: u32,
    pub limb_position: f32,
    pub limb_speed: f32,
    pub previous_limb_speed: f32,
    pub swell: i32,
    pub previous_swell: i32,
    pub ambient_timer: i32,
    pub fall_distance: f64,
    /// `Entity.stuckSpeedMultiplier`: the next move is scaled by it.
    pub stuck_multiplier: Vec3,
    /// `Entity.pistonDeltas` and `pistonDeltasGameTime`: how far a moving
    /// block has pushed this mob this tick (0.51 at most on any axis).
    pub piston: horizon_world::piston_movement::PistonMovement,
    pub persistent: bool,
    /// `CustomName` (plain text) and `CustomNameVisible`.
    pub custom_name: Option<String>,
    /// The styled `CustomName`, when it came from a component.
    pub custom_name_text: Option<crate::text::Text>,
    pub name_visible: bool,
    /// `Entity.DATA_SHARED_FLAGS_ID` bit 0x20: the model is hidden.
    pub invisible: bool,
    pub random: LegacyRandom,
    pub ai: Box<ai::Ai>,
    pub extra: Extra,
    pub fluids: living::Fluids,
    /// Entity.wasEyeInWater: the eyes were in water before this tick's fluid
    /// update (`isUnderWater` reads it).
    pub was_eye_in_water: bool,
    /// `Entity.firstTick`: no splash for a mob that appears in water.
    pub first_tick: bool,
    pub no_gravity: bool,
    /// `Entity.mainSupportingBlockPos`: the block it stood on after its last
    /// move.
    pub supporting: Option<[i32; 3]>,
    /// `NoAI`: no goals and no movement.
    pub no_ai: bool,
    /// `Silent`.
    pub silent: bool,
    /// `Invulnerable`: immune to damage (but not to `/kill`).
    pub invulnerable_flag: bool,
    pub climbing: bool,
    pub fire_ticks: i32,
    pub ticks_frozen: i32,
    pub glowing: bool,
    pub air: i32,
    pub last_hurt: f32,
    pub last_damage: Option<(Damage, i64)>,
    /// Set by `remove`: the mob disappears without dying (conversion, despawn, pickup).
    pub removed: bool,
    /// Client-side animation clocks (attack swing, eating, shaking...).
    pub anim: anim::AnimState,
    /// Attribute values that differ per mob (slime size, babies, effects).
    pub attrs: Attrs,
    /// The player riding this mob and its input.
    pub rider: Option<equine::Rider>,
    /// Events raised outside a tick (hurt and death sounds of damage taken
    /// from the game), handed over with the next tick's events or drained
    /// by the game.
    pub outbox: Vec<MobEvent>,
    /// The skin texture of a player mob (`PlayerInfo.getSkin`), which
    /// replaces the type's default when its download is ready.
    pub skin: Option<String>,
    /// `PlayerSkin.model` is `SLIM`: the player model with 3-pixel arms.
    pub slim: bool,
    /// `PlayerSkin.cape`: a player's cape texture, once every download of
    /// its skin is ready.
    pub cape: Option<String>,
    /// A mannequin's `DATA_PROFILE` (ResolvableProfile): the profile's
    /// `textures` property resolves to `skin` like a player's.
    pub profile: Option<horizon_protocol::entity_data::Profile>,
}
/// Events a mob tick asks the game to act on.
#[derive(Clone, Debug)]
pub enum MobEvent {
    /// Squid.handleEntityEvent(19), resetting the remote tentacle cycle.
    SquidCycle {
        mob: u32,
        at: [f64; 3],
    },
    /// Rabbit entity event 1, sent only when a ground jump actually happens.
    RabbitHop {
        mob: u32,
        at: [f64; 3],
    },
    /// `LivingEntity.baseTick`'s drowning damage: entity event 67, the
    /// clients' `makeDrownParticles`.
    Drown {
        mob: u32,
        at: [f64; 3],
    },
    /// `Level.broadcastEntityEvent` for a mob: guests run its
    /// `handleEntityEvent`.
    EntityEvent {
        mob: u32,
        event: i8,
        at: [f64; 3],
    },
    /// `ServerLevel.sendParticles`: drawn here and sent to the players within
    /// 32 blocks, which spread and speed the way the client does for a count
    /// (a count of 0 is one particle moving by speed times spread).
    LevelParticles {
        kind: &'static str,
        at: [f64; 3],
        spread: [f64; 3],
        speed: f64,
        count: u32,
    },
    Sound(&'static str, [f64; 3], f32, f32),
    /// `UseBonemeal`: the villager's bone meal grows the crop at `pos`
    /// (`BoneMealItem.growCrop`, then level event 1505).
    Bonemeal {
        pos: [i32; 3],
    },
    /// `RingBell`: the villager rings the meeting-point bell
    /// (`BellBlock.attemptToRing`: the bell shakes, the sound and the game
    /// event follow).
    RingBell {
        bell: [i32; 3],
    },
    /// `ChestBlockEntity.startOpen` / `stopOpen` by a copper golem: the
    /// chest's opener count goes up or down (interaction range 3).
    ChestOpen {
        chest: [i32; 3],
        open: bool,
    },
    /// `BeeEnterHiveGoal.start`: the bee goes into its hive (the host stores
    /// its tag with `BeehiveBlockEntity.addOccupant` and discards it).
    EnterHive {
        mob: u32,
        hive: [i32; 3],
    },
    /// `LandOnOwnersShoulderGoal`: a tame parrot touched its owner and sits
    /// on their shoulder (the host stores its tag and discards it).
    ParrotToShoulder {
        mob: u32,
    },
    /// `CopperGolem.turnToStatue`.
    BecomeStatue {
        mob: u32,
    },
    /// A copper golem took from or filled a chest slot.
    ChestSlot {
        chest: [i32; 3],
        slot: usize,
        stack: Option<crate::items::ItemStack>,
    },
    /// `InfestedMobEffect.onMobHurt`: the mob was hurt while infested.
    Infested {
        mob: u32,
    },
    /// `OfferFlowerGoal.stop`: the item goes to the target's antenna (saddle)
    /// slot if it is empty, as a guaranteed drop.
    Gift {
        target: u32,
        item: &'static str,
    },
    /// `AnimalMakeLove`: the partner remembers this mob as its breed target
    /// and walks to it and looks at it.
    BreedTarget {
        mob: u32,
        partner: u32,
    },
    /// `BehaviorUtils.lockGazeAndWalkToEachOther`'s other half: the partner
    /// looks at this mob and walks to it.
    LockGaze {
        mob: u32,
        partner: u32,
    },
    /// A sound event id that is not a static string (variant sounds).
    SoundId(String, [f64; 3], f32, f32),
    /// `Level.playLocalSound`: a sound the client plays for itself, never
    /// sent by a server (a breeze's whirl and idle sounds).
    LocalSound(&'static str, [f64; 3], f32, f32),
    /// A level event's sound (`levelEvent`/`globalLevelEvent`): the client
    /// plays it with its own random pitch spread (`(a - b) * spread + 1`),
    /// nothing is drawn from the mob.
    LevelEventSound {
        sound: &'static str,
        at: [f64; 3],
        volume: f32,
        spread: f32,
    },
    /// `Level.gameEvent` from a mob's own movement (`GameEvent.STEP`): the
    /// game's sculk layer tracks steps itself, the mob AI probe hears them
    /// through the wardens (`warden::listen`).
    GameEvent {
        event: &'static str,
        at: [f64; 3],
        source: u32,
        state: horizon_world::registry::BlockStateId,
    },
    AttackPlayer(f32, [f64; 3]),
    /// A melee hit whose effect comes only if the hit lands (husk hunger,
    /// cave spider poison, wither skeleton wither, bee and pufferfish
    /// poison): a shield-blocked hit gives nothing.
    AttackPlayerWithEffect {
        amount: f32,
        from: [f64; 3],
        effect: &'static str,
        ticks: i32,
        amplifier: i32,
    },
    /// Damage of a given type to the player (guardian lasers, thorns).
    DamagePlayer {
        amount: f32,
        from: [f64; 3],
        kind: &'static str,
    },
    /// `ElderGuardian`'s curse on players within 50 blocks.
    ElderCurse {
        at: [f64; 3],
    },
    /// `Entity.push`: add to a target's velocity.
    Push {
        target: Who,
        velocity: [f64; 3],
    },
    /// `SonicBoom`: 10 damage to the target, then a push along `direction`
    /// (the unit vector from the warden's chest to the target's eyes) only
    /// if the hit landed, less the target's knockback resistance.
    SonicBoom {
        target: Who,
        attacker: u32,
        from: [f64; 3],
        direction: [f64; 3],
    },
    /// `Entity.move(MoverType.SHULKER, delta)`: shift a target's position
    /// (an opening shulker lifts what stands on its shell).
    Shift {
        target: Who,
        delta: [f64; 3],
    },
    /// `EvokerWololoSpellGoal`: a blue sheep turns red.
    Wololo {
        mob: u32,
    },
    /// `EvokerFangs` rising at a point after `delay` ticks.
    Fangs {
        at: [f64; 3],
        yaw: f32,
        delay: i32,
        owner: u32,
    },
    Shoot([f64; 3], [f64; 3]),
    /// The mob the `Explode` right after this comes from
    /// (`ServerExplosion.source`), and whether it is a charged creeper: the
    /// exploding creeper is already out of the mob list when the game applies
    /// the blast, so its victims' loot cannot look it up, nor the death
    /// message its name (`name`, `getDisplayName`).
    ExplosionSource {
        mob: u32,
        charged: bool,
        name: crate::text::Text,
    },
    /// The centre, the radius and the kind of the mob blowing up (it may be
    /// gone from the list by the time the blast is handled).
    Explode([f64; 3], f32, Species),
    /// Melee or ranged damage to another mob.
    AttackMob {
        target: u32,
        amount: f32,
        source: Damage,
    },
    /// `LivingEntity.knockback(power, x, z)`: pushed away from the direction.
    Knockback {
        target: Who,
        power: f64,
        x: f64,
        z: f64,
    },
    /// Set another mob's attack target (HurtByTargetGoal alerts).
    SetTarget {
        mob: u32,
        target: Who,
    },
    /// `StartHuntingHoglin` for a nearby adult piglin: it takes the hoglin as
    /// its anger target (when it has none) and starts its `HUNTED_RECENTLY` wait.
    HuntHoglin {
        mob: u32,
        target: u32,
        ticks: i32,
    },
    Projectile(crate::projectiles::Projectile),
    /// Spawn a new mob (breeding, slime splitting, chicken eggs...).
    Spawn(Box<Mob>),
    /// Spawn a mob by type id at a position (conversions, splits).
    SpawnKind {
        kind: String,
        at: [f64; 3],
        setup: SpawnSetup,
    },
    DropItem {
        item: Arc<str>,
        count: u32,
        at: [f64; 3],
    },
    /// `Mob.dropFromGiftLootTable`: a gift loot table's items dropped at `at`.
    DropGift {
        table: &'static str,
        at: [f64; 3],
    },
    SetBlock {
        pos: [i32; 3],
        state: horizon_world::registry::BlockStateId,
    },
    /// `LivingEntity.startSleeping` / `stopSleeping`: the bed head at `pos`
    /// gets `occupied` (`setBlockAndUpdate`; the foot follows it).
    BedOccupied {
        pos: [i32; 3],
        occupied: bool,
    },
    /// `DoorBlock.setOpen` by a mob (`InteractWithDoor`): the door opens or
    /// closes, both halves, with its sound and game event.
    SetDoorOpen {
        pos: [i32; 3],
        open: bool,
    },
    /// `BreakDoorGoal.tick` at its end: the door is removed (`removeBlock`),
    /// then level events 1021 and 2001 with the state that is left there.
    BreakDoor {
        pos: [i32; 3],
    },
    /// `Level.destroyBlockProgress` for a mob's crack on a block (`-1`
    /// clears it).
    DestroyProgress {
        mob: u32,
        pos: [i32; 3],
        progress: i32,
    },
    /// `Level.levelEvent`: the game plays and sends the event.
    LevelEvent {
        id: i32,
        pos: [i32; 3],
        data: i32,
    },
    /// `InfestedBlock.infestedStateByHost` (`infested`) or `hostStateByInfested`
    /// for the block there, keeping its axis; `poof` is the silverfish that
    /// hid in it (`spawnAnim`).
    Infest {
        pos: [i32; 3],
        infested: bool,
        poof: Option<u32>,
    },
    /// Destroy a block, dropping nothing (sheep eating grass becomes dirt via SetBlock).
    BreakBlock {
        pos: [i32; 3],
        drop: bool,
    },
    Particles {
        kind: &'static str,
        at: [f64; 3],
        count: u32,
    },
    /// Client particles about a mob's box (`particles::around_self`) and the
    /// entity event `event` that shows them, sent for mob `mob`.
    AroundSelf {
        mob: u32,
        event: i8,
        kind: &'static str,
        feet: [f64; 3],
        width: f64,
        height: f64,
        burst: crate::particles::Burst,
    },
    /// One particle exactly at `at`, moving at `velocity`.
    ParticleAt {
        kind: &'static str,
        at: [f64; 3],
        velocity: [f64; 3],
    },
    Experience {
        at: [f64; 3],
        amount: u32,
    },
    /// Regeneration support after an axolotl-assisted kill, from the axolotl `mob`.
    AxolotlSupportPlayer {
        mob: u32,
    },
    /// A dolphin that was given a fish asks the game for the nearest
    /// shipwreck or ocean ruin (`DolphinSwimToTreasureGoal.start`): the game
    /// sets the dolphin's `extra.home` to it, or leaves it empty.
    DolphinTreasure {
        mob: u32,
    },
    /// Apply an effect to the player (witch potions, bee stings, cave spider poison).
    EffectPlayer {
        effect: &'static str,
        ticks: i32,
        amplifier: i32,
    },
    /// Set an entity on fire.
    Ignite {
        target: Who,
        seconds: i32,
    },
    /// Two animals bred: the game spawns the baby and resets the partner.
    Breed {
        parent: u32,
        partner: u32,
        /// A llama's baby, drawn from the parent's random as
        /// `Llama.getBreedOffspring` does.
        llama: Option<Box<super::mobs::llama::Offspring>>,
    },
    /// Replace a block by name (sheep turning grass to dirt), optionally with
    /// the destroy effect.
    SetBlockName {
        pos: [i32; 3],
        name: &'static str,
        effect: bool,
    },
    /// A cat hisses (phantoms flee).
    Hiss {
        mob: u32,
    },
    /// A horse's rider tamed it.
    Tamed {
        mob: u32,
    },
    /// A horse throws its rider off.
    Buck {
        mob: u32,
    },
    /// A piglin's admiring is over: barter or keep the item.
    FinishAdmiring {
        mob: u32,
    },
    /// Throw an entity upward (iron golem swings).
    Launch {
        target: Who,
        up: f64,
    },
    /// Damage the mob itself (snow golems melting).
    HurtSelf {
        mob: u32,
        kind: DamageKind,
        amount: f32,
    },
    /// Two villagers are close: swap gossip and share food and wares.
    VillagerMeet {
        mob: u32,
        other: u32,
    },
    /// Two villagers make a baby (`VillagerMakeLove.tryToGiveBirth`): the
    /// parent has eaten and, with a bed for the child, rests; the partner
    /// eats, and rests too if `child` holds the child's bed and type
    /// (`None`: the biome's).
    VillagerBirth {
        parent: u32,
        partner: u32,
        child: Option<([i32; 3], Option<u8>)>,
    },
    /// `WorkAtComposter.useWorkstation`: a farmer at its composter makes
    /// bread and composts seeds (the game applies it to the villager's
    /// inventory and the block).
    WorkAtComposter {
        mob: u32,
        pos: [i32; 3],
    },
    /// A villager asks whether enough neighbours want an iron golem.
    GolemCheck {
        mob: u32,
        needed: usize,
    },
    /// A villager launches a firework rocket (celebrating a won raid).
    Firework {
        at: [f64; 3],
        color: usize,
        flight: u8,
    },
    /// A villager throws a hero's gift (a loot table) toward the player.
    ThrowGift {
        table: String,
        from: [f64; 3],
        to: [f64; 3],
    },
    /// A patrol leader hands its patrol target to a companion.
    PatrolTarget {
        mob: u32,
        target: [i32; 3],
    },
    /// A raider is told to attack (`HoldGroundAttackGoal` shouting).
    RaiderTarget {
        mob: u32,
        target: Who,
        aggressive: bool,
    },
    /// Drop the loot of a gift table (chicken eggs, sniffer seeds) with the
    /// mob as `this` (`LivingEntity.dropFromGiftLootTable`).
    GiftLoot {
        mob: u32,
        table: &'static str,
        at: [f64; 3],
    },
    /// `Armadillo.customServerAiStep`: the armadillo sheds a scute.
    ShedScute {
        mob: u32,
        at: [f64; 3],
        pitch: f32,
    },
    /// A lead snaps or loses its holder: the lead item drops when asked.
    LeashBreak {
        mob: u32,
        drop: bool,
        sound: bool,
    },
    /// A zombie villager's cure is complete: the game turns it back into a villager.
    CureZombieVillager {
        mob: u32,
    },
    /// A vibration travelling to a listener (`VibrationParticleOption`).
    Vibration {
        from: [f64; 3],
        to: [f64; 3],
        ticks: i32,
    },
    /// The crumbs of the block under a digging or emerging warden.
    DigParticles {
        at: [f64; 3],
    },
    /// `Zombie.hurtServer`: a zombie hurt with a target in sight (its own,
    /// else the attacker) may call a reinforcement against it on Hard.
    Reinforcements {
        mob: u32,
        target: Who,
    },
    /// `LevelEvent.PARTICLES_DESTROY_BLOCK` (2001) at `pos` for the block at
    /// `block`: a turtle's digging.
    BlockBreakEffect {
        pos: [i32; 3],
        block: [i32; 3],
    },
    /// `Warden.applyDarknessAround`: darkness for survival players nearby.
    Darkness {
        at: [f64; 3],
        radius: f64,
    },
    /// A creaking bound to the heart at `home` was hurt (`creakingHurt` when a player did it).
    CreakingHurt {
        home: [i32; 3],
        by_player: bool,
        id: u32,
    },
    /// `Creaking.tearDown`: crumbling particles around the creaking.
    CreakingTearDown {
        center: [f64; 3],
        spread: [f64; 3],
    },
    /// `Panda.afterSneeze`: the grown pandas within 10 blocks start.
    PandaSneezed {
        mob: u32,
    },
}
/// How a spawned-by-kind mob should be set up.
#[derive(Clone, Debug, Default)]
pub struct SpawnSetup {
    pub baby: bool,
    pub size: i32,
    pub yaw: f32,
    pub velocity: [f64; 3],
    pub health: Option<f32>,
    pub persistent: bool,
    pub variant: Option<u8>,
    /// Zombified piglins and zoglins wake up nauseous (`ConversionParams`).
    pub nausea: bool,
    /// Vexes: the summoning mob, the bound origin and the life in ticks.
    pub owner_mob: Option<u32>,
    pub home: Option<[i32; 3]>,
    pub life: Option<i32>,
}
/// A snapshot of another entity mobs can see this tick.
#[derive(Clone, Debug)]
pub struct Seen {
    pub in_water: bool,
    pub playing_dead: bool,
    pub last_damage: Option<Damage>,
    pub id: u32,
    pub species: Species,
    pub position: Vec3,
    pub bounds: Aabb,
    pub eye_y: f64,
    pub alive: bool,
    pub baby: bool,
    pub in_love: bool,
    pub tame: bool,
    /// A tamed animal's owner (`getOwner`).
    pub owner: Option<[i32; 4]>,
    pub hostile: bool,
    pub target: Option<Who>,
    pub health: f32,
    pub variant: u8,
    /// A slime's or magma cube's size.
    pub size: i32,
    /// Dye colour (sheep).
    pub color: u8,
    /// A villager's claimed job site and bed.
    pub job: Option<[i32; 3]>,
    pub bed: Option<[i32; 3]>,
    pub yaw: f32,
    pub velocity: Vec3,
    /// Who holds this mob's lead.
    pub leash: Option<leash::Holder>,
    /// A llama's caravan head (its id plus one, 0 when not in a caravan).
    pub caravan: u32,
    /// A wandering trader's despawn delay.
    pub delay: i32,
    /// `getLastHurtByMob` and its timestamp.
    pub hurt_by: Option<(Who, i32)>,
    /// A raider in an active raid.
    pub active_raid: bool,
    /// A copper golem's `openedChestPos` (`ContainerUser.hasContainerOpen`).
    pub opened_chest: Option<[i32; 3]>,
    /// `isPassenger`.
    pub passenger: bool,
    /// A villager's reputation of the player (`getPlayerReputation`).
    pub player_rep: i32,
    /// `isPushable` and pushed by others (`Entity.push(Entity)` moves it).
    pub pushable: bool,
    /// A cold strider (`Strider.isSuffocating`).
    pub cold_strider: bool,
    /// `MobCategory` (`monster`, `creature`, ...).
    pub category: &'static str,
    /// A villager's `POTENTIAL_JOB_SITE` and `MEETING_POINT`.
    pub potential_job: Option<[i32; 3]>,
    pub meeting: Option<[i32; 3]>,
    /// `variant2`: a villager's profession (index into
    /// `render::PROFESSIONS`), a llama's strength. And a villager's experience.
    pub profession: u8,
    pub xp: i32,
    /// What other villagers read of a villager's state and brain.
    pub villager: Option<Arc<villager::Seen>>,
    /// `MAX_HEALTH`, `JUMP_STRENGTH` and `MOVEMENT_SPEED` base values (a
    /// horse's breeding stock).
    pub stats: [f64; 3],
    /// `Wolf.isInSittingPose`.
    pub sitting: bool,
    /// `Entity.isSilent`.
    pub silent: bool,
    /// `Cat.isLying` or `isRelaxStateOne`.
    pub lying: bool,
    /// `Entity.isVehicle` or `isPassenger`.
    pub ridden: bool,
    /// `Attributes.KNOCKBACK_RESISTANCE`.
    pub knockback_resistance: f64,
    /// The active effects, without namespace (`slowness`, `poison`...).
    pub effects: Vec<&'static str>,
    /// A piglin's `HUNTED_RECENTLY` memory.
    pub hunted_recently: bool,
}
impl Seen {
    /// `LivingEntity.hasEffect`, by id without namespace.
    pub fn has_effect(&self, effect: &str) -> bool {
        self.effects.contains(&effect)
    }
}
impl Mob {
    pub fn burning_time(&self) -> f64 {
        let base = finalize::value(self, finalize::Attr::BurningTime);
        let protection: u32 = gear::ARMOR
            .into_iter()
            .map(|slot| gear::level(self, slot, "minecraft:fire_protection"))
            .sum();
        (base - 0.15 * f64::from(protection)).max(0.0)
    }
    /// Entity.isOnFire: fire-immune mobs never count as burning, even on the
    /// tick a sun or fire check set their timer before `base_tick` clears it.
    pub fn is_on_fire(&self) -> bool {
        !self.kind.flags.fire_immune && self.fire_ticks > 0
    }
    pub fn ignite_for_ticks(&mut self, ticks: i32) {
        self.fire_ticks = self
            .fire_ticks
            .max((f64::from(ticks) * self.burning_time()).ceil() as i32);
        self.ticks_frozen = 0;
    }
    /// `Entity.isFullyFrozen`.
    pub fn is_fully_frozen(&self) -> bool {
        self.ticks_frozen >= crate::motion::PlayerMotion::TICKS_REQUIRED_TO_FREEZE
    }

    /// What other mobs see of this one this tick (`player`: the player's
    /// UUID, which villagers' reputation is keyed by).
    pub fn seen(&self, player: Option<[i32; 4]>) -> Seen {
        let m = self;
        let villager = m.species() == Species::Villager;
        Seen {
            in_water: self.fluids.in_water,
            playing_dead: axolotl::is_playing_dead(self),
            last_damage: self.last_damage.map(|(d, _)| d),
            id: m.id,
            species: m.species(),
            position: m.position,
            bounds: m.bounds(),
            eye_y: m.eye_y(),
            alive: m.alive(),
            baby: m.is_baby(),
            // A villager is "in love" when it is willing to breed (`canBreed`).
            in_love: if villager {
                villager::can_breed(m)
            } else {
                m.extra.in_love > 0
            },
            tame: m.extra.tame,
            owner: m.extra.owner,
            hostile: m.hostile(),
            target: m.ai.target,
            health: m.health,
            variant: m.extra.variant,
            size: m.extra.size,
            color: m.extra.color,
            job: villager.then_some(m.extra.home).flatten(),
            bed: villager.then_some(m.extra.bed).flatten(),
            yaw: m.yaw,
            velocity: m.velocity,
            leash: m.extra.leash,
            caravan: caravan::head_of(m),
            delay: m.extra.ints[5],
            hurt_by: m.ai.last_hurt_by.map(|w| (w, m.ai.last_hurt_by_time)),
            active_raid: raider::has_active_raid(m),
            opened_chest: m.extra.opened_chest,
            passenger: m.extra.mount.is_some(),
            player_rep: match (villager, player) {
                (true, Some(uuid)) => villager::ext(m).map_or(0, |e| e.gossips.reputation(uuid)),
                _ => 0,
            },
            pushable: m.pushable(),
            cold_strider: strider::suffocating(m),
            category: m.category(),
            potential_job: villager_brain::potential_job_site(m),
            meeting: villager.then_some(m.extra.meeting).flatten(),
            profession: m.extra.variant2,
            xp: villager::xp(m),
            villager: villager.then(|| Arc::new(villager::seen(m))),
            stats: [
                f64::from(m.attrs.max_health),
                m.extra.vector[0],
                m.attrs.speed,
            ],
            sitting: m.extra.sitting,
            silent: m.silent,
            lying: m.species() == Species::Cat && (m.extra.flag || m.extra.relax_one),
            ridden: m.rider.is_some() || m.extra.rider.is_some() || m.extra.mount.is_some(),
            knockback_resistance: m.attrs.knockback_resistance,
            effects: m
                .extra
                .effects
                .iter()
                .map(|e| e.0.trim_start_matches("minecraft:"))
                .collect(),
            hunted_recently: m.species() == Species::Piglin && m.extra.ints[0] > 0,
        }
    }
    /// `EntityType.getCategory`'s name.
    pub fn category(&self) -> &'static str {
        match self.kind.category.as_str() {
            "monster" => "monster",
            "creature" => "creature",
            "ambient" => "ambient",
            "axolotls" => "axolotls",
            "underground_water_creature" => "underground_water_creature",
            "water_creature" => "water_creature",
            "water_ambient" => "water_ambient",
            _ => "misc",
        }
    }
}
/// `Entity.push` on other mobs acts at once in vanilla: a mob ticking later
/// moves with it this tick. Takes the `Push` events aimed at mobs out of
/// `events[from..]` into `pending`, which the caller adds to each mob's
/// velocity before its tick (and to every mob once all have ticked). The
/// other events keep their order.
pub fn take_mob_pushes(events: &mut Vec<MobEvent>, from: usize, pending: &mut PendingPushes) {
    // One pass: a crowd's tick holds a push per neighbour, and removing each
    // in place would shift the rest every time.
    let mut kept = from;
    for i in from..events.len() {
        if let MobEvent::Push {
            target: Who::Mob(id),
            velocity,
        } = events[i]
        {
            pending.push(id, Vec3::new(velocity[0], velocity[1], velocity[2]));
        } else {
            events.swap(kept, i);
            kept += 1;
        }
    }
    events.truncate(kept);
}
/// Pushes waiting for their mob's tick, in push order. A list while short;
/// past [`PendingPushes::LIST_MAX`] (a crowd pushing each other) one list
/// per mob, as scanning the whole list for each mob grows with its square.
#[derive(Default)]
pub struct PendingPushes {
    list: Vec<(u32, Vec3)>,
    by_mob: crate::fast_hash::FxHashMap<u32, Vec<Vec3>>,
}
impl PendingPushes {
    const LIST_MAX: usize = 64;
    pub fn push(&mut self, id: u32, v: Vec3) {
        if self.by_mob.is_empty() && self.list.len() < Self::LIST_MAX {
            self.list.push((id, v));
            return;
        }
        // Older pushes first, so each mob's stay in order.
        for (id, v) in self.list.drain(..) {
            self.by_mob.entry(id).or_default().push(v);
        }
        self.by_mob.entry(id).or_default().push(v);
    }
}
/// Adds the pending pushes aimed at `m` to its velocity, in push order.
pub fn apply_mob_pushes(m: &mut Mob, pending: &mut PendingPushes) {
    // Entity.push(x, y, z) sets needsSync.
    let mut add = |v: Vec3| {
        m.velocity = Vec3::new(m.velocity.x + v.x, m.velocity.y + v.y, m.velocity.z + v.z);
        m.needs_sync = true;
    };
    if !pending.list.is_empty() {
        pending.list.retain(|(id, v)| {
            if *id != m.id {
                return true;
            }
            add(*v);
            false
        });
    }
    if !pending.by_mob.is_empty()
        && let Some(pushes) = pending.by_mob.remove(&m.id)
    {
        pushes.into_iter().for_each(add);
    }
}
/// Applies the brain memories a mob's tick set on other mobs (from event
/// index `from` on), at once: vanilla sets them on the other entity during
/// the tick, before the mobs after it tick.
pub fn apply_brain_events(mobs: &mut [Mob], events: &mut Vec<MobEvent>, from: usize) {
    let mut i = from;
    while i < events.len() {
        match events[i] {
            MobEvent::BreedTarget { mob, partner } => {
                if let Some(m) = mobs.iter_mut().find(|m| m.id == mob) {
                    animal_brain::set_breed_partner(m, partner);
                }
                events.remove(i);
            }
            MobEvent::LockGaze { mob, partner } => {
                if let Some(m) = mobs.iter_mut().find(|m| m.id == mob) {
                    animal_brain::lock_gaze(m, partner);
                    villager_brain::lock_gaze_half(m, Who::Mob(partner));
                }
                events.remove(i);
            }
            // Animal.finalizeSpawnChildFromBreeding: the partner rests and
            // forgets it at once (the game still makes the baby).
            MobEvent::Breed {
                parent, partner, ..
            } => {
                if let Some(m) = mobs.iter_mut().find(|m| m.id == partner) {
                    m.extra.age = 6000;
                    m.extra.in_love = 0;
                    animal_brain::forget_breed_target(m, parent);
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
}
/// The player as mobs see it.
#[derive(Clone, Debug)]
pub struct PlayerView {
    pub position: Vec3,
    pub bounds: Aabb,
    pub eye: [f64; 3],
    pub alive: bool,
    pub creative: bool,
    pub sneaking: bool,
    /// `Entity.isSwimming`.
    pub swimming: bool,
    pub look: [f64; 3],
    pub main_hand: Option<Arc<str>>,
    pub off_hand: Option<Arc<str>>,
    pub head: Option<Arc<str>>,
    pub armor_cover: f32,
    /// The Invisibility effect (`isInvisible`).
    pub invisible: bool,
    pub spectator: bool,
    /// `PiglinAi.isWearingSafeArmor`: some golden armour.
    pub gold_armor: bool,
    /// `getLastHurtByMob` (within 100 ticks) and when.
    pub hurt_by: Option<(Who, i64)>,
    /// `getLastHurtMob` and when.
    pub last_hurt: Option<(Who, i64)>,
    pub velocity: Vec3,
    pub yaw: f32,
    /// `Player.isSprinting` and `isPassenger` (what scares an armadillo).
    pub sprinting: bool,
    pub riding: bool,
    /// The Hero of the Village effect.
    pub hero: bool,
    /// The active effects, without namespace (`slowness`, `poison`...).
    pub effects: Vec<&'static str>,
    /// The player's UUID (villagers' gossip is keyed by it).
    pub uuid: [i32; 4],
    /// The mobs on the player's team (`isAlliedTo`): combat targeting
    /// leaves the player alone.
    pub allies: std::sync::Arc<std::collections::HashSet<u32>>,
    /// A LAN guest: its own client moves it, and touching mobs push it there
    /// (`EntitySelector.pushableBy` on the client).
    pub remote: bool,
    /// `isSleeping` and `getSleepTimer` (100 once slept through, then the
    /// ticks fading out after waking up).
    pub sleeping: bool,
    pub sleep_timer: u32,
    /// `isPassenger`.
    pub passenger: bool,
}
impl PlayerView {
    /// `LivingEntity.getVisibilityPercent` without the worn-skull scaling.
    pub fn visibility_percent(&self) -> f64 {
        let mut visibility = 1.0;
        if self.sneaking {
            visibility *= 0.8;
        }
        if self.invisible {
            visibility *= 0.7 * f64::from(self.armor_cover).max(0.1);
        }
        visibility
    }
    /// `LivingEntity.hasEffect`, by id without namespace.
    pub fn has_effect(&self, effect: &str) -> bool {
        self.effects.contains(&effect)
    }
}
#[derive(Clone)]
pub struct MobWorld<'a> {
    pub world: &'a World,
    pub registry: &'a horizon_world::registry::BlockRegistry,
    pub facts: &'a BlockFacts,
    /// Fluid states and flow facts, for the currents that push mobs.
    pub fluid_tables: &'a crate::motion::FluidResources,
    pub movement: &'a horizon_world::movement_properties::BoundMovementProperties,
    pub collision: &'a BlockCollisionRegistry<'a>,
    pub data: &'a MobData,
    pub player: Option<PlayerView>,
    /// `getNearestPlayer(mob, -1)`: where the nearest non-spectator of every
    /// player in the level stands, alive or not (the despawn distance).
    pub nearest_player: Option<Vec3>,
    pub others: &'a [Seen],
    /// `others` by position, when the caller keeps one (the mob ticks).
    pub others_grid: Option<&'a seen_grid::SeenGrid>,
    /// Where each id stands in `others`, when the caller keeps one.
    pub others_index: Option<&'a crate::fast_hash::FxHashMap<u32, u32>>,
    pub daylight: bool,
    /// The `gameplay/monsters_burn` attribute: the overworld day timeline's
    /// step track, false in every other dimension.
    pub monsters_burn: bool,
    pub game_time: i64,
    pub day_time: i64,
    pub raining: bool,
    pub thundering: bool,
    pub nether: bool,
    pub block_tags: &'a HashMap<String, std::collections::HashSet<String>>,
    pub biomes: &'a crate::biome::Biomes,
    /// The `mob_griefing` game rule.
    pub mob_griefing: bool,
    /// The `mob_drops` game rule (`shouldDropLoot`).
    pub mob_drops: bool,
    /// The `max_entity_cramming` game rule.
    pub max_entity_cramming: i32,
    /// `getCurrentDifficultyAt` at the mob being ticked (near the player
    /// outside the mob tick).
    pub difficulty: spawning::LocalDifficulty,
    /// The End (its dimension type's monster spawn light rules).
    pub end: bool,
    /// The world seed (slime chunks).
    pub world_seed: i64,
    /// `Level.getSeaLevel` (63 in the overworld, -63 in a flat world).
    pub sea_level: i32,
    /// The scoreboard teams of the player and the mobs (collision rules),
    /// shared: the mob ticks build them once a tick, not once a mob.
    pub teams: std::sync::Arc<Teams<'a>>,
    /// Item data (max stack sizes), when the host has it.
    pub items: Option<&'a crate::items::Items>,
    /// `ChestBlockEntity.getOpenCount`: the players with each chest open.
    pub lid_openers: &'a HashMap<(i32, i32, i32), i32>,
}
/// `Entity.getTeam` of the player and of the mobs on a team.
#[derive(Clone, Debug, Default)]
pub struct Teams<'a> {
    pub player: Option<&'a crate::scoreboard::Team>,
    pub mobs: HashMap<u32, &'a crate::scoreboard::Team>,
}
impl<'a> Teams<'a> {
    pub fn of(&self, who: Who) -> Option<&'a crate::scoreboard::Team> {
        match who {
            Who::Player => self.player,
            Who::Mob(id) => self.mobs.get(&id).copied(),
        }
    }
}
impl Mob {
    pub fn new(id: u32, kind: Arc<MobType>, position: Vec3, yaw: f32) -> Self {
        let mut random = LegacyRandom::new(
            i64::from(id).wrapping_mul(0x5DEE_CE66D)
                ^ (position.x.to_bits() as i64)
                ^ (position.z.to_bits() as i64).rotate_left(17),
        );
        let _ = random.next_int();
        let mut mob = Self {
            id,
            health: kind.max_health,
            position,
            previous: position,
            velocity: Vec3::ZERO,
            yaw,
            previous_yaw: yaw,
            body_yaw: yaw,
            previous_body_yaw: yaw,
            head_yaw: yaw,
            previous_head_yaw: yaw,
            head_pitch: 0.0,
            previous_head_pitch: 0.0,
            on_ground: false,
            horizontal_collision: false,
            vertical_collision_below: false,
            needs_sync: false,
            hurt_time: 0,
            invulnerable: 0,
            death_time: 0,
            age: 0,
            limb_position: 0.0,
            limb_speed: 0.0,
            previous_limb_speed: 0.0,
            swell: 0,
            previous_swell: 0,
            // Mob.ambientSoundTime starts at 0.
            ambient_timer: 0,
            fall_distance: 0.0,
            stuck_multiplier: Vec3::ZERO,
            piston: Default::default(),
            persistent: false,
            random,
            ai: Box::default(),
            extra: Extra::default(),
            fluids: living::Fluids::default(),
            was_eye_in_water: false,
            first_tick: true,
            no_gravity: false,
            supporting: None,
            no_ai: false,
            silent: false,
            invulnerable_flag: false,
            climbing: false,
            fire_ticks: 0,
            ticks_frozen: 0,
            glowing: false,
            air: 300,
            last_hurt: 0.0,
            last_damage: None,
            removed: false,
            custom_name: None,
            custom_name_text: None,
            name_visible: false,
            invisible: false,
            anim: anim::AnimState::default(),
            rider: None,
            outbox: Vec::new(),
            skin: None,
            slim: false,
            cape: None,
            profile: None,
            attrs: Attrs {
                speed: kind.speed,
                attack_damage: kind.attack_damage,
                max_health: kind.max_health,
                follow_range: kind.follow_range,
                knockback_resistance: kind.knockback_resistance,
                spawn_reinforcements: kind
                    .attributes
                    .get("minecraft:spawn_reinforcements")
                    .copied()
                    .unwrap_or(0.0),
                jump_strength: kind.jump_strength,
            },
            kind,
        };
        // setCanPickUpLoot(true) in the Villager, AbstractPiglin, Dolphin,
        // Fox and Panda constructors (an allay's is false until it holds
        // something: it overrides canPickUpLoot).
        mob.extra.pick_up_loot = matches!(
            mob.species(),
            Species::Villager
                | Species::Piglin
                | Species::PiglinBrute
                | Species::Dolphin
                | Species::Fox
                | Species::Panda
        );
        // The PathNavigation constructor: the base follow range.
        mob.ai.nav.max_visited_nodes = (mob.kind.follow_range * 16.0).floor() as i32;
        ai::register(&mut mob);
        mob
    }
    /// `Mob.canPickUpLoot`, with the overrides: an allay that holds
    /// something off its pickup cooldown, a sulfur cube with no block.
    pub fn can_pick_up_loot(&self) -> bool {
        match self.species() {
            Species::Allay => self.extra.main_hand.is_some() && self.extra.ints[3] <= 0,
            Species::SulfurCube => !sulfur_cube::has_body(self),
            _ => self.extra.pick_up_loot,
        }
    }
    pub fn bounds(&self) -> Aabb {
        self.bounds_at(self.position)
    }
    /// `Entity.makeBoundingBox(position)`.
    pub fn bounds_at(&self, position: Vec3) -> Aabb {
        let w = f64::from(self.width()) / 2.0;
        Aabb::new(
            Vec3::new(position.x - w, position.y, position.z - w),
            Vec3::new(
                position.x + w,
                position.y + f64::from(self.height()),
                position.z + w,
            ),
        )
    }
    pub fn alive(&self) -> bool {
        self.health > 0.0
    }
    pub fn hostile(&self) -> bool {
        self.kind.category == "monster"
    }
    pub fn species(&self) -> Species {
        self.kind.species
    }
    /// `Mob.getMainArm`: the left arm when the mob is left-handed, a player's
    /// `mainHand` option for a player mob.
    pub fn main_arm_left(&self) -> bool {
        self.extra.main_hand_left || self.extra.left_handed
    }
    /// The player hit this mob (melee): LivingEntity.hurt with knockback.
    pub fn hurt(
        &mut self,
        amount: f32,
        from: Option<[f64; 3]>,
        _random: &mut LegacyRandom,
    ) -> bool {
        let source = match from {
            Some(f) => Damage::player(f),
            None => Damage::of(DamageKind::Generic),
        };
        self.damage(amount, source)
    }
    /// LivingEntity.hurtServer: invulnerability frames, armor, knockback and
    /// the last-hurt-by bookkeeping goals read.
    /// The `armor` and `armor_toughness` modifiers of worn humanoid armour.
    pub fn worn_armor(&self) -> (f32, f32) {
        // An equine keeps its horse armor in `extra.head`: the item's `armor`
        // and `armor_toughness` modifiers of the body slot.
        if equine::is_equine(self.species()) {
            return self
                .extra
                .head
                .as_deref()
                .map_or((0.0, 0.0), spawning::horse_armor_stats);
        }
        [
            &self.extra.head,
            &self.extra.chest,
            &self.extra.legs,
            &self.extra.feet,
        ]
        .into_iter()
        .flatten()
        .map(|i| spawning::armor_stats(i))
        .fold((0.0, 0.0), |(a, t), (x, y)| (a + x, t + y))
    }
    pub fn damage(&mut self, amount: f32, source: Damage) -> bool {
        self.damage_with(amount, source, None)
    }
    /// `damage` by a source with a weapon: `armor_effectiveness` runs the
    /// weapon's armor-effectiveness enchantments (breach) over the armor fraction.
    pub fn damage_with(
        &mut self,
        amount: f32,
        source: Damage,
        armor_effectiveness: Option<&mut dyn FnMut(f32) -> f32>,
    ) -> bool {
        axolotl::before_damage(self, amount, &source);
        raider::note_hurt(self);
        // LivingEntity.isInvulnerableTo: only #bypasses_invulnerability gets past it.
        if !self.alive()
            || self.removed
            || (self.invulnerable_flag && !source.kind.bypasses_invulnerability())
        {
            return false;
        }
        // Ghast.isReflectedFireball: a fireball a player turned back is
        // no fire to it, and kills it (Ghast.hurtServer: 1000 damage).
        let reflected = &*self.kind.id == "minecraft:ghast"
            && source.kind == DamageKind::Fireball
            && source.attacker == Some(Who::Player);
        if source.is_fire() && (self.kind.flags.fire_immune) && !reflected {
            return false;
        }
        if !ai::accepts_damage(self, &source, amount) {
            return false;
        }
        let mut amount = if reflected { 1000.0 } else { amount };
        if self.species() == Species::Armadillo {
            amount = armadillo::reduce_damage(self, amount);
        }
        // #damages_helmet: falling anvils, blocks and stalactites.
        if matches!(
            source.kind,
            DamageKind::FallingBlock | DamageKind::FallingAnvil | DamageKind::FallingStalactite
        ) && !species::body_slot(self.species())
            && self.extra.head.is_some()
        {
            amount *= 0.75;
        }
        // LivingEntity.hurtServer: `#freeze_hurts_extra_types` take five times.
        if source.kind == DamageKind::Freeze
            && matches!(
                self.species(),
                Species::Strider | Species::Blaze | Species::MagmaCube
            )
        {
            amount *= 5.0;
        }
        let (worn, worn_toughness) = self.worn_armor();
        // Shulker.setRawPeekAmount: a closed shell adds the covered armor modifier.
        let covered = if self.species() == Species::Shulker && self.extra.counter == 0 {
            20.0
        } else {
            0.0
        };
        if !source.bypasses_armor() && self.kind.armor + worn + covered > 0.0 {
            // CombatRules.getDamageAfterAbsorb
            let armor = self.kind.armor + worn + covered;
            let toughness = 2.0 + (self.kind.armor_toughness + worn_toughness) / 4.0;
            let effective = (armor - amount / toughness).clamp(armor * 0.2, 20.0);
            let mut fraction = effective / 25.0;
            if let Some(modify) = armor_effectiveness {
                fraction = modify(fraction).clamp(0.0, 1.0);
            }
            amount *= 1.0 - fraction;
        }
        // CombatRules.getDamageAfterMagicAbsorb: enchantment protection.
        let epf = gear::protection(self, source.kind);
        if epf > 0.0 {
            amount *= 1.0 - epf / 25.0;
        }
        // Witch.getDamageAfterMagicAbsorb: none from its own doing, else
        // 85% less from `#witch_resistant_to`.
        if self.species() == Species::Witch {
            if source.attacker == Some(Who::Mob(self.id)) {
                amount = 0.0;
            } else if matches!(
                source.kind,
                DamageKind::Magic | DamageKind::SonicBoom | DamageKind::Thorns
            ) {
                amount *= 0.15;
            }
        }
        // Wolf.actuallyHurt: wolf armor takes the damage instead of the wolf.
        let absorbed = self.wolf_armor_absorbs(&source);
        let mut knock = true;
        if self.invulnerable as f32 > 10.0 {
            if amount <= self.last_hurt {
                return false;
            }
            let dealt = amount - self.last_hurt;
            if absorbed {
                self.hurt_wolf_armor(dealt);
            } else {
                self.health -= dealt;
            }
            self.last_hurt = amount;
            knock = false;
        } else {
            self.last_hurt = amount;
            self.invulnerable = 20;
            if absorbed {
                self.hurt_wolf_armor(amount);
            } else {
                self.health -= amount;
            }
            self.hurt_time = 10;
        }
        // Animal.actuallyHurt: damage taken ends love mode (`resetLove`).
        if drops::is_animal(self.species()) {
            self.extra.in_love = 0;
        }
        self.last_damage = Some((source, i64::from(self.age)));
        if let Some(who) = source.attacker {
            self.ai.last_hurt_by = Some(who);
            self.ai.last_hurt_by_time = self.age as i32;
            if who == Who::Player {
                self.ai.last_hurt_by_player = 100;
            }
        }
        // dealDefaultKnockback, unless the damage type is tagged no_knockback;
        // a source without a position knocks in a random direction.
        if knock && !source.no_knockback() {
            let (dx, dz) = source.from.map_or((0.0, 0.0), |f| {
                (f[0] - self.position.x, f[2] - self.position.z)
            });
            self.knockback(f64::from(0.4f32), dx, dz);
        }
        ai::after_hurt(self, &source);
        if self.health <= 0.0 {
            self.health = 0.0;
        }
        if knock {
            // playHurtSound (Mob: also resets the ambient sound clock) or the
            // death sound: makeSound draws the voice pitch whenever there is a sound.
            let dead = !self.alive();
            // Wolf.getHurtSound: the armor's own sound when it takes the hit.
            let sound = if absorbed {
                "item.wolf_armor.damage"
            } else {
                ai::voice_sounds(self)[if dead { 2 } else { 1 }]
            };
            if !dead {
                self.ambient_timer = -self.ambient_interval();
            }
            if !sound.is_empty() {
                let pitch = self.voice_pitch();
                if !self.silent {
                    let at = [self.position.x, self.position.y, self.position.z];
                    self.outbox
                        .push(MobEvent::Sound(sound, at, ai::sound_volume(self), pitch));
                }
            }
        }
        if self.has_effect("minecraft:infested") {
            self.outbox.push(MobEvent::Infested { mob: self.id });
        }
        // Squid.hurtServer: a squid hurt after a living entity hit it squirts ink.
        if matches!(self.species(), Species::Squid | Species::GlowSquid)
            && self.ai.last_hurt_by.is_some()
        {
            aquatic::spawn_ink(self);
        }
        true
    }
    /// A splash or cloud's harming effect on this mob: the potion's
    /// `indirect_magic` is in `#witch_resistant_to`, so a witch takes 15%.
    pub fn potion_harm(&self, amount: f32) -> f32 {
        if self.species() == Species::Witch {
            amount * 0.15
        } else {
            amount
        }
    }
    /// `Wolf.canArmorAbsorb`: it wears wolf armor and the damage type is not
    /// in `#bypasses_wolf_armor` (drowning, magic, thorns, in a wall, out of
    /// the world and the untyped kills).
    fn wolf_armor_absorbs(&self, source: &Damage) -> bool {
        self.species() == Species::Wolf
            && self.extra.head.as_deref() == Some("minecraft:wolf_armor")
            && !matches!(
                source.kind,
                DamageKind::Drown
                    | DamageKind::Magic
                    | DamageKind::InWall
                    | DamageKind::Thorns
                    | DamageKind::OutOfWorld
                    | DamageKind::Cramming
                    | DamageKind::Generic
                    | DamageKind::GenericKill
                    | DamageKind::OutsideBorder
                    | DamageKind::Starve
                    | DamageKind::Wither
            )
    }
    /// `Wolf.actuallyHurt`: the armor loses `ceil(damage)` durability, cracks
    /// at `Crackiness.WOLF_ARMOR` steps and breaks at its maximum.
    fn hurt_wolf_armor(&mut self, damage: f32) {
        let crack = |lost: i32| {
            let left = (WOLF_ARMOR_DURABILITY - lost) as f32 / WOLF_ARMOR_DURABILITY as f32;
            [0.95_f32, 0.69, 0.32].iter().filter(|t| left < **t).count()
        };
        let before = self.extra.body_damage;
        let after = before + damage.ceil() as i32;
        let at = [self.position.x, self.position.y, self.position.z];
        if after >= WOLF_ARMOR_DURABILITY {
            self.extra.head = None;
            self.extra.body_damage = 0;
            self.extra.body_extra = None;
            self.outbox
                .push(MobEvent::Sound("item.wolf_armor.break", at, 1.0, 1.0));
        } else {
            self.extra.body_damage = after;
            if crack(before) != crack(after) {
                self.outbox
                    .push(MobEvent::Sound("item.wolf_armor.crack", at, 1.0, 1.0));
            }
        }
    }
    /// LivingEntity.knockback(power, xd, zd) towards the negative direction.
    pub fn knockback(&mut self, power: f64, mut xd: f64, mut zd: f64) {
        let power = power * (1.0 - self.attrs.knockback_resistance);
        if power <= 0.0 {
            return;
        }
        self.needs_sync = true;
        while xd * xd + zd * zd < f64::from(1.0e-5_f32) {
            xd = (self.random.next_double() - self.random.next_double()) * 0.01;
            zd = (self.random.next_double() - self.random.next_double()) * 0.01;
        }
        let len = (xd * xd + zd * zd).sqrt();
        let (kx, kz) = (xd / len * power, zd / len * power);
        self.velocity = Vec3::new(
            self.velocity.x / 2.0 - kx,
            if self.on_ground {
                (self.velocity.y / 2.0 + power).min(0.4)
            } else {
                self.velocity.y
            },
            self.velocity.z / 2.0 - kz,
        );
    }
    /// One game tick. Returns false once the mob should be removed.
    pub fn tick(
        &mut self,
        w: &MobWorld,
        level_random: &mut LegacyRandom,
        events: &mut Vec<MobEvent>,
    ) -> bool {
        ai::tick(self, w, level_random, events)
    }
    pub fn distance_to(&self, p: [f64; 3]) -> f64 {
        let d = [
            p[0] - self.position.x,
            p[1] - self.position.y,
            p[2] - self.position.z,
        ];
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    }
    pub fn distance_sqr(&self, p: Vec3) -> f64 {
        (p.x - self.position.x).powi(2)
            + (p.y - self.position.y).powi(2)
            + (p.z - self.position.z).powi(2)
    }
    /// Movement speed attribute bonus (baby zombies, sprinting modifiers).
    /// MOVEMENT_SPEED including effects (speed +20%/level, slowness -15%/level).
    pub fn movement_speed(&self) -> f64 {
        let mut v = strider::movement_speed(
            self,
            witch::speed(self, self.attrs.speed) + enderman::attacking_speed(self),
        ) * self.speed_bonus();
        // LivingEntity.SPEED_MODIFIER_SPRINTING: 0.3F, ADD_MULTIPLIED_TOTAL.
        if self.extra.sprinting {
            v *= 1.0 + f64::from(0.3f32);
        }
        for (e, _, amp) in &self.extra.effects {
            match *e {
                "minecraft:speed" => v *= 1.0 + 0.2 * f64::from(amp + 1),
                "minecraft:slowness" => v *= (1.0 - 0.15 * f64::from(amp + 1)).max(0.0),
                _ => {}
            }
        }
        v
    }
    pub fn has_effect(&self, effect: &str) -> bool {
        self.extra.effects.iter().any(|e| e.0 == effect)
    }
    /// `LivingEntity.isCurrentlyGlowing`: the Glowing effect or the `Glowing` tag (on a
    /// server's client, `glowing` holds shared flag 6 instead).
    pub fn is_currently_glowing(&self) -> bool {
        self.glowing || self.has_effect("minecraft:glowing")
    }
    /// `Entity.isInvisible`: the shared flag 0x20 a server sends (`invisible`),
    /// or the Invisibility effect on a mob this game ticks itself.
    pub fn is_invisible(&self) -> bool {
        self.invisible || self.has_effect("minecraft:invisibility")
    }
    pub fn add_effect(&mut self, effect: &'static str, ticks: i32, amplifier: i32) {
        let effect = horizon_client_core::potions::namespaced(effect);
        if let Some(e) = self.extra.effects.iter_mut().find(|e| e.0 == effect) {
            // MobEffectInstance.update: a stronger effect replaces with its
            // own duration, an equal one only extends it; a weaker one is
            // ignored (vanilla keeps it hidden behind the active one).
            if amplifier > e.2 {
                e.1 = ticks;
                e.2 = amplifier;
            } else if amplifier == e.2 && e.1 != -1 && (ticks == -1 || e.1 < ticks) {
                e.1 = ticks;
            }
        } else {
            self.extra.effects.push((effect, ticks, amplifier));
        }
    }
    pub fn speed_bonus(&self) -> f64 {
        ai::speed_bonus(self)
    }
    pub fn center(&self) -> [f64; 3] {
        [
            self.position.x,
            self.position.y + f64::from(self.height()) / 2.0,
            self.position.z,
        ]
    }
    pub fn sound(&self, event: &'static str, events: &mut Vec<MobEvent>) {
        let at = [self.position.x, self.eye_y(), self.position.z];
        events.push(MobEvent::Sound(event, at, 1.0, 1.0));
    }
    /// `Mob.getAmbientSoundInterval`; `HappyGhast` waits six times as long
    /// while it carries a passenger.
    pub fn ambient_interval(&self) -> i32 {
        let ridden = self.rider.is_some() || self.extra.rider.is_some();
        if self.species() == Species::HappyGhast && ridden {
            self.kind.ambient_interval * 6
        } else {
            self.kind.ambient_interval
        }
    }
    /// LivingEntity.getVoicePitch.
    pub fn voice_pitch(&mut self) -> f32 {
        // HappyGhast.getVoicePitch: always 1.0, babies and all.
        if self.species() == Species::HappyGhast {
            return 1.0;
        }
        // ZombieVillager.getVoicePitch: babies are 2.0, not 1.5.
        let baby = if self.species() == Species::ZombieVillager {
            2.0
        } else {
            1.5
        };
        let pitch = (self.random.next_float() - self.random.next_float()) * 0.2
            + if self.is_baby() { baby } else { 1.0 };
        // Bat.getVoicePitch: a little lower.
        if self.species() == Species::Bat {
            pitch * 0.95
        } else {
            pitch
        }
    }
    /// Mob.playSound with LivingEntity.getVoicePitch.
    pub fn voice(&mut self, event: &'static str, events: &mut Vec<MobEvent>) {
        let pitch = self.voice_pitch();
        // Entity.playSound: at the entity's feet.
        let at = [self.position.x, self.position.y, self.position.z];
        events.push(MobEvent::Sound(event, at, ai::sound_volume(self), pitch));
    }
}
/// Model geometry for a mob, posed for its animation state, as camera-relative
/// vertices textured from the shared atlas.
pub fn render(
    mob: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    uv: [f32; 4],
    light: u32,
    out: &mut Vec<Vertex>,
) {
    render::render_single(mob, alpha, camera, uv, light, out);
}
/// A parsed `entity-models.json` document, shared while loading model batches.
/// Each lookup still constructs an independent part tree for the caller.
pub struct EntityModelDocument(Value);

impl EntityModelDocument {
    pub fn load(root: &Path) -> crate::Result<Self> {
        Ok(Self(serde_json::from_slice(&fs::read(
            root.join("entity-models.json"),
        )?)?))
    }

    pub fn model(&self, key: &str) -> Option<Part> {
        self.0.get(key).map(|m| Part::parse("root", m))
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.0
            .as_object()
            .into_iter()
            .flat_map(|all| all.keys().map(String::as_str))
    }
}

/// Load one baked model layer from `entity-models.json` (e.g. `minecraft:chest#main`).
/// Batch loaders should share an `EntityModelDocument` instead.
pub fn load_model(root: &Path, key: &str) -> Option<Part> {
    EntityModelDocument::load(root).ok()?.model(key)
}
/// ModelPart.render: translate, rotate ZYX, scale, then children.
pub fn render_model(
    part: &Part,
    base: glam::Mat4,
    pose_for: &dyn Fn(&Part) -> [f32; 9],
    uv: [f32; 4],
    light: u32,
    overlay: u32,
    out: &mut Vec<Vertex>,
) {
    visit(
        part,
        base,
        ModelNormals::new(base),
        pose_for,
        uv,
        light,
        overlay,
        out,
    );
    fn visit(
        part: &Part,
        parent: glam::Mat4,
        normals: ModelNormals,
        pose_for: &dyn Fn(&Part) -> [f32; 9],
        uv: [f32; 4],
        light: u32,
        overlay: u32,
        out: &mut Vec<Vertex>,
    ) {
        let p = pose_for(part);
        let m = parent
            * glam::Mat4::from_translation(glam::Vec3::new(p[0], p[1], p[2]) / 16.0)
            * glam::Mat4::from_quat(glam::Quat::from_euler(
                glam::EulerRot::ZYX,
                p[5],
                p[4],
                p[3],
            ))
            * glam::Mat4::from_scale(glam::Vec3::new(p[6], p[7], p[8]));
        let normals = normals.part(p);
        for poly in &part.polygons {
            let normal = poly.vertex_normal(normals);
            for v in poly.vertices {
                let pos = m.transform_point3(glam::Vec3::new(v[0], v[1], v[2]) / 16.0);
                out.push(Vertex {
                    normal,
                    pos: pos.to_array(),
                    uv: [
                        uv[0] + (uv[2] - uv[0]) * v[3],
                        uv[1] + (uv[3] - uv[1]) * v[4],
                    ],
                    layer: overlay,
                    color: 0xFFFF_FFFF,
                    light,
                    glint_uv: [0.0, 0.0],
                });
            }
        }
        for c in &part.children {
            visit(c, m, normals, pose_for, uv, light, overlay, out);
        }
    }
}

impl Mob {
    /// The saddle worn, with its components.
    pub fn saddle_item(&self) -> Option<crate::items::ItemStack> {
        self.extra.saddled.then(|| {
            self.extra
                .saddle_stack
                .clone()
                .unwrap_or_else(|| crate::items::ItemStack::new("minecraft:saddle", 1))
        })
    }
    /// Wear (or take off) a saddle stack, keeping its components.
    pub fn set_saddle(&mut self, saddle: Option<crate::items::ItemStack>) {
        self.extra.saddled = saddle.is_some();
        // A plain saddle is implied by `saddled`; anything else (a copper
        // golem's antenna flower) is kept as it is.
        self.extra.saddle_stack = saddle
            .map(|s| crate::items::ItemStack { count: 1, ..s })
            .filter(|s| s.damage != 0 || s.extra.is_some() || &*s.item != "minecraft:saddle");
    }
}

#[cfg(test)]
mod lighting_tests {
    use super::*;
    use glam::{Mat4, Vec3};
    use serde_json::json;

    fn plane(normal: [f32; 3]) -> Part {
        Part::parse(
            "root",
            &json!({
                "polygons": [[[0,0,0,0,0], [0,0,0,1,0], [0,0,0,1,1], [0,0,0,0,1]]],
                "normals": [normal]
            }),
        )
    }

    #[test]
    fn exported_normals_survive_collapsed_faces_and_mirroring() {
        let part = plane([1.0, 0.0, 0.0]);
        assert_eq!(
            part.polygons[0].vertex_normal(ModelNormals::new(Mat4::IDENTITY)),
            [127, 0, 0, -127]
        );
        assert_eq!(
            part.polygons[0].vertex_normal(ModelNormals::new(Mat4::from_scale(Vec3::new(
                -1.0, -1.0, 1.0
            )))),
            [-127, 0, 0, -127]
        );
    }

    #[test]
    fn normal_matrix_includes_parent_scale_and_child_rotation() {
        let mut child = plane([1.0, 0.0, 0.0]);
        child.pose[5] = std::f32::consts::FRAC_PI_4;
        let parent = Part {
            name: "parent".into(),
            pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 2.0, 1.0, 0.5],
            polygons: Vec::new(),
            children: vec![child],
        };
        let mut animated = Vec::new();
        render::draw(
            &parent,
            Mat4::IDENTITY,
            &anim::Poses::new(&parent),
            [0.0, 0.0, 1.0, 1.0],
            0x80402010,
            240,
            1,
            &mut animated,
        );
        let mut direct = Vec::new();
        render_model(
            &parent,
            Mat4::IDENTITY,
            &|p| p.pose,
            [0.0, 0.0, 1.0, 1.0],
            240,
            1,
            &mut direct,
        );
        assert_eq!(animated.len(), 4);
        for (a, b) in animated.iter().zip(&direct) {
            assert_eq!(a.normal, [56, 113, 0, -127]);
            assert_eq!(a.normal, b.normal);
            assert_eq!(a.pos, b.pos);
            assert_eq!(a.uv, b.uv);
            assert_eq!((a.color, a.light, a.layer), (0x80402010, 240, 1));
        }
    }

    #[test]
    fn trusted_rotation_retains_native_quaternion_rounding() {
        let mut part = plane([1.0, 0.0, 0.0]);
        part.pose[3] = std::f32::consts::FRAC_PI_2;
        let mut vertices = Vec::new();
        render::draw(
            &part,
            Mat4::IDENTITY,
            &anim::Poses::new(&part),
            [0.0, 0.0, 1.0, 1.0],
            0xffffffff,
            240,
            0,
            &mut vertices,
        );
        assert_eq!(vertices[0].normal, [126, 0, 0, -127]);
    }

    #[test]
    fn legacy_models_infer_non_degenerate_face_normals() {
        let part = Part::parse(
            "root",
            &json!({"polygons": [
                [[0,0,0,0,0], [1,0,0,1,0], [1,1,0,1,1], [0,1,0,0,1]]
            ]}),
        );
        assert_eq!(part.polygons[0].normal, [0.0, 0.0, 1.0]);
    }

    #[test]
    fn packed_normal_uses_native_snorm_bytes_without_changing_vertex_offsets() {
        assert_eq!(std::mem::offset_of!(Vertex, normal), 32);
        assert_eq!(std::mem::size_of::<Vertex>(), 44);
        let part = plane([0.0, 1.0, 0.0]);
        assert_eq!(
            part.polygons[0].vertex_normal(ModelNormals::new(Mat4::from_scale(Vec3::ZERO))),
            [0, 0, 0, -127]
        );
    }
}

/// `LivingEntity.elytraAnimationState`: eased wing angles and render interpolation.
#[derive(Clone, Copy, Debug, Default)]
pub struct ElytraAnimation {
    pub old: [f32; 3],
    pub angles: [f32; 3],
}
impl ElytraAnimation {
    pub fn tick(&mut self, flying: bool, crouching: bool, velocity: horizon_core::geometry::Vec3) {
        use std::f32::consts::PI;
        self.old = self.angles;
        let target = if flying {
            let length =
                (velocity.x * velocity.x + velocity.y * velocity.y + velocity.z * velocity.z)
                    .sqrt();
            let ratio = if velocity.y < 0.0 && length > 0.0 {
                1.0 - horizon_core::jmath::pow(-velocity.y / length, 1.5) as f32
            } else {
                1.0
            };
            [
                PI / 12.0 + (PI / 9.0 - PI / 12.0) * ratio,
                0.0,
                -PI / 12.0 + (-PI / 2.0 + PI / 12.0) * ratio,
            ]
        } else if crouching {
            [PI * 2.0 / 9.0, 0.08726646, -PI / 4.0]
        } else {
            [PI / 12.0, 0.0, -PI / 12.0]
        };
        for (angle, target) in self.angles.iter_mut().zip(target) {
            *angle += (target - *angle) * 0.3;
        }
    }
    pub fn angles(&self, partial: f32) -> [f32; 3] {
        std::array::from_fn(|i| self.old[i] + (self.angles[i] - self.old[i]) * partial)
    }
}

/// `LivingEntity.swimAmount` and the render state the swimming pose reads
/// (`isVisuallySwimming`, `isInWater`).
#[derive(Clone, Copy, Debug, Default)]
pub struct SwimAnimation {
    pub old: f32,
    pub amount: f32,
    /// `isVisuallySwimming`: in the swimming pose (or still in the gliding
    /// pose after a glide ended).
    pub visually: bool,
    pub in_water: bool,
}
impl SwimAnimation {
    /// `LivingEntity.updateSwimAmount`.
    pub fn tick(&mut self, visually: bool, in_water: bool) {
        self.old = self.amount;
        self.amount = if visually {
            (self.amount + 0.09).min(1.0)
        } else {
            (self.amount - 0.09).max(0.0)
        };
        self.visually = visually;
        self.in_water = in_water;
    }
    /// `LivingEntity.getSwimAmount`.
    pub fn amount(&self, partial: f32) -> f32 {
        self.old + partial * (self.amount - self.old)
    }
}

impl Mob {
    /// The part of `LivingEntity.tick` and `aiStep` that animates another
    /// player from its synched pose and flags: the glide count, the wings
    /// (`elytraAnimationState`) and the swim amount (`updateSwimAmount`).
    pub fn tick_avatar_pose(&mut self, in_water: bool) {
        let (p, v) = (self.previous, self.velocity);
        let on_ground_alive = self.on_ground && self.alive();
        let e = &mut self.extra;
        e.fall_fly_ticks = if e.fall_flying {
            e.fall_fly_ticks + 1
        } else {
            0
        };
        // LivingEntity.isVisuallySwimming: the swimming pose, or the gliding
        // pose after the glide ended.
        let visually = e.synced_pose == 3 || (!e.fall_flying && e.synced_pose == 1);
        e.elytra.tick(e.fall_flying, e.crouching, self.velocity);
        e.swim.tick(visually, in_water);
        // AbstractClientPlayer.tick's ClientAvatarState.tick (from where the
        // player was before this tick's move), RemotePlayer.aiStep's
        // updateBob and rideTick's resetBob.
        e.avatar.tick([p.x, p.y, p.z]);
        if e.passenger {
            e.avatar.reset_bob();
        } else {
            let swimming = e.synced_pose == 3;
            e.avatar
                .update_bob(on_ground_alive && !swimming, [v.x, v.y, v.z]);
        }
    }
}

#[cfg(test)]
mod elytra_tests {
    use super::*;
    #[test]
    fn wings_ease_between_ground_flight_and_crouch() {
        let mut wings = ElytraAnimation::default();
        wings.tick(false, false, Vec3::ZERO);
        assert!((wings.angles[0] - std::f32::consts::PI / 12.0 * 0.3).abs() < 0.000001);
        assert_eq!(wings.angles(0.0), wings.old);
        assert_eq!(wings.angles(1.0), wings.angles);
        for _ in 0..40 {
            wings.tick(true, false, Vec3::new(1.0, 0.0, 0.0));
        }
        assert!((wings.angles[2] + std::f32::consts::FRAC_PI_2).abs() < 0.00001);
        for _ in 0..40 {
            wings.tick(false, true, Vec3::ZERO);
        }
        assert!((wings.angles[1] - 0.08726646).abs() < 0.00001);
    }
}
#[cfg(test)]
mod teams_tests {
    use super::*;
    use crate::save::uuid_text;
    use crate::scoreboard::{Scoreboard, Team};

    /// A board where every other mob is on the player's team, as with a
    /// datapack that colours its mobs' names.
    fn board(mobs: u32) -> (Scoreboard, HashMap<u32, [i32; 4]>) {
        let mut board = Scoreboard::default();
        board.add_team("red");
        board.add_to_team("Steve", "red");
        let uuids: HashMap<u32, [i32; 4]> = (0..mobs)
            .map(|id| (id, [id as i32, 7, -(id as i32), 11]))
            .collect();
        for (id, uuid) in &uuids {
            if id % 2 == 0 {
                board.add_to_team(&uuid_text(*uuid), "red");
            }
        }
        (board, uuids)
    }
    /// `Game::mob_teams` as built for each mob before: owned clones of the teams.
    fn owned(board: &Scoreboard, uuids: &HashMap<u32, [i32; 4]>) -> HashMap<u32, Team> {
        uuids
            .iter()
            .filter_map(|(id, u)| Some((*id, board.players_team(&uuid_text(*u))?.clone())))
            .collect()
    }
    fn borrowed<'a>(board: &'a Scoreboard, uuids: &HashMap<u32, [i32; 4]>) -> Teams<'a> {
        Teams {
            player: board.players_team("Steve"),
            mobs: uuids
                .iter()
                .filter_map(|(id, u)| Some((*id, board.players_team(&uuid_text(*u))?)))
                .collect(),
        }
    }

    #[test]
    fn borrowed_teams_answer_as_the_clones() {
        let (board, uuids) = board(40);
        let teams = borrowed(&board, &uuids);
        let old = owned(&board, &uuids);
        for id in 0..41 {
            assert_eq!(teams.of(Who::Mob(id)), old.get(&id), "mob {id}");
        }
        assert_eq!(teams.of(Who::Player).map(|t| t.name.as_str()), Some("red"));
    }

    /// One tick's worth of team building: once per mob (before) against once.
    #[test]
    #[ignore]
    fn teams_cost() {
        for mobs in [100u32, 250, 500] {
            let (board, uuids) = board(mobs);
            let t = std::time::Instant::now();
            for _ in 0..mobs {
                std::hint::black_box(owned(&board, &uuids));
            }
            let before = t.elapsed();
            let t = std::time::Instant::now();
            let reps = 20;
            for _ in 0..reps {
                std::hint::black_box(borrowed(&board, &uuids));
            }
            let after = t.elapsed() / reps;
            println!(
                "{mobs} mobs, half on a team: per tick {:.3} ms -> {:.3} ms",
                before.as_secs_f64() * 1e3,
                after.as_secs_f64() * 1e3
            );
        }
    }
}
#[cfg(test)]
pub(crate) mod sensor_scan_tests {
    use super::*;
    use horizon_core::geometry::Vec3;
    use serde_json::json;

    pub(crate) fn kind() -> Arc<MobType> {
        Arc::new(MobType {
            id: "minecraft:cow".into(),
            species: Species::Cow,
            width: 0.9,
            height: 1.4,
            eye_height: 1.3,
            category: "creature".into(),
            max_health: 10.0,
            speed: 0.2,
            flying_speed: 0.4,
            attack_damage: 0.0,
            attack_knockback: 0.0,
            follow_range: 16.0,
            step_height: 0.6,
            armor: 0.0,
            armor_toughness: 0.0,
            knockback_resistance: 0.0,
            jump_strength: 0.42,
            attributes: Arc::default(),
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
            ambient_interval: 120,
            flags: Flags {
                animal: true,
                ..Default::default()
            },
            model: Arc::new(Part::parse("root", &json!({}))),
            layers: HashMap::new(),
            texture: String::new(),
            brain: Brain::Passive,
            anim: Anim::Rabbit,
            sounds: [""; 3],
            step_sound: None,
            food: Arc::default(),
            tempt: Arc::default(),
        })
    }
    /// `n` mobs in pens of 20 spread over a square of `side` blocks, as on a
    /// server's farms; their snapshot and its grid.
    fn herd(n: u32, side: f64) -> (Vec<Seen>, seen_grid::SeenGrid) {
        let kind = kind();
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = |range: f64| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (seed >> 11) as f64 / (1u64 << 53) as f64 * range
        };
        let mut pen = Vec3::ZERO;
        let seen: Vec<Seen> = (0..n)
            .map(|id| {
                if id % 20 == 0 {
                    pen = Vec3::new(next(side), 64.0, next(side));
                }
                let at = Vec3::new(pen.x + next(8.0), 64.0, pen.z + next(8.0));
                Mob::new(id, kind.clone(), at, 0.0).seen(None)
            })
            .collect();
        let grid = seen_grid::SeenGrid::new(seen.iter().map(|s| s.bounds));
        (seen, grid)
    }
    /// `NearestLivingEntitySensor` over every mob, as the brains ran it.
    fn scan(seen: &[Seen], me: &Seen, area: Aabb) -> Vec<u32> {
        seen.iter()
            .filter(|o| o.id != me.id && o.alive && o.bounds.intersects(area))
            .map(|o| o.id)
            .collect()
    }
    fn indexed(seen: &[Seen], grid: &seen_grid::SeenGrid, me: &Seen, area: Aabb) -> Vec<u32> {
        let mut near = Vec::new();
        assert!(grid.candidates(area, &mut near));
        near.iter()
            .map(|&i| &seen[i as usize])
            .filter(|o| o.id != me.id && o.alive && o.bounds.intersects(area))
            .map(|o| o.id)
            .collect()
    }

    #[test]
    fn the_grid_finds_what_the_scan_finds() {
        let (seen, grid) = herd(600, 160.0);
        for me in &seen {
            let area = me.bounds.inflate(Vec3::new(16.0, 16.0, 16.0));
            assert_eq!(indexed(&seen, &grid, me, area), scan(&seen, me, area));
        }
    }

    /// `MobWorld::seen` once per mob (a target, a breeding partner...), by
    /// scan as before and by the tick's id index.
    #[test]
    #[ignore]
    fn lookup_cost() {
        for n in [500u32, 1000, 2000] {
            let (seen, _) = herd(n, 256.0);
            let index: crate::fast_hash::FxHashMap<u32, u32> = seen
                .iter()
                .enumerate()
                .map(|(i, s)| (s.id, i as u32))
                .collect();
            let wanted: Vec<u32> = (0..n).map(|i| (i * 7919) % n).collect();
            let t = std::time::Instant::now();
            let a: usize = wanted
                .iter()
                .filter_map(|&id| seen.iter().find(|s| s.id == id))
                .map(|s| s.id as usize)
                .sum();
            let before = t.elapsed().as_secs_f64() * 1e3;
            let t = std::time::Instant::now();
            let b: usize = wanted
                .iter()
                .filter_map(|&id| index.get(&id).map(|&i| &seen[i as usize]))
                .map(|s| s.id as usize)
                .sum();
            let after = t.elapsed().as_secs_f64() * 1e3;
            assert_eq!(a, b);
            println!(
                "{n} lookups over {n} mobs ({} B each): {before:.3} ms -> {after:.3} ms",
                std::mem::size_of::<Seen>()
            );
        }
    }

    /// Natural spawning's `isUnobstructed` box test against every mob (real
    /// `Mob`s), by scan and by a grid built once for the pass.
    #[test]
    #[ignore]
    fn spawn_box_cost() {
        let kind = kind();
        for n in [500u32, 1000, 2000] {
            let (seen, _) = herd(n, 384.0);
            let mobs: Vec<Mob> = seen
                .iter()
                .map(|s| Mob::new(s.id, kind.clone(), s.position, 0.0))
                .collect();
            let mut seed = 7u64;
            let mut next = |range: f64| {
                seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                (seed >> 11) as f64 / (1u64 << 53) as f64 * range
            };
            let queries: Vec<Aabb> = (0..100)
                .map(|_| {
                    let (x, z) = (next(384.0), next(384.0));
                    Aabb::new(Vec3::new(x, 64.0, z), Vec3::new(x + 0.9, 65.4, z + 0.9))
                })
                .collect();
            let t = std::time::Instant::now();
            let a: Vec<bool> = queries
                .iter()
                .map(|&q| {
                    mobs.iter()
                        .any(|m| m.alive() && !m.removed && m.bounds().intersects(q))
                })
                .collect();
            let before = t.elapsed().as_secs_f64() * 1e3;
            let t = std::time::Instant::now();
            let boxes: Vec<Aabb> = mobs.iter().map(|m| m.bounds()).collect();
            let grid = seen_grid::SeenGrid::new(boxes.iter().copied());
            let build = t.elapsed().as_secs_f64() * 1e3;
            let mut near = Vec::new();
            let b: Vec<bool> = queries
                .iter()
                .map(|&q| {
                    assert!(grid.candidates(q, &mut near));
                    near.iter().any(|&i| boxes[i as usize].intersects(q))
                })
                .collect();
            let after = t.elapsed().as_secs_f64() * 1e3;
            assert_eq!(a, b);
            println!(
                "{n} mobs, 100 spawn box tests: {before:.3} ms -> {after:.3} ms (grid build {build:.3} ms included)"
            );
        }
    }

    /// One tick of sensing: a twentieth of the mobs fire their sensor.
    #[test]
    #[ignore]
    fn sensor_cost() {
        for (n, side) in [(500u32, 256.0), (1000, 384.0), (2000, 512.0)] {
            let (seen, grid) = herd(n, side);
            let time = |f: &dyn Fn(&Seen, Aabb) -> usize| {
                let t = std::time::Instant::now();
                let mut found = 0;
                for me in &seen {
                    found += f(me, me.bounds.inflate(Vec3::new(16.0, 16.0, 16.0)));
                }
                (t.elapsed().as_secs_f64() * 1e3 / 20.0, found)
            };
            let (before, a) = time(&|me, area| scan(&seen, me, area).len());
            let (after, b) = time(&|me, area| indexed(&seen, &grid, me, area).len());
            assert_eq!(a, b);
            println!("{n} mobs: per tick {before:.3} ms -> {after:.3} ms");
        }
    }
}
#[cfg(test)]
mod push_tests {
    use super::sensor_scan_tests::kind;
    use super::*;
    use horizon_core::geometry::Vec3;

    /// The pending pushes as one list, scanned for each mob, as before.
    fn take_old(events: &mut Vec<MobEvent>, from: usize, pending: &mut Vec<(u32, Vec3)>) {
        let mut i = from;
        while i < events.len() {
            if let MobEvent::Push {
                target: Who::Mob(id),
                velocity,
            } = events[i]
            {
                pending.push((id, Vec3::new(velocity[0], velocity[1], velocity[2])));
                events.remove(i);
            } else {
                i += 1;
            }
        }
    }
    fn apply_old(velocity: &mut Vec3, id: u32, pending: &mut Vec<(u32, Vec3)>) {
        pending.retain(|(to, v)| {
            if *to != id {
                return true;
            }
            *velocity = Vec3::new(velocity.x + v.x, velocity.y + v.y, velocity.z + v.z);
            false
        });
    }
    fn mobs(n: u32) -> Vec<Mob> {
        let kind = kind();
        (0..n)
            .map(|id| Mob::new(id, kind.clone(), Vec3::ZERO, 0.0))
            .collect()
    }
    /// A crowd tick: each mob takes its pushes, then pushes every other
    /// mob (and the player, and hisses now and then); then all take theirs.
    fn crowd_tick(
        n: u32,
        tick: u32,
        mut on_push: impl FnMut(&mut Vec<MobEvent>, usize),
        mut on_apply: impl FnMut(u32),
    ) -> Vec<MobEvent> {
        let mut events = Vec::new();
        for i in 0..n {
            on_apply(i);
            let from = events.len();
            for j in 0..n {
                if j != i && !(i * 31 + j * 17 + tick).is_multiple_of(5) {
                    let f = f64::from(i * 7 + j + tick) * 0.001;
                    events.push(MobEvent::Push {
                        target: Who::Mob(j),
                        velocity: [f, -f * 0.5, f * 0.25],
                    });
                }
                if j.is_multiple_of(9) {
                    events.push(MobEvent::Hiss { mob: j });
                }
            }
            events.push(MobEvent::Push {
                target: Who::Player,
                velocity: [0.1, 0.0, 0.1],
            });
            on_push(&mut events, from);
        }
        for i in 0..n {
            on_apply(i);
        }
        events
    }

    #[test]
    fn pushes_land_as_with_the_list() {
        for n in [1u32, 3, 8, 20, 60] {
            let mut new_mobs = mobs(n);
            let mut old_v = vec![Vec3::ZERO; n as usize];
            let mut pending = PendingPushes::default();
            let mut old_pending = Vec::new();
            for tick in 0..4 {
                let new_events = {
                    let pending = std::cell::RefCell::new(&mut pending);
                    let mobs = std::cell::RefCell::new(&mut new_mobs);
                    crowd_tick(
                        n,
                        tick,
                        |e, from| take_mob_pushes(e, from, &mut pending.borrow_mut()),
                        |i| {
                            apply_mob_pushes(
                                &mut mobs.borrow_mut()[i as usize],
                                &mut pending.borrow_mut(),
                            )
                        },
                    )
                };
                let old_events = {
                    let pending = std::cell::RefCell::new(&mut old_pending);
                    let v = std::cell::RefCell::new(&mut old_v);
                    crowd_tick(
                        n,
                        tick,
                        |e, from| take_old(e, from, &mut pending.borrow_mut()),
                        |i| {
                            apply_old(
                                &mut v.borrow_mut()[i as usize],
                                i,
                                &mut pending.borrow_mut(),
                            )
                        },
                    )
                };
                assert_eq!(format!("{new_events:?}"), format!("{old_events:?}"));
                for i in 0..n as usize {
                    let (a, b) = (new_mobs[i].velocity, old_v[i]);
                    assert_eq!(
                        (a.x.to_bits(), a.y.to_bits(), a.z.to_bits()),
                        (b.x.to_bits(), b.y.to_bits(), b.z.to_bits()),
                        "{n} mobs, tick {tick}, mob {i}"
                    );
                }
            }
        }
    }

    /// One tick of a pen of `n` mobs pushing each other.
    #[test]
    #[ignore]
    fn push_cost() {
        for n in [10u32, 30, 100, 200] {
            let mut ms = mobs(n);
            let mut v = vec![Vec3::ZERO; n as usize];
            let t = std::time::Instant::now();
            {
                let pending = std::cell::RefCell::new(Vec::new());
                let v = std::cell::RefCell::new(&mut v);
                crowd_tick(
                    n,
                    0,
                    |e, from| take_old(e, from, &mut pending.borrow_mut()),
                    |i| {
                        apply_old(
                            &mut v.borrow_mut()[i as usize],
                            i,
                            &mut pending.borrow_mut(),
                        )
                    },
                );
            }
            let before = t.elapsed().as_secs_f64() * 1e3;
            let t = std::time::Instant::now();
            {
                let pending = std::cell::RefCell::new(PendingPushes::default());
                let ms = std::cell::RefCell::new(&mut ms);
                crowd_tick(
                    n,
                    0,
                    |e, from| take_mob_pushes(e, from, &mut pending.borrow_mut()),
                    |i| {
                        apply_mob_pushes(
                            &mut ms.borrow_mut()[i as usize],
                            &mut pending.borrow_mut(),
                        )
                    },
                );
            }
            let after = t.elapsed().as_secs_f64() * 1e3;
            println!("{n} mobs pushing each other: {before:.3} ms -> {after:.3} ms");
        }
    }
}
