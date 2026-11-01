//! Client-side ambient block effects: `ClientLevel.animateTick` (667 random
//! blocks within 16 and 32 blocks per tick calling `Block.animateTick` and
//! `FluidState.animateTick`, fluid drips and biome ambient particles) and the
//! campfire block-entity particle tick. Particle positions, odds and sounds
//! follow the vanilla block classes.
use crate::{
    biome::Biomes,
    blocks::{BlockFacts, Fluid},
    command::args::GameMode,
    items::ItemStack,
    particles::{ParticleEngine, ParticleOptions, ParticleType as T, along_axis},
    world::World,
};
use horizon_core::{
    geometry::Axis,
    random::{LegacyRandom, RandomSource},
};
use horizon_world::{
    block_collision::BlockCollisionRegistry,
    registry::{BlockRegistry, BlockStateId},
    worldgen::tags::TagDefinitions,
};
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Animate {
    None,
    Torch(T),
    WallTorch(T, [i32; 3]),
    RedstoneTorch,
    RedstoneWallTorch([i32; 3]),
    Furnace([i32; 3]),
    BlastFurnace([i32; 3]),
    Smoker,
    Campfire {
        spawn_particles: bool,
    },
    /// Leaves: optional falling-leaf particle and chance; a tinted leaf takes
    /// the biome foliage colour unless a fixed colour is given (azalea); and
    /// whether they have the poplar ambient sound (`AmbientLeavesBlockSoundPlayer`).
    Leaves(Option<(T, f32)>, Option<u32>, bool),
    NetherPortal,
    EnchantingTable,
    EndRod([i32; 3]),
    SporeBlossom,
    CryingObsidian,
    Mycelium,
    Candles(u8),
    CandleCake,
    BrewingStand,
    EnderChest,
    /// FallingBlock with its dust colour; sand also has the ambient desert sound
    /// (`SandBlock.animateTick`).
    Falling(u32, bool),
    /// A beehive or bee nest full of honey (`BeehiveBlock.animateTick`).
    Beehive,
    RedstoneOre,
    RedstoneWire {
        power: u8,
        sides: [u8; 4],
    },
    RespawnAnchor,
    BubbleColumn {
        drag: bool,
    },
    /// `BaseFireBlock` (`SoulFireBlock.canBurn` is always true).
    Fire {
        soul: bool,
    },
    /// An active sculk sensor.
    SculkSensor,
    /// Potent sulfur that is not dry.
    PotentSulfur,
    DriedGhast {
        waterlogged: bool,
    },
    EndPortal,
    /// A powered lever: its facing and the direction it is attached toward.
    Lever {
        facing: [i32; 3],
        connected: [i32; 3],
    },
    /// A powered repeater.
    Repeater {
        facing: [i32; 3],
        delay: i32,
    },
    /// A free-hanging stalactite tip that is not waterlogged.
    PointedDripstone,
    WitherRose,
    EndGateway,
    /// A lightning rod, by the axis it points along (0, 1, 2 for X, Y, Z).
    LightningRod(usize),
    /// `WetSpongeBlock.animateTick`.
    WetSponge,
    FireflyBush,
    /// An open eyeblossom (`Type.OPEN.emitSounds()`).
    OpenEyeblossom,
    PaleHangingMoss,
    /// `DryVegetationBlock`: a dead bush.
    DeadBush,
    /// `ShortDryGrassBlock` and `TallDryGrassBlock`.
    DryGrass,
    /// A creaking heart that is not uprooted (`CreakingHeartBlock.animateTick`).
    CreakingHeart,
}
fn step(facing: &str) -> [i32; 3] {
    match facing {
        "north" => [0, 0, -1],
        "south" => [0, 0, 1],
        "west" => [-1, 0, 0],
        "east" => [1, 0, 0],
        "up" => [0, 1, 0],
        _ => [0, -1, 0],
    }
}
fn classify(name: &str, prop: &dyn Fn(&str) -> Option<String>) -> Animate {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    let lit = prop("lit").as_deref() == Some("true");
    let powered = prop("powered").as_deref() == Some("true");
    let facing = || step(&prop("facing").unwrap_or_default());
    match name {
        "torch" => Animate::Torch(T::Flame),
        "soul_torch" => Animate::Torch(T::SoulFireFlame),
        "copper_torch" => Animate::Torch(T::CopperFireFlame),
        "wall_torch" => Animate::WallTorch(T::Flame, facing()),
        "soul_wall_torch" => Animate::WallTorch(T::SoulFireFlame, facing()),
        "copper_wall_torch" => Animate::WallTorch(T::CopperFireFlame, facing()),
        "redstone_torch" if lit => Animate::RedstoneTorch,
        "redstone_wall_torch" if lit => Animate::RedstoneWallTorch(facing()),
        "furnace" if lit => Animate::Furnace(facing()),
        "blast_furnace" if lit => Animate::BlastFurnace(facing()),
        "smoker" if lit => Animate::Smoker,
        "campfire" if lit => Animate::Campfire {
            spawn_particles: true,
        },
        "soul_campfire" if lit => Animate::Campfire {
            spawn_particles: false,
        },
        "oak_leaves" | "birch_leaves" | "jungle_leaves" | "acacia_leaves" | "dark_oak_leaves"
        | "mangrove_leaves" => Animate::Leaves(Some((T::TintedLeaves, 0.01)), None, false),
        "cherry_leaves" => Animate::Leaves(Some((T::CherryLeaves, 0.1)), None, false),
        "pale_oak_leaves" => Animate::Leaves(Some((T::PaleOakLeaves, 0.02)), None, false),
        "red_poplar_leaves" => Animate::Leaves(Some((T::RedPoplarLeaves, 0.01)), None, true),
        "orange_poplar_leaves" => Animate::Leaves(Some((T::OrangePoplarLeaves, 0.01)), None, true),
        "yellow_poplar_leaves" => Animate::Leaves(Some((T::YellowPoplarLeaves, 0.01)), None, true),
        // The azalea blocks drop tinted leaves of the fixed colour -9399763.
        "azalea_leaves" | "flowering_azalea_leaves" => {
            Animate::Leaves(Some((T::TintedLeaves, 0.01)), Some(0x0070_922D), false)
        }
        "spruce_leaves" => Animate::Leaves(None, None, false),
        "nether_portal" => Animate::NetherPortal,
        "enchanting_table" => Animate::EnchantingTable,
        "end_rod" => Animate::EndRod(facing()),
        "spore_blossom" => Animate::SporeBlossom,
        "crying_obsidian" => Animate::CryingObsidian,
        "mycelium" => Animate::Mycelium,
        "brewing_stand" => Animate::BrewingStand,
        "ender_chest" => Animate::EnderChest,
        "sand" => Animate::Falling(0xDBD3A0, true),
        "red_sand" => Animate::Falling(0xA95821, true),
        "gravel" => Animate::Falling(0x807C7B, false),
        // AnvilBlock/ConcretePowderBlock.getDustColor is the map colour;
        // DragonEggBlock's is black.
        "anvil" | "chipped_anvil" | "damaged_anvil" => Animate::Falling(0xA7A7A7, false),
        "dragon_egg" => Animate::Falling(0x000000, false),
        _ if name.ends_with("_concrete_powder") => Animate::Falling(
            concrete_powder_color(name.trim_end_matches("_concrete_powder")),
            false,
        ),
        "beehive" | "bee_nest" if prop("honey_level").as_deref() == Some("5") => Animate::Beehive,
        "redstone_ore" | "deepslate_redstone_ore" if lit => Animate::RedstoneOre,
        "redstone_wire" => {
            let power = prop("power").and_then(|p| p.parse().ok()).unwrap_or(0);
            let side = |d: &str| match prop(d).as_deref() {
                Some("up") => 2,
                Some("side") => 1,
                _ => 0,
            };
            if power == 0 {
                Animate::None
            } else {
                Animate::RedstoneWire {
                    power,
                    sides: [side("north"), side("east"), side("south"), side("west")],
                }
            }
        }
        "respawn_anchor" if prop("charges").is_some_and(|c| c != "0") => Animate::RespawnAnchor,
        "potent_sulfur" if prop("potent_sulfur_state").is_some_and(|s| s != "dry") => {
            Animate::PotentSulfur
        }
        "bubble_column" => Animate::BubbleColumn {
            drag: prop("drag").as_deref() == Some("true"),
        },
        "fire" => Animate::Fire { soul: false },
        "soul_fire" => Animate::Fire { soul: true },
        "dried_ghast" => Animate::DriedGhast {
            waterlogged: prop("waterlogged").as_deref() == Some("true"),
        },
        "end_portal" => Animate::EndPortal,
        "lever" if powered => Animate::Lever {
            facing: facing(),
            // FaceAttachedHorizontalDirectionalBlock.getConnectedDirection.
            connected: match prop("face").as_deref() {
                Some("ceiling") => [0, -1, 0],
                Some("floor") => [0, 1, 0],
                _ => facing(),
            },
        },
        "repeater" if powered => Animate::Repeater {
            facing: facing(),
            delay: prop("delay").and_then(|d| d.parse().ok()).unwrap_or(1),
        },
        "pointed_dripstone"
            if prop("vertical_direction").as_deref() == Some("down")
                && prop("thickness").as_deref() == Some("tip")
                && prop("waterlogged").as_deref() != Some("true") =>
        {
            Animate::PointedDripstone
        }
        "wither_rose" => Animate::WitherRose,
        "end_gateway" => Animate::EndGateway,
        _ if name == "lightning_rod" || name.ends_with("_lightning_rod") => {
            let [dx, dy, _] = facing();
            Animate::LightningRod(if dx != 0 {
                0
            } else if dy != 0 {
                1
            } else {
                2
            })
        }
        "wet_sponge" => Animate::WetSponge,
        // BrushableBlock has no dust colour of its own, so the dust is the map colour.
        "suspicious_sand" => Animate::Falling(0xF7E9A3, false),
        "suspicious_gravel" => Animate::Falling(0x707070, false),
        "firefly_bush" => Animate::FireflyBush,
        "open_eyeblossom" => Animate::OpenEyeblossom,
        "pale_hanging_moss" => Animate::PaleHangingMoss,
        "creaking_heart" if prop("creaking_heart_state").as_deref() != Some("uprooted") => {
            Animate::CreakingHeart
        }
        "dead_bush" => Animate::DeadBush,
        "short_dry_grass" | "tall_dry_grass" => Animate::DryGrass,
        "sculk_sensor" | "calibrated_sculk_sensor"
            if prop("sculk_sensor_phase").as_deref() == Some("active") =>
        {
            Animate::SculkSensor
        }
        _ if name.ends_with("candle_cake") && lit => Animate::CandleCake,
        _ if (name == "candle" || name.ends_with("_candle")) && lit => {
            Animate::Candles(prop("candles").and_then(|c| c.parse().ok()).unwrap_or(1))
        }
        _ => Animate::None,
    }
}
/// `DyeColor.getMapColor` of a concrete powder colour.
fn concrete_powder_color(dye: &str) -> u32 {
    match dye {
        "white" => 0xFFFFFF,
        "orange" => 0xD87F33,
        "magenta" => 0xB24CD8,
        "light_blue" => 0x6699D8,
        "yellow" => 0xE5E533,
        "lime" => 0x7FCC19,
        "pink" => 0xF27FA5,
        "gray" => 0x4C4C4C,
        "light_gray" => 0x999999,
        "cyan" => 0x4C7F99,
        "purple" => 0x7F3FB2,
        "blue" => 0x334CB2,
        "brown" => 0x664C33,
        "green" => 0x667F33,
        "red" => 0x993333,
        _ => 0x191919,
    }
}
/// `RedstoneWireBlock.COLORS`.
fn redstone_color(power: u8) -> [f32; 3] {
    let f = f32::from(power) / 15.0;
    let r = f * 0.6 + if f > 0.0 { 0.4 } else { 0.3 };
    let g = (f * f * 0.7 - 0.5).clamp(0.0, 1.0);
    let b = (f * f * 0.6 - 0.7).clamp(0.0, 1.0);
    // DustParticleOptions stores the packed int colour.
    let q = |c: f32| ((c * 255.0) as u8) as f32 / 255.0;
    [q(r), q(g), q(b)]
}
/// `CandleBlock.PARTICLE_OFFSETS` by candle count (pixels).
fn candle_offsets(count: u8) -> &'static [[f64; 3]] {
    match count {
        1 => &[[8.0, 8.0, 8.0]],
        2 => &[[6.0, 7.0, 8.0], [10.0, 8.0, 7.0]],
        3 => &[[8.0, 5.0, 10.0], [6.0, 7.0, 8.0], [9.0, 8.0, 7.0]],
        _ => &[
            [7.0, 5.0, 9.0],
            [10.0, 7.0, 9.0],
            [6.0, 7.0, 6.0],
            [9.0, 8.0, 6.0],
        ],
    }
}

/// Per-state animate behaviour and the block tags it needs. Empty by default
/// (a dedicated server): every lookup finds nothing to animate.
#[derive(Default)]
pub struct AnimateTables {
    kinds: Vec<Animate>,
    impermeable: Vec<bool>,
    power_provider: Vec<bool>,
    power_transmitter: Vec<bool>,
    /// `#triggers_ambient_dried_ghast_block_sounds` (soul sand, soul soil).
    dried_ghast_sounds: Vec<bool>,
    /// `#triggers_ambient_desert_sand_block_sounds` (sand, red sand).
    desert_sand_sounds: Vec<bool>,
    /// `#required_for_poplar_leaf_ambience` (overworld natural logs).
    poplar_leaf_logs: Vec<bool>,
    /// `#triggers_ambient_desert_dry_vegetation_block_sounds`.
    desert_dry_vegetation_sounds: Vec<bool>,
    terracotta: Vec<bool>,
    pale_oak_logs: Vec<bool>,
    /// `FireBlock.canBurn`: states with ignite odds above zero (none waterlogged).
    burnable: Vec<bool>,
    /// Biome id -> ambient particles and probabilities (`minecraft:visual/ambient_particles`).
    ambient: Vec<Vec<(T, f32)>>,
    /// Per state: 1 for a pointed dripstone, 2 for one whose tip points down.
    dripstone: Vec<u8>,
}
impl AnimateTables {
    pub fn load(root: &Path, registry: &BlockRegistry, biomes: &Biomes) -> crate::Result<Self> {
        let kinds = (0..registry.len() as u32)
            .map(|id| {
                registry.state(BlockStateId(id)).map_or(Animate::None, |s| {
                    classify(&s.name, &|k| s.properties.get(k).cloned())
                })
            })
            .collect();
        let bundle: BTreeMap<String, String> =
            serde_json::from_slice(&std::fs::read(root.join("block-tags.json"))?)?;
        let mut definitions = TagDefinitions::default();
        for (name, text) in bundle {
            definitions.apply_json(&name, "vanilla", &text)?;
        }
        let built =
            definitions.build(|name, _| registry.default_state(name).map(|_| name.to_owned()))?;
        let by_state = |tag: &str| -> Vec<bool> {
            let names = built.tags.get(tag);
            (0..registry.len() as u32)
                .map(|id| {
                    registry
                        .state(BlockStateId(id))
                        .is_some_and(|s| names.is_some_and(|n| n.contains(&s.name)))
                })
                .collect()
        };
        let behaviour: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("block-behaviours.json"))?)?;
        let ignite_odds = behaviour["fire"]["igniteOdds"].as_object();
        let burnable = (0..registry.len() as u32)
            .map(|id| {
                // FireBlock.getIgniteOdds: a waterlogged state never burns.
                registry.state(BlockStateId(id)).is_some_and(|s| {
                    s.properties.get("waterlogged").map(String::as_str) != Some("true")
                        && ignite_odds
                            .and_then(|m| m.get(s.name.as_str()))
                            .and_then(serde_json::Value::as_i64)
                            .is_some_and(|odds| odds > 0)
                })
            })
            .collect();
        let mut ambient = vec![Vec::new(); biomes.list.len()];
        for (name, &id) in &biomes.index {
            let path = root.join(format!(
                "data/minecraft/worldgen/biome/{}.json",
                name.trim_start_matches("minecraft:")
            ));
            let Ok(bytes) = std::fs::read(path) else {
                continue;
            };
            let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                continue;
            };
            let list = &json["attributes"]["minecraft:visual/ambient_particles"];
            let entries = list["argument"].as_array().or_else(|| list.as_array());
            for entry in entries.into_iter().flatten() {
                let ty = entry["particle"]["type"].as_str().and_then(T::from_name);
                let probability = entry["probability"].as_f64().unwrap_or(0.0) as f32;
                if let (Some(ty), Some(slot)) = (ty, ambient.get_mut(id as usize)) {
                    slot.push((ty, probability));
                }
            }
        }
        let dripstone = (0..registry.len() as u32)
            .map(|id| {
                registry.state(BlockStateId(id)).map_or(0, |s| {
                    let name = s.name.strip_prefix("minecraft:").unwrap_or(&s.name);
                    if name != "pointed_dripstone" {
                        0
                    } else if s.properties.get("vertical_direction").map(String::as_str)
                        == Some("down")
                    {
                        2
                    } else {
                        1
                    }
                })
            })
            .collect();
        Ok(Self {
            kinds,
            dripstone,
            impermeable: by_state("minecraft:impermeable"),
            power_provider: by_state("minecraft:enchantment_power_provider"),
            power_transmitter: by_state("minecraft:enchantment_power_transmitter"),
            dried_ghast_sounds: by_state("minecraft:triggers_ambient_dried_ghast_block_sounds"),
            desert_sand_sounds: by_state("minecraft:triggers_ambient_desert_sand_block_sounds"),
            poplar_leaf_logs: by_state("minecraft:required_for_poplar_leaf_ambience"),
            desert_dry_vegetation_sounds: by_state(
                "minecraft:triggers_ambient_desert_dry_vegetation_block_sounds",
            ),
            terracotta: by_state("minecraft:terracotta"),
            pale_oak_logs: by_state("minecraft:pale_oak_logs"),
            burnable,
            ambient,
        })
    }
    fn kind(&self, state: BlockStateId) -> Animate {
        self.kinds
            .get(state.0 as usize)
            .copied()
            .unwrap_or(Animate::None)
    }
    /// Whether a state ticks campfire particles (`CampfireBlockEntity.particleTick`).
    pub fn is_lit_campfire(&self, state: u32) -> bool {
        matches!(self.kind(BlockStateId(state)), Animate::Campfire { .. })
    }
}

/// World access for ambient effects.
pub struct AnimateContext<'a> {
    pub world: &'a World,
    pub facts: &'a BlockFacts,
    pub collision: &'a BlockCollisionRegistry<'a>,
    pub biomes: &'a Biomes,
    pub registry: &'a BlockRegistry,
    /// `Level.isRainingAt`.
    pub raining_at: &'a dyn Fn([i32; 3]) -> bool,
    /// `getHeight(MOTION_BLOCKING_NO_LEAVES, x, z)` at a position's column.
    pub height_no_leaves: &'a dyn Fn([i32; 3]) -> i32,
    /// `EnvironmentAttributes.FIREFLY_BUSH_SOUNDS` (overworld day timeline).
    pub firefly_bush_sounds: bool,
    /// `EnvironmentAttributes.CREAKING_ACTIVE` (overworld day timeline).
    pub creaking_active: bool,
    /// `Level.getSkyDarken`, for `getMaxLocalRawBrightness`.
    pub sky_darken: i32,
    /// Creative main-hand marker block and a resolver for each placed state's sprite.
    pub marker: Option<(&'a str, &'a dyn Fn(BlockStateId) -> Option<ParticleOptions>)>,
    /// `BlockState.getOffset` of a state at (x, z), for blocks with a random model offset.
    pub model_offset: &'a dyn Fn(BlockStateId, i32, i32) -> Option<[f64; 3]>,
    /// `TheEndGatewayBlockEntity.getParticleAmount` of the gateway state at a position.
    pub gateway_particles: &'a dyn Fn(BlockStateId, [i32; 3]) -> usize,
    /// `Level.isThundering`.
    pub thundering: bool,
    /// The dimension has `water_evaporates` and drips lava from dripstone (the Nether).
    pub ultrawarm: bool,
}

/// `ClientLevel.getMarkerParticleTarget`: only the selected main-hand item counts.
pub fn marker_particle_target(
    mode: GameMode,
    main_hand: Option<&ItemStack>,
) -> Option<&'static str> {
    if mode != GameMode::Creative {
        return None;
    }
    let stack = main_hand.filter(|stack| stack.count > 0)?;
    match stack.item.as_ref() {
        "minecraft:barrier" => Some("minecraft:barrier"),
        "minecraft:light" => Some("minecraft:light"),
        _ => None,
    }
}
impl AnimateContext<'_> {
    fn state(&self, p: [i32; 3]) -> BlockStateId {
        self.world.block(p[0], p[1], p[2])
    }
    fn is_air(&self, p: [i32; 3]) -> bool {
        self.facts.is_air(self.state(p))
    }
    /// `isFaceSturdy` approximated by a full collision cube.
    fn sturdy(&self, p: [i32; 3]) -> bool {
        self.facts.get(self.state(p)).full_collision
    }
    fn shape_y(&self, p: [i32; 3]) -> Option<(f64, f64, [f64; 4])> {
        let shape = self
            .collision
            .cached_shapes()
            .cached_shape(self.state(p))
            .ok()??;
        if shape.is_empty() {
            return Some((0.0, 0.0, [0.0; 4]));
        }
        Some((
            shape.min(Axis::Y),
            shape.max(Axis::Y),
            [
                shape.min(Axis::X),
                shape.max(Axis::X),
                shape.min(Axis::Z),
                shape.max(Axis::Z),
            ],
        ))
    }
    /// `Level.getMaxLocalRawBrightness`: the brighter of block light and
    /// sky light less the sky darkening.
    fn max_local_raw_brightness(&self, p: [i32; 3]) -> i32 {
        let light = self.world.light(p[0], p[1], p[2]);
        i32::from(light & 15).max(i32::from(light >> 4) - self.sky_darken)
    }
    fn offset_of(&self, state: BlockStateId, p: [i32; 3]) -> [f64; 3] {
        (self.model_offset)(state, p[0], p[2]).unwrap_or([0.0; 3])
    }
    fn foliage(&self, p: [i32; 3]) -> u32 {
        let biome = self
            .world
            .chunks
            .get(&(p[0] >> 4, p[2] >> 4))
            .map_or(0, |c| c.biome(p[0], p[1], p[2]));
        self.biomes.get(biome).foliage
    }
}

fn add(engine: &mut ParticleEngine, ty: T, p: [f64; 3], v: [f64; 3]) {
    engine.add(&ParticleOptions::simple(ty), p, v);
}
/// `Level.addAlwaysVisibleParticle`: still shown under the Minimal option now and then.
fn add_visible(engine: &mut ParticleEngine, ty: T, p: [f64; 3], v: [f64; 3]) {
    engine.add_with(&ParticleOptions::simple(ty), p, v, false, true);
}
fn sound(engine: &mut ParticleEngine, name: &'static str, p: [f64; 3], volume: f32, pitch: f32) {
    engine.sounds.push((name, p, volume, pitch, false));
}
fn redstone(engine: &mut ParticleEngine, p: [f64; 3]) {
    engine.add(&ParticleOptions::dust([1.0, 0.0, 0.0], 1.0), p, [0.0; 3]);
}

/// `ClientLevel.animateTick` around the player's block position.
pub fn animate_tick(
    tables: &AnimateTables,
    engine: &mut ParticleEngine,
    ctx: &AnimateContext,
    player: [i32; 3],
) {
    for _ in 0..667 {
        do_animate_tick(tables, engine, ctx, player, 16);
        do_animate_tick(tables, engine, ctx, player, 32);
    }
}
fn next_int(r: &mut LegacyRandom, bound: i32) -> i32 {
    r.next_int_bound(bound).unwrap_or(0)
}
fn do_animate_tick(
    tables: &AnimateTables,
    engine: &mut ParticleEngine,
    ctx: &AnimateContext,
    center: [i32; 3],
    r: i32,
) {
    let random = engine.random();
    let x = center[0] + next_int(random, r) - next_int(random, r);
    let y = center[1] + next_int(random, r) - next_int(random, r);
    let z = center[2] + next_int(random, r) - next_int(random, r);
    let pos = [x, y, z];
    if !ctx.world.loaded(x, z) {
        return;
    }
    let state = ctx.state(pos);
    block_animate_tick(tables, engine, ctx, state, pos, center);
    let facts = ctx.facts.get(state);
    if facts.fluid != Fluid::None {
        fluid_animate_tick(
            engine,
            ctx,
            pos,
            facts.fluid,
            facts.fluid_level,
            facts.fluid_falling,
        );
        if next_int(engine.random(), 10) == 0 {
            let drip = if facts.fluid == Fluid::Water {
                T::DrippingWater
            } else {
                T::DrippingLava
            };
            let top_solid = ctx.sturdy(pos);
            try_spawn_drip(tables, engine, ctx, [x, y - 1, z], drip, top_solid);
        }
    }
    if let Some((block, options)) = ctx.marker
        && ctx.registry.state(state).is_some_and(|s| s.name == block)
        && let Some(options) = options(state)
    {
        engine.add(&options, pos.map(|v| f64::from(v) + 0.5), [0.0; 3]);
    }
    if !facts.full_collision {
        let biome = ctx
            .world
            .chunks
            .get(&(x >> 4, z >> 4))
            .map_or(0, |c| c.biome(x, y, z));
        for &(ty, probability) in tables.ambient.get(biome as usize).into_iter().flatten() {
            // AmbientParticle.canSpawn: random.nextFloat() <= probability.
            let random = engine.random();
            if random.next_float() <= probability {
                let p = [
                    f64::from(x) + random.next_double(),
                    f64::from(y) + random.next_double(),
                    f64::from(z) + random.next_double(),
                ];
                add(engine, ty, p, [0.0; 3]);
            }
        }
    }
}
/// `ClientLevel.trySpawnDripParticles`.
fn try_spawn_drip(
    tables: &AnimateTables,
    engine: &mut ParticleEngine,
    ctx: &AnimateContext,
    pos: [i32; 3],
    drip: T,
    top_solid: bool,
) {
    let state = ctx.state(pos);
    if ctx.facts.get(state).fluid != Fluid::None {
        return;
    }
    let Some((bottom, top, bounds)) = ctx.shape_y(pos) else {
        return;
    };
    let spawn = |engine: &mut ParticleEngine, x1: f64, x2: f64, z1: f64, z2: f64, y: f64| {
        let r = engine.random();
        let (a, b) = (r.next_double(), r.next_double());
        add(
            engine,
            drip,
            [x1 + (x2 - x1) * a, y, z1 + (z2 - z1) * b],
            [0.0; 3],
        );
    };
    let (fx, fy, fz) = (f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]));
    if top < 1.0 {
        if top_solid {
            spawn(engine, fx, fx + 1.0, fz, fz + 1.0, fy + 1.0 - 0.05);
        }
    } else if !tables
        .impermeable
        .get(state.0 as usize)
        .copied()
        .unwrap_or(false)
    {
        if bottom > 0.0 {
            spawn(
                engine,
                fx + bounds[0],
                fx + bounds[1],
                fz + bounds[2],
                fz + bounds[3],
                fy + bottom - 0.05,
            );
        } else {
            let below = [pos[0], pos[1] - 1, pos[2]];
            let below_top = ctx.shape_y(below).map_or(1.0, |s| s.1);
            if below_top < 1.0 && ctx.facts.get(ctx.state(below)).fluid == Fluid::None {
                spawn(
                    engine,
                    fx + bounds[0],
                    fx + bounds[1],
                    fz + bounds[2],
                    fz + bounds[3],
                    fy - 0.05,
                );
            }
        }
    }
}
/// `BeehiveBlock.trySpawnDripParticles`.
fn beehive_drip(
    tables: &AnimateTables,
    engine: &mut ParticleEngine,
    ctx: &AnimateContext,
    state: BlockStateId,
    pos: [i32; 3],
) {
    if ctx.facts.get(state).fluid != Fluid::None || engine.random().next_float() < 0.3 {
        return;
    }
    let Some((bottom, top, bounds)) = ctx.shape_y(pos) else {
        return;
    };
    if top < 1.0
        || tables
            .impermeable
            .get(state.0 as usize)
            .copied()
            .unwrap_or(false)
    {
        return;
    }
    let y = if bottom > 0.0 {
        f64::from(pos[1]) + bottom - 0.05
    } else {
        let below = [pos[0], pos[1] - 1, pos[2]];
        let below_top = ctx.shape_y(below).map_or(1.0, |s| s.1);
        // `belowTop < 1 || !isCollisionShapeFullBlock` and no fluid below.
        if (below_top < 1.0 || !ctx.sturdy(below))
            && ctx.facts.get(ctx.state(below)).fluid == Fluid::None
        {
            f64::from(pos[1]) - 0.05
        } else {
            return;
        }
    };
    let r = engine.random();
    let (a, b) = (r.next_double(), r.next_double());
    let (fx, fz) = (f64::from(pos[0]), f64::from(pos[2]));
    let p = [
        fx + bounds[0] + (bounds[1] - bounds[0]) * a,
        y,
        fz + bounds[2] + (bounds[3] - bounds[2]) * b,
    ];
    add(engine, T::DrippingHoney, p, [0.0; 3]);
}
/// `AmbientDesertBlockSoundsPlayer.shouldPlayDesertDryVegetationBlockSounds`:
/// sand or terracotta two blocks deep.
fn desert_dry_vegetation_sounds(
    tables: &AnimateTables,
    ctx: &AnimateContext,
    below: [i32; 3],
) -> bool {
    let triggers = |p: [i32; 3]| {
        tables
            .desert_dry_vegetation_sounds
            .get(ctx.state(p).0 as usize)
            .copied()
            .unwrap_or(false)
    };
    triggers(below) && triggers([below[0], below[1] - 1, below[2]])
}
/// `WaterFluid.animateTick` / `LavaFluid.animateTick`.
fn fluid_animate_tick(
    engine: &mut ParticleEngine,
    ctx: &AnimateContext,
    pos: [i32; 3],
    fluid: Fluid,
    level: u8,
    falling: bool,
) {
    let (fx, fy, fz) = (f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]));
    if fluid == Fluid::Water {
        let source = level >= 8 && !falling;
        let random = engine.random();
        if !source && !falling {
            if next_int(random, 64) == 0 {
                let volume = random.next_float() * 0.25 + 0.75;
                let pitch = random.next_float() + 0.5;
                sound(
                    engine,
                    "minecraft:block.water.ambient",
                    [fx + 0.5, fy + 0.5, fz + 0.5],
                    volume,
                    pitch,
                );
            }
        } else if next_int(random, 10) == 0 {
            let p = [
                fx + random.next_double(),
                fy + random.next_double(),
                fz + random.next_double(),
            ];
            add(engine, T::Underwater, p, [0.0; 3]);
        }
        return;
    }
    let above = [pos[0], pos[1] + 1, pos[2]];
    if ctx.is_air(above) {
        let random = engine.random();
        if next_int(random, 100) == 0 {
            let p = [
                fx + random.next_double(),
                fy + 1.0,
                fz + random.next_double(),
            ];
            add(engine, T::Lava, p, [0.0; 3]);
            let random = engine.random();
            let (volume, pitch) = (
                0.2 + random.next_float() * 0.2,
                0.9 + random.next_float() * 0.15,
            );
            sound(engine, "minecraft:block.lava.pop", p, volume, pitch);
        }
        let random = engine.random();
        if next_int(random, 200) == 0 {
            let (volume, pitch) = (
                0.2 + random.next_float() * 0.2,
                0.9 + random.next_float() * 0.15,
            );
            sound(
                engine,
                "minecraft:block.lava.ambient",
                [fx, fy, fz],
                volume,
                pitch,
            );
        }
    }
}
#[allow(clippy::too_many_lines)]
fn block_animate_tick(
    tables: &AnimateTables,
    engine: &mut ParticleEngine,
    ctx: &AnimateContext,
    state: BlockStateId,
    pos: [i32; 3],
    player: [i32; 3],
) {
    let (fx, fy, fz) = (f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]));
    match tables.kind(state) {
        Animate::None => {}
        Animate::Torch(flame) => {
            let p = [fx + 0.5, fy + 0.7, fz + 0.5];
            add(engine, T::Smoke, p, [0.0; 3]);
            add(engine, flame, p, [0.0; 3]);
        }
        Animate::WallTorch(flame, facing) => {
            let p = [
                fx + 0.5 - 0.27 * f64::from(facing[0]),
                fy + 0.7 + 0.22,
                fz + 0.5 - 0.27 * f64::from(facing[2]),
            ];
            add(engine, T::Smoke, p, [0.0; 3]);
            add(engine, flame, p, [0.0; 3]);
        }
        Animate::RedstoneTorch => {
            let r = engine.random();
            let p = [
                fx + 0.5 + (r.next_double() - 0.5) * 0.2,
                fy + 0.7 + (r.next_double() - 0.5) * 0.2,
                fz + 0.5 + (r.next_double() - 0.5) * 0.2,
            ];
            redstone(engine, p);
        }
        Animate::RedstoneWallTorch(facing) => {
            let r = engine.random();
            let p = [
                fx + 0.5 + (r.next_double() - 0.5) * 0.2 - 0.27 * f64::from(facing[0]),
                fy + 0.7 + (r.next_double() - 0.5) * 0.2 + 0.22,
                fz + 0.5 + (r.next_double() - 0.5) * 0.2 - 0.27 * f64::from(facing[2]),
            ];
            redstone(engine, p);
        }
        Animate::Furnace(facing) | Animate::BlastFurnace(facing) => {
            let blast = matches!(tables.kind(state), Animate::BlastFurnace(_));
            let (x, y, z) = (fx + 0.5, fy, fz + 0.5);
            let r = engine.random();
            if r.next_double() < 0.1 {
                let name = if blast {
                    "minecraft:block.blastfurnace.fire_crackle"
                } else {
                    "minecraft:block.furnace.fire_crackle"
                };
                sound(engine, name, [x, y, z], 1.0, 1.0);
            }
            let r = engine.random();
            let ss = r.next_double() * 0.6 - 0.3;
            let dx = if facing[0] != 0 {
                f64::from(facing[0]) * 0.52
            } else {
                ss
            };
            let dy = r.next_double() * if blast { 9.0 } else { 6.0 } / 16.0;
            let dz = if facing[2] != 0 {
                f64::from(facing[2]) * 0.52
            } else {
                ss
            };
            let p = [x + dx, y + dy, z + dz];
            add(engine, T::Smoke, p, [0.0; 3]);
            if !blast {
                add(engine, T::Flame, p, [0.0; 3]);
            }
        }
        Animate::Smoker => {
            let r = engine.random();
            if r.next_double() < 0.1 {
                sound(
                    engine,
                    "minecraft:block.smoker.smoke",
                    [fx + 0.5, fy, fz + 0.5],
                    1.0,
                    1.0,
                );
            }
            add(engine, T::Smoke, [fx + 0.5, fy + 1.1, fz + 0.5], [0.0; 3]);
        }
        Animate::Campfire { spawn_particles } => {
            let r = engine.random();
            if next_int(r, 10) == 0 {
                let (volume, pitch) = (0.5 + r.next_float(), r.next_float() * 0.7 + 0.6);
                sound(
                    engine,
                    "minecraft:block.campfire.crackle",
                    [fx + 0.5, fy + 0.5, fz + 0.5],
                    volume,
                    pitch,
                );
            }
            let r = engine.random();
            if spawn_particles && next_int(r, 5) == 0 {
                let count = next_int(r, 1) + 1;
                for _ in 0..count {
                    let r = engine.random();
                    let v = [
                        f64::from(r.next_float() / 2.0),
                        5.0e-5,
                        f64::from(r.next_float() / 2.0),
                    ];
                    add(engine, T::Lava, [fx + 0.5, fy + 0.5, fz + 0.5], v);
                }
            }
        }
        Animate::Leaves(falling, fixed_tint, poplar) => {
            let below = [pos[0], pos[1] - 1, pos[2]];
            // LeavesBlock.makeDrippingWaterParticles.
            if (ctx.raining_at)([pos[0], pos[1] + 1, pos[2]]) && next_int(engine.random(), 15) == 1
            {
                let below_facts = ctx.facts.get(ctx.state(below));
                if !below_facts.can_occlude || !ctx.sturdy(below) {
                    let r = engine.random();
                    let p = [fx + r.next_double(), fy - 0.05, fz + r.next_double()];
                    add(engine, T::DrippingWater, p, [0.0; 3]);
                }
            }
            if poplar {
                leaves_ambient_sound(tables, engine, ctx, state, pos);
            }
            // FallingParticlesLeavesBlock.makeFallingLeavesParticles.
            if let Some((ty, chance)) = falling
                && engine.random().next_float() < chance
                && !ctx.sturdy(below)
            {
                let r = engine.random();
                let p = [fx + r.next_double(), fy - 0.05, fz + r.next_double()];
                let options = if ty == T::TintedLeaves {
                    let c = fixed_tint.unwrap_or_else(|| ctx.foliage(pos));
                    let f = |s: u32| ((c >> s) & 255) as f32 / 255.0;
                    ParticleOptions::colored(ty, [f(16), f(8), f(0), 1.0])
                } else {
                    ParticleOptions::simple(ty)
                };
                engine.add(&options, p, [0.0; 3]);
            }
        }
        Animate::NetherPortal => {
            let r = engine.random();
            if next_int(r, 100) == 0 {
                let pitch = r.next_float() * 0.4 + 0.8;
                sound(
                    engine,
                    "minecraft:block.portal.ambient",
                    [fx + 0.5, fy + 0.5, fz + 0.5],
                    0.5,
                    pitch,
                );
            }
            let along_z = ctx.state([pos[0] - 1, pos[1], pos[2]]) == state
                || ctx.state([pos[0] + 1, pos[1], pos[2]]) == state;
            for _ in 0..4 {
                let r = engine.random();
                let mut p = [
                    fx + r.next_double(),
                    fy + r.next_double(),
                    fz + r.next_double(),
                ];
                let mut v = [
                    f64::from(r.next_float() - 0.5) * 0.5,
                    f64::from(r.next_float() - 0.5) * 0.5,
                    f64::from(r.next_float() - 0.5) * 0.5,
                ];
                let flip = f64::from(next_int(r, 2) * 2 - 1);
                if along_z {
                    p[2] = fz + 0.5 + 0.25 * flip;
                    v[2] = f64::from(r.next_float() * 2.0) * flip;
                } else {
                    p[0] = fx + 0.5 + 0.25 * flip;
                    v[0] = f64::from(r.next_float() * 2.0) * flip;
                }
                add(engine, T::Portal, p, v);
            }
        }
        Animate::EnchantingTable => {
            for ox in -2i32..=2 {
                for oy in 0..=1 {
                    for oz in -2i32..=2 {
                        if ox.abs() != 2 && oz.abs() != 2 {
                            continue;
                        }
                        if next_int(engine.random(), 16) != 0 {
                            continue;
                        }
                        let provider = ctx.state([pos[0] + ox, pos[1] + oy, pos[2] + oz]);
                        let between = ctx.state([pos[0] + ox / 2, pos[1] + oy, pos[2] + oz / 2]);
                        if tables
                            .power_provider
                            .get(provider.0 as usize)
                            .copied()
                            .unwrap_or(false)
                            && tables
                                .power_transmitter
                                .get(between.0 as usize)
                                .copied()
                                .unwrap_or(false)
                        {
                            let r = engine.random();
                            let v = [
                                f64::from(ox as f32 + r.next_float() - 0.5),
                                f64::from(oy as f32 - r.next_float() - 1.0),
                                f64::from(oz as f32 + r.next_float() - 0.5),
                            ];
                            add(engine, T::Enchant, [fx + 0.5, fy + 2.0, fz + 0.5], v);
                        }
                    }
                }
            }
        }
        Animate::EndRod(facing) => {
            let r = engine.random();
            let x = fx + 0.55 - f64::from(r.next_float() * 0.1);
            let y = fy + 0.55 - f64::from(r.next_float() * 0.1);
            let z = fz + 0.55 - f64::from(r.next_float() * 0.1);
            let d = f64::from(0.4 - (r.next_float() + r.next_float()) * 0.4);
            if next_int(r, 5) == 0 {
                let v = [
                    r.next_gaussian() * 0.005,
                    r.next_gaussian() * 0.005,
                    r.next_gaussian() * 0.005,
                ];
                let p = [
                    x + f64::from(facing[0]) * d,
                    y + f64::from(facing[1]) * d,
                    z + f64::from(facing[2]) * d,
                ];
                add(engine, T::EndRod, p, v);
            }
        }
        Animate::SporeBlossom => {
            let r = engine.random();
            let p = [fx + r.next_double(), fy + 0.7, fz + r.next_double()];
            add(engine, T::FallingSporeBlossom, p, [0.0; 3]);
            for _ in 0..14 {
                let r = engine.random();
                let a = [
                    pos[0] + next_int(r, 21) - 10,
                    pos[1] - next_int(r, 10),
                    pos[2] + next_int(r, 21) - 10,
                ];
                if !ctx.facts.get(ctx.state(a)).full_collision {
                    let r = engine.random();
                    let p = [
                        f64::from(a[0]) + r.next_double(),
                        f64::from(a[1]) + r.next_double(),
                        f64::from(a[2]) + r.next_double(),
                    ];
                    add(engine, T::SporeBlossomAir, p, [0.0; 3]);
                }
            }
        }
        Animate::CryingObsidian => {
            let r = engine.random();
            if next_int(r, 5) == 0 {
                // Direction.getRandom: down, up, north, south, west, east.
                let d = [
                    [0, -1, 0],
                    [0, 1, 0],
                    [0, 0, -1],
                    [0, 0, 1],
                    [-1, 0, 0],
                    [1, 0, 0],
                ][next_int(r, 6) as usize];
                if d != [0, 1, 0] {
                    let relative = [pos[0] + d[0], pos[1] + d[1], pos[2] + d[2]];
                    if !ctx.sturdy(relative) {
                        let r = engine.random();
                        let off = |s: i32, r: &mut LegacyRandom| {
                            if s == 0 {
                                r.next_double()
                            } else {
                                0.5 + f64::from(s) * 0.6
                            }
                        };
                        let p = [fx + off(d[0], r), fy + off(d[1], r), fz + off(d[2], r)];
                        add(engine, T::DrippingObsidianTear, p, [0.0; 3]);
                    }
                }
            }
        }
        Animate::PotentSulfur => {
            // PotentSulfurBlock.animateTick: under a water source, two
            // bubbles and, one time in ten, the noxious gas.
            let above = ctx.facts.get(ctx.state([pos[0], pos[1] + 1, pos[2]]));
            if above.fluid == Fluid::Water && above.fluid_level == 8 && !above.fluid_falling {
                for _ in 0..2 {
                    let r = engine.random();
                    let p = [
                        fx + f64::from(r.next_float()),
                        fy + 1.0 + f64::from(r.next_float()),
                        fz + f64::from(r.next_float()),
                    ];
                    add_visible(engine, T::SulfurBubbles, p, [0.0; 3]);
                }
                if next_int(engine.random(), 10) == 0 {
                    let name = "minecraft:block.potent_sulfur.noxious_gas";
                    sound(engine, name, [fx, fy, fz], 1.0, 1.0);
                }
            }
        }
        Animate::WetSponge => {
            // Direction.getRandom: DOWN, UP, NORTH, SOUTH, WEST, EAST.
            const DIRECTIONS: [[i32; 3]; 6] = [
                [0, -1, 0],
                [0, 1, 0],
                [0, 0, -1],
                [0, 0, 1],
                [-1, 0, 0],
                [1, 0, 0],
            ];
            let d = DIRECTIONS[next_int(engine.random(), 6) as usize];
            // The sponge occludes, so only a sturdy neighbour face hides the drip.
            if d != [0, 1, 0] && !ctx.sturdy([pos[0] + d[0], pos[1] + d[1], pos[2] + d[2]]) {
                let r = engine.random();
                let p = match d {
                    [0, -1, 0] => [fx + r.next_double(), fy - 0.05, fz + r.next_double()],
                    [dx, _, 0] => {
                        let y = fy + r.next_double() * 0.8;
                        let z = fz + r.next_double();
                        [fx + if dx > 0 { 1.1 } else { 0.05 }, y, z]
                    }
                    [_, _, dz] => {
                        let y = fy + r.next_double() * 0.8;
                        let x = fx + r.next_double();
                        [x, y, fz + if dz > 0 { 1.1 } else { 0.05 }]
                    }
                };
                add(engine, T::DrippingWater, p, [0.0; 3]);
            }
        }
        Animate::Mycelium => {
            let r = engine.random();
            if next_int(r, 10) == 0 {
                let p = [fx + r.next_double(), fy + 1.1, fz + r.next_double()];
                add(engine, T::Mycelium, p, [0.0; 3]);
            }
        }
        Animate::Candles(count) => {
            for o in candle_offsets(count) {
                candle(
                    engine,
                    [fx + o[0] / 16.0, fy + o[1] / 16.0, fz + o[2] / 16.0],
                );
            }
        }
        Animate::CandleCake => candle(engine, [fx + 0.5, fy + 1.0, fz + 0.5]),
        Animate::BrewingStand => {
            let r = engine.random();
            let p = [
                fx + 0.4 + f64::from(r.next_float() * 0.2),
                fy + 0.7 + f64::from(r.next_float() * 0.3),
                fz + 0.4 + f64::from(r.next_float() * 0.2),
            ];
            add(engine, T::Smoke, p, [0.0; 3]);
        }
        Animate::DriedGhast { waterlogged } => {
            // DriedGhastBlock.animateTick.
            let c = [fx + 0.5, fy + 0.5, fz + 0.5];
            if waterlogged {
                if next_int(engine.random(), 40) == 0 {
                    let name = "minecraft:block.dried_ghast.ambient_water";
                    sound(engine, name, c, 1.0, 1.0);
                }
                let r = engine.random();
                if next_int(r, 6) == 0 {
                    let x = c[0] + f64::from((r.next_float() * 2.0 - 1.0) / 3.0);
                    let y = c[1] + 0.4;
                    let z = c[2] + f64::from((r.next_float() * 2.0 - 1.0) / 3.0);
                    let v = [0.0, f64::from(r.next_float()), 0.0];
                    add(engine, T::HappyVillager, [x, y, z], v);
                }
            } else {
                let below = ctx.state([pos[0], pos[1] - 1, pos[2]]);
                let triggers = || {
                    tables
                        .dried_ghast_sounds
                        .get(below.0 as usize)
                        .copied()
                        .unwrap_or(false)
                };
                if next_int(engine.random(), 40) == 0 && triggers() {
                    sound(engine, "minecraft:block.dried_ghast.ambient", c, 1.0, 1.0);
                }
                if next_int(engine.random(), 6) == 0 {
                    add(engine, T::WhiteSmoke, c, [0.0, 0.02, 0.0]);
                }
            }
        }
        Animate::FireflyBush => {
            // FireflyBushBlock.animateTick: the idle sound on a quiet night
            // under open sky, and fireflies in the dark within 5 blocks.
            if next_int(engine.random(), 30) == 0
                && ctx.firefly_bush_sounds
                && (ctx.height_no_leaves)(pos) <= pos[1]
            {
                let name = "minecraft:block.firefly_bush.idle";
                sound(engine, name, [fx, fy, fz], 1.0, 1.0);
            }
            if ctx.max_local_raw_brightness(pos) <= 13 {
                let r = engine.random();
                if r.next_double() <= 0.7 {
                    let p = [
                        fx + r.next_double() * 10.0 - 5.0,
                        fy + r.next_double() * 5.0,
                        fz + r.next_double() * 10.0 - 5.0,
                    ];
                    add(engine, T::Firefly, p, [0.0; 3]);
                }
            }
        }
        Animate::OpenEyeblossom => {
            // EyeblossomBlock.animateTick: pale moss below.
            if next_int(engine.random(), 700) == 0 {
                let below = ctx.state([pos[0], pos[1] - 1, pos[2]]);
                if ctx
                    .registry
                    .state(below)
                    .is_some_and(|s| s.name == "minecraft:pale_moss_block")
                {
                    let name = "minecraft:block.eyeblossom.idle";
                    sound(engine, name, [fx, fy, fz], 1.0, 1.0);
                }
            }
        }
        Animate::PaleHangingMoss => {
            // HangingMossBlock.animateTick: under pale oak logs or leaves.
            if next_int(engine.random(), 500) == 0 {
                let above = ctx.state([pos[0], pos[1] + 1, pos[2]]);
                let pale_leaves = ctx
                    .registry
                    .state(above)
                    .is_some_and(|s| s.name == "minecraft:pale_oak_leaves");
                if pale_leaves
                    || tables
                        .pale_oak_logs
                        .get(above.0 as usize)
                        .copied()
                        .unwrap_or(false)
                {
                    let name = "minecraft:block.pale_hanging_moss.idle";
                    sound(engine, name, [fx, fy, fz], 1.0, 1.0);
                }
            }
        }
        Animate::CreakingHeart => {
            // CreakingHeartBlock.animateTick: the idle creak is drawn only while
            // the attribute is on, then needs pale oak logs on all six sides
            // (`CreakingHeartBlock.isSurroundedByLogs`).
            if ctx.creaking_active
                && next_int(engine.random(), 16) == 0
                && [
                    [-1, 0, 0],
                    [1, 0, 0],
                    [0, -1, 0],
                    [0, 1, 0],
                    [0, 0, -1],
                    [0, 0, 1],
                ]
                .iter()
                .all(|d| {
                    let p = [pos[0] + d[0], pos[1] + d[1], pos[2] + d[2]];
                    tables
                        .pale_oak_logs
                        .get(ctx.state(p).0 as usize)
                        .copied()
                        .unwrap_or(false)
                })
            {
                sound(
                    engine,
                    "minecraft:block.creaking_heart.idle",
                    [fx, fy, fz],
                    1.0,
                    1.0,
                );
            }
        }
        Animate::DeadBush => {
            // AmbientDesertBlockSoundsPlayer.playAmbientDeadBushSounds.
            if next_int(engine.random(), 130) == 0 {
                let below = [pos[0], pos[1] - 1, pos[2]];
                let state = ctx.state(below);
                let red_sand = ctx
                    .registry
                    .state(state)
                    .is_some_and(|s| s.name == "minecraft:red_sand");
                let terracotta = tables
                    .terracotta
                    .get(state.0 as usize)
                    .copied()
                    .unwrap_or(false);
                if (red_sand || terracotta) && next_int(engine.random(), 3) != 0 {
                    return;
                }
                if desert_dry_vegetation_sounds(tables, ctx, below) {
                    let name = "minecraft:block.deadbush.idle";
                    sound(engine, name, [fx, fy, fz], 1.0, 1.0);
                }
            }
        }
        Animate::DryGrass => {
            // AmbientDesertBlockSoundsPlayer.playAmbientDryGrassSounds:
            // played at the player.
            if next_int(engine.random(), 200) == 0
                && desert_dry_vegetation_sounds(tables, ctx, [pos[0], pos[1] - 1, pos[2]])
            {
                let at = [
                    f64::from(player[0]) + 0.5,
                    f64::from(player[1]),
                    f64::from(player[2]) + 0.5,
                ];
                sound(engine, "minecraft:block.dry_grass.ambient", at, 1.0, 1.0);
            }
        }
        Animate::EndPortal => {
            // EndPortalBlock.animateTick: one smoke particle above the surface.
            let r = engine.random();
            let p = [fx + r.next_double(), fy + 0.8, fz + r.next_double()];
            add(engine, T::Smoke, p, [0.0; 3]);
        }
        Animate::Lever { facing, connected } => {
            // LeverBlock.animateTick: a powered lever puffs red dust a quarter of the time.
            if engine.random().next_float() < 0.25 {
                // LeverBlock.makeParticle with scale 0.5.
                let at = |base: f64, i: usize| {
                    base + 0.5 + 0.1 * f64::from(-facing[i]) + 0.2 * f64::from(-connected[i])
                };
                let p = [at(fx, 0), at(fy, 1), at(fz, 2)];
                engine.add(&ParticleOptions::dust([1.0, 0.0, 0.0], 0.5), p, [0.0; 3]);
            }
        }
        Animate::Repeater { facing, delay } => {
            // RepeaterBlock.animateTick: dust at one of the torches.
            let r = engine.random();
            let x = fx + 0.5 + (r.next_double() - 0.5) * 0.2;
            let y = fy + 0.4 + (r.next_double() - 0.5) * 0.2;
            let z = fz + 0.5 + (r.next_double() - 0.5) * 0.2;
            let mut offset = -5.0f32;
            if r.next_boolean() {
                offset = (delay * 2 - 1) as f32;
            }
            offset /= 16.0;
            let xo = f64::from(offset * facing[0] as f32);
            let zo = f64::from(offset * facing[2] as f32);
            redstone(engine, [x + xo, y, z + zo]);
        }
        Animate::PointedDripstone => dripstone_drip(tables, engine, ctx, state, pos),
        Animate::WitherRose => {
            // WitherRoseBlock.animateTick: smoke puffs at the centre of the shape.
            let center = ctx.shape_y(pos).map_or([0.5; 2], |s| {
                [(s.2[0] + s.2[1]) / 2.0, (s.2[2] + s.2[3]) / 2.0]
            });
            let offset = ctx.offset_of(state, pos);
            let x = fx + center[0] + offset[0];
            let z = fz + center[1] + offset[2];
            for _ in 0..3 {
                let r = engine.random();
                if r.next_boolean() {
                    let p = [
                        x + r.next_double() / 5.0,
                        fy + (0.5 - r.next_double()),
                        z + r.next_double() / 5.0,
                    ];
                    add(engine, T::Smoke, p, [0.0; 3]);
                }
            }
        }
        Animate::EndGateway => {
            // EndGatewayBlock.animateTick: portal particles, one for each visible face.
            for _ in 0..(ctx.gateway_particles)(state, pos) {
                let r = engine.random();
                let mut p = [
                    fx + r.next_double(),
                    fy + r.next_double(),
                    fz + r.next_double(),
                ];
                let mut v = [
                    (r.next_double() - 0.5) * 0.5,
                    (r.next_double() - 0.5) * 0.5,
                    (r.next_double() - 0.5) * 0.5,
                ];
                let flip = next_int(r, 2) * 2 - 1;
                if r.next_boolean() {
                    p[2] = fz + 0.5 + 0.25 * f64::from(flip);
                    v[2] = f64::from(r.next_float() * 2.0 * flip as f32);
                } else {
                    p[0] = fx + 0.5 + 0.25 * f64::from(flip);
                    v[0] = f64::from(r.next_float() * 2.0 * flip as f32);
                }
                add(engine, T::Portal, p, v);
            }
        }
        Animate::LightningRod(axis) => {
            // LightningRodBlock.animateTick: in a thunderstorm, sparks along the
            // topmost rod, more often as the game time within 200 ticks grows.
            if ctx.thundering
                && i64::from(next_int(engine.level_random(), 200)) <= ctx.world.game_time % 200
                && pos[1]
                    == ctx
                        .world
                        .world_surface(pos[0], pos[2], |id| ctx.facts.is_air(id))
                        - 1
            {
                let mut sparks = Vec::new();
                along_axis(engine.level_random(), axis, pos, (1, 2), |p, v| {
                    sparks.push((p, v));
                });
                let options = ParticleOptions::simple(T::ElectricSpark);
                for (p, v) in sparks {
                    engine.add(&options, p, v);
                }
            }
        }
        Animate::EnderChest => {
            for _ in 0..3 {
                let r = engine.random();
                let flip_x = f64::from(next_int(r, 2) * 2 - 1);
                let flip_z = f64::from(next_int(r, 2) * 2 - 1);
                let p = [
                    fx + 0.5 + 0.25 * flip_x,
                    fy + f64::from(r.next_float()),
                    fz + 0.5 + 0.25 * flip_z,
                ];
                let v = [
                    f64::from(r.next_float()) * flip_x,
                    f64::from(r.next_float() - 0.5) * 0.125,
                    f64::from(r.next_float()) * flip_z,
                ];
                add(engine, T::Portal, p, v);
            }
        }
        Animate::Falling(color, sand) => {
            if next_int(engine.random(), 16) == 0 {
                let below = ctx.facts.get(ctx.state([pos[0], pos[1] - 1, pos[2]]));
                // FallingBlock.isFree: air, fire, liquids or replaceable.
                if below.replaceable
                    || below.fluid != Fluid::None
                    || ctx.is_air([pos[0], pos[1] - 1, pos[2]])
                {
                    let r = engine.random();
                    let p = [fx + r.next_double(), fy - 0.05, fz + r.next_double()];
                    engine.add(
                        &ParticleOptions::block(T::FallingDust, String::new(), color),
                        p,
                        [0.0; 3],
                    );
                }
            }
            // AmbientDesertBlockSoundsPlayer.playAmbientSandSounds: only for the air block.
            if sand
                && ctx.state([pos[0], pos[1] + 1, pos[2]]) == ctx.world.air
                && next_int(engine.random(), 2100) == 0
                && sand_sound_nearby(tables, ctx, pos)
            {
                sound(engine, "minecraft:block.sand.idle", [fx, fy, fz], 1.0, 1.0);
            }
        }
        Animate::Beehive => {
            // BeehiveBlock.animateTick: nextInt(1) + 1 attempts.
            let attempts = next_int(engine.random(), 1) + 1;
            for _ in 0..attempts {
                beehive_drip(tables, engine, ctx, state, pos);
            }
        }
        Animate::RedstoneOre => {
            for d in [
                [0, -1, 0],
                [0, 1, 0],
                [0, 0, -1],
                [0, 0, 1],
                [-1, 0, 0],
                [1, 0, 0],
            ] {
                let relative = [pos[0] + d[0], pos[1] + d[1], pos[2] + d[2]];
                if ctx.facts.get(ctx.state(relative)).solid_render {
                    continue;
                }
                let r = engine.random();
                let mut c = |s: i32| {
                    if s != 0 {
                        0.5 + 0.5625 * f64::from(s)
                    } else {
                        f64::from(r.next_float())
                    }
                };
                let p = [fx + c(d[0]), fy + c(d[1]), fz + c(d[2])];
                redstone(engine, p);
            }
        }
        Animate::RedstoneWire { power, sides } => {
            let color = redstone_color(power);
            let horizontals = [[0, 0, -1], [1, 0, 0], [0, 0, 1], [-1, 0, 0]];
            let line = |engine: &mut ParticleEngine,
                        side: [i32; 3],
                        along: [i32; 3],
                        from: f32,
                        to: f32| {
                let span = to - from;
                let r = engine.random();
                if r.next_float() < 0.2 * span {
                    let on = from + span * r.next_float();
                    let c = |i: usize| {
                        0.5 + 0.4375 * f64::from(side[i]) + f64::from(on) * f64::from(along[i])
                    };
                    engine.add(
                        &ParticleOptions::dust(color, 1.0),
                        [fx + c(0), fy + c(1), fz + c(2)],
                        [0.0; 3],
                    );
                }
            };
            for (i, h) in horizontals.into_iter().enumerate() {
                match sides[i] {
                    2 => {
                        line(engine, h, [0, 1, 0], -0.5, 0.5);
                        line(engine, [0, -1, 0], h, 0.0, 0.5);
                    }
                    1 => line(engine, [0, -1, 0], h, 0.0, 0.5),
                    _ => line(engine, [0, -1, 0], h, 0.0, 0.3),
                }
            }
        }
        Animate::SculkSensor => {
            // SculkSensorBlock.animateTick.
            let r = engine.random();
            let direction = r.next_int_bound(6).unwrap_or(0);
            // Direction.values(): down, up, north, south, west, east.
            if direction >= 2 {
                let step = [[0, 0, -1], [0, 0, 1], [-1, 0, 0], [1, 0, 0]][direction as usize - 2];
                let x = fx
                    + 0.5
                    + if step[0] == 0 {
                        0.5 - r.next_double()
                    } else {
                        f64::from(step[0]) * 0.6
                    };
                let z = fz
                    + 0.5
                    + if step[2] == 0 {
                        0.5 - r.next_double()
                    } else {
                        f64::from(step[2]) * 0.6
                    };
                let ya = f64::from(r.next_float() * 0.04);
                engine.add(
                    &crate::particles::ParticleOptions::sculk_to_redstone(),
                    [x, fy + 0.25, z],
                    [0.0, ya, 0.0],
                );
            }
        }
        Animate::RespawnAnchor => {
            let r = engine.random();
            if next_int(r, 100) == 0 {
                sound(
                    engine,
                    "minecraft:block.respawn_anchor.ambient",
                    [fx + 0.5, fy + 0.5, fz + 0.5],
                    1.0,
                    1.0,
                );
            }
            let r = engine.random();
            let p = [
                fx + 0.5 + (0.5 - r.next_double()),
                fy + 1.0,
                fz + 0.5 + (0.5 - r.next_double()),
            ];
            let ya = f64::from(r.next_float() * 0.04);
            add(engine, T::ReversePortal, p, [0.0, ya, 0.0]);
        }
        Animate::BubbleColumn { drag } => {
            if drag {
                add_visible(engine, T::CurrentDown, [fx + 0.5, fy + 0.8, fz], [0.0; 3]);
                let r = engine.random();
                if next_int(r, 200) == 0 {
                    let (volume, pitch) = (0.2 + r.next_float() * 0.2, 0.9 + r.next_float() * 0.15);
                    sound(
                        engine,
                        "minecraft:block.bubble_column.whirlpool_ambient",
                        [fx, fy, fz],
                        volume,
                        pitch,
                    );
                }
            } else {
                add_visible(
                    engine,
                    T::BubbleColumnUp,
                    [fx + 0.5, fy, fz + 0.5],
                    [0.0, 0.04, 0.0],
                );
                let r = engine.random();
                let p = [
                    fx + f64::from(r.next_float()),
                    fy + f64::from(r.next_float()),
                    fz + f64::from(r.next_float()),
                ];
                add_visible(engine, T::BubbleColumnUp, p, [0.0, 0.04, 0.0]);
                let r = engine.random();
                if next_int(r, 200) == 0 {
                    let (volume, pitch) = (0.2 + r.next_float() * 0.2, 0.9 + r.next_float() * 0.15);
                    sound(
                        engine,
                        "minecraft:block.bubble_column.upwards_ambient",
                        [fx, fy, fz],
                        volume,
                        pitch,
                    );
                }
            }
        }
        Animate::Fire { soul } => {
            let r = engine.random();
            if next_int(r, 24) == 0 {
                let (volume, pitch) = (1.0 + r.next_float(), r.next_float() * 0.7 + 0.3);
                sound(
                    engine,
                    "minecraft:block.fire.ambient",
                    [fx + 0.5, fy + 0.5, fz + 0.5],
                    volume,
                    pitch,
                );
            }
            // BaseFireBlock.animateTick: three large smoke above the fire, or
            // two at each burnable neighbour's face when nothing burnable or
            // sturdy supports it (SoulFireBlock.canBurn is always true).
            let can_burn = |p: [i32; 3]| {
                soul || tables
                    .burnable
                    .get(ctx.state(p).0 as usize)
                    .copied()
                    .unwrap_or(false)
            };
            let below = [pos[0], pos[1] - 1, pos[2]];
            let sturdy = ctx.facts.get(ctx.state(below)).sturdy_full & (1 << 1) != 0;
            if !can_burn(below) && !sturdy {
                // (neighbour, coordinate axis, face at the axis' high end): the
                // west, east, north, south and above faces, in vanilla order.
                for (at, axis, high) in [
                    ([pos[0] - 1, pos[1], pos[2]], 0usize, false),
                    ([pos[0] + 1, pos[1], pos[2]], 0, true),
                    ([pos[0], pos[1], pos[2] - 1], 2, false),
                    ([pos[0], pos[1], pos[2] + 1], 2, true),
                    ([pos[0], pos[1] + 1, pos[2]], 1, true),
                ] {
                    if !can_burn(at) {
                        continue;
                    }
                    for _ in 0..2 {
                        let r = engine.random();
                        let d = [r.next_double(), r.next_double(), r.next_double()];
                        let mut p = [fx + d[0], fy + d[1], fz + d[2]];
                        // The face coordinate hugs the edge: `* 0.1F` (float).
                        p[axis] = if high {
                            f64::from(pos[axis]) + 1.0 - d[axis] * f64::from(0.1f32)
                        } else {
                            f64::from(pos[axis]) + d[axis] * f64::from(0.1f32)
                        };
                        add(engine, T::LargeSmoke, p, [0.0; 3]);
                    }
                }
            } else {
                for _ in 0..3 {
                    let r = engine.random();
                    let p = [
                        fx + r.next_double(),
                        fy + r.next_double() * 0.5 + 0.5,
                        fz + r.next_double(),
                    ];
                    add(engine, T::LargeSmoke, p, [0.0; 3]);
                }
            }
        }
    }
}
/// `AmbientLeavesBlockSoundPlayer.playAmbientLeavesSounds` for the poplar leaves:
/// one log and three leaves of the same kind around the block.
fn leaves_ambient_sound(
    tables: &AnimateTables,
    engine: &mut ParticleEngine,
    ctx: &AnimateContext,
    state: BlockStateId,
    pos: [i32; 3],
) {
    if next_int(engine.random(), 300) != 0 {
        return;
    }
    let Some(name) = ctx.registry.state(state).map(|s| &s.name) else {
        return;
    };
    let (mut logs, mut leaves) = (0, 0);
    for d in [[0, -1], [1, 0], [0, 1], [-1, 0]] {
        let neighbor = ctx.state([pos[0] + d[0], pos[1], pos[2] + d[1]]);
        if tables
            .poplar_leaf_logs
            .get(neighbor.0 as usize)
            .copied()
            .unwrap_or(false)
        {
            logs += 1;
        }
        if ctx
            .registry
            .state(neighbor)
            .is_some_and(|s| &s.name == name)
        {
            leaves += 1;
        }
        if logs == 1 && leaves == 3 {
            let p = pos.map(f64::from);
            sound(engine, "minecraft:block.poplar_leaves.ambient", p, 1.0, 1.0);
            return;
        }
    }
}
/// `PointedDripstoneBlock.animateTick`: a free-hanging stalactite tip drips the
/// fluid above the root of its stalactite.
fn dripstone_drip(
    tables: &AnimateTables,
    engine: &mut ParticleEngine,
    ctx: &AnimateContext,
    state: BlockStateId,
    pos: [i32; 3],
) {
    let roll = engine.random().next_float();
    if roll > 0.12 {
        return;
    }
    // getFluidAboveStalactite: findRootBlock climbs at most ten blocks of
    // stalactite and stops at the first block that is not pointed dripstone.
    let mut root = None;
    for step in 1..11 {
        let p = [pos[0], pos[1] + step, pos[2]];
        match tables
            .dripstone
            .get(ctx.state(p).0 as usize)
            .copied()
            .unwrap_or(0)
        {
            0 => {
                root = Some(p);
                break;
            }
            2 => {}
            _ => return,
        }
    }
    let Some(root) = root else {
        return;
    };
    let above = ctx.facts.get(ctx.state([root[0], root[1] + 1, root[2]]));
    // Mud above the root counts as a water source unless water evaporates here.
    let (fluid, source) = if &*above.name == "minecraft:mud" && !ctx.ultrawarm {
        (Fluid::Water, true)
    } else {
        (above.fluid, above.fluid_level >= 8 && !above.fluid_falling)
    };
    // canFillCauldron: only a water or lava source passes below a 2% roll.
    if roll >= 0.02 && !(fluid != Fluid::None && source) {
        return;
    }
    // getDripParticle: no fluid drips what the dimension's default says.
    let lava = fluid == Fluid::Lava || (fluid == Fluid::None && ctx.ultrawarm);
    let ty = if lava {
        T::DrippingDripstoneLava
    } else {
        T::DrippingDripstoneWater
    };
    let offset = ctx.offset_of(state, pos);
    // STALACTITE_DRIP_START_PIXEL is the tip's lowest point, 5/16.
    let p = [
        f64::from(pos[0]) + 0.5 + offset[0],
        f64::from(pos[1]) + 0.3125 - 0.0625,
        f64::from(pos[2]) + 0.5 + offset[2],
    ];
    add(engine, ty, p, [0.0; 3]);
}
/// `AmbientDesertBlockSoundsPlayer.shouldPlayAmbientSandSound`: sand in the
/// surface column eight blocks away on all four sides.
fn sand_sound_nearby(tables: &AnimateTables, ctx: &AnimateContext, pos: [i32; 3]) -> bool {
    let is_sand = |p: [i32; 3]| {
        tables
            .desert_sand_sounds
            .get(ctx.state(p).0 as usize)
            .copied()
            .unwrap_or(false)
    };
    let (mut found, mut checked) = (0, 0);
    for d in [[0, -8], [8, 0], [0, 8], [-8, 0]] {
        let (x, z) = (pos[0] + d[0], pos[2] + d[1]);
        // columnContainsTriggeringBlock.
        let surface = ctx.world.world_surface(x, z, |s| ctx.facts.is_air(s)) - 1;
        let matches = if (surface - pos[1]).abs() > 5 {
            let mut y = pos[1] + 6;
            let mut above_air = ctx.is_air([x, y, z]);
            y -= 1;
            let mut hit = false;
            for _ in 0..10 {
                if above_air && is_sand([x, y, z]) {
                    hit = true;
                    break;
                }
                above_air = ctx.is_air([x, y, z]);
                y -= 1;
            }
            hit
        } else {
            ctx.is_air([x, surface + 1, z]) && is_sand([x, surface, z])
        };
        if matches {
            found += 1;
            if found > 3 {
                return true;
            }
        }
        checked += 1;
        if 4 - checked + found < 3 {
            return false;
        }
    }
    false
}
/// `AbstractCandleBlock.addParticlesAndSound`.
fn candle(engine: &mut ParticleEngine, p: [f64; 3]) {
    let r = engine.random();
    let chance = r.next_float();
    if chance < 0.3 {
        add(engine, T::Smoke, p, [0.0; 3]);
        if chance < 0.17 {
            let r = engine.random();
            let (volume, pitch) = (1.0 + r.next_float(), r.next_float() * 0.7 + 0.3);
            sound(
                engine,
                "minecraft:block.candle.ambient",
                [p[0] + 0.5, p[1] + 0.5, p[2] + 0.5],
                volume,
                pitch,
            );
        }
    }
    add(engine, T::SmallFlame, p, [0.0; 3]);
}
/// `CampfireBlockEntity.particleTick` for every lit campfire; `cooking` says
/// which of a campfire's four slots hold an item.
pub fn campfire_particles<'a>(
    tables: &AnimateTables,
    engine: &mut ParticleEngine,
    ctx: &AnimateContext,
    blocks: impl Iterator<Item = &'a ([i32; 3], u32)>,
    cooking: &dyn Fn([i32; 3]) -> [bool; 4],
) {
    for &(pos, state) in blocks {
        if !tables.is_lit_campfire(state) {
            continue;
        }
        let properties = ctx
            .registry
            .state(BlockStateId(state))
            .map(|s| &s.properties);
        let prop = |k: &str| properties.and_then(|p| p.get(k)).map(String::as_str);
        let signal = prop("signal_fire") == Some("true");
        // Direction.get2DDataValue: south, west, north, east.
        let rotation = match prop("facing") {
            Some("west") => 1,
            Some("north") => 2,
            Some("east") => 3,
            _ => 0,
        };
        let r = engine.random();
        if r.next_float() < 0.11 {
            let count = next_int(r, 2) + 2;
            for _ in 0..count {
                make_campfire_smoke(engine, pos, signal, false);
            }
        }
        // The smoke over each cooking item, at the slot's corner of the fire.
        let occupied = cooking(pos);
        for (slot, _) in occupied.iter().enumerate().filter(|(_, held)| **held) {
            if engine.random().next_float() >= 0.2 {
                continue;
            }
            // Direction.from2DDataValue((slot + rotation) % 4) and its clockwise side.
            let (step, clockwise) = match (slot + rotation) % 4 {
                0 => ([0.0, 1.0], [-1.0, 0.0]),
                1 => ([-1.0, 0.0], [0.0, -1.0]),
                2 => ([0.0, -1.0], [1.0, 0.0]),
                _ => ([1.0, 0.0], [0.0, 1.0]),
            };
            let at = [
                f64::from(pos[0]) + 0.5 - step[0] * 0.3125 + clockwise[0] * 0.3125,
                f64::from(pos[1]) + 0.5,
                f64::from(pos[2]) + 0.5 - step[1] * 0.3125 + clockwise[1] * 0.3125,
            ];
            for _ in 0..4 {
                add(engine, T::Smoke, at, [0.0, 5.0e-4, 0.0]);
            }
        }
    }
}
/// `CampfireBlock.makeParticles` (always visible, overriding the limiter);
/// `smoking` adds the plain smoke puff at the logs.
pub fn make_campfire_smoke(
    engine: &mut ParticleEngine,
    pos: [i32; 3],
    signal: bool,
    smoking: bool,
) {
    let r = engine.random();
    let sign = |r: &mut LegacyRandom| if r.next_boolean() { 1.0 } else { -1.0 };
    let dx = r.next_double() / 3.0;
    let x = f64::from(pos[0]) + 0.5 + dx * sign(r);
    let y = f64::from(pos[1]) + r.next_double() + r.next_double();
    let dz = r.next_double() / 3.0;
    let z = f64::from(pos[2]) + 0.5 + dz * sign(r);
    let ty = if signal {
        T::CampfireSignalSmoke
    } else {
        T::CampfireCosySmoke
    };
    engine.create(&ParticleOptions::simple(ty), [x, y, z], [0.0, 0.07, 0.0]);
    if smoking {
        let r = engine.random();
        let dx = r.next_double() / 4.0 * sign(r);
        let dz = r.next_double() / 4.0 * sign(r);
        let at = [
            f64::from(pos[0]) + 0.5 + dx,
            f64::from(pos[1]) + 0.4,
            f64::from(pos[2]) + 0.5 + dz,
        ];
        engine.create(&ParticleOptions::simple(T::Smoke), at, [0.0, 0.005, 0.0]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classifies_animated_blocks() {
        let none = |_: &str| None;
        assert_eq!(classify("minecraft:torch", &none), Animate::Torch(T::Flame));
        let lit = |k: &str| (k == "lit").then(|| "true".to_owned());
        assert_eq!(
            classify("minecraft:furnace", &lit),
            Animate::Furnace([0, -1, 0])
        );
        assert_eq!(classify("minecraft:furnace", &none), Animate::None);
        let wall = |k: &str| (k == "facing").then(|| "north".to_owned());
        assert_eq!(
            classify("minecraft:soul_wall_torch", &wall),
            Animate::WallTorch(T::SoulFireFlame, [0, 0, -1])
        );
        let candles = |k: &str| match k {
            "lit" => Some("true".to_owned()),
            "candles" => Some("3".to_owned()),
            _ => None,
        };
        assert_eq!(
            classify("minecraft:red_candle", &candles),
            Animate::Candles(3)
        );
        assert_eq!(
            classify("minecraft:red_candle_cake", &lit),
            Animate::CandleCake
        );
    }
    #[test]
    fn redstone_colours_match_wire_table() {
        // RedstoneWireBlock.COLORS[15] = (1.0, 0.2, 0.1) before int packing.
        let c = redstone_color(15);
        assert!((c[0] - 1.0).abs() < 1e-6);
        assert!((c[1] - (51.0 / 255.0)).abs() < 0.01);
    }
}
