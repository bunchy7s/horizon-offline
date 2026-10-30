use crate::Result;
use horizon_world::{
    DATA_VERSION,
    heightmap::HeightmapState,
    registry::{BlockRegistry, BlockStateId},
    worldgen::{tags::TagDefinitions, terrain::TerrainBlockStates},
};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs::File,
    io::{self, BufReader},
    path::Path,
    sync::Arc,
};

pub const MATERIALS: [&str; 6] = ["bedrock", "dirt", "stone", "sand", "oak_planks", "bricks"];
#[derive(Clone, Copy)]
pub struct StateFacts {
    air: bool,
    fluid: bool,
    air_block: bool,
    motion: bool,
    no_leaves: bool,
}
impl HeightmapState for StateFacts {
    fn is_air(self) -> bool {
        self.air
    }
    fn is_air_block(self) -> bool {
        self.air_block
    }
    fn has_fluid(self) -> bool {
        self.fluid
    }
    fn blocks_motion_in_heightmap(self) -> bool {
        self.motion
    }
    fn blocks_motion_in_heightmap_no_leaves(self) -> bool {
        self.no_leaves
    }
}
#[derive(Clone)]
pub struct Properties(Arc<Vec<StateFacts>>);
#[cfg(any(test, feature = "test-support"))]
impl Properties {
    /// Heightmap facts for tests, per state: (air, has a fluid, blocks
    /// motion, blocks motion but is not leaves). State 0 is minecraft:air.
    pub fn for_tests(states: &[(bool, bool, bool, bool)]) -> Self {
        Self(Arc::new(
            states
                .iter()
                .enumerate()
                .map(|(i, &(air, fluid, motion, no_leaves))| StateFacts {
                    air,
                    fluid,
                    air_block: i == 0,
                    motion,
                    no_leaves,
                })
                .collect(),
        ))
    }
}
impl TerrainBlockStates for Properties {
    type State = StateFacts;
    fn heightmap_state(&self, id: BlockStateId) -> StateFacts {
        self.0[id.0 as usize]
    }
}
/// The block registry and the facts read from `facts.json` and the block tags:
/// everything world generation and collisions need, with none of the block
/// models or baked sprites.
pub struct Assets {
    /// Leaked once per process: shared by world generation threads.
    pub registry: &'static BlockRegistry,
    pub properties: Properties,
    pub material_states: [BlockStateId; 6],
    /// Block names in `#minecraft:blocks_fluid_flow` (fluid flow vectors).
    pub blocks_fluid_flow: BTreeSet<String>,
}
/// The block models and their baked sprites, read from the pack at `root` over
/// an existing block registry: everything the mesher and renderer need.
pub struct RenderAssets {
    pub sprite_size: u32,
    pub sprite_minimum_size: u32,
    pub pixels: Vec<u8>,
    pub texture_layers: u32,
    pub model_catalog: crate::models::StateModels,
    /// Shared parsed resources; item-only validation errors surface at item preparation.
    pub model_library: std::result::Result<Arc<horizon_assets::resolve::ModelLibrary>, String>,
    pub model_placements: Vec<(horizon_core::position::BlockPos, BlockStateId)>,
    pub model_gallery: Vec<Value>,
    pub animations: Vec<crate::sprites::SpriteAnimation>,
    pub sprite_layers: std::collections::BTreeMap<String, u32>,
    pub translucent_layers: Vec<bool>,
    /// Each layer's `MipmapGenerator` settings.
    pub sprite_mips: Vec<crate::mipmap::Settings>,
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
impl Assets {
    pub fn load(root: &Path) -> Result<Self> {
        let manifest: Value = serde_json::from_reader(BufReader::new(File::open(
            root.join("horizon-resources.json"),
        )?))?;
        if manifest["data_version"].as_i64() != Some(i64::from(DATA_VERSION)) {
            return Err(invalid("resources do not match Horizon's pinned data version").into());
        }
        let (registry, flags) =
            BlockRegistry::from_facts_json_with_flags(File::open(root.join("facts.json"))?)?;
        let bundle: Value =
            serde_json::from_reader(BufReader::new(File::open(root.join("block-tags.json"))?))?;
        let mut definitions = TagDefinitions::default();
        for (name, text) in bundle
            .as_object()
            .ok_or_else(|| invalid("invalid block tag bundle"))?
        {
            definitions.apply_json(
                name,
                "vanilla",
                text.as_str()
                    .ok_or_else(|| invalid("invalid block tag text"))?,
            )?;
        }
        let built =
            definitions.build(|name, _| registry.default_state(name).map(|_| name.to_owned()))?;
        if !built.missing.is_empty() {
            return Err(invalid("unresolved block tags").into());
        }
        let motion: BTreeSet<_> = built
            .tags
            .get("minecraft:blocks_motion_in_heightmap")
            .ok_or_else(|| invalid("missing motion tag"))?
            .iter()
            .map(String::as_str)
            .collect();
        let no_leaves: BTreeSet<_> = built
            .tags
            .get("minecraft:blocks_motion_in_heightmap_no_leaves")
            .ok_or_else(|| invalid("missing no-leaves motion tag"))?
            .iter()
            .map(String::as_str)
            .collect();
        let blocks_fluid_flow = built
            .tags
            .get("minecraft:blocks_fluid_flow")
            .ok_or_else(|| invalid("missing fluid flow tag"))?
            .iter()
            .cloned()
            .collect();
        let properties = Properties(Arc::new(
            flags
                .iter()
                .enumerate()
                .map(|(i, &(air, fluid))| {
                    let name = &registry
                        .state(BlockStateId(i as u32))
                        .ok_or_else(|| invalid("missing registry state"))?
                        .name;
                    Ok(StateFacts {
                        air,
                        fluid,
                        air_block: name == "minecraft:air",
                        motion: motion.contains(name.as_str()),
                        no_leaves: no_leaves.contains(name.as_str()),
                    })
                })
                .collect::<Result<_>>()?,
        ));
        let mut material_states = [BlockStateId(0); 6];
        for (layer, name) in MATERIALS.iter().enumerate() {
            material_states[layer] = registry
                .default_state(&format!("minecraft:{name}"))
                .ok_or_else(|| invalid("missing test-scene material"))?;
        }
        // Chunks hold every state of this registry: content mods can take it
        // past vanilla's 16-bit global palette.
        crate::chunk::set_block_states(registry.len());
        let registry: &'static BlockRegistry = Box::leak(Box::new(registry));
        Ok(Self {
            registry,
            properties,
            material_states,
            blocks_fluid_flow,
        })
    }
    /// The models and sprites of another resource root over these blocks
    /// (resource packs changed): the registry and block facts come from the
    /// game data, which resource packs do not touch.
    pub fn reload(&self, root: &Path) -> Result<RenderAssets> {
        RenderAssets::load(root, self)
    }
    /// [`Assets::reload`] for a server, which draws nothing: the models and
    /// the sprites' layers, without the textures' pixels.
    pub fn reload_for_server(&self, root: &Path) -> Result<RenderAssets> {
        RenderAssets::load_with(root, self, false)
    }
}
impl RenderAssets {
    pub fn load(root: &Path, assets: &Assets) -> Result<Self> {
        Self::load_with(root, assets, true)
    }
    fn load_with(root: &Path, assets: &Assets, pixels: bool) -> Result<Self> {
        let registry = assets.registry;
        let mut sprites = crate::sprites::Sprites::with_disk_cache(root);
        for name in MATERIALS {
            sprites.insert(root, &format!("minecraft:block/{name}"))?;
        }
        let parsed = crate::model_library::ParsedModels::load(root);
        let models = crate::models::prepare_with_models(root, registry, &mut sprites, &parsed)?;
        let model_library = parsed.into_item_library();
        // Fluids are drawn by the fluid renderer, not block models; breaking
        // progress uses the destroy-stage overlays.
        for name in [
            "water_still",
            "water_flow",
            "water_overlay",
            "lava_still",
            "lava_flow",
        ] {
            sprites.insert(root, &format!("minecraft:block/{name}"))?;
        }
        for stage in 0..10 {
            sprites.insert(root, &format!("minecraft:block/destroy_stage_{stage}"))?;
        }
        sprites.save_disk_cache();
        if pixels {
            sprites.finish();
        } else {
            sprites.finish_without_pixels();
        }
        // `finish` grows the buffer by doubling: keep only the bytes in use.
        sprites.pixels.shrink_to_fit();
        Ok(Self {
            sprite_size: sprites.size,
            sprite_minimum_size: sprites.minimum_size,
            texture_layers: u32::try_from(sprites.count())?,
            pixels: sprites.pixels,
            model_catalog: models.catalog,
            model_library,
            model_placements: models.placements,
            model_gallery: models.states,
            animations: sprites.animations,
            sprite_layers: sprites.layers,
            translucent_layers: sprites.translucent,
            sprite_mips: sprites.mips,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::items::ConsumeEffect;
    use horizon_world::loot::PackOverlay;

    #[test]
    fn always_edible_and_consume_effects_loaded() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client/26.3");
        if !root.join("items.json").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let facts = crate::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let items =
            crate::items::Items::load(&root, assets.registry, &facts, &PackOverlay::NONE).unwrap();

        let apple = items.get("minecraft:golden_apple").expect("golden apple");
        assert!(apple.can_always_eat);
        assert!(apple.food.is_some());
        assert!(!apple.consumable.as_ref().unwrap().effects.is_empty());

        let enchanted = items
            .get("minecraft:enchanted_golden_apple")
            .expect("enchanted golden apple");
        assert!(enchanted.can_always_eat);
        assert!(enchanted.food.is_some());
        assert!(!enchanted.consumable.as_ref().unwrap().effects.is_empty());

        let beef = items.get("minecraft:cooked_beef").expect("cooked beef");
        assert!(!beef.can_always_eat);
        assert!(beef.food.is_some());

        // Consumables without food, their style and their remainders.
        use horizon_world::consumable::UseAnimation;
        let milk = items.get("minecraft:milk_bucket").expect("milk bucket");
        let drink = milk.consumable.as_ref().unwrap();
        assert!(milk.food.is_none());
        assert_eq!(drink.effects, [ConsumeEffect::ClearAllEffects]);
        assert_eq!((drink.ticks(), drink.animation), (32, UseAnimation::Drink));
        assert_eq!(drink.sound, "minecraft:entity.generic.drink");
        assert!(!drink.particles);
        assert_eq!(milk.use_remainder.as_deref(), Some("minecraft:bucket"));
        let honey = items.get("minecraft:honey_bottle").expect("honey bottle");
        let sip = honey.consumable.as_ref().unwrap();
        assert_eq!(sip.ticks(), 40);
        assert_eq!(sip.sound, "minecraft:item.honey_bottle.drink");
        assert_eq!(
            sip.effects,
            [ConsumeEffect::RemoveEffects(vec![
                "minecraft:poison".into()
            ])]
        );
        assert_eq!(
            honey.use_remainder.as_deref(),
            Some("minecraft:glass_bottle")
        );
        let kelp = items.get("minecraft:dried_kelp").unwrap();
        let bite = kelp.consumable.as_ref().unwrap();
        assert_eq!((bite.ticks(), bite.animation), (16, UseAnimation::Eat));
        assert_eq!(bite.sound, "minecraft:entity.generic.eat");
        assert!(bite.particles && kelp.use_remainder.is_none());
        let chorus = items.get("minecraft:chorus_fruit").unwrap();
        assert_eq!(
            chorus.consumable.as_ref().unwrap().effects,
            [ConsumeEffect::TeleportRandomly(
                horizon_world::teleport_consume::TeleportConsume {
                    diameter: 16.0,
                    directional_particles: true,
                }
            )]
        );
        let chicken = items.get("minecraft:chicken").unwrap();
        assert!(matches!(
            chicken.consumable.as_ref().unwrap().effects[..],
            [ConsumeEffect::ApplyEffects { probability, .. }] if probability == 0.3
        ));
    }
}
