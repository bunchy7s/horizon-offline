//! `horizon-ctl memory`: what holds the game's memory, by category. The
//! process's resident set and the allocator's committed bytes say how much;
//! the census of the client's columns, the integrated server's levels, the
//! renderer's bookkeeping and the GPU allocator's resources by label say
//! where it goes. Bytes the census doesn't reach show as the difference.
use super::Game;
use horizon_client::chunk::ColumnCensus;
use horizon_client::fast_hash::FxHashSet;
use horizon_client::world::World;
use serde_json::{Value, json};

const MB: f64 = 1048576.0;

fn mb(bytes: impl TryInto<u64>) -> f64 {
    let bytes: u64 = bytes.try_into().unwrap_or(0);
    (bytes as f64 / MB * 10.0).round() / 10.0
}

/// The columns of `worlds`, each section counted once however many columns
/// (or worlds) share it.
fn census<'a>(worlds: impl IntoIterator<Item = &'a World>) -> ColumnCensus {
    let mut census = ColumnCensus::default();
    let mut seen = FxHashSet::default();
    for world in worlds {
        for chunk in world.chunks.values() {
            chunk.census(&mut census, &mut seen);
        }
    }
    census
}

fn census_json(c: &ColumnCensus) -> Value {
    json!({
        "total_mb": mb(c.total_bytes()),
        "columns": c.columns,
        "sections": c.sections,
        "unique_sections": c.unique_sections,
        "uniform_block_sections": c.uniform_block_sections,
        "light_arrays": c.light_arrays,
        "blocks_mb": mb(c.block_bytes),
        "biomes_mb": mb(c.biome_bytes),
        "light_mb": mb(c.light_bytes),
        "heightmaps_mb": mb(c.heightmap_bytes),
        "section_records_mb": mb(c.section_record_bytes),
        "column_records_mb": mb(c.column_bytes),
    })
}

impl Game<'_> {
    /// The census of this game's levels: the one in place and the parked ones.
    fn level_census(&self) -> ColumnCensus {
        census(std::iter::once(&self.world).chain(self.levels.parked_worlds()))
    }

    pub(super) fn memory_report(&self) -> Value {
        let (commit, peak_commit) = horizon_threads::allocator_commit();
        let client = self.level_census();
        // The single-player server thread's own levels: a second copy of the
        // columns both hold, unless they share sections.
        let server = self
            .integrated
            .as_ref()
            .and_then(|i| i.call(|g| g.level_census()));
        let renderer = self.renderer.as_ref().map(|r| {
            let [arenas, vertices] = r.section_vertex_bytes();
            let cpu: serde_json::Map<String, Value> = r
                .cpu_census()
                .into_iter()
                .map(|(k, v)| (format!("{k}_mb"), mb(v).into()))
                .collect();
            let gpu = r
                .gpu_memory_by_label()
                .map(|(allocated, reserved, labels)| {
                    json!({
                        "allocated_mb": mb(allocated),
                        "reserved_mb": mb(reserved),
                        "by_label_mb": labels
                            .iter()
                            .take(40)
                            .map(|(label, bytes)| json!([label, mb(*bytes)]))
                            .collect::<Vec<_>>(),
                    })
                });
            json!({
                "section_arenas_mb": mb(arenas),
                "section_vertices_mb": mb(vertices),
                "cpu": cpu,
                "gpu": gpu,
            })
        });
        json!({
            "process": {
                "resident_mb": horizon_threads::memory_limit::resident_bytes().map(mb),
                "allocator_commit_mb": mb(commit),
                "allocator_peak_commit_mb": mb(peak_commit),
            },
            "render_distance": self.streaming.radius,
            "client_columns": census_json(&client),
            "server_columns": server.as_ref().map(census_json),
            "far_tiles": self.lod.as_ref().map(|lod| {
                let (active, cached) = lod.stats();
                json!({ "active": active, "cached": cached })
            }),
            "renderer": renderer,
        })
    }
}
