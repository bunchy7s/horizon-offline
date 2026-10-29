use horizon_core::position::ChunkPos;
use horizon_world::{
    DATA_VERSION,
    heightmap::{HeightmapState, HeightmapType},
    nbt::Tag,
    registry::{BlockRegistry, BlockStateId},
    worldgen::{
        anchor::WorldGenerationContext,
        feature::FeatureWorld,
        feature_region::{RegionError, TerrainFeatureAccess, TerrainRegion},
        placement::PlacementContext,
        terrain::{PreLightChunkStage, TerrainBlockStates, TerrainChunk},
    },
};
#[derive(Clone, Copy)]
struct State(u32);
impl HeightmapState for State {
    fn is_air(self) -> bool {
        self.0 < 2
    }
    fn is_air_block(self) -> bool {
        self.0 == 0
    }
    fn has_fluid(self) -> bool {
        self.0 == 3
    }
    fn blocks_motion_in_heightmap(self) -> bool {
        self.0 >= 2 && self.0 != 3
    }
    fn blocks_motion_in_heightmap_no_leaves(self) -> bool {
        self.blocks_motion_in_heightmap()
    }
}
struct Properties;
impl TerrainBlockStates for Properties {
    type State = State;
    fn heightmap_state(&self, id: BlockStateId) -> State {
        State(id.0)
    }
}
fn registry() -> BlockRegistry {
    let names = [
        "minecraft:air",
        "minecraft:void_air",
        "minecraft:stone",
        "minecraft:water",
        "minecraft:grass_block",
        "minecraft:mycelium",
        "minecraft:dirt",
        "minecraft:cave_air",
    ];
    BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        (
            "states",
            Tag::List(
                names
                    .iter()
                    .map(|name| {
                        let mut tag = Tag::compound([("id", Tag::String((*name).into()))]);
                        if *name == "minecraft:water" {
                            if let Tag::Compound(entries) = &mut tag {
                                entries.insert(
                                    "properties".into(),
                                    Tag::compound([("level", Tag::String("0".into()))]),
                                );
                            }
                        }
                        tag
                    })
                    .collect(),
            ),
        ),
        (
            "defaults",
            Tag::Compound(
                names
                    .iter()
                    .enumerate()
                    .map(|(id, name)| ((*name).into(), Tag::Int(id as i32)))
                    .collect(),
            ),
        ),
    ]))
    .unwrap()
}
#[test]
fn rejects_incompatible_regions_and_routes_negative_chunk_coordinates() {
    let other_registry = registry();
    let registry = registry();
    let context = WorldGenerationContext::new(0, 16, 8);
    let chunk =
        |x, min, reg| TerrainChunk::new(ChunkPos::new(x, 0), min, 16, reg, Properties).unwrap();
    assert!(TerrainRegion::<Properties>::new(vec![], context).is_err());
    assert!(
        TerrainRegion::new(
            vec![chunk(0, 0, &registry), chunk(0, 0, &registry)],
            context
        )
        .is_err()
    );
    assert!(
        TerrainRegion::new(
            vec![chunk(0, 0, &registry), chunk(1, 16, &registry)],
            context
        )
        .is_err()
    );
    assert!(
        TerrainRegion::new(
            vec![chunk(0, 0, &registry), chunk(1, 0, &other_registry)],
            context
        )
        .is_err()
    );
    let mut region = TerrainRegion::new(
        vec![chunk(-1, 0, &registry), chunk(0, 0, &registry)],
        context,
    )
    .unwrap();
    region
        .set_block_worldgen([-1, 7, 0], BlockStateId(2))
        .unwrap();
    assert_eq!(region.block([-1, 7, 0]).unwrap(), BlockStateId(2));
    assert_eq!(region.block([15, 7, 0]).unwrap(), BlockStateId(0));
    assert!(matches!(
        region.set_block_worldgen([16, 7, 0], BlockStateId(2)),
        Err(RegionError::MissingChunk([1, 0]))
    ));
    assert_eq!(region.block([0, 7, 0]).unwrap(), BlockStateId(0));
}
/// The column scan matches reading block by block: from inside and outside
/// the build range, both ways, across all-air and mixed sections.
#[test]
fn column_scan_matches_block_by_block_reads() {
    let registry = registry();
    let chunk = TerrainChunk::new(ChunkPos::new(-1, 0), -32, 64, &registry, Properties).unwrap();
    let mut region =
        TerrainRegion::new(vec![chunk], WorldGenerationContext::new(-32, 64, 8)).unwrap();
    for (y, state) in [
        (-30, 2),
        (-17, 3),
        (-16, 2),
        (5, 4),
        (6, 0),
        (20, 2),
        (31, 3),
    ] {
        region
            .set_block_worldgen([-13, y, 5], BlockStateId(state))
            .unwrap();
    }
    let keeps: [fn(BlockStateId) -> bool; 3] = [|s| s.0 < 2, |s| s.0 == 0, |s| s.0 != 2];
    for keep in keeps {
        for start in -40..40 {
            for step in [1, -1] {
                for limit in [0, 1, 3, 15, 16, 17, 40, 90] {
                    let mut y = start;
                    for _ in 0..limit {
                        if !keep(region.block([-13, y, 5]).unwrap()) {
                            break;
                        }
                        y += step;
                    }
                    assert_eq!(
                        region
                            .scan_column([-13, start, 5], step, limit, keep)
                            .unwrap(),
                        y,
                        "start {start} step {step} limit {limit}"
                    );
                }
            }
        }
    }
}
#[test]
fn write_callback_controls_mutation_and_receives_all_flags() {
    let registry = registry();
    let mut chunk = TerrainChunk::new(ChunkPos::new(0, 0), 0, 16, &registry, Properties).unwrap();
    chunk.set_generation_stage(PreLightChunkStage::Terrain);
    let mut region =
        TerrainRegion::new(vec![chunk], WorldGenerationContext::new(-16, 32, 8)).unwrap();
    assert_eq!(region.generation(), WorldGenerationContext::new(0, 16, 8));
    assert_eq!(region.height(HeightmapType::WorldSurface, 0, 0).unwrap(), 0);
    let mut flags = Vec::new();
    {
        let mut world = TerrainFeatureAccess::new(
            &mut region,
            |region: &mut TerrainRegion<'_, Properties>, p, state, flag| {
                flags.push(flag);
                if flag == 2 {
                    return Ok(false);
                }
                region.set_block_worldgen(p, state)?;
                Ok(true)
            },
        );
        assert!(!world.set_block([0, 7, 0], BlockStateId(2), 2).unwrap());
        assert_eq!(world.height(HeightmapType::WorldSurface, 0, 0).unwrap(), 0);
        assert!(world.set_block([0, 7, 0], BlockStateId(2), 3).unwrap());
        assert_eq!(world.height(HeightmapType::WorldSurface, 0, 0).unwrap(), 8);
    }
    assert_eq!(flags, [2, 3]);
}

#[test]
fn postprocessing_routes_to_resident_chunks_and_preserves_partial_marks() {
    let registry = registry();
    let chunk = TerrainChunk::new(ChunkPos::new(-1, 1), -16, 32, &registry, Properties).unwrap();
    let mut region =
        TerrainRegion::new(vec![chunk], WorldGenerationContext::new(-16, 32, 8)).unwrap();
    {
        let mut world = TerrainFeatureAccess::new(
            &mut region,
            |_: &mut TerrainRegion<'_, Properties>, _: [i32; 3], _: BlockStateId, _: i32| {
                Ok::<_, RegionError>(false)
            },
        );
        world.mark_postprocessing([-1, -1, 16]).unwrap();
        world.mark_postprocessing([-1, -1, 16]).unwrap();
        world.mark_postprocessing([-16, 0, 31]).unwrap();
        world.mark_postprocessing([-1, -17, 16]).unwrap();
        world.mark_postprocessing([-1, 16, 16]).unwrap();
        assert!(world.mark_postprocessing([0, 0, 16]).is_err());
    }
    assert_eq!(
        region.chunk(-1, 1).unwrap().postprocessing(),
        &[vec![255, 255], vec![3840]]
    );
}

#[test]
fn query_bindings_reject_foreign_registries_and_preserve_resolved_tag_snapshot() {
    use horizon_world::{
        fluid::FluidKind,
        worldgen::{block_feature::FeatureBlockBindings, placement::PlacementQueryError},
    };
    use std::collections::{BTreeMap, BTreeSet};
    let foreign = registry();
    let registry = registry();
    let chunk = || TerrainChunk::new(ChunkPos::new(-1, 0), 0, 16, &registry, Properties).unwrap();
    let context = WorldGenerationContext::new(0, 16, 8);
    let bindings = FeatureBlockBindings::new(&foreign, BTreeSet::new()).unwrap();
    assert!(matches!(
        TerrainRegion::new(vec![chunk()], context)
            .unwrap()
            .with_block_bindings(&bindings),
        Err(RegionError::InvalidBindings)
    ));
    let mut region = TerrainRegion::new(vec![chunk()], context).unwrap();
    assert_eq!(
        region.fluid_state([-1, 0, 0]),
        Err(PlacementQueryError::Unavailable)
    );
    let tags = BTreeMap::from([("horizon:ground".into(), vec!["minecraft:stone".into()])]);
    // Deliberately make the water tag contain EMPTY, not water, to catch a
    // hard-coded water-family check instead of resolved tag membership.
    let fluids = BTreeMap::from([("minecraft:water".into(), vec!["minecraft:empty".into()])]);
    let bindings = FeatureBlockBindings::with_tags(&registry, BTreeSet::new(), &tags)
        .unwrap()
        .with_fluid_tags(&fluids)
        .unwrap();
    region = region.with_block_bindings(&bindings).unwrap();
    drop(bindings);
    region
        .set_block_worldgen([-1, 0, 0], BlockStateId(3))
        .unwrap();
    assert_eq!(
        region.fluid_state([-1, 0, 0]).unwrap().kind(),
        FluidKind::Water
    );
    assert!(!region.water_at([-1, 0, 0]).unwrap());
    region
        .set_block_worldgen([-1, 0, 0], BlockStateId(2))
        .unwrap();
    assert!(
        region
            .block_matches_tag([-1, 0, 0], "horizon:ground")
            .unwrap()
    );
    assert!(
        !region
            .block_matches_tag([-1, 0, 0], "horizon:missing")
            .unwrap()
    );
    assert!(region.water_at([-1, 0, 0]).unwrap());
    assert!(region.water_at([-1, 16, 0]).unwrap());
    assert!(region.fluid_state([0, 0, 0]).is_err());
}

#[test]
fn upgrade_height_rejects_mutations_before_callbacks_but_preserves_reads_and_ticks() {
    use horizon_world::worldgen::write_zone::{GenerationWriteZone, UpgradeWriteHeight};
    use std::cell::Cell;
    let registry = registry();
    let mut chunk =
        TerrainChunk::new(ChunkPos::new(0, 0), -64, 128, &registry, Properties).unwrap();
    let stone = registry.default_state("minecraft:stone").unwrap();
    chunk.set_block_worldgen(0, 0, 0, stone).unwrap();
    let mut region =
        TerrainRegion::new(vec![chunk], WorldGenerationContext::new(-64, 128, 0)).unwrap();
    let calls = Cell::new(0);
    {
        let write = |region: &mut TerrainRegion<'_, Properties>, p, state, _| {
            calls.set(calls.get() + 1);
            region.set_block_worldgen(p, state)?;
            Ok(true)
        };
        let mut world = TerrainFeatureAccess::new(&mut region, write)
            .with_write_zone(GenerationWriteZone::new(ChunkPos::new(0, 0), 0))
            .with_upgrade_height(UpgradeWriteHeight::BELOW_ZERO);
        assert!(!world.set_block([0, 0, 0], stone, 2).unwrap());
        assert!(!world.destroy_block_without_entity([0, 0, 0], true).unwrap());
        assert!(!world.set_block([0, -65, 0], stone, 2).unwrap());
        assert!(!world.set_block([16, -1, 0], stone, 2).unwrap());
        assert_eq!(calls.get(), 0);
        assert_eq!(world.block_state([0, 0, 0]).unwrap(), stone);
        world.schedule_block_tick([0, 0, 0], stone, 1).unwrap();
        assert!(world.set_block([0, -1, 0], stone, 2).unwrap());
        assert!(
            world
                .destroy_block_without_entity([0, -1, 0], true)
                .unwrap()
        );
        assert_eq!(calls.get(), 2);
    }
    assert_eq!(region.block([0, 0, 0]).unwrap(), stone);
    assert_eq!(region.chunk(0, 0).unwrap().block_ticks().count(), 1);
    let write = |_: &mut TerrainRegion<'_, Properties>, _, _, _| {
        calls.set(calls.get() + 1);
        Ok(true)
    };
    let mut world = TerrainFeatureAccess::new(&mut region, write);
    assert!(world.set_block([0, i32::MAX, 0], stone, 2).unwrap());
    assert_eq!(calls.get(), 3);
}

#[test]
fn live_center_upgrade_policy_observes_callback_transitions() {
    use horizon_world::{chunk_status::ChunkStatus, retrogen::BelowZeroRetrogen};
    let registry = registry();
    let mut chunks = Vec::new();
    for x in 0..2 {
        let mut chunk =
            TerrainChunk::new(ChunkPos::new(x, 0), -64, 128, &registry, Properties).unwrap();
        chunk.set_below_zero_retrogen(Some(
            BelowZeroRetrogen::new(ChunkStatus::Features, vec![]).unwrap(),
        ));
        chunks.push(chunk);
    }
    let mut region = TerrainRegion::new(chunks, WorldGenerationContext::new(-64, 128, 0)).unwrap();
    let mut calls = Vec::new();
    {
        let write = |region: &mut TerrainRegion<'_, Properties>, p, _, flags| {
            calls.push((p, flags));
            let center = region.chunk_mut(0, 0)?;
            match flags {
                1 => center.set_generation_stage(PreLightChunkStage::Features),
                2 => center.set_below_zero_retrogen(Some(
                    BelowZeroRetrogen::new(ChunkStatus::Full, vec![]).unwrap(),
                )),
                3 => center.set_below_zero_retrogen(None),
                _ => {}
            }
            Ok(true)
        };
        let mut world = TerrainFeatureAccess::new(&mut region, write)
            .with_upgrade_center(ChunkPos::new(0, 0))
            .unwrap();
        assert!(!world.set_block([16, 0, 0], BlockStateId(2), 1).unwrap());
        assert!(world.set_block([16, -1, 0], BlockStateId(2), 1).unwrap());
        // The neighbor remains upgrading, but only the center selects the gate.
        assert!(world.set_block([16, 0, 0], BlockStateId(2), 2).unwrap());
        assert!(!world.set_block([16, 0, 0], BlockStateId(2), 3).unwrap());
        assert!(world.set_block([16, -64, 0], BlockStateId(2), 3).unwrap());
        assert!(
            world
                .set_block([16, i32::MAX, 0], BlockStateId(2), 0)
                .unwrap()
        );
    }
    assert_eq!(calls.len(), 4);
    assert!(!region.chunk(0, 0).unwrap().is_upgrading());
    assert!(region.chunk(1, 0).unwrap().is_upgrading());
    assert!(
        TerrainFeatureAccess::new(&mut region, ())
            .with_upgrade_center(ChunkPos::new(2, 0))
            .is_err()
    );
}

#[test]
fn retained_upgrade_status_allows_old_biome_reads() {
    use horizon_world::{
        chunk_status::ChunkStatus,
        retrogen::BelowZeroRetrogen,
        worldgen::biome_chunk::{BiomeChunk, BiomeRegion},
    };
    let registry = registry();
    let position = ChunkPos::new(-1, 1);
    let biomes =
        BiomeRegion::new(vec![BiomeChunk::new(position, -64, 128, 7).unwrap()], 0).unwrap();
    for stage in PreLightChunkStage::ALL {
        for target in ChunkStatus::ALL.into_iter().skip(1) {
            let mut chunk = TerrainChunk::new(position, -64, 128, &registry, Properties).unwrap();
            chunk.set_generation_stage(stage);
            chunk.set_below_zero_retrogen(Some(BelowZeroRetrogen::new(target, vec![]).unwrap()));
            let mut region =
                TerrainRegion::new(vec![chunk], WorldGenerationContext::new(-64, 128, 0)).unwrap();
            for y in [i32::MIN, -16, 0, 15, i32::MAX] {
                let result = region.noise_biome(&biomes, [-1, y, 4]);
                if stage.status() >= ChunkStatus::Biomes || target >= ChunkStatus::Biomes {
                    assert_eq!(*result.unwrap(), 7);
                } else {
                    assert!(matches!(
                        result,
                        Err(RegionError::BiomesNotGenerated([-1, 1]))
                    ));
                }
            }
            region
                .chunk_mut(-1, 1)
                .unwrap()
                .set_below_zero_retrogen(None);
            assert_eq!(
                region.noise_biome(&biomes, [-1, 0, 4]).is_ok(),
                stage.status() >= ChunkStatus::Biomes
            );
        }
    }
}
#[test]
fn spread_out_regions_route_like_dense_ones() {
    // Chunks too far apart for a grid over their bounding box (fixtures put
    // one at the far edge of the world beside the origin's neighbours).
    let registry = registry();
    let context = WorldGenerationContext::new(0, 16, 8);
    let chunk =
        |x, z| TerrainChunk::new(ChunkPos::new(x, z), 0, 16, &registry, Properties).unwrap();
    for far in [4_194_304, -4_194_304] {
        let mut region = TerrainRegion::new(
            vec![chunk(0, 0), chunk(far, 0), chunk(1, -1), chunk(-1, 1)],
            context,
        )
        .unwrap();
        let x = far * 16 + 3;
        region
            .set_block_worldgen([x, 7, 5], BlockStateId(2))
            .unwrap();
        assert_eq!(region.block([x, 7, 5]).unwrap(), BlockStateId(2));
        assert_eq!(region.block([3, 7, 5]).unwrap(), BlockStateId(0));
        assert_eq!(region.block([16, 7, -1]).unwrap(), BlockStateId(0));
        assert!(matches!(
            region.block([x + 16, 7, 5]),
            Err(RegionError::MissingChunk([_, 0]))
        ));
        assert!(matches!(
            region.block([0, 7, 16]),
            Err(RegionError::MissingChunk([0, 1]))
        ));
        let order: Vec<_> = region
            .into_chunks()
            .iter()
            .map(|c| (c.position().x, c.position().z))
            .collect();
        let mut sorted = order.clone();
        sorted.sort();
        assert_eq!(order, sorted, "into_chunks is ordered by X then Z");
        // A position twice is refused either way.
        assert!(
            TerrainRegion::new(vec![chunk(0, 0), chunk(far, 0), chunk(far, 0)], context).is_err()
        );
    }
}
