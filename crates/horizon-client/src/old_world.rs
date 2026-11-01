//! The chunks an older version saved, as `Blender.of` reads them: which saved
//! chunks are old (`IOWorker.isOldChunkAround`), their `blending_data`, border
//! columns (`BlendingData.calculateData`) and the biomes they keep.
use crate::{biome::Biomes, chunk::Chunk, save::SaveDir};
use horizon_core::position::ChunkPos;
use horizon_world::{
    nbt::Tag,
    registry::BlockStateId,
    worldgen::{
        biome::BiomeId,
        biome_chunk::BiomeChunk,
        blending::{BlendingData, Direction8, OldChunk, OldWorld, Sides},
        generator::GeneratorData,
    },
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

/// A level's saved chunks that an older version generated.
pub struct SavedOldWorld {
    save: Arc<SaveDir>,
    assets: Arc<crate::assets::Assets>,
    biomes: Arc<Biomes>,
    data: Arc<GeneratorData>,
    /// The old chunks of each region file (`SaveDir::old_chunk_bits`).
    regions: Mutex<HashMap<(i32, i32), Arc<OnceLock<[u64; 16]>>>>,
    /// Each old chunk's blending data, its border columns calculated.
    blending: Mutex<HashMap<(i32, i32), Option<Arc<BlendingData>>>>,
    kept_biomes: Mutex<HashMap<(i32, i32), Option<Arc<BiomeChunk<BiomeId>>>>>,
}
impl SavedOldWorld {
    pub fn new(
        save: Arc<SaveDir>,
        assets: Arc<crate::assets::Assets>,
        biomes: Arc<Biomes>,
        data: Arc<GeneratorData>,
    ) -> Self {
        Self {
            save,
            assets,
            biomes,
            data,
            regions: Default::default(),
            blending: Default::default(),
            kept_biomes: Default::default(),
        }
    }
    /// One scan per region: the workers that ask while it runs wait for it
    /// (outside the map's lock) instead of each decoding the region's chunks.
    fn region_bits(&self, rx: i32, rz: i32) -> Arc<OnceLock<[u64; 16]>> {
        let cell = self
            .regions
            .lock()
            .unwrap()
            .entry((rx, rz))
            .or_default()
            .clone();
        cell.get_or_init(|| self.save.old_chunk_bits(rx, rz));
        cell
    }
    fn is_old(&self, x: i32, z: i32) -> bool {
        let bits = self.region_bits(x >> 5, z >> 5);
        let bits = bits.get().expect("scanned");
        let index = ((x & 31) + (z & 31) * 32) as usize;
        bits[index / 64] & (1 << (index % 64)) != 0
    }
    /// The saved chunk, decoded, if it is old and complete.
    fn old_chunk(&self, x: i32, z: i32) -> Option<Chunk> {
        if !self.is_old(x, z) {
            return None;
        }
        self.save
            .tables
            .get_or_init(|| Arc::new(crate::save::SaveTables::new(&self.assets)));
        self.save.load(x, z, self.assets.registry, &self.biomes)
    }
    fn client_to_generator_biome(&self, id: u16) -> BiomeId {
        let name = &self.biomes.get(id).name;
        self.data.biome_id(name).unwrap_or(BiomeId(0))
    }
    /// The packed `blending_data` of a saved chunk.
    fn packed(&self, x: i32, z: i32) -> Option<BlendingData> {
        let root = self.save.raw_chunk(x, z)?;
        let tag = root.tag.get("blending_data")?;
        let int = |key: &str| match tag.get(key) {
            Some(Tag::Int(v)) => Some(*v),
            _ => None,
        };
        let heights = match tag.get("heights") {
            Some(Tag::List(list)) => Some(
                list.iter()
                    .map(|v| match v {
                        Tag::Float(bits) => f32::from_bits(*bits),
                        Tag::Double(bits) => f64::from_bits(*bits) as f32,
                        _ => f32::MAX,
                    })
                    .collect::<Vec<f32>>(),
            ),
            _ => None,
        };
        BlendingData::unpack(int("min_section")?, int("max_section")?, heights.as_deref())
    }
}
struct View<'a> {
    chunk: &'a Chunk,
    data: &'a GeneratorData,
    old: &'a SavedOldWorld,
}
impl OldChunk for View<'_> {
    fn has_surface_heightmap(&self) -> bool {
        // A saved full chunk keeps the four final heightmaps only.
        false
    }
    fn surface_height(&self, _: i32, _: i32) -> i32 {
        unreachable!("a saved chunk has no worldgen heightmap")
    }
    fn is_surface_block(&self, x: i32, y: i32, z: i32) -> bool {
        self.data
            .is_old_chunk_surface(BlockStateId(self.chunk.get(x, y, z)))
    }
    fn is_ground(&self, x: i32, y: i32, z: i32) -> bool {
        self.data
            .is_old_chunk_ground(BlockStateId(self.chunk.get(x, y, z)))
    }
    fn noise_biome(&self, qx: i32, qy: i32, qz: i32) -> BiomeId {
        self.old
            .client_to_generator_biome(self.chunk.biome(qx * 4, qy * 4, qz * 4))
    }
}
impl OldWorld for SavedOldWorld {
    fn is_old_chunk_around(&self, center: ChunkPos, range: i32) -> bool {
        for rx in (center.x - range) >> 5..=(center.x + range) >> 5 {
            for rz in (center.z - range) >> 5..=(center.z + range) >> 5 {
                let bits = self.region_bits(rx, rz);
                let bits = bits.get().expect("scanned");
                if bits.iter().all(|word| *word == 0) {
                    continue;
                }
                for x in (center.x - range).max(rx << 5)..=(center.x + range).min((rx << 5) + 31) {
                    for z in
                        (center.z - range).max(rz << 5)..=(center.z + range).min((rz << 5) + 31)
                    {
                        let index = ((x & 31) + (z & 31) * 32) as usize;
                        if bits[index / 64] & (1 << (index % 64)) != 0 {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
    fn saved_starts(&self, x: i32, z: i32) -> Vec<horizon_world::worldgen::blending::SavedStart> {
        self.save
            .raw_chunk(x, z)
            .map(|root| starts_of(&root.tag))
            .unwrap_or_default()
    }
    fn blending(&self, x: i32, z: i32) -> Option<Arc<BlendingData>> {
        if !self.is_old(x, z) {
            return None;
        }
        if let Some(found) = self.blending.lock().unwrap().get(&(x, z)) {
            return found.clone();
        }
        let made = (|| {
            let packed = self.packed(x, z)?;
            let chunk = self.old_chunk(x, z)?;
            let mut sides = Sides::default();
            for side in Direction8::ALL {
                let (dx, dz) = side.step();
                if !self.is_old(x + dx, z + dz) {
                    sides.insert(side);
                }
            }
            let view = View {
                chunk: &chunk,
                data: &self.data,
                old: self,
            };
            Some(Arc::new(packed.calculated(&view, sides)))
        })();
        self.blending.lock().unwrap().insert((x, z), made.clone());
        made
    }
    fn old_biomes(&self, x: i32, z: i32) -> Option<Arc<BiomeChunk<BiomeId>>> {
        if !self.is_old(x, z) {
            return None;
        }
        if let Some(found) = self.kept_biomes.lock().unwrap().get(&(x, z)) {
            return found.clone();
        }
        let made = (|| {
            let chunk = self.old_chunk(x, z)?;
            let (min, height) = (chunk.min_y(), chunk.sections.len() as i32 * 16);
            let mut biomes = BiomeChunk::new(ChunkPos::new(x, z), min, height, BiomeId(0)).ok()?;
            biomes
                .fill(min, min + height - 1, |[qx, qy, qz]| {
                    Ok::<_, ()>(self.client_to_generator_biome(chunk.biome(qx * 4, qy * 4, qz * 4)))
                })
                .ok()?;
            Some(Arc::new(biomes))
        })();
        let mut cache = self.kept_biomes.lock().unwrap();
        if cache.len() > 4096 {
            cache.clear();
        }
        cache.insert((x, z), made.clone());
        made
    }
}

/// The `structures.starts` of a saved chunk: each valid start's name and the
/// union of its pieces' `BB`.
fn starts_of(chunk: &Tag) -> Vec<horizon_world::worldgen::blending::SavedStart> {
    let Some(Tag::Compound(starts)) = chunk.get("structures").and_then(|s| s.get("starts")) else {
        return Vec::new();
    };
    starts
        .iter()
        .filter(|(_, start)| {
            !matches!(start.get("id"), Some(Tag::String(id)) if id.to_string_lossy() == "INVALID")
        })
        .map(|(name, start)| {
            let mut bounds: Option<[i32; 6]> = None;
            if let Some(Tag::List(children)) = start.get("Children") {
                for child in children {
                    if let Some(Tag::IntArray(bb)) = child.get("BB")
                        && let [a, b, c, d, e, f] = bb[..]
                    {
                        bounds = Some(match bounds {
                            None => [a, b, c, d, e, f],
                            Some([x0, y0, z0, x1, y1, z1]) => [
                                x0.min(a),
                                y0.min(b),
                                z0.min(c),
                                x1.max(d),
                                y1.max(e),
                                z1.max(f),
                            ],
                        });
                    }
                }
            }
            let pieces = match start.get("Children") {
                Some(Tag::List(children)) => children.clone(),
                _ => Vec::new(),
            };
            horizon_world::worldgen::blending::SavedStart {
                name: name.to_string_lossy(),
                bounds,
                pieces,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_starts_are_the_valid_ones_with_their_pieces_union() {
        let piece = |bb: [i32; 6]| Tag::compound([("BB", Tag::IntArray(bb.to_vec()))]);
        let chunk = Tag::compound([(
            "structures",
            Tag::compound([(
                "starts",
                Tag::compound([
                    (
                        "minecraft:village_plains",
                        Tag::compound([
                            ("id", Tag::String("minecraft:village_plains".into())),
                            (
                                "Children",
                                Tag::List(vec![
                                    piece([0, 60, 0, 10, 70, 8]),
                                    piece([-4, 62, 3, 6, 72, 20]),
                                ]),
                            ),
                        ]),
                    ),
                    (
                        "minecraft:mineshaft",
                        Tag::compound([("id", Tag::String("INVALID".into()))]),
                    ),
                ]),
            )]),
        )]);
        let starts = starts_of(&chunk);
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0].name, "minecraft:village_plains");
        assert_eq!(starts[0].bounds, Some([-4, 60, 0, 10, 72, 20]));
        assert_eq!(starts[0].pieces.len(), 2, "the Children come with it");
        assert!(starts_of(&Tag::compound([("DataVersion", Tag::Int(1))])).is_empty());
    }
}
