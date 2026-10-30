//! The monster spawner (`BaseSpawner` as `SpawnerBlockEntity` runs it): its
//! saved state and `SpawnData`, the server tick that spawns its mobs and the
//! client tick that spins the mob inside and puffs flames.
//!
//! Vanilla keeps a server and a client copy of the block entity; Horizon keeps
//! one [`Spawner`] with the client's own delay and spin beside the server's
//! state. The level's random drives the server tick in vanilla's draw order;
//! what the spawner asks of the level goes through [`SpawnerLevel`], so the
//! game and a differential test world answer the same questions.
use super::{
    Species,
    spawn_rules::{self, Reason},
    spawn_selection::{self, Category, NaturalLevel},
};
use crate::chunk::block_entity::Next;
pub use crate::chunk::block_entity::{CustomSpawnRules, SpawnData, Spawner};
use horizon_core::{geometry::Aabb, geometry::Vec3, random::RandomSource};
use horizon_world::nbt::{Compound, JavaString, Tag};

/// `SpawnData`'s codec and rules, over the data in `chunk::block_entity`.
pub trait SpawnDataExt: Sized {
    /// The `SpawnData` constructor: an `id` that is a valid identifier is
    /// stored normalised (`zombie` becomes `minecraft:zombie`), any other is
    /// removed.
    fn new(entity: Compound, rules: Option<CustomSpawnRules>, equipment: Option<Tag>) -> Self;
    /// The entity's `id`, if any.
    fn id(&self) -> Option<String>;
    /// `SpawnData.CODEC`: `entity` (required), `custom_spawn_rules` and
    /// `equipment`.
    fn decode(tag: &Tag) -> Option<Self>;
    fn encode(&self) -> Tag;
    /// The entity compound is only its `id` (`hasNoConfiguration`): the
    /// spawned mob is finalized.
    fn unconfigured(&self) -> bool;
    /// `CustomSpawnRules.isValidPosition`.
    fn valid_position<L: SpawnerLevel>(r: CustomSpawnRules, l: &L, x: i32, y: i32, z: i32) -> bool;
}
impl SpawnDataExt for SpawnData {
    fn new(mut entity: Compound, rules: Option<CustomSpawnRules>, equipment: Option<Tag>) -> Self {
        let key = JavaString::from("id");
        let id = match entity.get(&key) {
            Some(Tag::String(s)) => normalize_id(&s.to_string_lossy()),
            _ => None,
        };
        match id {
            Some(id) => {
                entity.insert(key, Tag::String(id.as_str().into()));
            }
            None => {
                entity.remove(&key);
            }
        }
        Self {
            entity,
            custom_spawn_rules: rules,
            equipment,
        }
    }
    fn id(&self) -> Option<String> {
        match self.entity.get(&JavaString::from("id")) {
            Some(Tag::String(s)) => Some(s.to_string_lossy()),
            _ => None,
        }
    }
    fn decode(tag: &Tag) -> Option<Self> {
        let Some(Tag::Compound(entity)) = tag.get("entity") else {
            return None;
        };
        let rules = match tag.get("custom_spawn_rules") {
            Some(Tag::Compound(_)) => {
                let rules = tag.get("custom_spawn_rules")?;
                Some(CustomSpawnRules {
                    block: light_limit(rules.get("block_light_limit"))?,
                    sky: light_limit(rules.get("sky_light_limit"))?,
                })
            }
            Some(_) => return None,
            None => None,
        };
        Some(Self::new(
            entity.clone(),
            rules,
            tag.get("equipment").cloned(),
        ))
    }
    fn encode(&self) -> Tag {
        let mut out = vec![("entity", Tag::Compound(self.entity.clone()))];
        if let Some(r) = self.custom_spawn_rules {
            let mut rules = Vec::new();
            for (name, range) in [("block_light_limit", r.block), ("sky_light_limit", r.sky)] {
                // lenientOptionalFieldOf: the full range is left out.
                if range != (0, 15) {
                    rules.push((name, encode_range(range)));
                }
            }
            out.push(("custom_spawn_rules", Tag::compound(rules)));
        }
        if let Some(e) = &self.equipment {
            out.push(("equipment", e.clone()));
        }
        Tag::compound(out)
    }
    fn unconfigured(&self) -> bool {
        self.entity.len() == 1 && self.id().is_some()
    }
    fn valid_position<L: SpawnerLevel>(r: CustomSpawnRules, l: &L, x: i32, y: i32, z: i32) -> bool {
        let block = l.block_light(x, y, z);
        let sky = (l.sky_light(x, y, z) - l.sky_darken()).max(0);
        (r.block.0..=r.block.1).contains(&block) && (r.sky.0..=r.sky.1).contains(&sky)
    }
}

pub use horizon_client_core::identifier::normalize_id;

/// How much a slime's or magma cube's box is scaled by the size its data
/// loads (`Slime.readAdditionalSaveData`: Size + 1, kept in 1..=127), which
/// the obstruction test sees before `finalizeSpawn` picks one.
pub(crate) fn cube_scale(id: &str, entity: &Compound) -> f32 {
    if !matches!(id, "minecraft:slime" | "minecraft:magma_cube") {
        return 1.0;
    }
    number(entity.get(&JavaString::from("Size")))
        .map_or(1, |size| (size as i32).saturating_add(1).clamp(1, 127)) as f32
}

/// `EquipmentTable.CODEC`: the loot table and the drop chance of each
/// `EquipmentSlot` (`drops::SLOTS` order) the table names, `slot_drop_chances`
/// being one number for every slot or a map by slot name.
pub fn equipment_table(tag: &Tag) -> Option<(String, [Option<f32>; 8])> {
    let chance = |t: &Tag| match t {
        Tag::Float(v) => Some(f32::from_bits(*v)),
        Tag::Double(v) => Some(f64::from_bits(*v) as f32),
        Tag::Byte(v) => Some(f32::from(*v)),
        Tag::Short(v) => Some(f32::from(*v)),
        Tag::Int(v) => Some(*v as f32),
        Tag::Long(v) => Some(*v as f32),
        _ => None,
    };
    let Some(Tag::String(table)) = tag.get("loot_table") else {
        return None;
    };
    let mut chances = [None; 8];
    match tag.get("slot_drop_chances") {
        Some(Tag::Compound(map)) => {
            for (i, slot) in super::drops::SLOTS.iter().enumerate() {
                chances[i] = map.get(&JavaString::from(*slot)).and_then(chance);
            }
        }
        Some(all) => chances = [Some(chance(all)?); 8],
        None => {}
    }
    Some((table.to_string_lossy(), chances))
}
/// A whole number from any numeric tag (`NumericTag.intValue`: floats floor).
pub(crate) fn number(tag: Option<&Tag>) -> Option<i64> {
    Some(match tag? {
        Tag::Byte(v) => i64::from(*v),
        Tag::Short(v) => i64::from(*v),
        Tag::Int(v) => i64::from(*v),
        Tag::Long(v) => *v,
        Tag::Float(b) => f32::from_bits(*b).floor() as i64,
        Tag::Double(b) => f64::from_bits(*b).floor() as i64,
        _ => return None,
    })
}
/// `InclusiveRange.INT` within 0..=15 (`lenientOptionalFieldOf` with the
/// full range as default): a number, a two-element list or
/// `{min_inclusive, max_inclusive}`. None is a decoding error.
fn light_limit(tag: Option<&Tag>) -> Option<(i32, i32)> {
    let int = |t: &Tag| match t {
        Tag::Int(v) => Some(*v),
        Tag::Byte(v) => Some(i32::from(*v)),
        Tag::Short(v) => Some(i32::from(*v)),
        _ => None,
    };
    let range = match tag {
        None => return Some((0, 15)),
        Some(Tag::IntArray(v)) if v.len() == 2 => Some((v[0], v[1])),
        Some(Tag::List(v)) if v.len() == 2 => int(&v[0]).zip(int(&v[1])),
        Some(t @ Tag::Compound(_)) => t
            .get("min_inclusive")
            .and_then(int)
            .zip(t.get("max_inclusive").and_then(int)),
        Some(t) => int(t).map(|v| (v, v)),
    };
    match range {
        // A malformed value falls back to the default (lenient); an
        // inverted range or one outside 0..=15 fails validation.
        None => Some((0, 15)),
        Some((lo, hi)) if lo <= hi && lo >= 0 && hi <= 15 => Some((lo, hi)),
        Some(_) => None,
    }
}
fn encode_range((lo, hi): (i32, i32)) -> Tag {
    if lo == hi {
        Tag::Int(lo)
    } else {
        Tag::IntArray(vec![lo, hi])
    }
}

/// What a spawner did that the level shows or sends.
#[derive(Clone, Debug, PartialEq)]
pub enum SpawnerEvent {
    /// `setNextSpawnData`: `sendBlockUpdated` with the new update tag.
    BlockUpdated,
    /// `broadcastEvent(1)`: the block event that resets the clients' delay.
    BlockEvent(i32),
    /// `levelEvent(2004)`: the flame and smoke burst at the spawner.
    LevelEvent(i32),
    /// `gameEvent(entity, ENTITY_PLACE, block)`.
    EntityPlace(u32, [i32; 3]),
    /// `mob.spawnAnim()`: entity event 20 (the poof).
    SpawnAnim(u32),
}

/// A mob or entity the spawner wants to add (`EntityType.loadEntityRecursive`
/// then `tryAddFreshEntityWithPassengers`).
#[derive(Clone, Debug)]
pub struct SpawnerSpawn<'a> {
    pub entity: &'a Compound,
    pub id: &'a str,
    pub position: [f64; 3],
    pub yaw: f32,
    /// `finalizeSpawn(SPAWNER)` runs (the data is just an id).
    pub finalize: bool,
    pub equipment: Option<&'a Tag>,
    /// A trial spawner's mob (`EntitySpawnReason.TRIAL_SPAWNER`): it is
    /// always persistent.
    pub trial: bool,
}
/// An entity the level added: its id, box and whether it is a mob.
#[derive(Clone, Copy, Debug)]
pub struct SpawnerAdded {
    pub id: u32,
    pub bounds: Aabb,
    pub mob: bool,
}

/// What a spawner reads from the level besides the spawn rules.
pub trait SpawnerLevel: NaturalLevel {
    /// `hasNearbyAlivePlayer`: a living non-spectator closer than `range`
    /// (any distance when negative).
    fn alive_player_near(&self, x: f64, y: f64, z: f64, range: f64) -> bool;
    /// The `spawner_blocks_work` game rule.
    fn spawner_blocks_work(&self) -> bool;
    /// `EntityType.byString`: the entity type exists.
    fn entity_exists(&self, id: &str) -> bool;
    /// Entities of the exact class of `id` whose box meets `bounds`.
    fn count_entities(&self, id: &str, bounds: Aabb) -> usize;
}

/// `BaseSpawner`'s behaviour, over the data in `chunk::block_entity`.
pub trait SpawnerExt: Sized {
    /// `BaseSpawner.load` (`SpawnerBlockEntity.loadAdditional`).
    fn load(tag: &Tag) -> Self;
    /// `BaseSpawner.save`: the fields as shorts, the next data when set and
    /// the potentials.
    fn save(&self) -> Vec<(&'static str, Tag)>;
    /// `SpawnerBlockEntity.getUpdateTag`: the saved fields without the potentials.
    fn update_tag(&self) -> Tag;
    fn next_data(&self) -> Option<&SpawnData>;
    fn next_data_mut(&mut self) -> Option<&mut SpawnData>;
    fn random_potential(&self, random: &mut impl RandomSource) -> Option<usize>;
    /// `getOrCreateNextSpawnData`: a potential at random, or empty data.
    fn next_or_create(&mut self, random: &mut impl RandomSource, events: &mut Vec<SpawnerEvent>);
    /// `setEntityId`: the next data's `id` (a spawn egg used on the spawner).
    fn set_entity_id(
        &mut self,
        id: &str,
        random: &mut impl RandomSource,
        events: &mut Vec<SpawnerEvent>,
    );
    /// `setEntityData` (`TypedEntityData.loadInto`): an op's spawn egg with
    /// entity data: the next data's `id`, then `data` merged into it.
    fn set_entity_data(
        &mut self,
        id: &str,
        data: &Compound,
        random: &mut impl RandomSource,
        events: &mut Vec<SpawnerEvent>,
    );
    /// `isNearPlayer`.
    fn near_player<L: SpawnerLevel>(&self, l: &L, pos: [i32; 3]) -> bool;
    /// `delay`: the next delay, a new potential (when there are any) and the
    /// block event.
    fn delay(&mut self, random: &mut impl RandomSource, events: &mut Vec<SpawnerEvent>);
    /// `serverTick`. `spawn` adds an entity the spawner accepted and returns
    /// it, or None when the level would not take it.
    fn server_tick<L: SpawnerLevel, R: RandomSource>(
        &mut self,
        l: &L,
        random: &mut R,
        pos: [i32; 3],
        spawn: &mut dyn FnMut(&L, &SpawnerSpawn) -> Option<SpawnerAdded>,
        events: &mut Vec<SpawnerEvent>,
    );
    /// `clientTick`: with a player near and a mob to show, a smoke and flame
    /// particle at a random point of the block (returned) and the spin.
    fn client_tick(
        &mut self,
        near_player: bool,
        has_display: bool,
        random: &mut impl RandomSource,
        pos: [i32; 3],
    ) -> Option<[f64; 3]>;
    /// `onEventTriggered` on the client: event 1 resets the delay.
    fn client_event(&mut self, id: i32) -> bool;
    /// The client copy received an update tag (`load`): its delay follows.
    fn client_loaded(&mut self);
    /// The display mob's spin in degrees and scale ([`display_pose`]).
    fn display_pose(&self, alpha: f32, width: f32, height: f32) -> (f32, f32);
}
impl SpawnerExt for Spawner {
    fn load(tag: &Tag) -> Self {
        let mut s = Self {
            spawn_delay: number(tag.get("Delay")).map_or(20, |v| i32::from(v as i16)),
            ..Self::default()
        };
        s.next = tag
            .get("SpawnData")
            .and_then(SpawnData::decode)
            .map(Next::Own);
        s.potentials = match tag.get("SpawnPotentials") {
            Some(Tag::List(list)) => list
                .iter()
                .filter_map(|e| {
                    let weight = match e.get("weight") {
                        Some(Tag::Int(w)) if *w >= 0 => *w,
                        _ => return None,
                    };
                    Some((SpawnData::decode(e.get("data")?)?, weight))
                })
                .collect(),
            // WeightedList.of(nextSpawnData), the same object, when the
            // potentials are missing or not a list.
            _ => {
                let data = match s.next.take() {
                    Some(Next::Own(d)) => d,
                    _ => SpawnData::default(),
                };
                let had = tag.get("SpawnData").and_then(SpawnData::decode).is_some();
                if had {
                    s.next = Some(Next::Potential(0));
                }
                vec![(data, 1)]
            }
        };
        let int = |k: &str, d: i32| number(tag.get(k)).map_or(d, |v| v as i32);
        s.min_spawn_delay = int("MinSpawnDelay", 200);
        s.max_spawn_delay = int("MaxSpawnDelay", 800);
        s.spawn_count = int("SpawnCount", 4);
        s.max_nearby_entities = int("MaxNearbyEntities", 6);
        s.required_player_range = int("RequiredPlayerRange", 16);
        s.spawn_range = int("SpawnRange", 4);
        s.client_delay = s.spawn_delay;
        s
    }
    fn save(&self) -> Vec<(&'static str, Tag)> {
        let short = |v: i32| Tag::Short(v as i16);
        let mut out = vec![
            ("Delay", short(self.spawn_delay)),
            ("MinSpawnDelay", short(self.min_spawn_delay)),
            ("MaxSpawnDelay", short(self.max_spawn_delay)),
            ("SpawnCount", short(self.spawn_count)),
            ("MaxNearbyEntities", short(self.max_nearby_entities)),
            ("RequiredPlayerRange", short(self.required_player_range)),
            ("SpawnRange", short(self.spawn_range)),
        ];
        if let Some(next) = self.next_data() {
            out.push(("SpawnData", next.encode()));
        }
        out.push((
            "SpawnPotentials",
            Tag::List(
                self.potentials
                    .iter()
                    .map(|(d, w)| Tag::compound([("data", d.encode()), ("weight", Tag::Int(*w))]))
                    .collect(),
            ),
        ));
        out
    }
    fn update_tag(&self) -> Tag {
        Tag::compound(
            self.save()
                .into_iter()
                .filter(|(k, _)| *k != "SpawnPotentials"),
        )
    }
    fn next_data(&self) -> Option<&SpawnData> {
        match self.next.as_ref()? {
            Next::Own(d) => Some(d),
            Next::Potential(i) => self.potentials.get(*i).map(|(d, _)| d),
        }
    }
    fn next_data_mut(&mut self) -> Option<&mut SpawnData> {
        match self.next.as_mut()? {
            Next::Own(d) => Some(d),
            Next::Potential(i) => self.potentials.get_mut(*i).map(|(d, _)| d),
        }
    }
    fn random_potential(&self, random: &mut impl RandomSource) -> Option<usize> {
        weighted_index(self.potentials.iter().map(|(_, w)| *w), random)
    }
    fn next_or_create(&mut self, random: &mut impl RandomSource, events: &mut Vec<SpawnerEvent>) {
        if self.next.is_some() {
            return;
        }
        self.next = Some(match self.random_potential(random) {
            Some(i) => Next::Potential(i),
            None => Next::Own(SpawnData::default()),
        });
        events.push(SpawnerEvent::BlockUpdated);
    }
    fn set_entity_id(
        &mut self,
        id: &str,
        random: &mut impl RandomSource,
        events: &mut Vec<SpawnerEvent>,
    ) {
        self.next_or_create(random, events);
        if let Some(d) = self.next_data_mut() {
            d.entity.insert("id".into(), Tag::String(id.into()));
        }
    }
    fn set_entity_data(
        &mut self,
        id: &str,
        data: &Compound,
        random: &mut impl RandomSource,
        events: &mut Vec<SpawnerEvent>,
    ) {
        self.set_entity_id(id, random, events);
        if let Some(d) = self.next_data_mut() {
            crate::data_cmd::merge_compound(&mut d.entity, data);
        }
    }
    fn near_player<L: SpawnerLevel>(&self, l: &L, pos: [i32; 3]) -> bool {
        l.alive_player_near(
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
            f64::from(self.required_player_range),
        )
    }
    fn delay(&mut self, random: &mut impl RandomSource, events: &mut Vec<SpawnerEvent>) {
        self.spawn_delay = if self.max_spawn_delay <= self.min_spawn_delay {
            self.min_spawn_delay
        } else {
            self.min_spawn_delay
                + random
                    .next_int_bound(self.max_spawn_delay - self.min_spawn_delay)
                    .unwrap_or(0)
        };
        if let Some(i) = self.random_potential(random) {
            self.next = Some(Next::Potential(i));
            events.push(SpawnerEvent::BlockUpdated);
        }
        events.push(SpawnerEvent::BlockEvent(1));
    }
    fn server_tick<L: SpawnerLevel, R: RandomSource>(
        &mut self,
        l: &L,
        random: &mut R,
        pos: [i32; 3],
        spawn: &mut dyn FnMut(&L, &SpawnerSpawn) -> Option<SpawnerAdded>,
        events: &mut Vec<SpawnerEvent>,
    ) {
        if !self.near_player(l, pos) || !l.spawner_blocks_work() {
            return;
        }
        if self.spawn_delay == -1 {
            self.delay(random, events);
        }
        if self.spawn_delay > 0 {
            self.spawn_delay -= 1;
            return;
        }
        let mut delay = false;
        self.next_or_create(random, events);
        let Some(data) = self.next_data().cloned() else {
            return;
        };
        // Boxes of what this tick added, for the obstruction test.
        let mut added: Vec<Aabb> = Vec::new();
        for _ in 0..self.spawn_count {
            // EntityType.by: an unknown type delays.
            let Some(id) = data.id().filter(|id| l.entity_exists(id)) else {
                self.delay(random, events);
                return;
            };
            let at = match read_pos(data.entity.get(&JavaString::from("Pos"))) {
                Some(p) => p,
                None => {
                    let range = f64::from(self.spawn_range);
                    let x = f64::from(pos[0])
                        + (random.next_double() - random.next_double()) * range
                        + 0.5;
                    let y = f64::from(pos[1] + random.next_int_bound(3).unwrap_or(0) - 1);
                    let z = f64::from(pos[2])
                        + (random.next_double() - random.next_double()) * range
                        + 0.5;
                    [x, y, z]
                }
            };
            let kind = l.mob_type(&id);
            let (species, width, height, category) = match kind {
                Some((s, w, h, c)) => (Some(s), w, h, c),
                // Entity types Horizon cannot create: a unit box, misc.
                None => (None, 1.0, 1.0, Category::Misc),
            };
            let spawn_box = match species {
                Some(s) => spawn_selection::spawn_aabb(s, at[0], at[1], at[2], width, height),
                None => spawn_selection::spawn_box(at[0], at[1], at[2], width, height),
            };
            if !l.no_collision(spawn_box) {
                continue;
            }
            let block = [
                at[0].floor() as i32,
                at[1].floor() as i32,
                at[2].floor() as i32,
            ];
            if let Some(rules) = data.custom_spawn_rules {
                if !category.friendly() && l.peaceful() {
                    continue;
                }
                if !SpawnData::valid_position(rules, l, block[0], block[1], block[2]) {
                    continue;
                }
            } else if let Some(s) = species
                && !spawn_rules::check_spawn_rules(
                    s,
                    l,
                    Reason::Spawner,
                    random,
                    block[0],
                    block[1],
                    block[2],
                )
            {
                continue;
            }
            // loadEntityRecursive: a type the game cannot create delays.
            let Some(s) = species else {
                self.delay(random, events);
                return;
            };
            let around = Aabb::new(
                Vec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])),
                Vec3::new(
                    f64::from(pos[0] + 1),
                    f64::from(pos[1] + 1),
                    f64::from(pos[2] + 1),
                ),
            )
            .inflate(Vec3::new(
                f64::from(self.spawn_range),
                f64::from(self.spawn_range),
                f64::from(self.spawn_range),
            ));
            if l.count_entities(&id, around) as i64 >= i64::from(self.max_nearby_entities) {
                self.delay(random, events);
                return;
            }
            let yaw = random.next_float() * 360.0;
            // The entity's own position (Entity.setPos, which a shulker snaps
            // to its block) for its box and its block.
            let placed = entity_position(s, at);
            let own = [
                placed[0].floor() as i32,
                placed[1].floor() as i32,
                placed[2].floor() as i32,
            ];
            let scale = cube_scale(&id, &data.entity);
            let bounds = spawn_selection::spawn_box(
                placed[0],
                placed[1],
                placed[2],
                width * scale,
                height * scale,
            );
            let rules_ok = data.custom_spawn_rules.is_some()
                || spawn_selection::mob_spawn_rules(s, l, own[0], own[1], own[2]);
            if !rules_ok
                || !spawn_selection::spawn_obstruction(
                    s,
                    l,
                    bounds,
                    Some(&added),
                    own[0],
                    own[1],
                    own[2],
                )
            {
                continue;
            }
            let request = SpawnerSpawn {
                entity: &data.entity,
                id: &id,
                position: placed,
                yaw,
                finalize: data.unconfigured(),
                equipment: data.equipment.as_ref(),
                trial: false,
            };
            let Some(result) = spawn(l, &request) else {
                self.delay(random, events);
                return;
            };
            added.push(result.bounds);
            events.push(SpawnerEvent::LevelEvent(2004));
            events.push(SpawnerEvent::EntityPlace(result.id, block));
            if result.mob {
                events.push(SpawnerEvent::SpawnAnim(result.id));
            }
            delay = true;
        }
        if delay {
            self.delay(random, events);
        }
    }
    fn client_tick(
        &mut self,
        near_player: bool,
        has_display: bool,
        random: &mut impl RandomSource,
        pos: [i32; 3],
    ) -> Option<[f64; 3]> {
        if !near_player {
            self.o_spin = self.spin;
            return None;
        }
        if !has_display {
            return None;
        }
        let p = [
            f64::from(pos[0]) + random.next_double(),
            f64::from(pos[1]) + random.next_double(),
            f64::from(pos[2]) + random.next_double(),
        ];
        if self.client_delay > 0 {
            self.client_delay -= 1;
        }
        self.o_spin = self.spin;
        self.spin =
            (self.spin + f64::from(1000.0_f32 / (self.client_delay as f32 + 200.0))) % 360.0;
        Some(p)
    }
    fn client_event(&mut self, id: i32) -> bool {
        if id == 1 {
            self.client_delay = self.min_spawn_delay;
            true
        } else {
            false
        }
    }
    fn client_loaded(&mut self) {
        self.client_delay = self.spawn_delay;
    }
    fn display_pose(&self, alpha: f32, width: f32, height: f32) -> (f32, f32) {
        display_pose(self.o_spin, self.spin, alpha, width, height)
    }
}

/// `TrialSpawnerRenderer.extractSpawnerData`: the spin in degrees and the
/// scale of a display mob of the given size, between the last two spins.
pub fn display_pose(o_spin: f64, spin: f64, alpha: f32, width: f32, height: f32) -> (f32, f32) {
    let spin = (o_spin + f64::from(alpha) * (spin - o_spin)) as f32 * 10.0;
    let longest = width.max(height);
    let scale = if longest > 1.0 {
        0.53125 / longest
    } else {
        0.53125
    };
    (spin, scale)
}

/// `WeightedList.getRandom`: one draw over the total weight, none when the
/// list weighs nothing.
pub fn weighted_index(
    weights: impl Iterator<Item = i32> + Clone,
    random: &mut impl RandomSource,
) -> Option<usize> {
    let total: i64 = weights.clone().map(i64::from).sum();
    if total == 0 {
        return None;
    }
    let mut pick = i64::from(
        random
            .next_int_bound(total.min(i64::from(i32::MAX)) as i32)
            .unwrap_or(0),
    );
    for (i, w) in weights.enumerate() {
        pick -= i64::from(w);
        if pick < 0 {
            return Some(i);
        }
    }
    None
}

/// `Entity.setPos` of a fresh mob: a shulker sits at the centre of its block.
pub fn entity_position(s: Species, at: [f64; 3]) -> [f64; 3] {
    if s == Species::Shulker {
        [
            at[0].floor() + 0.5,
            (at[1] + 0.5).floor(),
            at[2].floor() + 0.5,
        ]
    } else {
        at
    }
}

/// `Vec3.CODEC`: a list of three doubles.
pub(crate) fn read_pos(tag: Option<&Tag>) -> Option<[f64; 3]> {
    let Tag::List(v) = tag? else {
        return None;
    };
    if v.len() != 3 {
        return None;
    }
    let mut out = [0.0; 3];
    for (o, t) in out.iter_mut().zip(v) {
        *o = match t {
            Tag::Double(b) => f64::from_bits(*b),
            Tag::Float(b) => f64::from(f32::from_bits(*b)),
            _ => return None,
        };
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::random::LegacyRandom;

    fn tag(snbt: &str) -> Tag {
        horizon_world::nbt::parse_snbt(snbt).expect("snbt").0
    }

    #[test]
    fn load_without_potentials_shares_the_next_data() {
        let mut s = Spawner::load(&tag("{SpawnData:{entity:{id:\"zombie\"}}}"));
        assert_eq!(s.potentials.len(), 1);
        assert_eq!(
            s.next_data().and_then(SpawnData::id).as_deref(),
            Some("minecraft:zombie")
        );
        let mut events = Vec::new();
        s.set_entity_id("minecraft:skeleton", &mut LegacyRandom::new(0), &mut events);
        assert!(events.is_empty());
        assert_eq!(
            s.potentials[0].0.id().as_deref(),
            Some("minecraft:skeleton")
        );
    }

    #[test]
    fn entity_data_sets_the_id_and_merges_into_the_next_entity() {
        let mut s = Spawner::load(&tag(
            "{SpawnData:{entity:{id:\"zombie\",Health:5.0f,equipment:{head:{id:\"minecraft:iron_helmet\"}}}}}",
        ));
        let Tag::Compound(data) = tag(
            "{id:\"minecraft:skeleton\",CustomName:\"Guard\",equipment:{mainhand:{id:\"minecraft:bow\"}}}",
        ) else {
            panic!("compound");
        };
        let mut events = Vec::new();
        s.set_entity_data(
            "minecraft:skeleton",
            &data,
            &mut LegacyRandom::new(0),
            &mut events,
        );
        let entity = &s.next_data().unwrap().entity;
        let get = |k: &str| entity.get(&JavaString::from(k)).cloned();
        // CompoundTag.merge: the egg's keys over the old ones, nested
        // compounds merged, the rest kept.
        assert_eq!(get("id"), Some(Tag::String("minecraft:skeleton".into())));
        assert_eq!(get("CustomName"), Some(Tag::String("Guard".into())));
        assert_eq!(get("Health"), Some(tag("5.0f")));
        let Some(Tag::Compound(equipment)) = get("equipment") else {
            panic!("equipment");
        };
        assert!(equipment.contains_key(&JavaString::from("head")));
        assert!(equipment.contains_key(&JavaString::from("mainhand")));
    }

    #[test]
    fn generated_spawner_keeps_no_potentials() {
        let s = Spawner::load(&tag(
            "{SpawnData:{entity:{id:\"minecraft:cave_spider\"}},SpawnPotentials:[]}",
        ));
        assert!(s.potentials.is_empty());
        let saved = Tag::compound(s.save());
        assert_eq!(saved.get("SpawnPotentials"), Some(&Tag::List(Vec::new())));
        assert_eq!(saved.get("Delay"), Some(&Tag::Short(20)));
    }

    #[test]
    fn invalid_ids_are_removed() {
        let d = SpawnData::new(
            match tag("{id:\"Zombie\"}") {
                Tag::Compound(c) => c,
                _ => unreachable!(),
            },
            None,
            None,
        );
        assert!(d.id().is_none());
        assert_eq!(normalize_id(":pig").as_deref(), Some("minecraft:pig"));
    }

    #[test]
    fn light_limits_decode_and_encode() {
        assert_eq!(light_limit(Some(&tag("7"))), Some((7, 7)));
        assert_eq!(light_limit(Some(&tag("[I;0,7]"))), Some((0, 7)));
        assert_eq!(
            light_limit(Some(&tag("{min_inclusive:2,max_inclusive:9}"))),
            Some((2, 9))
        );
        assert_eq!(light_limit(Some(&tag("[I;9,2]"))), None);
        assert_eq!(encode_range((3, 3)), Tag::Int(3));
    }

    #[test]
    fn spawner_defaults_empty() {
        let s = Spawner::default();
        assert_eq!(s.next_data(), None);
        assert!(s.potentials.is_empty());
    }
}
