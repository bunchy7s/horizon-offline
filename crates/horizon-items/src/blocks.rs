//! Per-state gameplay facts exported from vanilla's own block-state accessors
//! (`gameplay-facts.json`), indexed by registry state ID.
mod decode;
use crate::Result;
use horizon_world::{
    level::{Behaviours, GameplayStateSource},
    registry::{BlockRegistry, BlockStateId},
};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs::{self, File},
    io::BufReader,
    path::Path,
    sync::Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderShape {
    Model,
    Invisible,
    Animated,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fluid {
    None,
    Water,
    Lava,
}
#[derive(Clone, Debug)]
pub struct Sounds {
    pub volume: f32,
    pub pitch: f32,
    pub break_sound: Arc<str>,
    pub step: Arc<str>,
    pub place: Arc<str>,
    pub hit: Arc<str>,
    pub fall: Arc<str>,
}
#[derive(Clone, Debug)]
pub struct StateFacts {
    pub name: Arc<str>,
    pub emission: u8,
    pub dampening: u8,
    pub skylight_down: bool,
    pub shape_occlusion: bool,
    pub can_occlude: bool,
    pub solid_render: bool,
    pub hardness: f32,
    pub requires_tool: bool,
    pub replaceable: bool,
    pub random_ticks: bool,
    pub block_entity: bool,
    pub render_shape: RenderShape,
    pub full_collision: bool,
    pub friction: f32,
    pub explosion_resistance: f32,
    pub item: Arc<str>,
    pub sounds: Sounds,
    pub fluid: Fluid,
    /// Fluid amount 1..=8 (8 = source/falling surface), 0 when there is no fluid.
    pub fluid_level: u8,
    pub fluid_falling: bool,
    pub waterlogged: bool,
    /// `hasOffsetFunction`: the model is jittered per column.
    pub random_offset: bool,
    /// Whether the outline, not just the model, follows BlockState.getOffset.
    pub outline_offset: bool,
    /// `BlockState.isSolid()` (legacy solidity; fluid surfaces read it).
    pub legacy_solid: bool,
    /// `isFaceSturdy(.., SupportType.FULL)` by Direction ordinal.
    pub sturdy_full: u8,
    /// The block's class chain (`TransparentBlock`, `LeavesBlock`, ...), most specific first.
    pub classes: Arc<[Arc<str>]>,
    /// `getShape` (the outline used for picking) as boxes in block space,
    /// without the random offset; and its edges as LevelRenderer draws them.
    pub outline: Arc<[[f64; 6]]>,
    pub outline_edges: Arc<[[f64; 6]]>,
    /// `getMapColor(..).id` (0 = NONE).
    pub map_color: u8,
}
/// `MapColor.col` by id.
pub const MAP_COLORS: [u32; 62] = [
    0, 8368696, 16247203, 13092807, 16711680, 10526975, 10987431, 31744, 16777215, 10791096,
    9923917, 7368816, 4210943, 9402184, 16776437, 14188339, 11685080, 6724056, 15066419, 8375321,
    15892389, 5000268, 10066329, 5013401, 8339378, 3361970, 6704179, 6717235, 10040115, 1644825,
    16445005, 6085589, 4882687, 55610, 8476209, 7340544, 13742497, 10441252, 9787244, 7367818,
    12223780, 6780213, 10505550, 3746083, 8874850, 5725276, 8014168, 4996700, 4993571, 5001770,
    9321518, 2430480, 12398641, 9715553, 6035741, 1474182, 3837580, 5647422, 1356933, 6579300,
    14200723, 8365974,
];
/// The 12 edges of the unit cube (outline data predating the export).
pub fn cube_edges() -> Vec<[f64; 6]> {
    let mut edges = Vec::new();
    for a in 0..3 {
        for (u, v) in [(0., 0.), (1., 0.), (0., 1.), (1., 1.)] {
            let mut from = [0.0f64; 3];
            let (b, c) = ((a + 1) % 3, (a + 2) % 3);
            from[b] = u;
            from[c] = v;
            let mut to = from;
            to[a] = 1.0;
            edges.push([from[0], from[1], from[2], to[0], to[1], to[2]]);
        }
    }
    edges
}
impl StateFacts {
    pub fn has_class(&self, name: &str) -> bool {
        self.classes.iter().any(|c| &**c == name)
    }
}
/// Compact, owned behaviour inputs retained until the normal behaviour startup stage.
/// No borrowed gameplay JSON or process-wide cache survives the fact loader.
pub struct GameplayProjection {
    flags: u16,
    emission: u64,
    dampening: u64,
    hardness: f64,
    push: Arc<str>,
    volume: f64,
    pitch: f64,
    place: Arc<str>,
    break_sound: Arc<str>,
    fall: Arc<str>,
}

impl GameplayProjection {
    fn from_entry(entry: &decode::Entry<'_>, strings: &mut HashMap<String, Arc<str>>) -> Self {
        fn intern(strings: &mut HashMap<String, Arc<str>>, value: &str) -> Arc<str> {
            if let Some(existing) = strings.get(value) {
                return Arc::clone(existing);
            }
            let value_arc: Arc<str> = Arc::from(value);
            strings.insert(value.to_owned(), Arc::clone(&value_arc));
            value_arc
        }
        const FLAGS: [&str; 11] = [
            "replaceable",
            "random_ticks",
            "solid_render",
            "liquid",
            "block_entity",
            "ignited_by_lava",
            "can_occlude",
            "full_collision",
            "skylight_down",
            "requires_tool",
            "shape_occlusion",
        ];
        Self {
            flags: FLAGS.iter().enumerate().fold(0, |flags, (i, name)| {
                flags | ((u16::from(entry.flag(name))) << i)
            }),
            emission: entry.unsigned("emission"),
            dampening: entry.unsigned("dampening"),
            hardness: entry.number("hardness"),
            push: intern(strings, entry.text("push")),
            volume: entry.sound_number("volume"),
            pitch: entry.sound_number("pitch"),
            place: intern(strings, entry.sound_id("place")),
            break_sound: intern(strings, entry.sound_id("break")),
            fall: intern(strings, entry.sound_id("fall")),
        }
    }
}

impl GameplayStateSource for GameplayProjection {
    fn flag(&self, name: &str) -> bool {
        const FLAGS: [&str; 11] = [
            "replaceable",
            "random_ticks",
            "solid_render",
            "liquid",
            "block_entity",
            "ignited_by_lava",
            "can_occlude",
            "full_collision",
            "skylight_down",
            "requires_tool",
            "shape_occlusion",
        ];
        FLAGS
            .iter()
            .position(|field| *field == name)
            .is_some_and(|index| self.flags & (1 << index) != 0)
    }
    fn unsigned(&self, name: &str) -> u64 {
        match name {
            "emission" => self.emission,
            "dampening" => self.dampening,
            _ => 0,
        }
    }
    fn number(&self, name: &str) -> f64 {
        if name == "hardness" {
            self.hardness
        } else {
            0.0
        }
    }
    fn text(&self, name: &str) -> &str {
        if name == "push" { &self.push } else { "" }
    }
    fn sound_number(&self, name: &str) -> f64 {
        match name {
            "volume" => self.volume,
            "pitch" => self.pitch,
            _ => 1.0,
        }
    }
    fn sound_id(&self, name: &str) -> &str {
        match name {
            "place" => &self.place,
            "break" => &self.break_sound,
            "fall" => &self.fall,
            _ => "",
        }
    }
}

pub struct BlockFacts {
    pub states: Vec<StateFacts>,
    pub air: BlockStateId,
}
impl BlockFacts {
    pub fn load(root: &Path, registry: &BlockRegistry) -> Result<Self> {
        let behaviours: Value = serde_json::from_reader(BufReader::new(File::open(
            root.join("block-behaviours.json"),
        )?))?;
        Self::load_with_behaviour_json(root, registry, &behaviours)
    }

    /// Load both tables from one resource root, sharing each JSON decode.
    /// No data is cached across calls: changes to prepared packs or mods are read again.
    pub fn load_pair(root: &Path, registry: &BlockRegistry) -> Result<(Self, Behaviours)> {
        let behaviours: Value = serde_json::from_reader(BufReader::new(File::open(
            root.join("block-behaviours.json"),
        )?))?;
        Self::load_with_behaviours(root, registry, &behaviours)
    }

    /// Decode state facts once, retaining only the small owned fields needed later.
    /// `parallel`: the JSON validation pass may run on a thread of its own.
    pub fn load_with_projection(
        root: &Path,
        registry: &BlockRegistry,
        behaviours: &Value,
        parallel: bool,
    ) -> Result<(Self, Vec<GameplayProjection>)> {
        Self::load_with_callback(root, registry, behaviours, parallel, |entries| {
            let mut strings = HashMap::new();
            Ok(entries
                .iter()
                .map(|entry| GameplayProjection::from_entry(entry, &mut strings))
                .collect())
        })
    }

    pub fn load_with_behaviour_json(
        root: &Path,
        registry: &BlockRegistry,
        behaviours: &Value,
    ) -> Result<Self> {
        Ok(Self::load_with_callback(root, registry, behaviours, true, |_| Ok(()))?.0)
    }

    /// Build both tables from the same behaviour document and gameplay decoder.
    pub fn load_with_behaviours(
        root: &Path,
        registry: &BlockRegistry,
        behaviours: &Value,
    ) -> Result<(Self, Behaviours)> {
        Self::load_with_callback(root, registry, behaviours, true, |entries| {
            Ok(Behaviours::load_with_gameplay(
                registry, root, behaviours, entries,
            )?)
        })
    }

    fn load_with_callback<T>(
        root: &Path,
        registry: &BlockRegistry,
        behaviours: &Value,
        parallel: bool,
        callback: impl FnOnce(&[decode::Entry<'_>]) -> Result<T>,
    ) -> Result<(Self, T)> {
        let bytes = fs::read(root.join("gameplay-facts.json"))?;
        let validate = || serde_json::from_slice::<decode::Validate>(&bytes).map(drop);
        if !parallel {
            validate()?;
            return Self::decode_with_callback(&bytes, registry, behaviours, callback);
        }
        // The validation pass (a third of the load) only reports errors: it
        // reads the file beside the decode, and its error still wins.
        std::thread::scope(|scope| {
            let validated = scope.spawn(validate);
            let decoded = Self::decode_with_callback(&bytes, registry, behaviours, callback);
            validated
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic))?;
            decoded
        })
    }

    fn decode_with_callback<T>(
        bytes: &[u8],
        registry: &BlockRegistry,
        behaviours: &Value,
        callback: impl FnOnce(&[decode::Entry<'_>]) -> Result<T>,
    ) -> Result<(Self, T)> {
        let raw: decode::Gameplay<'_> = serde_json::from_slice(bytes)?;
        let entries = raw
            .states
            .0
            .as_ref()
            .ok_or("missing gameplay state facts")?;
        if entries.len() != registry.len() {
            return Err("gameplay facts do not match the block registry".into());
        }
        // Class chains and per-state behaviour flags exported from the actual block objects.
        let behaviour_states = behaviours["states"]
            .as_array()
            .filter(|s| s.len() == registry.len())
            .ok_or("block behaviour facts do not match the block registry")?;
        let mut class_names: HashMap<String, Arc<str>> = HashMap::new();
        let classes: HashMap<&str, Arc<[Arc<str>]>> = behaviours["blocks"]
            .as_array()
            .ok_or("missing block behaviour classes")?
            .iter()
            .filter_map(|b| {
                let chain = b["class"]
                    .as_array()?
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|c| {
                        class_names
                            .entry(c.to_owned())
                            .or_insert_with(|| Arc::from(c))
                            .clone()
                    });
                Some((b["name"].as_str()?, chain.collect()))
            })
            .collect();
        let mut names: HashMap<String, Arc<str>> = HashMap::new();
        let mut intern = |s: &str| -> Arc<str> {
            if let Some(name) = names.get(s) {
                return name.clone();
            }
            let name: Arc<str> = Arc::from(s);
            names.insert(s.to_owned(), name.clone());
            name
        };
        let mut shapes = decode::Shapes::default();
        let mut states = Vec::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            let id = BlockStateId(index as u32);
            if entry.id != Some(index as u64) {
                return Err("gameplay facts are not in registry order".into());
            }
            let state = registry.state(id).ok_or("missing registry state")?;
            let sound = &entry.sound;
            let prop = |k: &str| state.properties.get(k).map(String::as_str);
            let (fluid, fluid_level, fluid_falling) = match state.name.as_str() {
                "minecraft:water" | "minecraft:lava" => {
                    let level: u8 = prop("level").and_then(|v| v.parse().ok()).unwrap_or(0);
                    let kind = if state.name == "minecraft:water" {
                        Fluid::Water
                    } else {
                        Fluid::Lava
                    };
                    // LiquidBlock: level 0 is the source; 1..7 flowing; >=8 falling.
                    if level == 0 {
                        (kind, 8, false)
                    } else if level >= 8 {
                        (kind, 8, true)
                    } else {
                        (kind, 8 - level, false)
                    }
                }
                "minecraft:bubble_column"
                | "minecraft:kelp"
                | "minecraft:kelp_plant"
                | "minecraft:seagrass"
                | "minecraft:tall_seagrass" => (Fluid::Water, 8, false),
                _ if prop("waterlogged") == Some("true") => (Fluid::Water, 8, false),
                _ => (Fluid::None, 0, false),
            };
            let air = matches!(
                state.name.as_str(),
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            );
            let cube = move || {
                if air {
                    Vec::new()
                } else {
                    vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]
                }
            };
            let edges = move || if air { Vec::new() } else { cube_edges() };
            let outline = shapes.intern(entry.outline, cube)?;
            let outline_edges = shapes.intern(entry.outline_edges, edges)?;
            // The export holds MapColor.col; ids are positions in MAP_COLORS.
            let col = entry.map_color.unwrap_or(0) as u32;
            let map_color = if col == 0 {
                0
            } else {
                MAP_COLORS.iter().position(|c| *c == col).unwrap_or(0) as u8
            };
            states.push(StateFacts {
                map_color,
                outline,
                outline_edges,
                name: intern(&state.name),
                emission: entry.emission.as_f64().unwrap_or(0.0) as u8,
                dampening: entry.dampening.as_f64().unwrap_or(0.0) as u8,
                skylight_down: entry.skylight_down,
                shape_occlusion: entry.shape_occlusion,
                can_occlude: entry.can_occlude,
                solid_render: entry.solid_render,
                hardness: entry.hardness.as_f64().unwrap_or(0.0) as f32,
                requires_tool: entry.requires_tool,
                replaceable: entry.replaceable,
                random_ticks: entry.random_ticks,
                block_entity: entry.block_entity,
                render_shape: match entry.render_shape.as_deref() {
                    Some("INVISIBLE") => RenderShape::Invisible,
                    Some("ENTITYBLOCK_ANIMATED") => RenderShape::Animated,
                    _ => RenderShape::Model,
                },
                full_collision: entry.full_collision,
                friction: entry.friction as f32,
                explosion_resistance: entry.explosion_resistance as f32,
                item: intern(entry.item.as_deref().unwrap_or("minecraft:air")),
                sounds: Sounds {
                    volume: sound.volume as f32,
                    pitch: sound.pitch as f32,
                    break_sound: intern(sound.r#break.id.as_deref().unwrap_or("")),
                    step: intern(sound.step.id.as_deref().unwrap_or("")),
                    place: intern(sound.place.id.as_deref().unwrap_or("")),
                    hit: intern(sound.hit.id.as_deref().unwrap_or("")),
                    fall: intern(sound.fall.id.as_deref().unwrap_or("")),
                },
                fluid,
                fluid_level,
                fluid_falling,
                waterlogged: prop("waterlogged") == Some("true"),
                random_offset: entry.offset,
                outline_offset: entry.outline_offset.unwrap_or(entry.offset),
                legacy_solid: behaviour_states[index][1].as_u64().unwrap_or(0) & 1 != 0,
                sturdy_full: (behaviour_states[index][0].as_u64().unwrap_or(0) & 63) as u8,
                classes: classes
                    .get(state.name.as_str())
                    .cloned()
                    .ok_or("missing block behaviour class chain")?,
            });
        }
        let extra = callback(entries)?;
        Ok((
            Self {
                states,
                air: registry
                    .default_state("minecraft:air")
                    .ok_or("missing air")?,
            },
            extra,
        ))
    }
    pub fn get(&self, id: BlockStateId) -> &StateFacts {
        &self.states[id.0 as usize]
    }
    pub fn is_air(&self, id: BlockStateId) -> bool {
        matches!(
            &*self.states[id.0 as usize].name,
            "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
        )
    }
}

impl GameplayStateSource for decode::Entry<'_> {
    fn flag(&self, name: &str) -> bool {
        match name {
            "replaceable" => self.replaceable,
            "random_ticks" => self.random_ticks,
            "solid_render" => self.solid_render,
            "liquid" => self.liquid,
            "block_entity" => self.block_entity,
            "ignited_by_lava" => self.ignited_by_lava,
            "can_occlude" => self.can_occlude,
            "full_collision" => self.full_collision,
            "skylight_down" => self.skylight_down,
            "requires_tool" => self.requires_tool,
            "shape_occlusion" => self.shape_occlusion,
            _ => false,
        }
    }
    fn unsigned(&self, name: &str) -> u64 {
        match name {
            "emission" => self.emission.as_u64().unwrap_or(0),
            "dampening" => self.dampening.as_u64().unwrap_or(0),
            _ => 0,
        }
    }
    fn number(&self, name: &str) -> f64 {
        match name {
            "hardness" => self.hardness.as_f64().unwrap_or(0.0),
            _ => 0.0,
        }
    }
    fn text(&self, name: &str) -> &str {
        match name {
            "push" => self.push.as_deref().unwrap_or(""),
            _ => "",
        }
    }
    fn sound_number(&self, name: &str) -> f64 {
        match name {
            "volume" => self.sound.volume,
            "pitch" => self.sound.pitch,
            _ => 1.0,
        }
    }
    fn sound_id(&self, name: &str) -> &str {
        match name {
            "place" => self.sound.place.id.as_deref().unwrap_or(""),
            "break" => self.sound.r#break.id.as_deref().unwrap_or(""),
            "fall" => self.sound.fall.id.as_deref().unwrap_or(""),
            _ => "",
        }
    }
}
