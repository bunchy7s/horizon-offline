//! Feature-placing growth: `TreeGrower.growTree` (saplings, mangrove
//! propagules, azaleas), huge mushrooms and nether fungi, run on the live level
//! through [`LiveFeatures`].
use super::{Behaviours, LevelAccess, LevelEvent, Sim, feature_world::LevelFeatureWorld, flags};
use crate::{registry::BlockStateId, worldgen::live::LiveFeatures};
use horizon_core::{position::BlockPos, random::RandomSource};
use std::sync::{Arc, OnceLock};

/// Runtime extensions of [`Behaviours`] that depend on the worldgen bundle.
#[derive(Default)]
pub struct Growth {
    features: OnceLock<Arc<LiveFeatures>>,
}

impl Behaviours {
    /// Install the live feature registry used by sapling growth and bone meal.
    /// Without it, saplings report [`LevelEvent::GrowTree`] instead of growing.
    pub fn set_live_features(&self, features: Arc<LiveFeatures>) -> bool {
        self.growth.features.set(features).is_ok()
    }
    pub fn live_features(&self) -> Option<&Arc<LiveFeatures>> {
        self.growth.features.get()
    }
    /// `JigsawPlacement.generateJigsaw`: `assemble` builds the pieces (from
    /// the generation context's own random), then each piece is placed into
    /// the level with the level random, an unbounded box and `reference` as
    /// the origin; `keep_jigsaws` skips the jigsaw replacement. Returns whether pieces were assembled
    /// and the entities the pieces carried (their NBT, for the caller to spawn).
    pub fn place_jigsaw<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut super::Ticks,
        reference: [i32; 3],
        keep_jigsaws: bool,
        assemble: impl FnOnce() -> Option<crate::worldgen::generator::JigsawPieces>,
    ) -> (bool, Vec<crate::nbt::Compound>) {
        let Some(features) = self.live_features() else {
            return (false, Vec::new());
        };
        let Some(jigsaw) = assemble() else {
            return (false, Vec::new());
        };
        let mut sim = self.sim(level, ticks);
        let mut random = sim.random().clone();
        let entities = {
            let mut world = LevelFeatureWorld {
                sim: &mut sim,
                features,
                entities: Vec::new(),
            };
            let unbounded =
                crate::worldgen::structure::BoundingBox::new([i32::MIN; 3], [i32::MAX; 3]);
            for piece in &jigsaw.pieces {
                let mut place_feature =
                    |w: &mut LevelFeatureWorld<'_, '_, L>,
                     r: &mut horizon_core::random::LegacyRandom,
                     name: &str,
                     p: [i32; 3]| { Ok(features.place_placed(name, w, r, p)) };
                if let Err(e) = piece.place(
                    &mut world,
                    &jigsaw.resources,
                    &mut random,
                    unbounded,
                    reference,
                    keep_jigsaws,
                    &mut place_feature,
                ) {
                    eprintln!("Horizon: placing a jigsaw piece failed: {e}");
                }
            }
            world.entities
        };
        *sim.random() = random;
        (true, entities)
    }
    /// `/place structure`: `PlaceCommand.placeStructure` after the start is
    /// generated, `StructureStart.placeInChunk` for every chunk of the start's
    /// bounding box (`ChunkPos.rangeClosed` of its corners), each clipped to
    /// the chunk's whole height, drawing from the level random. False when the
    /// level has no structure resources, or a piece failed to place. The
    /// entities the pieces carried (villagers, guardians, template entities)
    /// come back as NBT for the caller to spawn.
    pub fn place_structure<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut super::Ticks,
        start: &crate::worldgen::structure::start::StructureStart,
    ) -> (bool, Vec<crate::nbt::Compound>) {
        let Some(features) = self.live_features() else {
            return (false, Vec::new());
        };
        let (Some(resources), Some(blocks)) = (features.structures(), features.tree_blocks())
        else {
            return (false, Vec::new());
        };
        let (min, max) = (start.bounding_box.min, start.bounding_box.max);
        let (cx0, cx1) = (min[0] >> 4, max[0] >> 4);
        let (cz0, cz1) = (min[2] >> 4, max[2] >> 4);
        let mut sim = self.sim(level, ticks);
        // PlaceCommand's per-chunk clip box: BoundingBox(minX, getMinY(), minZ,
        // maxX, getMaxY() + 1, maxZ), the level's whole height of the chunk
        // (`max_y` is exclusive here, so the top layer is `max_y - 1`).
        let (low_y, high_y) = (sim.level.min_y(), sim.level.max_y() - 1);
        let mut random = sim.random().clone();
        let mut placed = true;
        let entities = {
            let mut world = LevelFeatureWorld {
                sim: &mut sim,
                features,
                entities: Vec::new(),
            };
            let mut place_feature =
                |w: &mut LevelFeatureWorld<'_, '_, L>,
                 r: &mut horizon_core::random::LegacyRandom,
                 name: &str,
                 p: [i32; 3]| { Ok(features.place_placed(name, w, r, p)) };
            for cx in cx0..=cx1 {
                for cz in cz0..=cz1 {
                    let chunk_box = crate::worldgen::structure::BoundingBox::new(
                        [cx * 16, low_y, cz * 16],
                        [cx * 16 + 15, high_y, cz * 16 + 15],
                    );
                    if crate::worldgen::structure::start::place_in_chunk(
                        start,
                        &mut world,
                        resources,
                        &blocks,
                        &mut random,
                        chunk_box,
                        &mut place_feature,
                    )
                    .is_err()
                    {
                        placed = false;
                    }
                }
            }
            world.entities
        };
        *sim.random() = random;
        (placed, entities)
    }
}

/// The four (state, position) pairs of a 2x2 sapling square.
type Saplings = [(BlockStateId, BlockPos); 4];

/// `TreeGrower` (vanilla's registered growers, by name).
pub(crate) struct TreeGrower {
    pub name: &'static str,
    trees: &'static [(&'static str, i32)],
    mega_trees: &'static [(&'static str, i32)],
    flower_trees: &'static [(&'static str, i32)],
    shortest: Option<&'static str>,
}

pub(crate) const GROWERS: &[TreeGrower] = &[
    TreeGrower {
        name: "oak",
        trees: &[("minecraft:oak", 9), ("minecraft:fancy_oak", 1)],
        mega_trees: &[],
        flower_trees: &[
            ("minecraft:oak_bees_005", 9),
            ("minecraft:fancy_oak_bees_005", 1),
        ],
        shortest: Some("minecraft:oak"),
    },
    TreeGrower {
        name: "spruce",
        trees: &[("minecraft:spruce", 1)],
        mega_trees: &[("minecraft:mega_spruce", 1), ("minecraft:mega_pine", 1)],
        flower_trees: &[],
        shortest: Some("minecraft:spruce"),
    },
    TreeGrower {
        name: "mangrove",
        trees: &[("minecraft:mangrove", 15), ("minecraft:tall_mangrove", 85)],
        mega_trees: &[],
        flower_trees: &[],
        shortest: Some("minecraft:mangrove"),
    },
    TreeGrower {
        name: "azalea",
        trees: &[("minecraft:azalea_tree", 1)],
        mega_trees: &[],
        flower_trees: &[],
        shortest: Some("minecraft:azalea_tree"),
    },
    TreeGrower {
        name: "birch",
        trees: &[("minecraft:birch", 1)],
        mega_trees: &[],
        flower_trees: &[("minecraft:birch_bees_005", 1)],
        shortest: Some("minecraft:birch"),
    },
    TreeGrower {
        name: "jungle",
        trees: &[("minecraft:jungle_tree_no_vine", 1)],
        mega_trees: &[("minecraft:mega_jungle_tree", 1)],
        flower_trees: &[],
        shortest: Some("minecraft:jungle_tree_no_vine"),
    },
    TreeGrower {
        name: "acacia",
        trees: &[("minecraft:acacia", 1)],
        mega_trees: &[],
        flower_trees: &[],
        shortest: Some("minecraft:acacia"),
    },
    TreeGrower {
        name: "cherry",
        trees: &[("minecraft:cherry", 1)],
        mega_trees: &[],
        flower_trees: &[("minecraft:cherry_bees_005", 1)],
        shortest: Some("minecraft:cherry"),
    },
    TreeGrower {
        name: "dark_oak",
        trees: &[],
        mega_trees: &[("minecraft:dark_oak", 1)],
        flower_trees: &[],
        shortest: None,
    },
    TreeGrower {
        name: "pale_oak",
        trees: &[],
        mega_trees: &[("minecraft:pale_oak_bonemeal", 1)],
        flower_trees: &[],
        shortest: None,
    },
    TreeGrower {
        name: "poplar",
        trees: &[
            ("minecraft:red_poplar", 1),
            ("minecraft:orange_poplar", 1),
            ("minecraft:yellow_poplar", 1),
        ],
        mega_trees: &[],
        flower_trees: &[],
        shortest: Some("minecraft:red_poplar"),
    },
];

pub(crate) fn grower(name: &str) -> Option<&'static TreeGrower> {
    GROWERS.iter().find(|g| g.name == name)
}

/// `WeightedList.getRandom`: no draw for an empty list.
fn weighted<R: RandomSource>(
    list: &'static [(&'static str, i32)],
    random: &mut R,
) -> Option<&'static str> {
    let total: i32 = list.iter().map(|e| e.1).sum();
    if total == 0 {
        return None;
    }
    let mut selection = random.next_int_bound(total).unwrap_or(0);
    for &(name, weight) in list {
        if selection < weight {
            return Some(name);
        }
        selection -= weight;
    }
    None
}

impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    /// `ConfiguredFeature.place(level, generator, level.random, pos)`. The
    /// feature draws from a copy of the level random that replaces it after.
    pub(crate) fn place_configured_feature(&mut self, name: &str, pos: BlockPos) -> bool {
        let b = self.b;
        let Some(features) = b.live_features() else {
            return false;
        };
        let mut random = self.random().clone();
        let placed = {
            let mut world = LevelFeatureWorld {
                sim: self,
                features,
                entities: Vec::new(),
            };
            features.place_configured(name, &mut world, &mut random, [pos.x, pos.y, pos.z])
        };
        *self.random() = random;
        placed
    }
    /// `PlacedFeature.place(level, generator, level.random, pos)`.
    pub(crate) fn place_placed_feature(&mut self, name: &str, pos: BlockPos) -> bool {
        let b = self.b;
        let Some(features) = b.live_features() else {
            return false;
        };
        let mut random = self.random().clone();
        let placed = {
            let mut world = LevelFeatureWorld {
                sim: self,
                features,
                entities: Vec::new(),
            };
            features.place_placed(name, &mut world, &mut random, [pos.x, pos.y, pos.z])
        };
        *self.random() = random;
        placed
    }
    fn feature_exists(&self, name: &str) -> bool {
        self.b
            .live_features()
            .is_some_and(|f| f.configured(name).is_some())
    }

    /// `TreeGrower.hasFlowers`: a flower within 2 blocks horizontally, 1 vertically.
    fn tree_has_flowers(&self, pos: BlockPos) -> bool {
        let b = self.b;
        let Some(flowers) = b.tag("minecraft:flowers") else {
            return false;
        };
        for y in -1..=1 {
            for z in -2..=2 {
                for x in -2..=2 {
                    if b.in_set(self.state(pos.offset(x, y, z)), flowers) {
                        return true;
                    }
                }
            }
        }
        false
    }
    /// `TreeGrower.findTwoByTwoSaplingPos`.
    fn two_by_two(&self, state: BlockStateId, pos: BlockPos) -> Option<(i32, i32, Saplings)> {
        let block = self.b.block(state);
        for dx in [0, -1] {
            for dz in [0, -1] {
                let at = |x: i32, z: i32| {
                    let p = pos.offset(dx + x, 0, dz + z);
                    (self.state(p), p)
                };
                let around = [at(0, 0), at(1, 0), at(0, 1), at(1, 1)];
                if around.iter().all(|(s, _)| self.b.is_block(*s, block)) {
                    return Some((dx, dz, around));
                }
            }
        }
        None
    }
    /// `TreeGrower.removeSapling`: the fluid's legacy block, flags 818.
    fn remove_sapling(&mut self, pos: BlockPos) {
        let empty = self.b.legacy_block(self.fluid_at(pos));
        self.set_block(
            pos,
            empty,
            flags::UPDATE_SKIP_ALL_SIDEEFFECTS | flags::UPDATE_CLIENTS,
        );
    }
    /// `TreeGrower.growTree`.
    pub(crate) fn grow_tree(
        &mut self,
        grower: &TreeGrower,
        pos: BlockPos,
        state: BlockStateId,
    ) -> bool {
        if let Some(mega) = weighted(grower.mega_trees, self.random())
            && self.feature_exists(mega)
            && let Some((dx, dz, around)) = self.two_by_two(state, pos)
        {
            for &(_, p) in &around {
                self.remove_sapling(p);
            }
            if self.place_configured_feature(mega, pos.offset(dx, 0, dz)) {
                return true;
            }
            for &(s, p) in &around {
                self.set_block(p, s, flags::UPDATE_NONE);
            }
            return false;
        }
        let flowers = self.tree_has_flowers(pos);
        let list = if flowers && !grower.flower_trees.is_empty() {
            grower.flower_trees
        } else {
            grower.trees
        };
        let Some(feature) = weighted(list, self.random()) else {
            return false;
        };
        if !self.feature_exists(feature) {
            return false;
        }
        self.remove_sapling(pos);
        if self.place_configured_feature(feature, pos) {
            return true;
        }
        self.set_block(pos, state, flags::UPDATE_NONE);
        false
    }
    /// `TreeGrower.canGrow` (draws as vanilla: feature, then mega feature).
    pub(crate) fn tree_can_grow(
        &mut self,
        grower: &TreeGrower,
        pos: BlockPos,
        state: BlockStateId,
    ) -> bool {
        let flowers = self.tree_has_flowers(pos);
        let list = if flowers && !grower.flower_trees.is_empty() {
            grower.flower_trees
        } else {
            grower.trees
        };
        let feature = weighted(list, self.random());
        let mega = weighted(grower.mega_trees, self.random());
        if feature.is_none() && mega.is_some() {
            self.two_by_two(state, pos).is_some()
        } else {
            true
        }
    }
    /// `TreeGrower.getMinimumHeight`: the shortest tree's trunk base height.
    pub(crate) fn tree_minimum_height(&self, grower: &TreeGrower) -> i32 {
        grower
            .shortest
            .and_then(|name| self.b.live_features()?.configured_json(name))
            .filter(|json| json["type"] == "minecraft:tree")
            .and_then(|json| json["trunk_placer"]["base_height"].as_i64())
            .map_or(0, |h| h as i32)
    }
    /// `SaplingBlock.advanceTree` with the grower run inline when features are
    /// installed; otherwise reported as [`LevelEvent::GrowTree`].
    pub(crate) fn advance_tree(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        if b.table.int(state, b.props.stage) == 0 {
            self.set_block(pos, b.table.cycle(state, b.props.stage), flags::UPDATE_NONE);
            return;
        }
        let Some(name) = b.data(state).feature.clone() else {
            return;
        };
        match grower(&name) {
            Some(g) if b.live_features().is_some() => {
                self.grow_tree(g, pos, state);
            }
            _ => self.emit(LevelEvent::GrowTree {
                pos,
                state,
                grower: name,
            }),
        }
    }
    /// `MushroomBlock.growMushroom`.
    pub(crate) fn grow_mushroom(&mut self, pos: BlockPos, state: BlockStateId) -> bool {
        let Some(feature) = self.b.data(state).feature.clone() else {
            return false;
        };
        if !self.feature_exists(&feature) {
            return false;
        }
        self.remove_block(pos, false);
        if self.place_configured_feature(&feature, pos) {
            return true;
        }
        self.set_block_and_update(pos, state);
        false
    }
}
