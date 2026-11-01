//! A server's chunk columns (`ClientboundLevelChunkWithLightPacket`): the
//! sections and light of each packet become a client column on a decoder
//! thread as soon as the packet arrives, so the frame that handles it in
//! order only inserts the result (`ClientPacketListener.handleLevelChunkWithLight`).
use super::Game;
use horizon_client::{
    chunk::{BIOME_STRATEGY, Chunk, ColumnLayout, Light, Section, block_strategy},
    streaming::Streaming,
};
use horizon_core::paletted::{PalettedContainer, Strategy};
use horizon_protocol::{
    chunk::{LightData, Paletted, SectionLight},
    packets::play::{self, Clientbound as C},
    registry::DimensionType,
};
use std::sync::{
    Arc,
    mpsc::{self, TryRecvError},
};

/// What turning a chunk packet into a client column depends on: the
/// dimension's height and sky, and the server's biome registry mapped to
/// ours. The connection replaces it when either changes, so a column decoded
/// with an older one is decoded again.
#[derive(Default)]
pub(super) struct ChunkFormat {
    /// The column decoded chunks hold: the client's for the level.
    column: ColumnLayout,
    min_y: i32,
    sections: usize,
    has_skylight: bool,
    biome_map: Vec<u16>,
    biome_bits: u8,
}

impl ChunkFormat {
    pub(super) fn new(dimension: &DimensionType, biome_map: Vec<u16>, biome_bits: u8) -> Self {
        Self {
            column: ColumnLayout::VANILLA,
            min_y: dimension.min_y,
            sections: dimension.sections(),
            has_skylight: dimension.has_skylight,
            biome_map,
            biome_bits,
        }
    }
    /// Decodes into chunks of `column`.
    pub(super) fn in_column(self, column: ColumnLayout) -> Self {
        Self { column, ..self }
    }
    /// The column decoded chunks hold.
    pub(super) fn layout(&self) -> ColumnLayout {
        self.column
    }
    pub(super) fn with_dimension(&self, dimension: &DimensionType) -> Self {
        Self {
            column: self.column,
            ..Self::new(dimension, self.biome_map.clone(), self.biome_bits)
        }
    }

    /// LevelChunkSection data into the client's column.
    fn column(&self, p: &play::LevelChunkWithLight) -> Result<Chunk, String> {
        let sections = p
            .chunk
            .sections(self.sections, 16, self.biome_bits)
            .map_err(|e| e.to_string())?;
        let mut chunk = Chunk::with_layout(p.x, p.z, self.column);
        let offset = (self.min_y - self.column.min_y()).div_euclid(16);
        let column = self.column.sections() as i32;
        for (i, s) in sections.iter().enumerate() {
            let target = offset + i as i32;
            if !(0..column).contains(&target) {
                continue;
            }
            // A global section is as wide as the server's registry: 16 bits
            // from a vanilla server, wider from a Horizon host with the same
            // content mods. Read it at that width, then widen it to ours.
            let ours = block_strategy();
            let strategy = if ours.wire_has_palette(s.states.bits) {
                ours
            } else {
                Strategy::blocks(s.states.bits)
            };
            let palette = s.states.palette.iter().map(|&p| p as u32).collect();
            let words = s.states.data.iter().map(|&w| w as u64).collect();
            let mut blocks = PalettedContainer::from_wire(strategy, s.states.bits, palette, words)
                .map_err(|e| e.to_string())?;
            if strategy.global_bits != ours.global_bits {
                let mut values = [0u32; 4096];
                blocks.unpack(&mut values);
                // Ids past our registry (a host with mods we lack) read as air.
                let limit = 1u64 << ours.global_bits;
                for v in &mut values {
                    if u64::from(*v) >= limit {
                        *v = 0;
                    }
                }
                blocks = PalettedContainer::from_values(ours, &values);
            }
            let mut section = Section {
                blocks,
                biomes: PalettedContainer::new(BIOME_STRATEGY, 0),
                light: Light::Uniform(0xF0),
            };
            section.compact();
            chunk.sections[target as usize] = Arc::new(section);
        }
        let biomes: Vec<_> = sections.iter().map(|s| &s.biomes).collect();
        self.put_biomes(&mut chunk, &biomes);
        // ClientPacketListener.applyLightData: the column keeps the light
        // the server sent; nothing is relit.
        let (layers, above) = self.light(&p.light);
        horizon_client::light::light_from_layers(&mut chunk, &layers, above, self.has_skylight);
        Ok(chunk)
    }

    /// The dimension's sections' biome containers (in the server's
    /// registry) into the column's sections. Sections outside the
    /// dimension's range take its nearest biomes (the column's top or bottom
    /// cell layer repeated).
    fn put_biomes(&self, chunk: &mut Chunk, biomes: &[&Paletted]) {
        if biomes.is_empty() {
            return;
        }
        let offset = (self.min_y - self.column.min_y()).div_euclid(16);
        // Biome table index per quart cell of the dimension's own sections.
        let biome_cells: Vec<[u32; 64]> = biomes
            .iter()
            .map(|b| {
                std::array::from_fn(|j| {
                    u32::from(self.biome_map.get(b.get(j) as usize).copied().unwrap_or(0))
                })
            })
            .collect();
        let last = biome_cells.len() - 1;
        for si in 0..self.column.sections() as i32 {
            let i = si - offset;
            let cells = if (0..biome_cells.len() as i32).contains(&i) {
                biome_cells[i as usize]
            } else {
                let (edge, layer) = if i < 0 { (0, 0) } else { (last, 3) };
                std::array::from_fn(|j| biome_cells[edge][layer * 16 + j % 16])
            };
            let mut container = PalettedContainer::from_values(BIOME_STRATEGY, &cells);
            if let Some(v) = container.uniform() {
                container = PalettedContainer::new(BIOME_STRATEGY, v);
            }
            Arc::make_mut(&mut chunk.sections[si as usize]).biomes = container;
        }
    }

    /// `LevelChunk.replaceBiomes`: each of the dimension's sections reads
    /// its biome container from the packet's buffer, in order.
    fn replace_biomes(&self, chunk: &mut Chunk, data: &play::ChunkBiomeData) -> Result<(), String> {
        let biomes = data
            .biomes(self.sections, self.biome_bits)
            .map_err(|e| e.to_string())?;
        self.put_biomes(chunk, &biomes.iter().collect::<Vec<_>>());
        Ok(())
    }

    /// `ClientPacketListener.readSectionList`: the sky and block layers of a
    /// light packet, by section of the client's column, and the sky light
    /// above the column's top section (from the light section just above
    /// it, else open sky).
    pub(super) fn light(&self, light: &LightData) -> (Vec<SectionLight>, [u8; 256]) {
        let column = self.column.sections() as i32;
        let mut layers: Vec<SectionLight> = vec![(None, None); self.column.sections()];
        let sky = self.has_skylight;
        let mut above = [if sky { 15 } else { 0 }; 256];
        // Light section b is the dimension's section b - 1.
        let first = (self.min_y - self.column.min_y()).div_euclid(16) - 1;
        for (b, layer) in light.layers(self.sections + 2).into_iter().enumerate() {
            let si = first + b as i32;
            if (0..column).contains(&si) {
                layers[si as usize] = layer;
            } else if si == column
                && sky
                && let Some(l) = &layer.0
            {
                for (i, a) in above.iter_mut().enumerate() {
                    *a = (l[i >> 1] >> ((i & 1) * 4)) & 15;
                }
            }
        }
        (layers, above)
    }
}

type Decoded = (Box<C>, Result<Chunk, String>);

/// One thread that decodes chunk packets in the order they arrived; its
/// results come back in that order, one per `submit`.
pub(super) struct ChunkDecoder {
    jobs: mpsc::Sender<(Arc<ChunkFormat>, Box<C>)>,
    done: mpsc::Receiver<Decoded>,
}

impl ChunkDecoder {
    pub(super) fn new() -> Self {
        let (jobs, todo) = mpsc::channel::<(Arc<ChunkFormat>, Box<C>)>();
        let (finished, done) = mpsc::channel();
        // The thread ends with the connection, when `jobs` is dropped.
        let spawned = std::thread::Builder::new()
            .name("Chunk decoder".into())
            .spawn(move || {
                // A frame waits for its columns, as it would for the render
                // thread's own work.
                horizon_threads::favour_current();
                for (format, packet) in todo {
                    let column = match &*packet {
                        C::LevelChunkWithLight(p) => format.column(p),
                        _ => Err("not a chunk packet".into()),
                    };
                    if finished.send((packet, column)).is_err() {
                        break;
                    }
                }
            });
        if let Err(e) = spawned {
            log::warn!("Horizon: no chunk decoder thread ({e}); chunks decode in the frame");
        }
        Self { jobs, done }
    }

    /// Start decoding a chunk packet with the format it arrived under.
    /// Without a decoder thread the packet comes back undecoded.
    pub(super) fn submit(&self, format: Arc<ChunkFormat>, packet: Box<C>) -> Option<Box<C>> {
        self.jobs.send((format, packet)).err().map(|e| (e.0).1)
    }

    /// The oldest submitted packet and its column, unless it is still
    /// decoding: a frame never waits for the decoder.
    pub(super) fn next(&self) -> Next {
        match self.done.try_recv() {
            Ok(decoded) => Next::Ready(decoded),
            Err(TryRecvError::Empty) => Next::Decoding,
            // The decoder thread panicked: its columns are gone.
            Err(TryRecvError::Disconnected) => Next::Lost,
        }
    }
}

pub(super) enum Next {
    Ready(Decoded),
    Decoding,
    Lost,
}

impl Game<'_> {
    /// `handleLevelChunkWithLight`: the column replaces any loaded one, and
    /// its block entities load their update tags. `decoded` is the column the
    /// decoder made with `format`; another format since means decode again.
    pub(super) fn remote_chunk(
        &mut self,
        p: &play::LevelChunkWithLight,
        decoded: Option<(&Arc<ChunkFormat>, Result<Chunk, String>)>,
    ) {
        // ClientChunkCache.replaceWithPacketData: a column outside the
        // storage around the view centre is ignored.
        if !self.remote_chunk_in_range(p.x, p.z) {
            log::warn!(
                "Ignoring chunk since it's not in the view range: {}, {}",
                p.x,
                p.z
            );
            return;
        }
        let Some(format) = self.remote.as_ref().map(|r| r.chunk_format.clone()) else {
            return;
        };
        let column = match decoded {
            Some((used, column)) if Arc::ptr_eq(used, &format) => column,
            _ => format.column(p),
        };
        let chunk = match column {
            Ok(chunk) => chunk,
            Err(e) => {
                log::warn!("Horizon: chunk {} {}: {e}", p.x, p.z);
                return;
            }
        };
        let (x, z) = (p.x, p.z);
        self.streaming.insert_remote(&mut self.world, chunk);
        self.world
            .block_entities
            .retain(|k, _| (k.0 >> 4, k.2 >> 4) != (x, z));
        for info in &p.chunk.block_entities {
            let pos = (
                x * 16 + i32::from(info.packed_xz as u8 >> 4),
                i32::from(info.y),
                z * 16 + i32::from(info.packed_xz as u8 & 15),
            );
            let state = self.world.block(pos.0, pos.1, pos.2);
            self.sync_block_entity(pos, None, state);
            // LevelChunk.replaceWithPacketData: the block entity loads its update tag.
            self.remote_block_entity_data([pos.0, pos.1, pos.2], info.kind, &info.tag);
        }
    }

    /// `ClientPacketListener.handleChunksBiomes`: each loaded column takes
    /// its new biomes (`ClientChunkCache.replaceBiomes`), then every section
    /// of it and of its neighbours is drawn again, as their colours blend
    /// across columns (`LevelExtractor.setSectionDirty`).
    pub(super) fn remote_chunks_biomes(&mut self, p: &play::ChunksBiomes) {
        let Some(format) = self.remote.as_ref().map(|r| r.chunk_format.clone()) else {
            return;
        };
        for data in &p.chunks {
            let [x, z] = data.pos;
            if !self.remote_chunk_in_range(x, z) {
                log::warn!("Ignoring chunk since it's not in the view range: {x}, {z}");
                continue;
            }
            let Some(chunk) = self.world.chunks.get_mut(&(x, z)) else {
                log::warn!("Ignoring chunk since it's not present: {x}, {z}");
                continue;
            };
            if let Err(e) = format.replace_biomes(Arc::make_mut(chunk), data) {
                log::warn!("Horizon: biomes of chunk {x} {z}: {e}");
            }
        }
        for data in &p.chunks {
            let [x, z] = data.pos;
            for dz in -1..=1 {
                for dx in -1..=1 {
                    if !self.world.chunks.contains_key(&(x + dx, z + dz)) {
                        continue;
                    }
                    for sy in 0..self.world.layout().sections() as i32 {
                        self.world.dirty_sections.insert((x + dx, sy, z + dz));
                    }
                }
            }
        }
    }

    /// `handleLightUpdatePacket`: the sections it names take its light.
    pub(super) fn remote_light_update(&mut self, p: &play::LightUpdate) {
        let Some(format) = self.remote.as_ref().map(|r| r.chunk_format.clone()) else {
            return;
        };
        if !self.world.chunks.contains_key(&(p.x, p.z)) {
            return;
        }
        let (layers, _) = format.light(&p.light);
        let Some(chunk) = self.world.chunks.get_mut(&(p.x, p.z)) else {
            return;
        };
        let changed = horizon_client::light::update_light_layers(
            Arc::make_mut(chunk),
            &layers,
            format.has_skylight,
        );
        Streaming::light_updated(&mut self.world, p.x, p.z, &changed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_protocol::chunk::{ChunkData, Paletted, Section as Wire, Strategy as Wiring};

    /// A column of `state` with biome 1 everywhere, unlit.
    fn packet(x: i32, state: i32) -> Box<C> {
        let section = Wire {
            non_empty: 4096,
            fluids: 0,
            states: Paletted::from_values(&[state; 4096], Wiring::Blocks, 16),
            biomes: Paletted::from_values(&[1; 64], Wiring::Biomes, 2),
        };
        let none = LightData {
            sky_mask: Vec::new(),
            block_mask: Vec::new(),
            empty_sky_mask: Vec::new(),
            empty_block_mask: Vec::new(),
            sky: Vec::new(),
            block: Vec::new(),
        };
        Box::new(C::LevelChunkWithLight(play::LevelChunkWithLight {
            x,
            z: -3,
            chunk: ChunkData {
                heightmaps: Vec::new(),
                buffer: ChunkData::from_sections(&vec![section; 24]),
                block_entities: Vec::new(),
            },
            light: none,
        }))
    }

    /// Columns come back one per packet, in the order the packets went in,
    /// as the frame would have decoded them itself.
    #[test]
    fn the_decoder_returns_columns_in_order() {
        let overworld = DimensionType::vanilla("minecraft:overworld");
        let format = Arc::new(ChunkFormat::new(&overworld, vec![0, 7, 9], 2));
        let decoder = ChunkDecoder::new();
        for x in 0..20 {
            assert!(decoder.submit(format.clone(), packet(x, 1 + x)).is_none());
        }
        for x in 0..20 {
            let (packet, column) = loop {
                match decoder.next() {
                    Next::Ready(decoded) => break decoded,
                    Next::Decoding => std::thread::yield_now(),
                    Next::Lost => panic!("the decoder stopped"),
                }
            };
            let C::LevelChunkWithLight(p) = *packet else {
                panic!("not a chunk packet");
            };
            let (column, direct) = (column.unwrap(), format.column(&p).unwrap());
            assert_eq!((column.x, column.z), (x, -3));
            for (cx, y, cz) in [(0, -64, 0), (5, 70, 9), (15, 319, 15)] {
                assert_eq!(column.get(cx, y, cz), 1 + x as u32);
                assert_eq!(column.get(cx, y, cz), direct.get(cx, y, cz));
                // Wire biome 1 is our biome 7.
                assert_eq!(column.biome(cx, y, cz), 7);
            }
        }
        assert!(matches!(decoder.next(), Next::Decoding));
    }

    /// `LevelChunk.replaceBiomes`: the packet's containers go to the
    /// dimension's sections (y 0 to 255 in the Nether), mapped to our biome
    /// table; the blocks stay, and the sections outside the dimension take
    /// its nearest biomes as a chunk packet's do.
    #[test]
    fn chunks_biomes_replace_a_columns_biomes() {
        let nether = DimensionType::vanilla("minecraft:the_nether");
        let format = ChunkFormat::new(&nether, vec![0, 7, 9], 2);
        let mut chunk = Chunk::empty(2, -3);
        chunk.set(3, 70, 4, 5);
        // Wire biome 1 everywhere, with 2 in the lowest and the highest cells.
        let containers: Vec<Paletted> = (0..nether.sections())
            .map(|i| {
                let mut cells = [1; 64];
                if i == 0 {
                    cells[0] = 2;
                }
                if i == nether.sections() - 1 {
                    cells[63] = 2;
                }
                Paletted::from_values(&cells, Wiring::Biomes, 2)
            })
            .collect();
        let data = play::ChunkBiomeData::from_biomes([2, -3], &containers);
        format.replace_biomes(&mut chunk, &data).unwrap();
        assert_eq!(chunk.get(3, 70, 4), 5);
        assert_eq!(chunk.biome(8, 70, 8), 7);
        assert_eq!(chunk.biome(0, 0, 0), 9);
        assert_eq!(chunk.biome(4, 0, 0), 7);
        assert_eq!(chunk.biome(15, 255, 15), 9);
        // Below and above the Nether: its bottom and top cell layers.
        assert_eq!(chunk.biome(0, -64, 0), 9);
        assert_eq!(chunk.biome(15, 319, 15), 9);
        assert_eq!(chunk.biome(0, 319, 0), 7);
        // A buffer short of the dimension's sections changes nothing.
        let short = play::ChunkBiomeData::from_biomes([2, -3], &containers[..3]);
        let mut other = Chunk::empty(2, -3);
        assert!(format.replace_biomes(&mut other, &short).is_err());
        assert_eq!(other.biome(8, 70, 8), 0);
    }
}
