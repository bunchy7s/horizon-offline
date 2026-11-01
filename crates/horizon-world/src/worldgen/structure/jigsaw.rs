//! Template pools, pool elements and JigsawPlacement.
use super::{
    BoundingBox, Mirror, Rotation, StructureResources, StructureTemplate,
    processor::{PlaceSettings, Processor, ProcessorList, place_in_world},
};
use crate::{
    heightmap::HeightmapType,
    nbt::{Compound, Tag},
    worldgen::{
        DecodeError,
        block_feature::FeatureBlockBindings,
        feature::FeatureWorld,
        placement::{PlacementContext, PlacementQueryError},
    },
};
use horizon_core::{
    geometry::Direction,
    position::BlockPos,
    random::{LegacyRandom, RandomSource, position_seed},
};
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, VecDeque},
    rc::Rc,
    sync::{Arc, OnceLock, RwLock},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Projection {
    Rigid,
    TerrainMatching,
}
impl Projection {
    /// `StructureTemplatePool.Projection.getSerializedName`.
    pub fn saved_name(self) -> &'static str {
        match self {
            Self::Rigid => "rigid",
            Self::TerrainMatching => "terrain_matching",
        }
    }
}
#[derive(Clone, Debug)]
pub enum PoolElement {
    Empty,
    Single {
        template: Arc<StructureTemplate>,
        processors: ProcessorList,
        /// The element's `processors` as written: a list id or an inline list,
        /// saved back as it was.
        processors_source: Value,
        legacy: bool,
        projection: Projection,
        waterlogging: Option<bool>,
        /// The piece's own terrain kernel in a structure that has one
        /// (`"horizon:enhanced_terrain_adaptation"`).
        enhanced_terrain: Option<Arc<super::enhanced_terrain::Adaptation>>,
        /// getGroundLevelDelta: 1, or Formations' `ground_level`
        /// (`"horizon:ground_level"`).
        ground_level: i32,
    },
    Feature {
        feature: String,
        projection: Projection,
    },
    List {
        elements: Vec<PoolElement>,
        projection: Projection,
    },
}
#[derive(Clone, Debug)]
pub struct Jigsaw {
    pub pos: [i32; 3],
    pub front: Direction,
    pub top: Direction,
    pub rollable: bool,
    pub name: Option<String>,
    pub pool: String,
    pub target: String,
    pub placement_priority: i32,
    pub selection_priority: i32,
}
fn direction(name: &str) -> Direction {
    match name {
        "down" => Direction::Down,
        "up" => Direction::Up,
        "north" => Direction::North,
        "south" => Direction::South,
        "west" => Direction::West,
        _ => Direction::East,
    }
}
fn opposite(d: Direction) -> Direction {
    match d {
        Direction::Down => Direction::Up,
        Direction::Up => Direction::Down,
        Direction::North => Direction::South,
        Direction::South => Direction::North,
        Direction::West => Direction::East,
        Direction::East => Direction::West,
    }
}
pub(super) fn step(d: Direction) -> [i32; 3] {
    let n = d.normal();
    [n[0], n[1], n[2]]
}
/// Util.shuffle.
pub(super) fn shuffle<T>(list: &mut [T], random: &mut impl RandomSource) {
    for i in (2..=list.len()).rev() {
        let swap = random.next_int_bound(i as i32).unwrap_or(0) as usize;
        list.swap(i - 1, swap);
    }
}
fn projection(value: &Value) -> Result<Projection, DecodeError> {
    match value["projection"].as_str() {
        Some("rigid") => Ok(Projection::Rigid),
        Some("terrain_matching") => Ok(Projection::TerrainMatching),
        _ => Err(DecodeError::Field("pool element projection")),
    }
}
impl PoolElement {
    fn decode(
        value: &Value,
        resources: &StructureResources,
        bindings: &FeatureBlockBindings<'_>,
        inherited: Option<Projection>,
    ) -> Result<Self, DecodeError> {
        let kind = value["element_type"].as_str().unwrap_or_default();
        let projection = || match inherited {
            Some(p) => Ok(p),
            None => projection(value),
        };
        Ok(match kind {
            "minecraft:empty_pool_element" => Self::Empty,
            // Lithostitched's elements place what their `delegate` places;
            // their limits are the pool entry's (`Pool::limits`).
            "lithostitched:limited" | "lithostitched:guaranteed" | "lithostitched:delegating" => {
                Self::decode(&value["delegate"], resources, bindings, inherited)?
            }
            // YUNG's API's element places what a single element places; its
            // own fields are the pool entry's rules (`Pool::rules`).
            "minecraft:single_pool_element"
            | "minecraft:legacy_single_pool_element"
            | "yungsapi:yung_single_element" => {
                let location = value["location"]
                    .as_str()
                    .ok_or(DecodeError::Field("location"))?;
                Self::Single {
                    template: resources.template(location).ok_or_else(|| {
                        DecodeError::Registry(format!("missing template {location}"))
                    })?,
                    processors: ProcessorList::decode(
                        &value["processors"].to_string(),
                        resources,
                        bindings,
                    )?,
                    processors_source: value["processors"].clone(),
                    legacy: kind == "minecraft:legacy_single_pool_element",
                    projection: projection()?,
                    waterlogging: match value
                        .get("override_liquid_settings")
                        .and_then(Value::as_str)
                    {
                        Some("apply_waterlogging") => Some(true),
                        Some("ignore_waterlogging") => Some(false),
                        _ => None,
                    },
                    enhanced_terrain: value
                        .get("horizon:enhanced_terrain_adaptation")
                        .map(|a| super::enhanced_terrain::Adaptation::decode(a).map(Arc::new))
                        .transpose()?,
                    ground_level: match value.get("horizon:ground_level") {
                        None => 1,
                        Some(v) => v
                            .as_u64()
                            .and_then(|g| i32::try_from(g).ok())
                            .ok_or(DecodeError::Field("horizon:ground_level"))?,
                    },
                }
            }
            "minecraft:feature_pool_element" => Self::Feature {
                feature: value["feature"].as_str().unwrap_or_default().to_owned(),
                projection: projection()?,
            },
            "minecraft:list_pool_element" => {
                let p = projection()?;
                Self::List {
                    elements: value["elements"]
                        .as_array()
                        .ok_or(DecodeError::Field("list elements"))?
                        .iter()
                        .map(|e| Self::decode(e, resources, bindings, Some(p)))
                        .collect::<Result<_, _>>()?,
                    projection: p,
                }
            }
            _ => return Err(DecodeError::Field("pool element type")),
        })
    }
    /// `StructurePoolElement.CODEC`'s encoding, as a jigsaw piece saves it.
    pub fn save(&self) -> Tag {
        let text = |s: &str| Tag::String(s.into());
        match self {
            Self::Empty => Tag::compound([("element_type", text("minecraft:empty_pool_element"))]),
            Self::Single {
                template,
                processors_source,
                legacy,
                projection,
                waterlogging,
                ..
            } => {
                let kind = if *legacy {
                    "minecraft:legacy_single_pool_element"
                } else {
                    "minecraft:single_pool_element"
                };
                let mut entries = vec![
                    ("element_type", text(kind)),
                    ("location", text(&template.location)),
                    (
                        "processors",
                        super::processor::json_to_nbt(processors_source),
                    ),
                    ("projection", text(projection.saved_name())),
                ];
                if let Some(apply) = waterlogging {
                    let settings = if *apply {
                        "apply_waterlogging"
                    } else {
                        "ignore_waterlogging"
                    };
                    entries.push(("override_liquid_settings", text(settings)));
                }
                Tag::compound(entries)
            }
            Self::Feature {
                feature,
                projection,
            } => Tag::compound([
                ("element_type", text("minecraft:feature_pool_element")),
                ("feature", text(feature)),
                ("projection", text(projection.saved_name())),
            ]),
            Self::List {
                elements,
                projection,
            } => Tag::compound([
                ("element_type", text("minecraft:list_pool_element")),
                (
                    "elements",
                    Tag::List(elements.iter().map(Self::save).collect()),
                ),
                ("projection", text(projection.saved_name())),
            ]),
        }
    }
    pub fn projection(&self) -> Projection {
        match self {
            Self::Empty => Projection::TerrainMatching,
            Self::Single { projection, .. }
            | Self::Feature { projection, .. }
            | Self::List { projection, .. } => *projection,
        }
    }
    pub fn size(&self, rotation: Rotation) -> [i32; 3] {
        match self {
            Self::Empty | Self::Feature { .. } => [0; 3],
            Self::Single { template, .. } => template.size(rotation),
            Self::List { elements, .. } => elements.iter().fold([0; 3], |acc, e| {
                let s = e.size(rotation);
                std::array::from_fn(|i| acc[i].max(s[i]))
            }),
        }
    }
    pub fn bounding_box(&self, position: [i32; 3], rotation: Rotation) -> BoundingBox {
        match self {
            Self::Empty => BoundingBox::new(position, position),
            Self::Feature { .. } => BoundingBox::new(position, position),
            Self::Single { template, .. } => {
                template.bounding_box(position, rotation, [0; 3], Mirror::None)
            }
            Self::List { elements, .. } => {
                let mut boxes = elements
                    .iter()
                    .filter(|e| !matches!(e, Self::Empty))
                    .map(|e| e.bounding_box(position, rotation));
                let mut first = boxes.next().unwrap_or(BoundingBox::new(position, position));
                for b in boxes {
                    first.encapsulate(&b);
                }
                first
            }
        }
    }
    /// getShuffledJigsawBlocks.
    pub fn shuffled_jigsaws(
        &self,
        resources: &StructureResources,
        position: [i32; 3],
        rotation: Rotation,
        random: &mut impl RandomSource,
    ) -> Vec<Jigsaw> {
        match self {
            Self::Empty => Vec::new(),
            Self::Feature { .. } => vec![Jigsaw {
                pos: position,
                front: Direction::Down,
                top: Direction::South,
                rollable: true,
                name: None,
                pool: "minecraft:empty".into(),
                target: "minecraft:empty".into(),
                placement_priority: 0,
                selection_priority: 0,
            }],
            Self::List { elements, .. } => elements
                .first()
                .map(|e| e.shuffled_jigsaws(resources, position, rotation, random))
                .unwrap_or_default(),
            Self::Single { template, .. } => {
                let mut jigsaws = template_jigsaws(resources, template, position, rotation);
                shuffle(&mut jigsaws, random);
                jigsaws.sort_by_key(|j| std::cmp::Reverse(j.selection_priority));
                jigsaws
            }
        }
    }
    pub fn ground_level_delta(&self) -> i32 {
        match self {
            Self::Single { ground_level, .. } => *ground_level,
            _ => 1,
        }
    }
    /// The piece's own terrain kernel, in place of its structure's.
    pub fn enhanced_terrain(&self) -> Option<&super::enhanced_terrain::Adaptation> {
        match self {
            Self::Single {
                enhanced_terrain, ..
            } => enhanced_terrain.as_deref(),
            _ => None,
        }
    }
}
/// StructureTemplate.getJigsaws: palette chosen by a position-seeded random.
fn template_jigsaws(
    resources: &StructureResources,
    template: &StructureTemplate,
    position: [i32; 3],
    rotation: Rotation,
) -> Vec<Jigsaw> {
    if template.palettes().is_empty() {
        return Vec::new();
    }
    let palette = LegacyRandom::new(position_seed(BlockPos::new(
        position[0],
        position[1],
        position[2],
    )))
    .next_int_bound(template.palettes().len() as i32)
    .unwrap_or(0) as usize;
    template
        .jigsaws(palette, &|state| resources.jigsaw_horizontal(state))
        .iter()
        .map(|j| {
            let relative = StructureTemplate::transform(j.info.pos, Mirror::None, rotation, [0; 3]);
            let state = resources.transforms.rotate(j.info.state, rotation);
            let orientation = resources
                .registry
                .state(state)
                .and_then(|s| s.properties.get("orientation").cloned())
                .unwrap_or_default();
            let (front, top) = orientation.split_once('_').unwrap_or(("north", "up"));
            Jigsaw {
                pos: std::array::from_fn(|i| relative[i] + position[i]),
                front: direction(front),
                top: direction(top),
                rollable: j.rollable,
                name: Some(j.name.clone()),
                pool: j.pool.clone(),
                target: j.target.clone(),
                placement_priority: j.placement_priority,
                selection_priority: j.selection_priority,
            }
        })
        .collect()
}
pub(super) fn can_attach(source: &Jigsaw, target: &Jigsaw) -> bool {
    source.front == opposite(target.front)
        && (source.rollable || source.top == target.top)
        && target
            .name
            .as_ref()
            .is_none_or(|name| *name == source.target)
}

#[derive(Debug)]
pub struct Pool {
    /// Each element repeated by its weight.
    pub(super) templates: Vec<Arc<PoolElement>>,
    /// The entry of `raw` each of `templates` repeats.
    entries: Vec<usize>,
    /// The elements with their weights, in file order (`rawTemplates`).
    pub(super) raw: Vec<(Arc<PoolElement>, i32)>,
    /// Per entry of `raw`: a YUNG element's rules.
    rules: Vec<Rules>,
    /// Per entry of `raw`: a Lithostitched `limited` or `guaranteed`
    /// element's limit.
    limits: Vec<Option<Limit>>,
    /// The entries of `raw` that are Lithostitched's `guaranteed` elements.
    guaranteed: Vec<usize>,
    pub(super) fallback: String,
    max_size: OnceLock<i32>,
}
impl Pool {
    pub(super) fn random_template(&self, random: &mut impl RandomSource) -> Arc<PoolElement> {
        if self.templates.is_empty() {
            return Arc::new(PoolElement::Empty);
        }
        self.templates[random
            .next_int_bound(self.templates.len() as i32)
            .unwrap_or(0) as usize]
            .clone()
    }
    /// The entries of `templates`, shuffled.
    fn shuffled(&self, random: &mut impl RandomSource) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.templates.len()).collect();
        shuffle(&mut order, random);
        order.into_iter().map(|i| self.entries[i]).collect()
    }
    /// YUNG's API's pick: `nextInt` of the entries' total weight, walked in
    /// `order` (indices into `raw`); the entry that takes it below zero.
    fn weighted(&self, order: &[usize], random: &mut impl RandomSource) -> Option<usize> {
        let total: i32 = order.iter().map(|&i| self.raw[i].1).sum();
        let mut pick = random.next_int_bound(total).ok()?;
        order.iter().position(|&i| {
            pick -= self.raw[i].1;
            pick < 0
        })
    }
    pub(super) fn max_size(&self) -> i32 {
        *self.max_size.get_or_init(|| {
            self.templates
                .iter()
                .filter(|t| !matches!(t.as_ref(), PoolElement::Empty))
                .map(|t| t.bounding_box([0; 3], Rotation::None).span()[1])
                .max()
                .unwrap_or(0)
        })
    }
}
/// A `yungsapi:yung_single_element`'s own fields (spec-yungsapi §3, §3.2).
#[derive(Clone, Debug, Default)]
struct Rules {
    /// `name` and `max_count`: the most pieces of that name in one start,
    /// the start not counted.
    limit: Option<(Arc<str>, i32)>,
    /// `is_priority`: tried before the pool's other entries, in the
    /// shuffled order and without a weighted draw.
    priority: bool,
    /// `min_required_depth`: the least depth of the parent piece (the start
    /// is depth 0).
    min_depth: i32,
    /// `ignore_bounds`: placed without the free-space test.
    ignore_bounds: bool,
}
impl Rules {
    fn of(element: &Value) -> Self {
        if element["element_type"] != "yungsapi:yung_single_element" {
            return Self::default();
        }
        Self {
            limit: element["name"]
                .as_str()
                .zip(element["max_count"].as_i64())
                .map(|(name, max)| (name.into(), max as i32)),
            priority: element["is_priority"].as_bool().unwrap_or(false),
            min_depth: element["min_required_depth"].as_i64().unwrap_or(0) as i32,
            ignore_bounds: element["ignore_bounds"].as_bool().unwrap_or(false),
        }
    }
}
/// How many pieces of a Lithostitched `limited` or `guaranteed` element one
/// start may hold, and where (spec-lithostitched §3).
#[derive(Clone, Debug)]
struct Limit {
    /// The pieces counted together: the element itself (its JSON), wherever
    /// it is listed.
    key: Arc<str>,
    max: i32,
    /// The parent piece's least depth (the start is at 0).
    min_depth: i32,
    /// `lithostitched:guaranteed`: tried before its pool's other entries.
    guaranteed: bool,
}
impl Limit {
    fn of(element: &Value) -> Option<Self> {
        let int = |key: &str| {
            element[key]
                .as_i64()
                .map(|v| v.clamp(0, i32::MAX.into()) as i32)
        };
        let (max, guaranteed) = match element["element_type"].as_str()? {
            "lithostitched:limited" => (int("limit")?, false),
            "lithostitched:guaranteed" => (int("count").unwrap_or(1), true),
            _ => return None,
        };
        Some(Self {
            key: element.to_string().into(),
            max,
            min_depth: int("min_depth").unwrap_or(0),
            guaranteed,
        })
    }
    /// Whether one more piece may go below a parent at `depth`.
    fn allows(&self, counts: &HashMap<Arc<str>, i32>, depth: i32) -> bool {
        depth >= self.min_depth && counts.get(&self.key).copied().unwrap_or(0) < self.max
    }
}
/// Lazily decoded template pools.
pub struct Pools {
    raw: BTreeMap<String, String>,
    decoded: RwLock<HashMap<String, Option<Arc<Pool>>>>,
}
impl std::fmt::Debug for Pools {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Pools")
    }
}
impl Pools {
    pub fn new(raw: BTreeMap<String, String>) -> Self {
        Self {
            raw,
            decoded: RwLock::new(HashMap::new()),
        }
    }
    pub fn get(
        &self,
        name: &str,
        resources: &StructureResources,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Option<Arc<Pool>> {
        if let Some(found) = self.decoded.read().unwrap().get(name) {
            return found.clone();
        }
        let pool = self.raw.get(name).and_then(|json| {
            let value: Value = serde_json::from_str(json).ok()?;
            let (mut templates, mut entries, mut raw, mut rules, mut limits) =
                (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
            for entry in value["elements"].as_array()? {
                let element =
                    match PoolElement::decode(&entry["element"], resources, bindings, None) {
                        Ok(element) => Arc::new(element),
                        Err(error) => {
                            if std::env::var_os("HORIZON_JIGSAW_DEBUG").is_some() {
                                eprintln!("pool {name}: {error}");
                            }
                            return None;
                        }
                    };
                let weight = entry["weight"].as_i64().unwrap_or(1);
                for _ in 0..weight {
                    templates.push(element.clone());
                    entries.push(raw.len());
                }
                raw.push((element, weight as i32));
                rules.push(Rules::of(&entry["element"]));
                limits.push(Limit::of(&entry["element"]));
            }
            let guaranteed = (limits.iter().enumerate())
                .filter(|(_, l)| l.as_ref().is_some_and(|l| l.guaranteed))
                .map(|(i, _)| i)
                .collect();
            Some(Arc::new(Pool {
                templates,
                entries,
                raw,
                rules,
                limits,
                guaranteed,
                fallback: value["fallback"]
                    .as_str()
                    .unwrap_or("minecraft:empty")
                    .to_owned(),
                max_size: OnceLock::new(),
            }))
        });
        self.decoded
            .write()
            .unwrap()
            .insert(name.to_owned(), pool.clone());
        pool
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Junction {
    pub source_x: i32,
    pub source_ground_y: i32,
    pub source_z: i32,
    /// Saved only (`JigsawJunction.deltaY`, `destProjection`).
    pub delta_y: i32,
    pub dest_projection: Projection,
}
#[derive(Debug)]
pub struct PoolPiece {
    pub element: Arc<PoolElement>,
    pub position: [i32; 3],
    pub ground_level_delta: i32,
    pub rotation: Rotation,
    pub bounding_box: BoundingBox,
    pub junctions: Vec<Junction>,
    pub waterlogging: bool,
}
impl PoolPiece {
    /// A piece as a saved start lists it (PoolElementStructurePiece's tag:
    /// `pool_element`, `PosX`/`PosY`/`PosZ`, `rotation`, `ground_level_delta`,
    /// `BB`, `junctions`, `liquid_settings`), in JSON. Harnesses place another
    /// game's layout with it.
    pub fn from_saved(
        v: &Value,
        resources: &StructureResources,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Result<Self, DecodeError> {
        let int = |v: &Value, key: &'static str| {
            v[key]
                .as_i64()
                .and_then(|i| i32::try_from(i).ok())
                .ok_or(DecodeError::Field(key))
        };
        let corners: Vec<i32> = v["BB"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| i32::try_from(i.as_i64()?).ok())
            .collect();
        let [x0, y0, z0, x1, y1, z1]: [i32; 6] =
            corners.try_into().map_err(|_| DecodeError::Field("BB"))?;
        let rotation = match v["rotation"].as_str() {
            Some("NONE") => Rotation::None,
            Some("CLOCKWISE_90") => Rotation::Clockwise90,
            Some("CLOCKWISE_180") => Rotation::Clockwise180,
            Some("COUNTERCLOCKWISE_90") => Rotation::Counterclockwise90,
            _ => return Err(DecodeError::Field("rotation")),
        };
        let junctions = v["junctions"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|j| {
                Ok(Junction {
                    source_x: int(j, "source_x")?,
                    source_ground_y: int(j, "source_ground_y")?,
                    source_z: int(j, "source_z")?,
                    delta_y: int(j, "delta_y")?,
                    dest_projection: match j["dest_proj"].as_str() {
                        Some("rigid") => Projection::Rigid,
                        Some("terrain_matching") => Projection::TerrainMatching,
                        _ => return Err(DecodeError::Field("dest_proj")),
                    },
                })
            })
            .collect::<Result<_, DecodeError>>()?;
        Ok(Self {
            element: Arc::new(PoolElement::decode(
                &v["pool_element"],
                resources,
                bindings,
                None,
            )?),
            position: [int(v, "PosX")?, int(v, "PosY")?, int(v, "PosZ")?],
            ground_level_delta: int(v, "ground_level_delta")?,
            rotation,
            bounding_box: BoundingBox::new([x0, y0, z0], [x1, y1, z1]),
            junctions,
            waterlogging: v["liquid_settings"].as_str() != Some("ignore_waterlogging"),
        })
    }
    /// `PoolElementStructurePiece`'s tag (always genDepth 0, no orientation).
    pub fn save(&self) -> Compound {
        let mut tag = super::legacy::piece_tag("minecraft:jigsaw", self.bounding_box, None, 0);
        let junctions = self
            .junctions
            .iter()
            .map(|j| {
                Tag::compound([
                    ("source_x", Tag::Int(j.source_x)),
                    ("source_ground_y", Tag::Int(j.source_ground_y)),
                    ("source_z", Tag::Int(j.source_z)),
                    ("delta_y", Tag::Int(j.delta_y)),
                    (
                        "dest_proj",
                        Tag::String(j.dest_projection.saved_name().into()),
                    ),
                ])
            })
            .collect();
        for (key, value) in [
            ("PosX", Tag::Int(self.position[0])),
            ("PosY", Tag::Int(self.position[1])),
            ("PosZ", Tag::Int(self.position[2])),
            ("ground_level_delta", Tag::Int(self.ground_level_delta)),
            ("pool_element", self.element.save()),
            ("rotation", Tag::String(self.rotation.saved_name().into())),
            ("junctions", Tag::List(junctions)),
        ] {
            tag.insert(key.into(), value);
        }
        // LiquidSettings.APPLY_WATERLOGGING, the default, is not written.
        if !self.waterlogging {
            tag.insert(
                "liquid_settings".into(),
                Tag::String("ignore_waterlogging".into()),
            );
        }
        tag
    }
    pub(super) fn moved(&mut self, dy: i32) {
        self.position[1] += dy;
        self.bounding_box = self.bounding_box.moved([0, dy, 0]);
    }
    /// PoolElementStructurePiece.postProcess.
    #[allow(clippy::too_many_arguments)]
    pub fn place<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        resources: &StructureResources,
        random: &mut R,
        chunk_box: BoundingBox,
        reference: [i32; 3],
        keep_jigsaws: bool,
        place_feature: &mut PlaceFeature<'_, W, R>,
    ) -> Result<bool, PlacementQueryError> {
        place_element(
            &self.element,
            world,
            resources,
            random,
            chunk_box,
            self.position,
            reference,
            self.rotation,
            self.waterlogging,
            keep_jigsaws,
            place_feature,
        )
    }
}
static GRAVITY: OnceLock<Processor> = OnceLock::new();
static IGNORE_STRUCTURE_BLOCK: OnceLock<Processor> = OnceLock::new();
static IGNORE_STRUCTURE_AND_AIR: OnceLock<Processor> = OnceLock::new();
#[allow(clippy::too_many_arguments)]
fn place_element<W: FeatureWorld + PlacementContext, R: RandomSource>(
    element: &PoolElement,
    world: &mut W,
    resources: &StructureResources,
    random: &mut R,
    chunk_box: BoundingBox,
    position: [i32; 3],
    reference: [i32; 3],
    rotation: Rotation,
    waterlogging: bool,
    keep_jigsaws: bool,
    place_feature: &mut PlaceFeature<'_, W, R>,
) -> Result<bool, PlacementQueryError> {
    match element {
        PoolElement::Empty => Ok(true),
        PoolElement::Feature { feature, .. } => place_feature(world, random, feature, position),
        PoolElement::List { elements, .. } => {
            for e in elements {
                if !place_element(
                    e,
                    world,
                    resources,
                    random,
                    chunk_box,
                    position,
                    reference,
                    rotation,
                    waterlogging,
                    keep_jigsaws,
                    place_feature,
                )? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        PoolElement::Single {
            template,
            processors,
            legacy,
            projection,
            waterlogging: override_liquid,
            ..
        } => {
            let ignore = if *legacy {
                IGNORE_STRUCTURE_AND_AIR.get_or_init(|| {
                    Processor::BlockIgnore(
                        [
                            "minecraft:structure_block".to_owned(),
                            "minecraft:air".to_owned(),
                        ]
                        .into(),
                    )
                })
            } else {
                IGNORE_STRUCTURE_BLOCK.get_or_init(|| {
                    Processor::BlockIgnore(["minecraft:structure_block".to_owned()].into())
                })
            };
            let mut settings = PlaceSettings::new(rotation);
            settings.known_shape = true;
            // SinglePoolElement.getSettings.
            settings.finalize_entities = true;
            settings.bounding_box = Some(chunk_box);
            settings.waterlogging = override_liquid.unwrap_or(waterlogging);
            if !*legacy {
                settings.processors.push(ignore);
            }
            // SinglePoolElement.getSettings: keepJigsaws skips the replacement.
            if !keep_jigsaws {
                settings.processors.push(&Processor::JigsawReplacement);
            }
            settings.processors.extend(processors.0.iter());
            if *projection == Projection::TerrainMatching {
                settings
                    .processors
                    .push(GRAVITY.get_or_init(|| Processor::Gravity {
                        heightmap: HeightmapType::WorldSurfaceWg,
                        offset: -1,
                    }));
            }
            // LegacySinglePoolElement pops the structure-block ignore and
            // appends STRUCTURE_AND_AIR last, after jigsaw replacement.
            if *legacy {
                settings.processors.push(ignore);
            }
            place_in_world(
                world,
                resources,
                template,
                position,
                reference,
                &mut settings,
                Some(random),
                18,
            )
        }
    }
}

/// Places a named placed feature (feature pool elements) with the piece random.
pub type PlaceFeature<'a, W, R> =
    dyn FnMut(&mut W, &mut R, &str, [i32; 3]) -> Result<bool, PlacementQueryError> + 'a;

/// The world queries jigsaw assembly needs from the chunk generator.
pub trait JigsawContext {
    fn min_y(&self) -> i32;
    fn max_y(&self) -> i32;
    /// ChunkGenerator.getBaseHeight (getFirstFreeHeight).
    fn base_height(&mut self, x: i32, z: i32, kind: HeightmapType) -> i32;
    /// couldStructureExistInColumn over inclusive block Y bounds.
    fn column_has_valid_biome(&mut self, x: i32, z: i32, min_y: i32, max_y: i32) -> bool;
}
/// How a jigsaw structure draws its pieces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Draws {
    /// JigsawPlacement's.
    Vanilla,
    /// YUNG's API's `yungsapi:yung_jigsaw` (spec-yungsapi §2.1, §3.1, §3.2):
    /// pools are drawn by weight from their file entries, without
    /// replacement, priority entries first, and an element at its
    /// `max_count` or too shallow is skipped.
    Yung,
}

/// The elements a jigsaw tries, in order.
enum Candidates {
    /// Vanilla: the target pool's weight-expanded entries, then the
    /// fallback's, each shuffled up front. A pool's Lithostitched
    /// `guaranteed` entries come first when it is reached, in file order,
    /// and are not drawn again (spec-lithostitched §3, OPEN-L4).
    Listed(std::vec::IntoIter<(Arc<Pool>, usize)>),
    /// YUNG's: picks from each pool in turn (the fallback's only once the
    /// target's are spent), each pool's entries shuffled when it is
    /// reached: its priority entries in that order, then weighted picks
    /// from the others.
    Weighted {
        pools: Vec<Arc<Pool>>,
        current: Option<Drawing>,
    },
}
/// A YUNG pool being drawn: entries (indices into `raw`) not tried yet.
struct Drawing {
    pool: Arc<Pool>,
    priority: VecDeque<usize>,
    others: Vec<usize>,
}
/// An element to try.
struct Pick {
    element: Arc<PoolElement>,
    /// Its YUNG name or Lithostitched limit's key, counted when it is
    /// placed.
    name: Option<Arc<str>>,
    ignore_bounds: bool,
}
impl Candidates {
    fn new(
        draws: Draws,
        target: &Arc<Pool>,
        fallback: &Arc<Pool>,
        below_max_depth: bool,
        random: &mut impl RandomSource,
    ) -> Self {
        let pools = if below_max_depth {
            vec![target.clone(), fallback.clone()]
        } else {
            vec![fallback.clone()]
        };
        match draws {
            Draws::Vanilla => {
                let mut listed = Vec::new();
                for pool in &pools {
                    listed.extend(pool.guaranteed.iter().map(|&e| (pool.clone(), e)));
                    let drawn = pool.shuffled(random).into_iter();
                    let drawn = drawn.filter(|&e| !pool.guaranteed.contains(&e));
                    listed.extend(drawn.map(|e| (pool.clone(), e)));
                }
                Self::Listed(listed.into_iter())
            }
            Draws::Yung => Self::Weighted {
                pools: pools.into_iter().rev().collect(),
                current: None,
            },
        }
    }
    /// The next element to try for a jigsaw of a piece at `depth`.
    fn next(
        &mut self,
        counts: &HashMap<Arc<str>, i32>,
        depth: i32,
        random: &mut impl RandomSource,
    ) -> Option<Pick> {
        match self {
            Self::Listed(entries) => entries.find_map(|(pool, entry)| {
                let limit = pool.limits[entry].as_ref();
                limit.is_none_or(|l| l.allows(counts, depth)).then(|| Pick {
                    element: pool.raw[entry].0.clone(),
                    name: limit.map(|l| l.key.clone()),
                    ignore_bounds: false,
                })
            }),
            Self::Weighted { pools, current } => loop {
                let drawing = match current {
                    Some(d) if !d.priority.is_empty() || !d.others.is_empty() => d,
                    _ => {
                        let pool = pools.pop()?;
                        let mut order: Vec<usize> = (0..pool.raw.len()).collect();
                        shuffle(&mut order, random);
                        let (priority, others): (Vec<usize>, _) =
                            order.into_iter().partition(|&i| pool.rules[i].priority);
                        *current = Some(Drawing {
                            pool,
                            priority: priority.into(),
                            others,
                        });
                        continue;
                    }
                };
                let entry = match drawing.priority.pop_front() {
                    Some(entry) => entry,
                    None => {
                        let Some(at) = drawing.pool.weighted(&drawing.others, random) else {
                            drawing.others.clear();
                            continue;
                        };
                        drawing.others.remove(at)
                    }
                };
                // spec-yungsapi §3.1 item 8: an empty element ends the
                // jigsaw (the caller's), then `max_count`, then the depth.
                let rules = &drawing.pool.rules[entry];
                if let Some((name, max)) = &rules.limit
                    && counts.get(name).copied().unwrap_or(0) >= *max
                {
                    continue;
                }
                if depth < rules.min_depth {
                    continue;
                }
                let limit = drawing.pool.limits[entry].as_ref();
                if limit.is_some_and(|l| !l.allows(counts, depth)) {
                    continue;
                }
                return Some(Pick {
                    element: drawing.pool.raw[entry].0.clone(),
                    name: (rules.limit.as_ref().map(|(name, _)| name))
                        .or(limit.map(|l| &l.key))
                        .cloned(),
                    ignore_bounds: rules.ignore_bounds,
                });
            },
        }
    }
}

pub struct JigsawConfig<'a> {
    pub start_pool: &'a str,
    pub start_jigsaw: Option<&'a str>,
    pub max_depth: i32,
    pub use_expansion_hack: bool,
    pub project_to: Option<HeightmapType>,
    pub max_distance: [i32; 2],
    pub aliases: &'a BTreeMap<String, String>,
    pub padding: [i32; 2],
    pub waterlogging: bool,
}

/// Free space for children: an inclusive container box minus placed boxes.
#[derive(Debug)]
pub(super) struct Free {
    pub(super) container: BoundingBox,
    pub(super) removed: Vec<BoundingBox>,
}
impl Free {
    /// Whether every point of the (0.25-deflated) box lies in free space.
    pub(super) fn fits(&self, b: &BoundingBox) -> bool {
        (0..3).all(|i| b.min[i] >= self.container.min[i] && b.max[i] <= self.container.max[i])
            && !self.removed.iter().any(|r| r.intersects(b))
    }
}
struct PieceState {
    piece: usize,
    free: Rc<RefCell<Free>>,
    depth: i32,
}

/// JigsawPlacement.addPieces. Returns the structure start position and the
/// pieces, or None when the start is invalid.
pub fn assemble(
    config: &JigsawConfig<'_>,
    context: &mut impl JigsawContext,
    pools: &Pools,
    resources: &StructureResources,
    bindings: &FeatureBlockBindings<'_>,
    random: &mut impl RandomSource,
    position: [i32; 3],
) -> Option<([i32; 3], Vec<PoolPiece>)> {
    assemble_drawing(
        Draws::Vanilla,
        config,
        context,
        pools,
        resources,
        bindings,
        random,
        position,
    )
}

/// [`assemble`] with the given draws.
#[allow(clippy::too_many_arguments)]
pub fn assemble_drawing(
    draws: Draws,
    config: &JigsawConfig<'_>,
    context: &mut impl JigsawContext,
    pools: &Pools,
    resources: &StructureResources,
    bindings: &FeatureBlockBindings<'_>,
    random: &mut impl RandomSource,
    position: [i32; 3],
) -> Option<([i32; 3], Vec<PoolPiece>)> {
    let alias = |name: &str| {
        config
            .aliases
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_owned())
    };
    let center_pool = pools.get(&alias(config.start_pool), resources, bindings)?;
    let (rotation, element) = match draws {
        Draws::Vanilla => {
            let rotation = Rotation::random(random);
            (rotation, center_pool.random_template(random))
        }
        // The entries shuffled, then the rotation, then a weighted pick in
        // the shuffled order.
        Draws::Yung => {
            let mut order: Vec<usize> = (0..center_pool.raw.len()).collect();
            shuffle(&mut order, random);
            let rotation = Rotation::random(random);
            let entry = order[center_pool.weighted(&order, random)?];
            (rotation, center_pool.raw[entry].0.clone())
        }
    };
    if matches!(element.as_ref(), PoolElement::Empty) {
        return None;
    }
    let anchored = match config.start_jigsaw {
        Some(name) => {
            element
                .shuffled_jigsaws(resources, position, rotation, random)
                .into_iter()
                .find(|j| j.name.as_deref() == Some(name))?
                .pos
        }
        None => position,
    };
    let local: [i32; 3] = std::array::from_fn(|i| anchored[i] - position[i]);
    let adjusted: [i32; 3] = std::array::from_fn(|i| position[i] - local[i]);
    let mut center = PoolPiece {
        bounding_box: element.bounding_box(adjusted, rotation),
        ground_level_delta: element.ground_level_delta(),
        element: element.clone(),
        position: adjusted,
        rotation,
        junctions: Vec::new(),
        waterlogging: config.waterlogging,
    };
    let b = center.bounding_box;
    let cx = (b.max[0] + b.min[0]) / 2;
    let cz = (b.max[2] + b.min[2]) / 2;
    let bottom = match config.project_to {
        None => adjusted[1],
        Some(kind) => {
            if !context.column_has_valid_biome(cx, cz, context.min_y(), context.max_y()) {
                return None;
            }
            position[1] + context.base_height(cx, cz, kind)
        }
    };
    let old_ground = b.min[1] + center.ground_level_delta;
    center.moved(bottom - old_ground);
    let b = center.bounding_box;
    if config.padding != [0, 0]
        && (b.min[1] < context.min_y() + config.padding[0]
            || b.max[1] > context.max_y() - config.padding[1])
    {
        return None;
    }
    let cy = bottom + local[1];
    let mut pieces = vec![center];
    if config.max_depth > 0 {
        let [h, v] = config.max_distance;
        let low_y = (cy - v).max(context.min_y() + config.padding[0]);
        let high_y = (cy + v + 1).min(context.max_y() + 1 - config.padding[1]);
        let container = BoundingBox::new([cx - h, low_y, cz - h], [cx + h, high_y - 1, cz + h]);
        // The start does not count toward `max_count` (spec-yungsapi §3).
        add_children_drawing(
            draws,
            HashMap::new(),
            config,
            context,
            pools,
            resources,
            bindings,
            random,
            &mut pieces,
            container,
        );
    }
    Some(([cx, cy, cz], pieces))
}

/// The inner JigsawPlacement.addPieces: lays out the children of
/// `pieces[0]`, a start piece already placed, to `config.max_depth`,
/// within `container` (inclusive) less the start's box. A structure with
/// its own start (Formations' pieced one) reaches the jigsaw layout here.
#[allow(clippy::too_many_arguments)]
pub(super) fn add_children(
    config: &JigsawConfig<'_>,
    context: &mut impl JigsawContext,
    pools: &Pools,
    resources: &StructureResources,
    bindings: &FeatureBlockBindings<'_>,
    random: &mut impl RandomSource,
    pieces: &mut Vec<PoolPiece>,
    container: BoundingBox,
) {
    add_children_drawing(
        Draws::Vanilla,
        HashMap::new(),
        config,
        context,
        pools,
        resources,
        bindings,
        random,
        pieces,
        container,
    );
}
#[allow(clippy::too_many_arguments)]
fn add_children_drawing(
    draws: Draws,
    counts: HashMap<Arc<str>, i32>,
    config: &JigsawConfig<'_>,
    context: &mut impl JigsawContext,
    pools: &Pools,
    resources: &StructureResources,
    bindings: &FeatureBlockBindings<'_>,
    random: &mut impl RandomSource,
    pieces: &mut Vec<PoolPiece>,
    container: BoundingBox,
) {
    let free = Rc::new(RefCell::new(Free {
        container,
        removed: vec![pieces[0].bounding_box],
    }));
    let mut placer = Placer {
        config,
        pools,
        resources,
        bindings,
        pieces,
        queue: BTreeMap::new(),
        draws,
        counts,
    };
    placer.children(0, free, 0, context, random);
    while let Some(state) = placer.next() {
        placer.children(state.piece, state.free, state.depth, context, random);
    }
}
struct Placer<'a, 'p> {
    config: &'a JigsawConfig<'a>,
    pools: &'a Pools,
    resources: &'a StructureResources,
    bindings: &'a FeatureBlockBindings<'a>,
    pieces: &'p mut Vec<PoolPiece>,
    /// SequencedPriorityIterator: highest priority first, FIFO within one.
    queue: BTreeMap<i32, VecDeque<PieceState>>,
    draws: Draws,
    /// Pieces placed per limited element (`Limit::key`).
    counts: HashMap<Arc<str>, i32>,
}
impl Placer<'_, '_> {
    fn next(&mut self) -> Option<PieceState> {
        let mut entry = self.queue.last_entry()?;
        let state = entry.get_mut().pop_front();
        if entry.get().is_empty() {
            entry.remove();
        }
        state
    }
    fn pool(&self, name: &str) -> Option<Arc<Pool>> {
        let name = self.config.aliases.get(name).map_or(name, String::as_str);
        self.pools.get(name, self.resources, self.bindings)
    }
    fn children(
        &mut self,
        source_index: usize,
        context_free: Rc<RefCell<Free>>,
        depth: i32,
        context: &mut impl JigsawContext,
        random: &mut impl RandomSource,
    ) {
        let (source_element, source_position, source_rotation, source_box, source_ground) = {
            let s = &self.pieces[source_index];
            (
                s.element.clone(),
                s.position,
                s.rotation,
                s.bounding_box,
                s.ground_level_delta,
            )
        };
        let source_rigid = source_element.projection() == Projection::Rigid;
        let mut source_free: Option<Rc<RefCell<Free>>> = None;
        let source_y = source_box.min[1];
        let jigsaws = source_element.shuffled_jigsaws(
            self.resources,
            source_position,
            source_rotation,
            random,
        );
        'jigsaws: for jigsaw in jigsaws {
            let direction = jigsaw.front;
            let s = step(direction);
            let target_pos: [i32; 3] = std::array::from_fn(|i| jigsaw.pos[i] + s[i]);
            let local_y = jigsaw.pos[1] - source_y;
            let mut base_height = i32::MIN;
            let Some(target_pool) = self.pool(&jigsaw.pool) else {
                continue;
            };
            if target_pool.templates.is_empty() && jigsaw.pool != "minecraft:empty" {
                continue;
            }
            let Some(fallback) = self.pool(&target_pool.fallback) else {
                continue;
            };
            if fallback.templates.is_empty() && target_pool.fallback != "minecraft:empty" {
                continue;
            }
            let children_free = if source_box.is_inside(target_pos) {
                source_free
                    .get_or_insert_with(|| {
                        Rc::new(RefCell::new(Free {
                            container: source_box,
                            removed: Vec::new(),
                        }))
                    })
                    .clone()
            } else {
                context_free.clone()
            };
            let mut candidates = Candidates::new(
                self.draws,
                &target_pool,
                &fallback,
                depth != self.config.max_depth,
                random,
            );
            while let Some(Pick {
                element: target,
                name,
                ignore_bounds,
            }) = candidates.next(&self.counts, depth, random)
            {
                if matches!(target.as_ref(), PoolElement::Empty) {
                    break;
                }
                let mut rotations = Rotation::ALL;
                shuffle(&mut rotations, random);
                for rotation in rotations {
                    let target_jigsaws =
                        target.shuffled_jigsaws(self.resources, [0; 3], rotation, random);
                    let hack_box = target.bounding_box([0; 3], rotation);
                    let expand_to = if self.config.use_expansion_hack && hack_box.span()[1] <= 16 {
                        target_jigsaws
                            .iter()
                            .map(|tj| {
                                let st = step(tj.front);
                                if !hack_box.is_inside(std::array::from_fn(|i| tj.pos[i] + st[i])) {
                                    return 0;
                                }
                                let child = self.pool(&tj.pool);
                                let child_size = child.as_ref().map_or(0, |p| p.max_size());
                                let fallback_size = child
                                    .and_then(|p| self.pool(&p.fallback))
                                    .map_or(0, |p| p.max_size());
                                child_size.max(fallback_size)
                            })
                            .max()
                            .unwrap_or(0)
                    } else {
                        0
                    };
                    for target_jigsaw in &target_jigsaws {
                        if !can_attach(&jigsaw, target_jigsaw) {
                            continue;
                        }
                        let target_local = target_jigsaw.pos;
                        let raw_position: [i32; 3] =
                            std::array::from_fn(|i| target_pos[i] - target_local[i]);
                        let raw_box = target.bounding_box(raw_position, rotation);
                        let raw_y = raw_box.min[1];
                        let target_rigid = target.projection() == Projection::Rigid;
                        let target_local_y = target_local[1];
                        let delta_y = local_y - target_local_y + step(jigsaw.front)[1];
                        let target_y = if source_rigid && target_rigid {
                            source_y + delta_y
                        } else {
                            if base_height == i32::MIN {
                                base_height = context.base_height(
                                    jigsaw.pos[0],
                                    jigsaw.pos[2],
                                    HeightmapType::WorldSurfaceWg,
                                );
                            }
                            base_height - target_local_y
                        };
                        let offset = target_y - raw_y;
                        let mut target_box = raw_box.moved([0, offset, 0]);
                        let target_position =
                            [raw_position[0], raw_position[1] + offset, raw_position[2]];
                        if expand_to > 0 {
                            let size = (expand_to + 1).max(target_box.max[1] - target_box.min[1]);
                            target_box.max[1] = target_box.max[1].max(target_box.min[1] + size);
                        }
                        if !ignore_bounds && !children_free.borrow().fits(&target_box) {
                            continue;
                        }
                        children_free.borrow_mut().removed.push(target_box);
                        let target_ground = if target_rigid {
                            source_ground - delta_y
                        } else {
                            target.ground_level_delta()
                        };
                        let junction_y = if source_rigid {
                            source_y + local_y
                        } else if target_rigid {
                            target_y + target_local_y
                        } else {
                            if base_height == i32::MIN {
                                base_height = context.base_height(
                                    jigsaw.pos[0],
                                    jigsaw.pos[2],
                                    HeightmapType::WorldSurfaceWg,
                                );
                            }
                            base_height + delta_y / 2
                        };
                        self.pieces[source_index].junctions.push(Junction {
                            source_x: target_pos[0],
                            source_ground_y: junction_y - local_y + source_ground,
                            source_z: target_pos[2],
                            delta_y,
                            dest_projection: target.projection(),
                        });
                        self.pieces.push(PoolPiece {
                            element: target.clone(),
                            position: target_position,
                            ground_level_delta: target_ground,
                            rotation,
                            bounding_box: target_box,
                            junctions: vec![Junction {
                                source_x: jigsaw.pos[0],
                                source_ground_y: junction_y - target_local_y + target_ground,
                                source_z: jigsaw.pos[2],
                                delta_y: -delta_y,
                                dest_projection: if source_rigid {
                                    Projection::Rigid
                                } else {
                                    Projection::TerrainMatching
                                },
                            }],
                            waterlogging: self.config.waterlogging,
                        });
                        if let Some(name) = name {
                            *self.counts.entry(name).or_default() += 1;
                        }
                        if depth < self.config.max_depth {
                            self.queue
                                .entry(jigsaw.placement_priority)
                                .or_default()
                                .push_back(PieceState {
                                    piece: self.pieces.len() - 1,
                                    free: children_free.clone(),
                                    depth: depth + 1,
                                });
                        }
                        continue 'jigsaws;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A pool of empty elements, told apart by identity, with these limits.
    fn pool(entries: &[(i32, Value)]) -> Arc<Pool> {
        let (mut templates, mut indices, mut raw, mut rules, mut limits) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for (i, (weight, element)) in entries.iter().enumerate() {
            let e = Arc::new(PoolElement::Empty);
            for _ in 0..*weight {
                templates.push(e.clone());
                indices.push(i);
            }
            raw.push((e, *weight));
            rules.push(Rules::of(element));
            limits.push(Limit::of(element));
        }
        let guaranteed = (0..entries.len())
            .filter(|&i| limits[i].as_ref().is_some_and(|l: &Limit| l.guaranteed))
            .collect();
        Arc::new(Pool {
            templates,
            entries: indices,
            raw,
            rules,
            limits,
            guaranteed,
            fallback: "minecraft:empty".into(),
            max_size: OnceLock::new(),
        })
    }
    fn delegate() -> Value {
        json!({"element_type": "minecraft:single_pool_element", "location": "a:b"})
    }
    /// The entries a vanilla jigsaw tries below a parent at `depth`: the
    /// target's by index, the fallback's by index plus 100.
    fn tried(
        target: &Arc<Pool>,
        fallback: &Arc<Pool>,
        counts: &HashMap<Arc<str>, i32>,
        depth: i32,
        below_max: bool,
    ) -> Vec<usize> {
        let mut random = LegacyRandom::new(7);
        let mut candidates =
            Candidates::new(Draws::Vanilla, target, fallback, below_max, &mut random);
        let index = |pool: &Arc<Pool>, element: &Arc<PoolElement>| {
            pool.raw.iter().position(|(e, _)| Arc::ptr_eq(e, element))
        };
        let mut out = Vec::new();
        while let Some(pick) = candidates.next(counts, depth, &mut random) {
            out.push(
                index(target, &pick.element)
                    .or_else(|| index(fallback, &pick.element).map(|i| i + 100))
                    .unwrap(),
            );
        }
        out
    }

    #[test]
    fn limits_read_lithostitcheds_fields() {
        let limited =
            json!({"element_type": "lithostitched:limited", "limit": 1, "delegate": delegate()});
        let l = Limit::of(&limited).unwrap();
        assert_eq!((l.max, l.min_depth, l.guaranteed), (1, 0, false));
        assert_eq!(*l.key, *limited.to_string());
        let guaranteed = json!({"element_type": "lithostitched:guaranteed", "count": 2, "min_depth": 3,
            "delegate": delegate()});
        let g = Limit::of(&guaranteed).unwrap();
        assert_eq!((g.max, g.min_depth, g.guaranteed), (2, 3, true));
        // No limit, or YUNG's (`Rules`): none to keep here.
        for none in [
            json!({"element_type": "lithostitched:limited", "delegate": delegate()}),
            json!({"element_type": "yungsapi:yung_single_element", "name": "pile", "max_count": 2}),
            delegate(),
        ] {
            assert!(Limit::of(&none).is_none(), "{none}");
        }
    }

    #[test]
    fn vanilla_jigsaws_keep_lithostitcheds_limits() {
        let limited =
            json!({"element_type": "lithostitched:limited", "limit": 1, "delegate": delegate()});
        let guaranteed = json!({"element_type": "lithostitched:guaranteed", "count": 1, "min_depth": 3,
            "delegate": delegate()});
        let p = pool(&[
            (3, delegate()),
            (2, limited.clone()),
            (1, guaranteed.clone()),
        ]);
        let empty = pool(&[]);
        let mut counts = HashMap::new();
        // Too shallow for the guaranteed entry: the weight-expanded list without it.
        let mut shallow = tried(&p, &empty, &counts, 2, true);
        shallow.sort();
        assert_eq!(shallow, [0, 0, 0, 1, 1]);
        // Deep enough: it comes first, and is not drawn again.
        let deep = tried(&p, &empty, &counts, 3, true);
        assert_eq!(deep[0], 2);
        assert_eq!(deep[1..].iter().filter(|&&e| e == 2).count(), 0);
        // At the most depth only the fallback is tried.
        assert!(tried(&p, &empty, &counts, 3, false).is_empty());
        // Entries at their limit are skipped.
        counts.insert(Arc::from(limited.to_string()), 1);
        counts.insert(Arc::from(guaranteed.to_string()), 1);
        let full = tried(&p, &empty, &counts, 3, true);
        assert_eq!(full, [0, 0, 0]);
    }

    /// A fallback pool's guaranteed entries come first once it is reached,
    /// below the most depth as at it (rdbtCVS's review of #1777).
    #[test]
    fn a_fallbacks_guaranteed_entries_are_tried() {
        let guaranteed = json!({"element_type": "lithostitched:guaranteed", "count": 1,
            "delegate": delegate()});
        let target = pool(&[(2, delegate())]);
        let fallback = pool(&[(1, delegate()), (1, guaranteed)]);
        let counts = HashMap::new();
        let below = tried(&target, &fallback, &counts, 0, true);
        assert_eq!(below[..2], [0, 0]);
        assert_eq!(below[2..], [101, 100]);
        assert_eq!(tried(&target, &fallback, &counts, 0, false), [101, 100]);
    }
}
