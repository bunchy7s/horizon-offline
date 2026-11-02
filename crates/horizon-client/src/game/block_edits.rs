//! A server mod's bulk block edit (`native_edit_blocks`): its palette and
//! filters read as commands read blocks, its heights read, and its runs made
//! at once through `World::edit_runs`, with what `fill ... strict` does for
//! the block entities and the watched blocks of each block changed.
use super::Game;
use super::native_world_columns::NativeColumnBlocks as ColumnBlocks;
use super::native_world_edit::{NativeBlockEdit as BlockEdit, NativeHeightmap as Heightmap};
use horizon_client::{
    block_edit::Run,
    chunk::ColumnLayout,
    command::{Reader, args::BlockInput, block},
    containers::BlockEntity,
    fast_hash::{FxHashMap, FxHashSet},
    world::World,
};
use horizon_world::{
    heightmap::HeightmapType, registry::BlockStateId, worldgen::terrain::TerrainBlockStates,
};
use std::sync::Arc;

/// A change `World::edit_runs` reported: position, previous and new state.
type Tracked = ((i32, i32, i32), u32, u32);

impl Game<'_> {
    /// Make a mod's block edit; how many blocks changed, or why none did.
    /// Every palette entry, filter and run is checked before anything
    /// changes: an entry or filter that doesn't read, one with NBT (`{...}`,
    /// refused: the bulk path sets states only), a run naming a palette entry
    /// or filter past the end, or more than 65536 filters, and nothing is
    /// made.
    pub(super) fn edit_blocks(&mut self, edit: &BlockEdit) -> Result<u64, String> {
        let parse = |name: &str, testing: bool| -> Result<BlockInput, String> {
            let mut reader = Reader::new(name);
            let b = block::parse(&mut reader, &self.ui.registries, testing)
                .map_err(|e| format!("{name:?}: {}", e.message.plain()))?;
            if reader.can_read() {
                return Err(format!("{name:?}: more after the block"));
            }
            if b.nbt.is_some() {
                return Err(format!("{name:?}: block NBT is not supported here"));
            }
            Ok(b)
        };
        let mut palette = Vec::with_capacity(edit.palette.len());
        for name in &edit.palette {
            let b = parse(name, false)?;
            let state = self
                .block_state(&b)
                .map_err(|e| format!("{name:?}: {}", e.message.plain()))?;
            palette.push(state.0);
        }
        let filters = edit
            .filters
            .iter()
            .map(|f| parse(f, true))
            .collect::<Result<Vec<_>, _>>()?;
        let runs = resolve_runs(edit, &palette, filters.len(), |h, x, z| {
            self.edit_height(h, x, z)
        })?;
        // (what the registry, the tags and the facts tell of a state, beside the world being changed)
        let registry = self.pack.assets.registry;
        // (the tags as `block_matches` reads them for commands)
        let world_items = match &self.world_tables {
            Some(t) => &t.items,
            None => self.res.items(),
        };
        let tags = [&self.res.items().block_tags, &world_items.block_tags];
        let facts = self.res.facts();
        let watched = std::mem::take(&mut self.world.watched);
        let out = self.world.edit_runs(
            &runs,
            |f, s| {
                super::commands::block_input_matches(
                    registry,
                    tags,
                    &filters[f as usize],
                    BlockStateId(s),
                )
            },
            |s| {
                let f = facts.get(BlockStateId(s));
                watched.get(s as usize).copied().unwrap_or(false)
                    || f.block_entity
                    || BlockEntity::for_block(&f.name).is_some()
            },
        );
        self.world.watched = watched;
        // The block entities the new blocks bring, as `native_set_block` makes them (a mod block's own, API 15,
        // included), in the order `replace_block_entities` takes them: the positions whose block changed.
        let name = |s: u32| self.res.facts().get(BlockStateId(s)).name.clone();
        let wanted: Vec<Arc<str>> = out
            .tracked
            .iter()
            .filter(|&&(_, old, new)| name(old) != name(new))
            .map(|&(_, _, new)| name(new))
            .collect();
        let mut made = make_block_entities(&wanted, |n| self.new_block_entity(n)).into_iter();
        let facts = self.res.facts();
        let removed = replace_block_entities(
            &mut self.world,
            &out.tracked,
            |s| facts.get(BlockStateId(s)).name.clone(),
            |_| made.next().flatten(),
        );
        for pos in removed {
            self.jukebox_removed(pos);
        }
        Ok(out.changed)
    }

    /// Each column's height on `h` as `edit_height` reads it, none where its chunk is not loaded.
    pub(super) fn surface_heights(&self, columns: &[(i32, i32)], h: Heightmap) -> Vec<Option<i32>> {
        loaded_heights(&self.world, columns, |x, z| self.edit_height(h, x, z))
    }

    /// Each column's height on `h` (none where its chunk is not loaded) and its blocks from `below` under it to
    /// `above` - 1 over it, bottom first (air outside the column and where it is not loaded), as indices into the
    /// names seen, with the biome's grass, foliage and water tints at the column's top (0 where it is not loaded;
    /// unblended). `Err`, reading nothing, when `below + above` is 0 or more than the level's height, or the read
    /// would be more than `MAX_READ` blocks.
    pub(super) fn read_columns(
        &self,
        columns: &[(i32, i32)],
        h: Heightmap,
        below: u32,
        above: u32,
    ) -> Result<ColumnBlocks, String> {
        read_size(columns.len(), below, above, self.world.layout().height())?;
        let bases = self.surface_heights(columns, h);
        let facts = self.res.facts();
        let (palette, blocks) = column_blocks(&self.world, columns, &bases, below, above, |s| {
            facts.get(s).name.clone()
        })
        .ok_or("the read is too big")?;
        let biomes = self.res.biomes();
        let mut tints = Vec::with_capacity(columns.len() * 3);
        for (&(x, z), base) in columns.iter().zip(&bases) {
            // (the biome tints at the column's top, as the block there is tinted)
            match base.and_then(|b| {
                self.world
                    .chunks
                    .get(&(x >> 4, z >> 4))
                    .map(|c| c.biome(x, b - 1, z))
            }) {
                Some(id) => {
                    let b = biomes.get(id);
                    tints.extend([
                        biomes.grass(id, f64::from(x), f64::from(z)),
                        b.foliage,
                        b.water,
                    ]);
                }
                None => tints.extend([0, 0, 0]),
            }
        }
        Ok(ColumnBlocks {
            palette,
            bases,
            blocks,
            tints,
        })
    }

    /// One above the highest block of column (x, z) that counts on heightmap `h`, or the bottom of the column (also
    /// where it is not loaded).
    fn edit_height(&self, h: Heightmap, x: i32, z: i32) -> i32 {
        let properties = &self.pack.assets.properties;
        column_height(
            &self.world,
            h,
            (x, z),
            |s| self.res.facts().is_air(s),
            |t, s| t.is_opaque(properties.heightmap_state(s)),
        )
    }
}

/// The most blocks one `read_columns` reads (2 MiB of indices): it runs on the
/// server tick, so a mod reads a big area in pieces.
pub(super) const MAX_READ: usize = 1 << 20;
/// The most columns one `surface_heights` reads.
pub(super) const MAX_COLUMNS: usize = 1 << 16;

/// The blocks a column read of `columns` columns, `below` + `above` each,
/// reads, or why it can't: a span of 0 or more than `height` (the level's),
/// or the whole more than `MAX_READ`.
fn read_size(columns: usize, below: u32, above: u32, height: i32) -> Result<usize, String> {
    let per = below
        .checked_add(above)
        .filter(|&per| per > 0 && i64::from(per) <= i64::from(height))
        .ok_or_else(|| format!("below + above must be 1 to {height}, the level's height"))?;
    columns
        .checked_mul(per as usize)
        .filter(|&n| n <= MAX_READ)
        .ok_or_else(|| {
            format!("the read is more than {MAX_READ} blocks: read fewer columns a call")
        })
}

/// One above the highest block of column `(x, z)` on heightmap `h`, or the
/// bottom of the column (also where it is not loaded): `air` says which
/// blocks the world surface passes over, and `counts(kind, state)` which
/// count on the other heightmaps.
fn column_height(
    world: &World,
    h: Heightmap,
    (x, z): (i32, i32),
    air: impl Fn(BlockStateId) -> bool,
    counts: impl Fn(HeightmapType, BlockStateId) -> bool,
) -> i32 {
    if h == Heightmap::WorldSurface {
        return world.world_surface(x, z, air);
    }
    // (as `rain_height` reads it: both others count only blocks that block motion, so they are at or under it)
    let top = world.motion_blocking(x, z, |s| counts(HeightmapType::MotionBlocking, s));
    let kind = match h {
        Heightmap::MotionBlockingNoLeaves => HeightmapType::MotionBlockingNoLeaves,
        Heightmap::OceanFloor => HeightmapType::OceanFloor,
        _ => return top,
    };
    let bottom = world
        .chunks
        .get(&(x >> 4, z >> 4))
        .map_or(world.layout().min_y(), |c| c.min_y());
    let mut y = top;
    while y > bottom && !counts(kind, world.block(x, y - 1, z)) {
        y -= 1;
    }
    y
}

/// `height(x, z)` of each column, none where its chunk is not loaded. (A
/// chunk is looked up once for a run of columns in it.)
fn loaded_heights(
    world: &World,
    columns: &[(i32, i32)],
    height: impl Fn(i32, i32) -> i32,
) -> Vec<Option<i32>> {
    let mut last: Option<((i32, i32), bool)> = None;
    columns
        .iter()
        .map(|&(x, z)| {
            let key = (x >> 4, z >> 4);
            let loaded = match last {
                Some((k, l)) if k == key => l,
                _ => {
                    let l = world.chunks.get(&key).is_some();
                    last = Some((key, l));
                    l
                }
            };
            loaded.then(|| height(x, z))
        })
        .collect()
}

/// The blocks of each column from `below` under its base to `above` - 1 over
/// it, bottom first, as indices into the names (`name`) seen: air outside the
/// column's chunk's own height and where there is no base. None, reading
/// nothing, past `read_size`'s bound.
fn column_blocks(
    world: &World,
    columns: &[(i32, i32)],
    bases: &[Option<i32>],
    below: u32,
    above: u32,
    name: impl Fn(BlockStateId) -> Arc<str>,
) -> Option<(Vec<String>, Vec<u16>)> {
    let total = read_size(
        columns.len(),
        below,
        above,
        ColumnLayout::HIGHEST_Y + 1 - ColumnLayout::LOWEST_Y,
    )
    .ok()?;
    let per = (below + above) as usize;
    let mut blocks = Vec::with_capacity(total);
    let mut palette: Vec<String> = Vec::new();
    // (a state's index into the names, worked out once a state)
    let mut seen: FxHashMap<u32, u16> = FxHashMap::default();
    let mut index = |s: BlockStateId| -> u16 {
        *seen.entry(s.0).or_insert_with(|| {
            let name = name(s);
            let i = palette
                .iter()
                .position(|n| **n == *name)
                .unwrap_or_else(|| {
                    palette.push(name.to_string());
                    palette.len() - 1
                });
            i as u16
        })
    };
    for (&(x, z), base) in columns.iter().zip(bases) {
        let chunk = world.chunks.get(&(x >> 4, z >> 4));
        match (base, chunk) {
            (Some(b), Some(chunk)) => {
                let (bottom, top) = (chunk.min_y(), chunk.max_y());
                let y0 = i64::from(*b) - i64::from(below);
                for y in y0..y0 + per as i64 {
                    let s = match i32::try_from(y) {
                        Ok(y) if (bottom..top).contains(&y) => chunk.block(x & 15, y, z & 15),
                        _ => world.air,
                    };
                    blocks.push(index(s));
                }
            }
            _ => {
                let air = index(world.air);
                blocks.extend(std::iter::repeat_n(air, per));
            }
        }
    }
    Some((palette, blocks))
}

/// `edit`'s runs as `World::edit_runs` takes them: each palette entry `block`
/// as its state in `palette`, each `y` counted from `height(surface, x, z)`
/// when the run names a heightmap (read before any change, once a column).
/// Err, with nothing resolved, for a run naming a palette entry or a filter
/// past the end, or for more filters than a run can name (65536).
fn resolve_runs(
    edit: &BlockEdit,
    palette: &[u32],
    filters: usize,
    mut height: impl FnMut(Heightmap, i32, i32) -> i32,
) -> Result<Vec<Run>, String> {
    if filters > usize::from(u16::MAX) + 1 {
        return Err(format!("{filters} filters: 65536 at most"));
    }
    let mut heights: FxHashMap<(i32, i32, u8), i32> = FxHashMap::default();
    let mut runs = Vec::with_capacity(edit.runs.len());
    for (i, r) in edit.runs.iter().enumerate() {
        let Some(&state) = usize::try_from(r.block).ok().and_then(|b| palette.get(b)) else {
            return Err(format!(
                "run {i}: palette entry {} of {}",
                r.block,
                palette.len()
            ));
        };
        let only = match r.only {
            None => None,
            Some(f) if (f as usize) < filters => Some(f as u16),
            Some(f) => return Err(format!("run {i}: filter {f} of {filters}")),
        };
        if r.height == 0 {
            continue;
        }
        let base = match r.surface {
            None => i64::from(r.y),
            Some(h) => {
                let top = *heights
                    .entry((r.x, r.z, h as u8))
                    .or_insert_with(|| height(h, r.x, r.z));
                i64::from(top) + i64::from(r.y)
            }
        };
        let clamp = |v: i64| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        runs.push(Run {
            x: r.x,
            z: r.z,
            y0: clamp(base),
            y1: clamp(base + i64::from(r.height) - 1),
            state,
            only,
        });
    }
    Ok(runs)
}

/// What `fill ... strict` does for the block entities and the watched blocks
/// of `tracked`'s changes: the watched writes recorded, the block entities
/// told of their new state, and where a different block replaced the block,
/// its block entity removed (as `place_state` does) and `new_entity(name)`'s,
/// if any, put in its place. Returns the positions whose block was replaced
/// (a jukebox there stops).
/// A block entity for each new block, in order: made for each position, as vanilla calls
/// `EntityBlock.newBlockEntity` for each (a mod's may number its instances). A block that makes none (stone) is
/// asked once.
fn make_block_entities(
    names: &[Arc<str>],
    mut make: impl FnMut(&str) -> Option<BlockEntity>,
) -> Vec<Option<BlockEntity>> {
    let mut none: FxHashSet<&str> = FxHashSet::default();
    names
        .iter()
        .map(|n| {
            if none.contains(&**n) {
                return None;
            }
            let entity = make(n);
            if entity.is_none() {
                none.insert(n);
            }
            entity
        })
        .collect()
}

fn replace_block_entities(
    world: &mut World,
    tracked: &[Tracked],
    name: impl Fn(u32) -> Arc<str>,
    mut new_entity: impl FnMut(&str) -> Option<BlockEntity>,
) -> Vec<[i32; 3]> {
    let mut replaced = Vec::new();
    for &((x, y, z), old, new) in tracked {
        let watches = |id: u32| world.watched.get(id as usize).copied().unwrap_or(false);
        if watches(old) || watches(new) {
            world.watched_writes.push(((x, y, z), old, new));
        }
        world.block_entities.state_changed((x, y, z), old, new);
        let (was, now) = (name(old), name(new));
        if was != now {
            replaced.push([x, y, z]);
            world.block_entities.remove(&(x, y, z));
            if let Some(entity) = new_entity(&now) {
                world.block_entities.insert((x, y, z), entity);
            }
        }
    }
    replaced
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::native_world_edit::NativeBlockRun as BlockRun;
    use horizon_client::chunk::Chunk;

    fn run(block: u32, only: Option<u32>) -> BlockRun {
        BlockRun {
            x: 1,
            z: 2,
            y: 3,
            height: 4,
            surface: None,
            block,
            only,
        }
    }
    fn edit(runs: Vec<BlockRun>) -> BlockEdit {
        BlockEdit {
            palette: vec!["stone".into(), "air".into()],
            filters: vec!["#dirt".into()],
            runs,
        }
    }

    #[test]
    fn runs_naming_entries_past_the_end_are_refused_whole() {
        let palette = [1, 0];
        let ok = resolve_runs(&edit(vec![run(1, Some(0))]), &palette, 1, |_, _, _| 0).unwrap();
        assert_eq!(
            ok,
            [Run {
                x: 1,
                z: 2,
                y0: 3,
                y1: 6,
                state: 0,
                only: Some(0),
            }]
        );
        // a bad run after a good one: nothing at all, and no height read
        let mut asked = 0;
        let mut bad = |runs| {
            resolve_runs(&edit(runs), &palette, 1, |_, _, _| {
                asked += 1;
                0
            })
        };
        assert!(bad(vec![run(0, None), run(2, None)]).is_err());
        assert!(bad(vec![run(0, None), run(u32::MAX, None)]).is_err());
        assert!(bad(vec![run(0, Some(1))]).is_err());
        assert!(bad(vec![run(0, Some(u32::MAX))]).is_err());
        assert_eq!(asked, 0);
        // more filters than a run's u16 names
        assert!(resolve_runs(&edit(vec![]), &palette, 65_537, |_, _, _| 0).is_err());
        assert!(resolve_runs(&edit(vec![]), &palette, 65_536, |_, _, _| 0).is_ok());
    }

    #[test]
    fn a_run_counts_from_its_heightmap_read_once_a_column() {
        let mut r = run(0, None);
        r.surface = Some(Heightmap::OceanFloor);
        r.y = -2;
        let mut asked = Vec::new();
        let runs = resolve_runs(&edit(vec![r, r]), &[7], 0, |h, x, z| {
            asked.push((h, x, z));
            70
        })
        .unwrap();
        assert_eq!(asked, [(Heightmap::OceanFloor, 1, 2)]);
        assert_eq!((runs[0].y0, runs[0].y1, runs[0].state), (68, 71, 7));
        assert_eq!(runs[1], runs[0]);
        // a run far past the world's height clamps rather than wraps
        let mut high = run(0, None);
        high.y = i32::MAX;
        high.height = u32::MAX;
        let runs = resolve_runs(&edit(vec![high]), &[7], 0, |_, _, _| 0).unwrap();
        assert_eq!((runs[0].y0, runs[0].y1), (i32::MAX, i32::MAX));
    }

    fn world() -> World {
        let mut w = World::new(BlockStateId(0), 0);
        w.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        w
    }
    fn names(s: u32) -> Arc<str> {
        match s {
            0 => "minecraft:air",
            1 => "minecraft:stone",
            2 => "minecraft:chest",
            _ => "machines:press",
        }
        .into()
    }

    #[test]
    fn a_chest_replaced_by_stone_loses_its_block_entity() {
        let mut w = world();
        w.set(1, 5, 1, BlockStateId(2)).unwrap();
        let chest = BlockEntity::for_block("minecraft:chest").expect("a chest's");
        w.block_entities.insert((1, 5, 1), chest);
        let out = w.edit_runs(
            &[Run {
                x: 1,
                z: 1,
                y0: 4,
                y1: 6,
                state: 1,
                only: None,
            }],
            |_, _| true,
            |s| s == 2,
        );
        assert_eq!(out.tracked, [((1, 5, 1), 2, 1)]);
        let replaced = replace_block_entities(&mut w, &out.tracked, names, BlockEntity::for_block);
        assert_eq!(replaced, [[1, 5, 1]]);
        assert!(w.block_entities.get(&(1, 5, 1)).is_none());
        assert_eq!(w.block(1, 5, 1).0, 1);
    }

    #[test]
    fn a_new_block_gets_the_block_entity_made_for_it() {
        let mut w = world();
        let out = w.edit_runs(
            &[Run {
                x: 3,
                z: 3,
                y0: 10,
                y1: 10,
                state: 3,
                only: None,
            }],
            |_, _| true,
            |s| s == 3,
        );
        // a mod block's, as `Game::new_block_entity` makes it
        let made = |n: &str| {
            (n == "machines:press").then(|| {
                BlockEntity::Mod(Box::new(
                    horizon_client::containers::ModBlockEntityFields {
                        id: "machines:press".into(),
                        ..Default::default()
                    }
                    .into(),
                ))
            })
        };
        replace_block_entities(&mut w, &out.tracked, names, made);
        assert!(matches!(
            w.block_entities.get(&(3, 10, 3)),
            Some(BlockEntity::Mod(e)) if e.id == "machines:press"
        ));
    }

    /// air 0, stone 1, leaves 2, water 3: column (1, 1) stone to 60, water, then leaves to 64; column (2, 2)
    /// stone to 60, leaves, then water to 64; at (3, 3) stone at the top of the world.
    fn columns_world() -> World {
        let mut w = world();
        for (x, z, low, high) in [(1, 1, 3, 2), (2, 2, 2, 3)] {
            for y in -64..=64 {
                let s = match y {
                    ..=60 => 1,
                    61..=62 => low,
                    _ => high,
                };
                w.set(x, y, z, BlockStateId(s)).unwrap();
            }
        }
        w.set(3, 319, 3, BlockStateId(1)).unwrap();
        w
    }
    fn height(w: &World, h: Heightmap, x: i32, z: i32) -> i32 {
        column_height(
            w,
            h,
            (x, z),
            |s| s.0 == 0,
            |kind, s| match kind {
                HeightmapType::MotionBlockingNoLeaves => matches!(s.0, 1 | 3),
                HeightmapType::OceanFloor => matches!(s.0, 1 | 2),
                _ => s.0 != 0,
            },
        )
    }

    #[test]
    fn each_heightmap_has_its_own_height() {
        let w = columns_world();
        assert_eq!(height(&w, Heightmap::WorldSurface, 1, 1), 65);
        assert_eq!(height(&w, Heightmap::MotionBlocking, 1, 1), 65);
        assert_eq!(height(&w, Heightmap::MotionBlockingNoLeaves, 1, 1), 63);
        assert_eq!(height(&w, Heightmap::OceanFloor, 1, 1), 65);
        assert_eq!(height(&w, Heightmap::OceanFloor, 2, 2), 63);
        assert_eq!(height(&w, Heightmap::MotionBlockingNoLeaves, 2, 2), 65);
        // an empty column, and one not loaded: the bottom
        assert_eq!(height(&w, Heightmap::OceanFloor, 5, 5), -64);
        assert_eq!(height(&w, Heightmap::WorldSurface, 100, 100), -64);
    }

    #[test]
    fn columns_not_loaded_have_no_height() {
        let w = columns_world();
        let heights = loaded_heights(&w, &[(1, 1), (100, 100), (2, 2), (-1, 0)], |x, z| {
            height(&w, Heightmap::OceanFloor, x, z)
        });
        assert_eq!(heights, [Some(65), None, Some(63), None]);
    }

    #[test]
    fn column_blocks_read_air_past_the_column_and_where_not_loaded() {
        let w = columns_world();
        let (palette, blocks) = column_blocks(
            &w,
            &[(1, 1), (3, 3), (100, 100)],
            &[Some(65), Some(319), None],
            2,
            3,
            |s| names(s.0),
        )
        .unwrap();
        let read: Vec<&str> = blocks.iter().map(|&i| &*palette[i as usize]).collect();
        // (`names` calls state 2 a chest: here it stands for leaves)
        let (leaves, stone, air) = ("minecraft:chest", "minecraft:stone", "minecraft:air");
        assert_eq!(
            read,
            [
                leaves, leaves, air, air, air, // (1, 1) from 63
                air, air, stone, air, air, // (3, 3) from 317: stone at 319, nothing over 319
                air, air, air, air, air, // not loaded
            ]
        );
        // names once each
        assert_eq!(palette.len(), 3);
    }

    #[test]
    fn a_read_is_bounded() {
        assert_eq!(read_size(10, 200, 184, 384), Ok(3840));
        assert!(
            read_size(1, 200, 185, 384).is_err(),
            "more than the level's height"
        );
        assert!(read_size(1, 0, 0, 384).is_err(), "an empty span");
        assert!(read_size(1, u32::MAX, 1, 4064).is_err());
        assert!(read_size(1 << 14, 100, 0, 384).is_err(), "over MAX_READ");
        assert!(read_size(usize::MAX, 2, 0, 384).is_err());
        assert_eq!(read_size(1 << 12, 256, 0, 384), Ok(MAX_READ));
        let w = columns_world();
        assert!(column_blocks(&w, &[(1, 1)], &[Some(0)], u32::MAX, 2, |s| names(s.0)).is_none());
    }

    #[test]
    fn each_new_block_gets_its_own_block_entity() {
        // a mod's constructor numbering its instances; stone makes none
        let mut calls = Vec::new();
        let made = make_block_entities(
            &[
                "machines:press",
                "minecraft:stone",
                "machines:press",
                "minecraft:stone",
            ]
            .map(Arc::<str>::from),
            |n| {
                calls.push(n.to_string());
                (n == "machines:press").then(|| {
                    BlockEntity::Mod(Box::new(
                        horizon_client::containers::ModBlockEntityFields {
                            id: format!("press-{}", calls.len()),
                            ..Default::default()
                        }
                        .into(),
                    ))
                })
            },
        );
        let ids: Vec<Option<String>> = made
            .iter()
            .map(|e| match e {
                Some(BlockEntity::Mod(e)) => Some(e.id.to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(
            ids,
            [
                Some("press-1".to_string()),
                None,
                Some("press-3".to_string()),
                None
            ]
        );
        assert_eq!(
            calls,
            ["machines:press", "minecraft:stone", "machines:press"]
        );
    }
}
