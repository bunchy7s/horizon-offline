//! Structure templates, processors and placement shared by template features
//! and structure pieces.
pub mod end_city;
pub mod enhanced_terrain;
pub mod features;
pub mod formations;
pub mod fortress;
pub mod jigsaw;
pub mod legacy;
pub mod mansion;
pub mod mineshaft;
pub mod monument;
pub mod nether_fossil;
pub mod ocean_ruin;
pub mod piece_limited;
pub mod processor;
pub mod ruined_portal;
pub mod simple;
pub mod start;
pub mod start_checks;
pub mod stronghold;
pub mod template;
pub mod temple;

use crate::{
    nbt::{self, Limits},
    registry::{BlockRegistry, BlockStateId},
};
use horizon_core::geometry::Direction;
use processor::ProcessorList;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};
pub use template::{BlockInfo, EntityInfo, JigsawInfo, StructureTemplate, save_nbt};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Rotation {
    None,
    Clockwise90,
    Clockwise180,
    Counterclockwise90,
}
impl Rotation {
    pub const ALL: [Self; 4] = [
        Self::None,
        Self::Clockwise90,
        Self::Clockwise180,
        Self::Counterclockwise90,
    ];
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "none" => Self::None,
            "clockwise_90" => Self::Clockwise90,
            "180" => Self::Clockwise180,
            "counterclockwise_90" => Self::Counterclockwise90,
            _ => return None,
        })
    }
    /// Rotation.getRandom: one bounded draw over the four values.
    pub fn random(random: &mut impl horizon_core::random::RandomSource) -> Self {
        Self::ALL[random.next_int_bound(4).unwrap_or(0) as usize]
    }
    pub fn rotate_direction(self, direction: Direction) -> Direction {
        if matches!(direction, Direction::Up | Direction::Down) {
            return direction;
        }
        let order = [
            Direction::North,
            Direction::East,
            Direction::South,
            Direction::West,
        ];
        let index = order.iter().position(|d| *d == direction).unwrap();
        order[(index + self as usize) % 4]
    }
    /// `Rotation.getSerializedName`: the value of `Rotation.CODEC`.
    pub fn serialized_name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Clockwise90 => "clockwise_90",
            Self::Clockwise180 => "180",
            Self::Counterclockwise90 => "counterclockwise_90",
        }
    }
    pub fn get_rotated(self, other: Self) -> Self {
        Self::ALL[(self as usize + other as usize) % 4]
    }
    /// `Rotation.name()`, as structure pieces save it.
    pub fn saved_name(self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::Clockwise90 => "CLOCKWISE_90",
            Self::Clockwise180 => "CLOCKWISE_180",
            Self::Counterclockwise90 => "COUNTERCLOCKWISE_90",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mirror {
    None,
    LeftRight,
    FrontBack,
}
impl Mirror {
    /// `Mirror.name()`, as structure pieces save it.
    pub fn saved_name(self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::LeftRight => "LEFT_RIGHT",
            Self::FrontBack => "FRONT_BACK",
        }
    }
}

/// Inclusive integer box (vanilla BoundingBox).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BoundingBox {
    pub min: [i32; 3],
    pub max: [i32; 3],
}
impl BoundingBox {
    pub fn new(min: [i32; 3], max: [i32; 3]) -> Self {
        Self { min, max }
    }
    pub fn from_corners(a: [i32; 3], b: [i32; 3]) -> Self {
        Self {
            min: std::array::from_fn(|i| a[i].min(b[i])),
            max: std::array::from_fn(|i| a[i].max(b[i])),
        }
    }
    pub fn moved(self, d: [i32; 3]) -> Self {
        Self {
            min: std::array::from_fn(|i| self.min[i] + d[i]),
            max: std::array::from_fn(|i| self.max[i] + d[i]),
        }
    }
    pub fn is_inside(&self, p: [i32; 3]) -> bool {
        (0..3).all(|i| p[i] >= self.min[i] && p[i] <= self.max[i])
    }
    pub fn intersects(&self, other: &Self) -> bool {
        (0..3).all(|i| self.max[i] >= other.min[i] && self.min[i] <= other.max[i])
    }
    pub fn encapsulate(&mut self, other: &Self) {
        for i in 0..3 {
            self.min[i] = self.min[i].min(other.min[i]);
            self.max[i] = self.max[i].max(other.max[i]);
        }
    }
    pub fn inflated(self, xz: i32, y: i32) -> Self {
        Self {
            min: [self.min[0] - xz, self.min[1] - y, self.min[2] - xz],
            max: [self.max[0] + xz, self.max[1] + y, self.max[2] + xz],
        }
    }
    pub fn span(&self) -> [i32; 3] {
        std::array::from_fn(|i| self.max[i] - self.min[i] + 1)
    }
    pub fn center(&self) -> [i32; 3] {
        std::array::from_fn(|i| self.min[i] + (self.max[i] - self.min[i] + 1) / 2)
    }
    pub fn corners(&self) -> [[i32; 3]; 8] {
        let (a, b) = (self.min, self.max);
        [
            [a[0], a[1], a[2]],
            [a[0], a[1], b[2]],
            [a[0], b[1], a[2]],
            [a[0], b[1], b[2]],
            [b[0], a[1], a[2]],
            [b[0], a[1], b[2]],
            [b[0], b[1], a[2]],
            [b[0], b[1], b[2]],
        ]
    }
}

/// Per-state BlockState.rotate/mirror results and template ordering flags,
/// exported from vanilla (`blocks/structure-blocks.json`).
#[derive(Debug, Default)]
pub struct StateTransforms {
    rotate: Vec<[u32; 3]>,
    mirror: Vec<[u32; 2]>,
    full: Vec<bool>,
    loot: BTreeSet<BlockStateId>,
    /// Block entity type of every state that has one (`BlockState.hasBlockEntity`).
    entity_types: HashMap<BlockStateId, Arc<str>>,
}
impl StateTransforms {
    pub fn from_json(json: &str, states: usize) -> Result<Self, super::DecodeError> {
        let value: serde_json::Value = serde_json::from_str(json)?;
        let arr = |key: &'static str| {
            value[key]
                .as_array()
                .cloned()
                .ok_or(super::DecodeError::Field(key))
        };
        let ids = |v: &serde_json::Value| -> Vec<u32> {
            v.as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_u64())
                        .map(|x| x as u32)
                        .collect()
                })
                .unwrap_or_default()
        };
        let rotate = arr("rotate")?
            .iter()
            .map(|v| {
                let v = ids(v);
                [v[0], v[1], v[2]]
            })
            .collect::<Vec<_>>();
        let mirror = arr("mirror")?
            .iter()
            .map(|v| {
                let v = ids(v);
                [v[0], v[1]]
            })
            .collect::<Vec<_>>();
        let full = arr("full_block")?
            .iter()
            .map(|v| v.as_bool().unwrap_or(false))
            .collect::<Vec<_>>();
        if rotate.len() != states || mirror.len() != states || full.len() != states {
            return Err(super::DecodeError::Field("structure block table size"));
        }
        let loot = ids(&value["loot_containers"])
            .into_iter()
            .map(BlockStateId)
            .collect();
        let mut entity_types = HashMap::new();
        if let Some(types) = value["block_entities"].as_object() {
            for (name, states) in types {
                let name: Arc<str> = Arc::from(name.as_str());
                for id in ids(states) {
                    entity_types.insert(BlockStateId(id), name.clone());
                }
            }
        }
        Ok(Self {
            rotate,
            mirror,
            full,
            loot,
            entity_types,
        })
    }
    /// The block entity type a state carries, when it has one.
    pub fn entity_type(&self, state: BlockStateId) -> Option<&Arc<str>> {
        self.entity_types.get(&state)
    }
    pub fn rotate(&self, state: BlockStateId, rotation: Rotation) -> BlockStateId {
        match rotation {
            Rotation::None => state,
            r => self
                .rotate
                .get(state.0 as usize)
                .map_or(state, |t| BlockStateId(t[r as usize - 1])),
        }
    }
    pub fn mirror(&self, state: BlockStateId, mirror: Mirror) -> BlockStateId {
        match mirror {
            Mirror::None => state,
            m => self
                .mirror
                .get(state.0 as usize)
                .map_or(state, |t| BlockStateId(t[m as usize - 1])),
        }
    }
    pub fn full_block(&self, state: BlockStateId) -> bool {
        self.full.get(state.0 as usize).copied().unwrap_or(false)
    }
    pub fn loot_container(&self, state: BlockStateId) -> bool {
        self.loot.contains(&state)
    }
}

/// Lazily loaded templates (`structure/<namespace>/<path>.nbt`, gzip NBT) and
/// processor lists, shared by all generators of one resource bundle.
pub struct StructureResources {
    pub registry: &'static BlockRegistry,
    pub transforms: StateTransforms,
    dir: PathBuf,
    templates: RwLock<HashMap<String, Option<Arc<StructureTemplate>>>>,
    pub processor_lists: BTreeMap<String, String>,
    jigsaw_states: BTreeMap<BlockStateId, bool>,
    pub(crate) waterlogged: HashMap<BlockStateId, BlockStateId>,
    sources: BTreeSet<BlockStateId>,
    /// Block names in each block tag, for processors matching tags by block.
    pub block_tags: BTreeMap<String, BTreeSet<String>>,
}
impl std::fmt::Debug for StructureResources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StructureResources")
    }
}
impl StructureResources {
    pub fn new(
        registry: &'static BlockRegistry,
        transforms: StateTransforms,
        dir: &Path,
        processor_lists: BTreeMap<String, String>,
        block_tags: &BTreeMap<String, Vec<String>>,
    ) -> Self {
        let mut sources = BTreeSet::new();
        let fluids = crate::fluid::FluidStateTable::new(registry).ok();
        let mut jigsaw_states = BTreeMap::new();
        let mut waterlogged = HashMap::new();
        for id in 0..registry.len() {
            let id = BlockStateId(id as u32);
            let state = registry.state(id).unwrap();
            if state.name == "minecraft:jigsaw" {
                let orientation = state
                    .properties
                    .get("orientation")
                    .cloned()
                    .unwrap_or_default();
                let front = orientation.split('_').next().unwrap_or("");
                jigsaw_states.insert(id, !matches!(front, "up" | "down"));
            }
            if fluids
                .as_ref()
                .and_then(|table| table.get(id))
                .is_some_and(|fluid| fluid.is_source())
            {
                sources.insert(id);
            }
            // SlabBlock.placeLiquid refuses double slabs.
            let double_slab = state.name.ends_with("_slab")
                && state.properties.get("type").map(String::as_str) == Some("double");
            if !double_slab
                && state.properties.get("waterlogged").map(String::as_str) == Some("false")
            {
                let mut wet = state.clone();
                wet.properties.insert("waterlogged".into(), "true".into());
                if let Some(wet) = registry.id(&wet) {
                    waterlogged.insert(id, wet);
                }
            }
        }
        Self {
            registry,
            transforms,
            dir: dir.to_owned(),
            templates: RwLock::new(HashMap::new()),
            processor_lists,
            jigsaw_states,
            waterlogged,
            sources,
            block_tags: block_tags
                .iter()
                .map(|(k, v)| (k.clone(), v.iter().cloned().collect()))
                .collect(),
        }
    }
    /// StructureTemplateManager.getOrCreate; missing templates are empty.
    pub fn template(&self, id: &str) -> Option<Arc<StructureTemplate>> {
        let id = if id.contains(':') {
            id.to_owned()
        } else {
            format!("minecraft:{id}")
        };
        if let Some(found) = self.templates.read().unwrap().get(&id) {
            return found.clone();
        }
        let (namespace, path) = id.split_once(':').unwrap();
        let file = self.dir.join(namespace).join(format!("{path}.nbt"));
        let loaded = std::fs::read(&file).ok().and_then(|bytes| {
            let mut raw = Vec::new();
            flate2::read::GzDecoder::new(bytes.as_slice())
                .read_to_end(&mut raw)
                .ok()?;
            let tag = nbt::read_named(&raw, Limits::templates()).ok()?.0.tag;
            Some(StructureTemplate::from_nbt(
                &tag,
                self.registry,
                &self.transforms,
            ))
        });
        // StructureTemplateManager.getOrCreate: missing templates are empty.
        let mut loaded = loaded.unwrap_or_else(StructureTemplate::empty);
        loaded.location = id.as_str().into();
        let loaded = Some(Arc::new(loaded));
        self.templates.write().unwrap().insert(id, loaded.clone());
        loaded
    }
    /// Whether the state's fluid is a source (water/lava level 0, waterlogged).
    pub fn source_fluid(&self, state: BlockStateId) -> bool {
        self.sources.contains(&state)
    }
    pub fn jigsaw_horizontal(&self, state: BlockStateId) -> Option<bool> {
        self.jigsaw_states.get(&state).copied()
    }
    pub fn processor_list(
        &self,
        json: &str,
        bindings: &super::block_feature::FeatureBlockBindings<'_>,
    ) -> Result<ProcessorList, super::DecodeError> {
        ProcessorList::decode(json, self, bindings)
    }
    pub fn is_jigsaw(&self, state: BlockStateId) -> bool {
        self.jigsaw_states.contains_key(&state)
    }
}
