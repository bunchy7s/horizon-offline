//! Small End islands at far distances.
//!
//! The End's small islands are decoration (`minecraft:end_island_decorated` in
//! `minecraft:small_end_islands`), not terrain density. The generator replays
//! vanilla's decoration seed and feature on a no-op world, so the origins and
//! heights are exact without generating chunks; the visible shape is the cone
//! the direct sampler always drew. Refined, visited and saved chunks still win.
use super::vegetation::hash;
use crate::{
    fast_hash::FxHashMap,
    lod::{Column, Span, Surface},
    vanillagen::VanillaTerrain,
};
use horizon_core::position::ChunkPos;
use horizon_world::registry::BlockStateId;
use std::{collections::VecDeque, sync::Arc};

/// A cell is drawn as a whole column; past this width a 4–6 block island would
/// be a slab. At 8 blocks a whole island is about one cell, which is enough at
/// the distance where direct sampling takes over.
const END_ISLAND_CELL: i32 = 8;
/// `EndIslandFeature` starts at radius 4..=6.
const MAX_RADIUS: i32 = 6;
const CACHE: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Island {
    x: i32,
    z: i32,
    y: i32,
    /// Top radius, 4..=6 blocks.
    radius: i32,
    /// Per-layer shrink in half blocks: 1 = 0.5, 3 = 1.5.
    shrink: i32,
}

pub(super) struct EndIslands {
    generator: Arc<VanillaTerrain>,
    end_stone: Option<BlockStateId>,
    cache: FxHashMap<(i32, i32), Arc<[Island]>>,
    order: VecDeque<(i32, i32)>,
}
impl EndIslands {
    pub(super) fn new(generator: Arc<VanillaTerrain>) -> Self {
        let end_stone = generator
            .generator
            .data()
            .registry()
            .default_state("minecraft:end_stone");
        Self {
            generator,
            end_stone,
            cache: FxHashMap::default(),
            order: VecDeque::new(),
        }
    }
    /// Independent sampling state: the generator is shared, the cache is not.
    pub(super) fn fork(&self) -> Self {
        Self {
            generator: self.generator.clone(),
            end_stone: self.end_stone,
            cache: FxHashMap::default(),
            order: VecDeque::new(),
        }
    }
    /// Paint the small islands of the chunks around a direct-sampled void cell.
    pub(super) fn apply(&mut self, column: &mut Column, x: i32, z: i32, width: i32, light: u16) {
        let Some(state) = self.end_stone else {
            return;
        };
        if width > END_ISLAND_CELL || width <= 0 || column.surface.is_some() {
            return;
        }
        let (last_x, last_z) = (x + width - 1, z + width - 1);
        let mut spans = Vec::new();
        for cz in (z - MAX_RADIUS).div_euclid(16)..=(last_z + MAX_RADIUS).div_euclid(16) {
            for cx in (x - MAX_RADIUS).div_euclid(16)..=(last_x + MAX_RADIUS).div_euclid(16) {
                let islands = self.layout(cx, cz);
                for island in islands.iter() {
                    let nx = island.x.clamp(x, last_x);
                    let nz = island.z.clamp(z, last_z);
                    let d2 = (nx - island.x) * (nx - island.x) + (nz - island.z) * (nz - island.z);
                    let Some(bottom) = island_bottom(island, d2) else {
                        continue;
                    };
                    spans.push((bottom, island.y + 1));
                }
            }
        }
        if spans.is_empty() {
            return;
        }
        spans.sort_unstable_by_key(|span| span.0);
        let mut merged: Vec<(i32, i32)> = Vec::with_capacity(spans.len());
        for (min, max) in spans {
            if let Some(last) = merged.last_mut()
                && min <= last.1
            {
                last.1 = last.1.max(max);
            } else {
                merged.push((min, max));
            }
        }
        for &(min, max) in &merged {
            column.push(Span {
                min: min as i16,
                max: max as i16,
                state,
                biome: 0,
                light,
            });
        }
        let top = merged.last().unwrap().1;
        column.surface = Some(Surface {
            height: top,
            state,
            water: None,
            biome: 0,
        });
    }
    fn layout(&mut self, cx: i32, cz: i32) -> Arc<[Island]> {
        let key = (cx, cz);
        if let Some(islands) = self.cache.get(&key) {
            return islands.clone();
        }
        let origins = self
            .generator
            .generator
            .end_island_origins(ChunkPos::new(cx, cz));
        let islands: Arc<[Island]> = origins
            .into_iter()
            .map(|[x, y, z]| {
                let n = hash(
                    self.generator.seed as u64
                        ^ (x as u64).rotate_left(17)
                        ^ (z as u64).rotate_left(33)
                        ^ (y as u64),
                    x,
                    z,
                );
                Island {
                    x,
                    z,
                    y,
                    radius: 4 + (n % 3) as i32,
                    shrink: 1 + 2 * ((n >> 8) & 1) as i32,
                }
            })
            .collect();
        self.cache.insert(key, islands.clone());
        self.order.push_back(key);
        while self.cache.len() > CACHE {
            if let Some(old) = self.order.pop_front() {
                self.cache.remove(&old);
            }
        }
        islands
    }
    #[cfg(test)]
    pub(super) fn origins(&self, cx: i32, cz: i32) -> Vec<[i32; 3]> {
        self.generator
            .generator
            .end_island_origins(ChunkPos::new(cx, cz))
    }
}

/// The lowest layer of the island's cone that reaches a column at squared
/// distance `d2`. The top layer is always `island.y` when this returns `Some`.
fn island_bottom(island: &Island, d2: i32) -> Option<i32> {
    let mut half = island.radius * 2;
    let mut y = island.y;
    let mut bottom = None;
    while half > 1 {
        if d2 * 4 <= half * half {
            bottom = Some(y);
        }
        half -= island.shrink;
        y -= 1;
    }
    bottom
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn island_columns_are_cones_with_a_flat_top() {
        let island = Island {
            x: 0,
            z: 0,
            y: 60,
            radius: 4,
            shrink: 1,
        };
        assert_eq!(island_bottom(&island, 0), Some(54));
        assert_eq!(island_bottom(&island, 3), Some(56));
        assert_eq!(island_bottom(&island, 16), Some(60));
        assert_eq!(island_bottom(&island, 17), None);
        let steep = Island {
            shrink: 3,
            ..island
        };
        assert_eq!(island_bottom(&steep, 0), Some(58));
    }
}
