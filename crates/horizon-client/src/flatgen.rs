//! FlatLevelSource: superflat worlds (`minecraft:flat` generator).
use crate::{
    chunk::{Chunk, LevelShape},
    streaming::ChunkGenerator,
};
use horizon_world::{
    nbt::Tag,
    registry::{BlockRegistry, BlockStateId},
    worldgen::feature::{BasicFeature, FeatureBlocks, FeatureWorld},
};

/// The layers of a flat generator, bottom up from the world's minimum Y.
pub struct FlatTerrain {
    /// The Overworld's shape: the layers start at its minimum Y and stop at
    /// its top.
    shape: LevelShape,
    layers: Vec<u32>,
    biome: u16,
    surface: Option<crate::lod::Surface>,
    /// The blocks of `minecraft:void_start_platform`, when the adjusted
    /// generation settings place it (the Void preset).
    void_platform: Option<FeatureBlocks>,
}
/// `GenerationStep.Decoration` ordinals `adjustGenerationSettings` leaves out.
const LAKES: usize = 1;
const UNDERGROUND_STRUCTURES: usize = 3;
const SURFACE_STRUCTURES: usize = 4;
/// FlatLevelGeneratorSettings.adjustGenerationSettings for the world's own
/// biome: its placed features, by step, when `features` is on and the world
/// isn't only air (`voidGen`) or its biome is the_void. The structure steps
/// are left out, and the lakes step when `lakes` puts the lava lakes there.
fn decoration_features(
    biome_features: &[Vec<String>],
    features: bool,
    lakes: bool,
    void_gen: bool,
    the_void: bool,
) -> impl Iterator<Item = &str> {
    let decorate = (!void_gen || the_void) && features;
    biome_features
        .iter()
        .enumerate()
        .filter(move |&(step, _)| {
            decorate
                && step != UNDERGROUND_STRUCTURES
                && step != SURFACE_STRUCTURES
                && !(lakes && step == LAKES)
        })
        .flat_map(|(_, step)| step.iter().map(String::as_str))
}
/// The proto chunk as the world a feature writes in.
struct ChunkWorld<'a>(&'a mut Chunk);
impl FeatureWorld for ChunkWorld<'_> {
    type Error = std::convert::Infallible;
    fn block(&mut self, [x, y, z]: [i32; 3]) -> Result<BlockStateId, Self::Error> {
        Ok(self.0.block(x, y, z))
    }
    fn set_block(
        &mut self,
        [x, y, z]: [i32; 3],
        state: BlockStateId,
        _: i32,
    ) -> Result<bool, Self::Error> {
        self.0.set(x, y, z, state.0);
        Ok(true)
    }
}
impl FlatTerrain {
    /// The overworld generator of `world_gen_settings` (`data.dimensions`), if it
    /// is flat. Its layers, as FlatLevelSource places them, and of the features
    /// its settings decorate the world with, `void_start_platform`; the other
    /// features and the lakes are not placed yet.
    pub fn from_settings(
        settings: &Tag,
        registry: &BlockRegistry,
        biomes: &crate::biome::Biomes,
        shape: LevelShape,
    ) -> Option<Self> {
        let generator = settings
            .get("dimensions")?
            .get("minecraft:overworld")?
            .get("generator")?;
        if crate::save::text(generator.get("type"))?.trim_start_matches("minecraft:") != "flat" {
            return None;
        }
        let s = generator.get("settings")?;
        let biome = crate::save::text(s.get("biome")).unwrap_or_else(|| "minecraft:plains".into());
        let mut layers = Vec::new();
        if let Some(Tag::List(list)) = s.get("layers") {
            for layer in list {
                let block = crate::save::text(layer.get("block"))?;
                let height = crate::save::number(layer.get("height")).unwrap_or(0).max(0);
                let state = registry.default_state(&block)?.0;
                layers.extend(std::iter::repeat_n(state, height as usize));
            }
        }
        let biome = biomes.id(&biome);
        // FlatLevelGeneratorSettings.updateLayers: every layer is Blocks.AIR.
        let void_gen = layers.iter().all(|&id| {
            registry
                .state(BlockStateId(id))
                .is_some_and(|s| s.name == "minecraft:air")
        });
        let flag = |key| crate::save::number(s.get(key)).is_some_and(|v| v != 0);
        let info = biomes.get(biome);
        let void_platform = if decoration_features(
            &info.features,
            flag("features"),
            flag("lakes"),
            void_gen,
            info.name == "minecraft:the_void",
        )
        .any(|f| f == "minecraft:void_start_platform")
        {
            FeatureBlocks::from_registry(registry)
                .inspect_err(|e| log::warn!("Horizon: no void_start_platform: {e}"))
                .ok()
        } else {
            None
        };
        let mut water = None;
        let mut surface = None;
        for (i, &id) in layers.iter().take(shape.height as usize).enumerate().rev() {
            let state = horizon_world::registry::BlockStateId(id);
            let name = registry.state(state)?.name.as_str();
            if matches!(
                name,
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            ) {
                continue;
            }
            let height = shape.min_y + i as i32 + 1;
            if matches!(name, "minecraft:water" | "minecraft:lava") {
                water.get_or_insert((height, state));
            } else {
                surface = Some(crate::lod::Surface {
                    height,
                    state,
                    water,
                    biome,
                });
                break;
            }
        }
        Some(Self {
            shape,
            layers,
            biome,
            surface,
            void_platform,
        })
    }
    /// The settings of a superflat overworld that lets structure sets generate
    /// (`FlatLevelSource.createState`): its layers, biome and `structure_overrides`
    /// (no key means every set; `[]` none). `None` for anything else, a world with no
    /// structure set to generate, the Void preset's biome (its platform is a feature
    /// this generator places) or one that is not superflat.
    pub fn structure_settings(
        settings: &Tag,
    ) -> Option<horizon_world::worldgen::generator::FlatSettings> {
        let generator = settings
            .get("dimensions")?
            .get("minecraft:overworld")?
            .get("generator")?;
        if crate::save::text(generator.get("type"))?.trim_start_matches("minecraft:") != "flat" {
            return None;
        }
        let s = generator.get("settings")?;
        let biome = crate::save::text(s.get("biome")).unwrap_or_else(|| "minecraft:plains".into());
        if biome == "minecraft:the_void" {
            return None;
        }
        let mut layers = Vec::new();
        if let Some(Tag::List(list)) = s.get("layers") {
            for layer in list {
                let block = crate::save::text(layer.get("block"))?;
                let height = crate::save::number(layer.get("height")).unwrap_or(0).max(0) as i32;
                layers.push((block, height));
            }
        }
        let structure_overrides = match s.get("structure_overrides") {
            Some(Tag::List(sets)) => Some(
                sets.iter()
                    .filter_map(|set| crate::save::text(Some(set)))
                    .collect::<Vec<_>>(),
            ),
            Some(Tag::String(set)) => Some(vec![set.to_string_lossy()]),
            _ => None,
        };
        if structure_overrides.as_ref().is_some_and(Vec::is_empty) {
            return None;
        }
        Some(horizon_world::worldgen::generator::FlatSettings {
            layers,
            biome,
            structure_overrides,
        })
    }
    /// FlatLevelSource.getSpawnHeight: the top of the layers.
    pub fn spawn_height(&self) -> i32 {
        self.shape.min_y + self.layers.len().min(self.shape.height as usize) as i32
    }
}
/// DimensionType.Y_SIZE: the most layers FlatLevelGeneratorSettings takes.
const Y_SIZE: i64 = 4064;

/// FlatLevelGeneratorSettings.CODEC reading a dedicated server's
/// `generator-settings` (DedicatedServerProperties.WorldDimensionData for
/// the `minecraft:flat` preset): the settings as `world_gen_settings` stores
/// them, and the errors vanilla logs, in order (the constructor's, then
/// `resultOrPartial`'s). No settings means the preset's own layers; a layer
/// that doesn't parse is left out, as the codec's partial result leaves it.
/// `is_block` says whether a block id is registered, `is_biome` a biome id.
pub fn settings_from_json(
    json: &crate::server::json::Json,
    is_block: impl Fn(&str) -> bool,
    is_biome: impl Fn(&str) -> bool,
) -> (Option<Tag>, Vec<String>) {
    use crate::server::json::Json;
    let id = |s: &str| {
        if s.contains(':') {
            s.to_owned()
        } else {
            format!("minecraft:{s}")
        }
    };
    let Some(Json::Array(list)) = json.get("layers") else {
        let error = match json.get("layers") {
            None => format!("No key layers in MapLike[{}]", json.compact()),
            Some(v) => format!("Not a list: {}", v.compact()),
        };
        return (None, vec![error]);
    };
    let mut errors = Vec::new();
    let mut layers = Vec::new();
    let mut total = 0;
    for layer in list {
        let height = layer.get("height").and_then(Json::as_string);
        let height = height.as_deref().and_then(|h| h.parse::<i64>().ok());
        let block = layer.get("block").and_then(Json::as_string).map(|b| id(&b));
        match (height, block) {
            (Some(h), _) if !(0..=Y_SIZE).contains(&h) => {
                errors.push(format!("Value {h} outside of range [0:{Y_SIZE}]"));
            }
            (Some(h), Some(b)) if is_block(&b) => {
                total += h;
                layers.push(Tag::compound([
                    ("height", Tag::Int(h as i32)),
                    ("block", Tag::String(b.as_str().into())),
                ]));
            }
            (Some(_), Some(b)) => errors.push(format!(
                "Unknown registry key in ResourceKey[minecraft:root / minecraft:block]: {b}"
            )),
            (None, _) => errors.push(format!("No key height in MapLike[{}]", layer.compact())),
            (_, None) => errors.push(format!("No key block in MapLike[{}]", layer.compact())),
        }
    }
    // The constructor: Biome.CODEC is lenient, and a biome that is missing
    // or unknown is Plains.
    let biome = json
        .get("biome")
        .and_then(Json::as_string)
        .map(|b| id(&b))
        .filter(|b| is_biome(b));
    let mut logged = Vec::new();
    if biome.is_none() {
        logged.push("Unknown biome, defaulting to plains".to_owned());
    }
    let biome = biome.unwrap_or_else(|| "minecraft:plains".to_owned());
    // validateHeight.
    if total > Y_SIZE {
        errors.push(format!("Sum of layer heights is > {Y_SIZE}"));
        logged.push(errors.join("; "));
        return (None, logged);
    }
    let flag = |key: &str| match json.get(key) {
        Some(Json::Bool(b)) => *b,
        Some(Json::Number(n)) => n.parse::<f64>().is_ok_and(|v| v != 0.0),
        _ => false,
    };
    let mut settings = Vec::new();
    // RegistryCodecs.holderSet, lenient too: a tag or a list of ids.
    match json.get("structure_overrides") {
        Some(Json::String(s)) => {
            settings.push(("structure_overrides", Tag::String(s.as_str().into())))
        }
        Some(Json::Array(ids)) if ids.iter().all(|i| matches!(i, Json::String(_))) => settings
            .push((
                "structure_overrides",
                Tag::List(
                    ids.iter()
                        .filter_map(Json::as_string)
                        .map(|i| Tag::String(id(&i).as_str().into()))
                        .collect(),
                ),
            )),
        _ => {}
    }
    settings.extend([
        ("layers", Tag::List(layers)),
        ("lakes", Tag::Byte(i8::from(flag("lakes")))),
        ("features", Tag::Byte(i8::from(flag("features")))),
        ("biome", Tag::String(biome.as_str().into())),
    ]);
    if !errors.is_empty() {
        logged.push(errors.join("; "));
    }
    (Some(Tag::compound(settings)), logged)
}
impl ChunkGenerator for FlatTerrain {
    type Worker = ();
    fn worker(&self) {}
    fn lod_surface(&self, _: &mut (), _: i32, _: i32) -> Option<crate::lod::Surface> {
        self.surface
    }
    fn proto(&self, _: &mut (), cx: i32, cz: i32) -> crate::streaming::Proto {
        let mut chunk = Chunk::with_layout(cx, cz, self.shape.column());
        for s in &mut chunk.sections {
            std::sync::Arc::make_mut(s).biomes = horizon_core::paletted::PalettedContainer::new(
                crate::chunk::BIOME_STRATEGY,
                u32::from(self.biome),
            );
        }
        let height = self.shape.height as usize;
        for (i, &block) in self.layers.iter().enumerate().take(height) {
            if block == 0 {
                continue;
            }
            for z in 0..16 {
                for x in 0..16 {
                    chunk.set(x, self.shape.min_y + i as i32, z, block);
                }
            }
        }
        // ChunkGenerator.applyBiomeDecoration at the chunk's section origin.
        // VoidStartPlatformFeature writes only inside its own chunk and draws
        // no random numbers, so placing it here gives the FEATURES result.
        if let Some(blocks) = self.void_platform {
            let origin = [cx * 16, self.shape.min_y, cz * 16];
            let mut random = horizon_core::random::LegacyRandom::new(0);
            if let Err(e) = BasicFeature::VoidStartPlatform.place(
                &mut ChunkWorld(&mut chunk),
                &mut random,
                origin,
                blocks,
            ) {
                log::warn!("Horizon: placing void_start_platform failed: {e}");
            }
        }
        for s in &mut chunk.sections {
            std::sync::Arc::make_mut(s).compact();
        }
        Box::new(chunk)
    }
    /// FlatLevelSource.spawnOriginalMobs is empty: no generation-time spawns.
    fn finish(&self, proto: crate::streaming::Proto) -> Chunk {
        *proto.downcast::<Chunk>().expect("flat terrain proto")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::{number, text};
    use crate::server::json::{self, Json};

    fn read(text: &str) -> (Option<Tag>, Vec<String>) {
        let json: Json = json::parse(text, false).unwrap();
        settings_from_json(
            &json,
            |b| ["minecraft:stone", "minecraft:bedrock"].contains(&b),
            |b| ["minecraft:plains", "minecraft:desert"].contains(&b),
        )
    }

    /// A world_gen_settings with a flat overworld of `layers` (block, height).
    fn flat_world(biome: &str, features: bool, layers: &[(&str, i32)]) -> Tag {
        let layers = layers
            .iter()
            .map(|&(block, height)| {
                Tag::compound([
                    ("block", Tag::String(block.into())),
                    ("height", Tag::Int(height)),
                ])
            })
            .collect();
        let settings = Tag::compound([
            ("biome", Tag::String(biome.into())),
            ("features", Tag::Byte(i8::from(features))),
            ("lakes", Tag::Byte(0)),
            ("layers", Tag::List(layers)),
        ]);
        let generator = Tag::compound([
            ("type", Tag::String("minecraft:flat".into())),
            ("settings", settings),
        ]);
        Tag::compound([(
            "dimensions",
            Tag::compound([(
                "minecraft:overworld",
                Tag::compound([("generator", generator)]),
            )]),
        )])
    }

    #[test]
    fn the_void_preset_places_the_start_platform() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let registry = assets.registry;
        let biomes = crate::biome::Biomes::load(&root).unwrap();
        let terrain = |biome, features, layers: &[(&str, i32)]| {
            FlatTerrain::from_settings(
                &flat_world(biome, features, layers),
                registry,
                &biomes,
                LevelShape::OVERWORLD,
            )
            .unwrap()
        };
        let block = |terrain: &FlatTerrain, [x, y, z]: [i32; 3]| {
            let chunk = terrain.finish(terrain.proto(&mut (), x >> 4, z >> 4));
            registry.state(chunk.block(x, y, z)).unwrap().name.clone()
        };
        // The Void preset (flat_level_generator_preset/the_void): the_void's
        // only feature, VoidStartPlatformFeature, at the minimum Y + 3: a
        // 33×33 stone square around 8, 8 with cobblestone at its centre, in
        // the chunks within one of chunk 0, 0.
        let void = terrain("minecraft:the_void", true, &[("minecraft:air", 1)]);
        for (pos, expected) in [
            ([8, -61, 8], "minecraft:cobblestone"),
            ([0, -61, 0], "minecraft:stone"),
            ([-8, -61, -8], "minecraft:stone"),
            ([24, -61, 24], "minecraft:stone"),
            ([24, -61, -8], "minecraft:stone"),
            ([-9, -61, 8], "minecraft:air"),
            ([25, -61, 8], "minecraft:air"),
            ([8, -60, 8], "minecraft:air"),
            ([8, -62, 8], "minecraft:air"),
            ([40, -61, 8], "minecraft:air"),
        ] {
            assert_eq!(block(&void, pos), expected, "{pos:?}");
        }
        // The decoration is off without `features`.
        let plain = terrain("minecraft:the_void", false, &[("minecraft:air", 1)]);
        assert_eq!(block(&plain, [8, -61, 8]), "minecraft:air");
        // the_void over solid layers still gets its platform.
        let floor = terrain("minecraft:the_void", true, &[("minecraft:bedrock", 1)]);
        assert_eq!(block(&floor, [8, -64, 8]), "minecraft:bedrock");
        assert_eq!(block(&floor, [8, -61, 8]), "minecraft:cobblestone");
    }

    #[test]
    fn adjusted_generation_settings_keep_the_biome_features_outside_structures() {
        let steps: Vec<Vec<String>> = (0..11).map(|step| vec![format!("f{step}")]).collect();
        let kept = |features, lakes, void_gen, the_void| {
            decoration_features(&steps, features, lakes, void_gen, the_void)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        let all_but_structures = ["f0", "f1", "f2", "f5", "f6", "f7", "f8", "f9", "f10"];
        assert_eq!(kept(true, false, false, false), all_but_structures);
        // The lava lakes take the LAKES step's place.
        assert_eq!(kept(true, true, false, false)[1..], all_but_structures[2..]);
        // An all-air world is decorated only in the_void.
        assert!(kept(true, false, true, false).is_empty());
        assert_eq!(kept(true, false, true, true), all_but_structures);
        assert!(kept(false, false, false, true).is_empty());
    }

    #[test]
    fn generator_settings_read_as_flat_level_generator_settings() {
        // server.properties' default: the preset's own layers, and the codec's error.
        assert_eq!(
            read("{}"),
            (None, vec!["No key layers in MapLike[{}]".into()])
        );
        let (settings, errors) = read(
            r#"{"layers":[{"block":"bedrock","height":1},{"block":"minecraft:stone","height":3}],"biome":"desert","features":true}"#,
        );
        assert!(errors.is_empty());
        let settings = settings.unwrap();
        let Some(Tag::List(layers)) = settings.get("layers") else {
            panic!("{settings:?}")
        };
        assert_eq!(layers.len(), 2);
        assert_eq!(
            text(layers[0].get("block")).as_deref(),
            Some("minecraft:bedrock")
        );
        assert_eq!(number(layers[1].get("height")), Some(3));
        assert_eq!(
            text(settings.get("biome")).as_deref(),
            Some("minecraft:desert")
        );
        assert_eq!(number(settings.get("features")), Some(1));
        assert_eq!(number(settings.get("lakes")), Some(0));
        // A layer that doesn't parse is left out; an unknown biome is Plains, and logged first.
        let (settings, errors) = read(
            r#"{"layers":[{"block":"nope","height":2},{"block":"stone","height":2}],"biome":"x"}"#,
        );
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0], "Unknown biome, defaulting to plains");
        assert!(errors[1].contains("minecraft:nope"));
        let settings = settings.unwrap();
        let Some(Tag::List(layers)) = settings.get("layers") else {
            panic!("{settings:?}")
        };
        assert_eq!(layers.len(), 1);
        assert_eq!(
            text(settings.get("biome")).as_deref(),
            Some("minecraft:plains")
        );
        // validateHeight.
        assert_eq!(
            read(
                r#"{"layers":[{"block":"stone","height":4000},{"block":"stone","height":100}],"biome":"plains"}"#
            ),
            (None, vec!["Sum of layer heights is > 4064".into()])
        );
    }

    /// `FlatLevelSource.createState`: the settings a flat world's structure sets run with.
    #[test]
    fn a_flat_world_names_the_structure_sets_that_may_generate() {
        let world = |generator: Tag| {
            Tag::compound([(
                "dimensions",
                Tag::compound([(
                    "minecraft:overworld",
                    Tag::compound([("generator", generator)]),
                )]),
            )])
        };
        let flat = |extra: Vec<(&str, Tag)>, biome: &str| {
            let mut settings = vec![
                ("biome", Tag::String(biome.into())),
                (
                    "layers",
                    Tag::List(vec![Tag::compound([
                        ("block", Tag::String("minecraft:stone".into())),
                        ("height", Tag::Int(3)),
                    ])]),
                ),
            ];
            settings.extend(extra);
            world(Tag::compound([
                ("type", Tag::String("minecraft:flat".into())),
                ("settings", Tag::compound(settings)),
            ]))
        };
        // No key: every set. A list: those. An empty list: none. The void and a noise world: none.
        let all = FlatTerrain::structure_settings(&flat(vec![], "minecraft:plains")).unwrap();
        assert_eq!(all.structure_overrides, None);
        assert_eq!(all.layers, [("minecraft:stone".to_owned(), 3)]);
        let some = FlatTerrain::structure_settings(&flat(
            vec![(
                "structure_overrides",
                Tag::List(vec![Tag::String("minecraft:villages".into())]),
            )],
            "minecraft:desert",
        ))
        .unwrap();
        assert_eq!(
            some.structure_overrides,
            Some(vec!["minecraft:villages".to_owned()])
        );
        assert_eq!(some.biome, "minecraft:desert");
        assert!(
            FlatTerrain::structure_settings(&flat(
                vec![("structure_overrides", Tag::List(vec![]))],
                "minecraft:plains"
            ))
            .is_none()
        );
        assert!(FlatTerrain::structure_settings(&flat(vec![], "minecraft:the_void")).is_none());
        assert!(
            FlatTerrain::structure_settings(&world(Tag::compound([(
                "type",
                Tag::String("minecraft:noise".into())
            )])))
            .is_none()
        );
    }
}
