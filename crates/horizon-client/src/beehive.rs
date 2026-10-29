//! Beehives and bee nests (`BeehiveBlockEntity`): up to three bees stored as
//! entity tags, let out when their time inside is up (never at night or in
//! the rain, nor through a blocked front, unless the hive is emptied in an
//! emergency), honey for every bee coming home with nectar. The host (the game
//! or the mob AI probe) loads and adds the bees through [`HiveHost`].
pub use crate::chunk::block_entity::{Beehive, Occupant};
use crate::mobs::{Mob, Species, bee};
use horizon_core::random::{LegacyRandom, RandomSource};
use horizon_world::nbt::{Compound, Tag};

pub const MAX_OCCUPANTS: usize = 3;
/// `BeehiveBlockEntity.IGNORED_BEE_TAGS`: dropped from a bee going in, and
/// again from the tag it comes out with.
const IGNORED_BEE_TAGS: [&str; 25] = [
    "Air",
    "drop_chances",
    "equipment",
    "Brain",
    "CanPickUpLoot",
    "DeathTime",
    "fall_distance",
    "FallFlying",
    "Fire",
    "HurtTime",
    "LeftHanded",
    "Motion",
    "NoGravity",
    "OnGround",
    "PortalCooldown",
    "Pos",
    "Rotation",
    "sleeping_pos",
    "CannotEnterHiveTicks",
    "TicksSincePollination",
    "CropsGrownSincePollination",
    "hive_pos",
    "Passengers",
    "leash",
    "UUID",
];

/// `BeehiveBlockEntity.BeeReleaseStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Release {
    HoneyDelivered,
    BeeReleased,
    Emergency,
}

/// What a hive needs from the level it is in.
pub trait HiveHost {
    /// `level.getRandom()`.
    fn random(&mut self) -> &mut LegacyRandom;
    /// `EnvironmentAttributes.BEES_STAY_IN_HIVE` at a position.
    fn bees_stay_in_hive(&self, at: [i32; 3]) -> bool;
    /// A non-empty collision shape at a block.
    fn blocked(&self, at: [i32; 3]) -> bool;
    /// `BlockState.setValue(HONEY_LEVEL, level)` on the hive.
    fn set_honey_level(&mut self, at: [i32; 3], level: i32);
    /// `EntityType.loadEntityRecursive` (the tag has its `id` and a `Pos`):
    /// a new entity, or `None` for anything but a `#beehive_inhabitors`.
    fn load(&mut self, tag: &Tag) -> Option<Mob>;
    /// `Level.addFreshEntity`.
    fn add(&mut self, m: Mob) -> bool;
    fn sound(&mut self, name: &'static str, at: [f64; 3]);
}

fn strip_ignored(c: &mut Compound) {
    for key in IGNORED_BEE_TAGS {
        c.remove(&key.into());
    }
}
fn int(tag: Option<&Tag>) -> Option<i32> {
    match tag {
        Some(Tag::Int(v)) => Some(*v),
        _ => None,
    }
}
fn pos(tag: Option<&Tag>) -> Option<[i32; 3]> {
    match tag {
        Some(Tag::IntArray(p)) if p.len() == 3 => Some([p[0], p[1], p[2]]),
        _ => None,
    }
}

/// The bee occupant's codec and constructors, over the data in `chunk::block_entity`.
pub trait OccupantExt: Sized {
    /// `Occupant.of`: the bee's saved tag without the ignored tags, staying
    /// 2400 ticks with nectar, else 600.
    fn of(tag: Tag) -> Self;
    /// `Occupant.create`: a worldgen nest's bee, a bare bee tag.
    fn create(ticks_in_hive: i32) -> Self;
    fn has_nectar(&self) -> bool;
    fn from_nbt(tag: &Tag) -> Option<Self>;
    fn to_nbt(&self) -> Tag;
}
impl OccupantExt for Occupant {
    fn of(mut tag: Tag) -> Self {
        if let Tag::Compound(c) = &mut tag {
            strip_ignored(c);
        }
        let nectar = matches!(tag.get("HasNectar"), Some(Tag::Byte(v)) if *v != 0);
        Self {
            entity_data: tag,
            ticks_in_hive: 0,
            min_ticks_in_hive: if nectar { 2400 } else { 600 },
        }
    }
    fn create(ticks_in_hive: i32) -> Self {
        Self {
            entity_data: Tag::compound([("id", Tag::String("minecraft:bee".into()))]),
            ticks_in_hive,
            min_ticks_in_hive: 600,
        }
    }
    fn has_nectar(&self) -> bool {
        matches!(self.entity_data.get("HasNectar"), Some(Tag::Byte(v)) if *v != 0)
    }
    fn from_nbt(tag: &Tag) -> Option<Self> {
        let data = tag.get("entity_data")?;
        if !matches!(data, Tag::Compound(c) if c.contains_key(&"id".into())) {
            return None;
        }
        Some(Self {
            entity_data: data.clone(),
            ticks_in_hive: int(tag.get("ticks_in_hive"))?,
            min_ticks_in_hive: int(tag.get("min_ticks_in_hive"))?,
        })
    }
    fn to_nbt(&self) -> Tag {
        Tag::compound([
            ("entity_data", self.entity_data.clone()),
            ("ticks_in_hive", Tag::Int(self.ticks_in_hive)),
            ("min_ticks_in_hive", Tag::Int(self.min_ticks_in_hive)),
        ])
    }
}

/// `BeehiveBlockEntity`'s behaviour, over the data in `chunk::block_entity`.
pub trait BeehiveExt: Sized {
    fn full(&self) -> bool;
    /// `loadAdditional`: `bees` (all or none, as the list codec reads them)
    /// and `flower_pos`.
    fn from_nbt(tag: &Tag) -> Self;
    /// `saveAdditional`.
    fn to_nbt(&self) -> Vec<(&'static str, Tag)>;
    /// `addOccupant` (the bee's own tag, saved by the host): the hive keeps
    /// the bee's flower, or now and then swaps its own for it. The host then
    /// discards the bee.
    fn add_occupant(
        &mut self,
        at: [i32; 3],
        tag: Tag,
        bee_flower: Option<[i32; 3]>,
        host: &mut impl HiveHost,
    ) -> bool;
    /// `serverTick`: the bees whose time is up come out, then the hive
    /// hums now and then while it holds any. `honey` is the hive state's
    /// honey level as the tick starts (`None` without the property);
    /// `facing` its front.
    fn server_tick(
        &mut self,
        at: [i32; 3],
        facing: [i32; 2],
        honey: Option<i32>,
        host: &mut impl HiveHost,
    );
    /// `releaseAllOccupants`: the ids of the bees let out.
    fn release_all(
        &mut self,
        at: [i32; 3],
        facing: [i32; 2],
        honey: Option<i32>,
        status: Release,
        host: &mut impl HiveHost,
    ) -> Vec<u32>;
}
impl BeehiveExt for Beehive {
    fn full(&self) -> bool {
        self.bees.len() == MAX_OCCUPANTS
    }
    fn from_nbt(tag: &Tag) -> Self {
        let bees = match tag.get("bees") {
            Some(Tag::List(list)) => list
                .iter()
                .map(Occupant::from_nbt)
                .collect::<Option<Vec<_>>>()
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        Self {
            bees,
            flower: pos(tag.get("flower_pos")),
        }
    }
    fn to_nbt(&self) -> Vec<(&'static str, Tag)> {
        let mut out = vec![(
            "bees",
            Tag::List(self.bees.iter().map(Occupant::to_nbt).collect()),
        )];
        if let Some(f) = self.flower {
            out.push(("flower_pos", Tag::IntArray(f.to_vec())));
        }
        out
    }
    fn add_occupant(
        &mut self,
        at: [i32; 3],
        tag: Tag,
        bee_flower: Option<[i32; 3]>,
        host: &mut impl HiveHost,
    ) -> bool {
        if self.bees.len() >= MAX_OCCUPANTS {
            return false;
        }
        self.bees.push(Occupant::of(tag));
        if let Some(f) = bee_flower
            && (self.flower.is_none() || host.random().next_boolean())
        {
            self.flower = Some(f);
        }
        let corner = at.map(f64::from);
        host.sound("block.beehive.enter", corner);
        true
    }
    fn server_tick(
        &mut self,
        at: [i32; 3],
        facing: [i32; 2],
        honey: Option<i32>,
        host: &mut impl HiveHost,
    ) {
        let flower = self.flower;
        let mut i = 0;
        while i < self.bees.len() {
            // BeeData.tick: ticksInHive++ > minTicksInHive.
            let b = &mut self.bees[i];
            let up = b.ticks_in_hive > b.min_ticks_in_hive;
            b.ticks_in_hive += 1;
            if up {
                let status = if b.has_nectar() {
                    Release::HoneyDelivered
                } else {
                    Release::BeeReleased
                };
                let occupant = b.clone();
                if release(host, at, facing, honey, &occupant, status, flower).is_some() {
                    self.bees.remove(i);
                    continue;
                }
            }
            i += 1;
        }
        if !self.bees.is_empty() && host.random().next_double() < 0.005 {
            let hum = [
                f64::from(at[0]) + 0.5,
                f64::from(at[1]),
                f64::from(at[2]) + 0.5,
            ];
            host.sound("block.beehive.work", hum);
        }
    }
    fn release_all(
        &mut self,
        at: [i32; 3],
        facing: [i32; 2],
        honey: Option<i32>,
        status: Release,
        host: &mut impl HiveHost,
    ) -> Vec<u32> {
        let flower = self.flower;
        let mut out = Vec::new();
        self.bees.retain(
            |b| match release(host, at, facing, honey, b, status, flower) {
                Some(id) => {
                    out.extend(id);
                    false
                }
                None => true,
            },
        );
        out
    }
}

/// `releaseOccupant`: `None` while the bee stays in, else the id of the bee
/// let out (`Some(None)` when the level refused it).
fn release(
    host: &mut impl HiveHost,
    at: [i32; 3],
    facing: [i32; 2],
    honey: Option<i32>,
    occupant: &Occupant,
    status: Release,
    flower: Option<[i32; 3]>,
) -> Option<Option<u32>> {
    let emergency = status == Release::Emergency;
    if host.bees_stay_in_hive(at) && !emergency {
        return None;
    }
    let blocked = host.blocked([at[0] + facing[0], at[1], at[2] + facing[1]]);
    if blocked && !emergency {
        return None;
    }
    // Occupant.createEntity: the tag without the ignored tags.
    let mut tag = occupant.entity_data.clone();
    if let Tag::Compound(c) = &mut tag {
        strip_ignored(c);
        let center = [at[0], at[1], at[2]].map(|v| Tag::Double((f64::from(v) + 0.5).to_bits()));
        c.insert("Pos".into(), Tag::List(center.to_vec()));
    }
    let mut m = host.load(&tag)?;
    m.no_gravity = true;
    if m.species() == Species::Bee {
        bee::set_hive(&mut m, Some(at));
        // setBeeReleaseData: the time inside counts towards growing up and
        // away from love.
        let ticks = occupant.ticks_in_hive;
        if !m.extra.age_locked {
            let age = m.extra.age;
            m.extra.age = if age < 0 {
                (age + ticks).min(0)
            } else if age > 0 {
                (age - ticks).max(0)
            } else {
                age
            };
        }
        m.extra.in_love = (m.extra.in_love - ticks).max(0);
        if let Some(f) = flower
            && bee::get(&m).is_none_or(|b| b.flower.is_none())
            && host.random().next_float() < 0.9
        {
            bee::set_flower(&mut m, Some(f));
        }
        if status == Release::HoneyDelivered {
            bee::drop_off_nectar(&mut m);
            if let Some(level) = honey.filter(|&l| l < 5) {
                let mut increase = if host.random().next_int_bound(100).unwrap_or(1) == 0 {
                    2
                } else {
                    1
                };
                if level + increase > 5 {
                    increase -= 1;
                }
                host.set_honey_level(at, level + increase);
            }
        }
        let delta = if blocked {
            0.0
        } else {
            0.55 + f64::from(m.width() / 2.0)
        };
        let p = horizon_core::geometry::Vec3::new(
            f64::from(at[0]) + 0.5 + delta * f64::from(facing[0]),
            f64::from(at[1]) + 0.5 - f64::from(m.height() / 2.0),
            f64::from(at[2]) + 0.5 + delta * f64::from(facing[1]),
        );
        m.position = p;
        m.previous = p;
    }
    host.sound(
        "block.beehive.exit",
        [
            f64::from(at[0]) + 0.5,
            f64::from(at[1]) + 0.5,
            f64::from(at[2]) + 0.5,
        ],
    );
    let id = m.id;
    Some(host.add(m).then_some(id))
}

/// `CampfireBlock.isSmokeyPos`: a lit campfire up to five blocks below,
/// unless a block whose collision crosses the smoke's centre post comes
/// first (a lit campfire right under that block still counts).
pub fn smokey(w: &crate::mobs::MobWorld, at: [i32; 3]) -> bool {
    let lit = |y: i32| {
        w.has_tag("minecraft:campfires", w.block_name(at[0], y, at[2]))
            && w.prop(at[0], y, at[2], "lit") == Some("true")
    };
    for i in 1..=5 {
        let y = at[1] - i;
        if lit(y) {
            return true;
        }
        // SHAPE_VIRTUAL_POST: Block.column(4, 0, 16), the middle quarter.
        let (x0, z0) = (f64::from(at[0]), f64::from(at[2]));
        let blocked = w.block_collision_boxes(at[0], y, at[2]).iter().any(|b| {
            let (min, max) = (b.min_position(), b.max_position());
            min.x < x0 + 0.625 && max.x > x0 + 0.375 && min.z < z0 + 0.625 && max.z > z0 + 0.375
        });
        if blocked {
            return lit(y - 1);
        }
    }
    false
}

/// `HorizontalDirectionalBlock.FACING` as an x/z step.
pub fn facing_step(facing: Option<&str>) -> [i32; 2] {
    match facing {
        Some("south") => [0, 1],
        Some("west") => [-1, 0],
        Some("east") => [1, 0],
        _ => [0, -1],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nbt_round_trips_in_vanilla_order() {
        let tag = Tag::compound([
            (
                "bees",
                Tag::List(vec![
                    Occupant::create(37).to_nbt(),
                    Occupant::create(0).to_nbt(),
                ]),
            ),
            ("flower_pos", Tag::IntArray(vec![1, -60, 3])),
        ]);
        let hive = Beehive::from_nbt(&tag);
        assert_eq!(hive.bees.len(), 2);
        assert_eq!(hive.bees[0].ticks_in_hive, 37);
        assert_eq!(hive.flower, Some([1, -60, 3]));
        assert_eq!(Tag::compound(hive.to_nbt()), tag);
    }

    #[test]
    fn a_bad_bee_drops_the_whole_list() {
        let tag = Tag::compound([(
            "bees",
            Tag::List(vec![
                Occupant::create(1).to_nbt(),
                Tag::compound([("ticks_in_hive", Tag::Int(3))]),
            ]),
        )]);
        assert!(Beehive::from_nbt(&tag).bees.is_empty());
    }

    #[test]
    fn going_in_strips_the_ignored_tags() {
        let tag = Tag::compound([
            ("id", Tag::String("minecraft:bee".into())),
            ("UUID", Tag::IntArray(vec![1, 2, 3, 4])),
            ("Health", Tag::Float(10.0_f32.to_bits())),
            ("HasNectar", Tag::Byte(1)),
            ("hive_pos", Tag::IntArray(vec![0, 0, 0])),
        ]);
        let o = Occupant::of(tag);
        assert_eq!(o.min_ticks_in_hive, 2400);
        assert!(o.entity_data.get("UUID").is_none());
        assert!(o.entity_data.get("hive_pos").is_none());
        assert!(o.entity_data.get("Health").is_some());
    }
}
