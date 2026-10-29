//! Seed-stable visual vegetation from the world's own feature data: trees, and
//! the End's chorus plants.
//! shortcut: these positions approximate native placement; saved/visited and
//! completed native chunks replace them rather than changing gameplay worldgen.
use crate::{
    fast_hash::FxHashMap,
    lod::{Column, Span, Surface},
};
use horizon_world::{
    registry::{BlockRegistry, BlockStateId},
    worldgen::generator::GeneratorData,
};
use serde_json::Value;

/// The widest LOD cell that draws chorus: a cell is drawn as a full-footprint
/// column, so a one-block plant in a wider cell would read as a coloured slab.
const CHORUS_CELL: i32 = 4;

#[derive(Clone, Copy)]
struct Tree {
    leaves: BlockStateId,
    log: BlockStateId,
    height: i32,
    variation: i32,
    radius: i32,
    count: f64,
}
#[derive(Clone, Copy)]
struct Chorus {
    plant: BlockStateId,
    flower: BlockStateId,
    /// Expected plants per chunk (`minecraft:chorus_plant`'s placement).
    count: f64,
}
pub(super) struct Vegetation {
    trees: FxHashMap<u16, Vec<Tree>>,
    chorus: FxHashMap<u16, Chorus>,
    seed: u64,
    registry: &'static BlockRegistry,
}
fn mean(v: &Value) -> f64 {
    if let Some(n) = v.as_f64() {
        return n;
    }
    let distribution = v["distribution"].as_array();
    if let Some(entries) = distribution {
        let total: f64 = entries
            .iter()
            .map(|e| e["weight"].as_f64().unwrap_or(0.))
            .sum();
        if total > 0. {
            return entries
                .iter()
                .map(|e| mean(&e["data"]) * e["weight"].as_f64().unwrap_or(0.))
                .sum::<f64>()
                / total;
        }
    }
    (v["min_inclusive"].as_f64().unwrap_or(0.) + v["max_inclusive"].as_f64().unwrap_or(0.)) * 0.5
}
fn block(v: &Value, registry: &BlockRegistry) -> Option<BlockStateId> {
    let name = v
        .as_str()
        .or_else(|| v["id"].as_str())
        .or_else(|| v["Name"].as_str())?;
    registry.default_state(name)
}
fn gather(
    data: &GeneratorData,
    value: &Value,
    placed: bool,
    count: f64,
    depth: usize,
    out: &mut Vec<Tree>,
    chorus: &mut Option<Chorus>,
) {
    if depth > 12 || count <= 0. {
        return;
    }
    if let Some(name) = value.as_str() {
        let json = if placed {
            data.placed_feature_json(name)
        } else {
            data.configured_feature_json(name)
        };
        if let Some(json) = json.and_then(|s| serde_json::from_str::<Value>(s).ok()) {
            gather(data, &json, placed, count, depth + 1, out, chorus);
        }
        return;
    }
    if placed {
        let mut count = count;
        if let Some(modifiers) = value["placement"].as_array() {
            for modifier in modifiers {
                match modifier["type"]
                    .as_str()
                    .unwrap_or("")
                    .strip_prefix("minecraft:")
                    .unwrap_or("")
                {
                    "count" | "count_on_every_layer" => count *= mean(&modifier["count"]),
                    "rarity_filter" => count /= modifier["chance"].as_f64().unwrap_or(1.).max(1.),
                    "count_extra" => {
                        count *= modifier["count"].as_f64().unwrap_or(0.)
                            + modifier["extra_count"].as_f64().unwrap_or(0.)
                                * modifier["extra_chance"].as_f64().unwrap_or(0.)
                    }
                    _ => {}
                }
            }
        }
        gather(
            data,
            &value["feature"],
            false,
            count,
            depth + 1,
            out,
            chorus,
        );
        return;
    }
    let config = if value["config"].is_object() {
        &value["config"]
    } else {
        value
    };
    match value["type"].as_str().unwrap_or("") {
        "minecraft:chorus_plant" => {
            // A stateless feature: the tip is always a fully grown flower, and
            // the trunk height (2..=5) is a property of its block behaviour.
            let registry = data.registry();
            let (Some(plant), Some(base)) = (
                registry.default_state("minecraft:chorus_plant"),
                registry.default_state("minecraft:chorus_flower"),
            ) else {
                return;
            };
            let Some(mut state) = registry.state(base).cloned() else {
                return;
            };
            state.properties.insert("age".into(), "5".into());
            let Some(flower) = registry.id(&state) else {
                return;
            };
            match chorus {
                Some(existing) => existing.count += count,
                None => {
                    *chorus = Some(Chorus {
                        plant,
                        flower,
                        count,
                    })
                }
            }
        }
        "minecraft:tree" => {
            let (Some(leaves), Some(log)) = (
                block(&config["foliage_provider"], data.registry()),
                block(&config["trunk_provider"], data.registry()),
            ) else {
                return;
            };
            let trunk = &config["trunk_placer"];
            out.push(Tree {
                leaves,
                log,
                height: trunk["base_height"].as_i64().unwrap_or(5) as i32,
                variation: (trunk["height_rand_a"].as_i64().unwrap_or(0)
                    + trunk["height_rand_b"].as_i64().unwrap_or(0))
                    as i32,
                radius: mean(&config["foliage_placer"]["radius"])
                    .round()
                    .clamp(2., 6.) as i32,
                count,
            });
        }
        "minecraft:random_selector" => {
            let mut remaining = count;
            if let Some(features) = config["features"].as_array() {
                for entry in features {
                    let chance = entry["chance"].as_f64().unwrap_or(0.).clamp(0., 1.);
                    gather(
                        data,
                        &entry["feature"],
                        true,
                        remaining * chance,
                        depth + 1,
                        out,
                        chorus,
                    );
                    remaining *= 1. - chance;
                }
            }
            gather(
                data,
                &config["default"],
                true,
                remaining,
                depth + 1,
                out,
                chorus,
            );
        }
        "minecraft:random_boolean_selector" => {
            for key in ["feature_true", "feature_false"] {
                gather(
                    data,
                    &config[key],
                    true,
                    count * 0.5,
                    depth + 1,
                    out,
                    chorus,
                );
            }
        }
        "minecraft:simple_random_selector" => {
            if let Some(features) = config["features"].as_array() {
                for feature in features {
                    gather(
                        data,
                        feature,
                        true,
                        count / features.len().max(1) as f64,
                        depth + 1,
                        out,
                        chorus,
                    );
                }
            }
        }
        _ => {}
    }
}
pub(super) fn hash(seed: u64, x: i32, z: i32) -> u64 {
    let mut n = seed
        ^ (x as u64).wrapping_mul(0x9e3779b97f4a7c15)
        ^ (z as u64).wrapping_mul(0xd1b54a32d192ed03);
    n = (n ^ n >> 30).wrapping_mul(0xbf58476d1ce4e5b9);
    n = (n ^ n >> 27).wrapping_mul(0x94d049bb133111eb);
    n ^ n >> 31
}
/// The chorus plant whose root falls in this output cell: its height, or none.
/// `count` is per chunk, so a 4×4 cell holds one with probability `count/16`;
/// a wider cell scans the cells it covers and keeps the expected density.
fn chorus_at(seed: u64, count: f64, x: i32, z: i32, width: i32) -> Option<i32> {
    if width > CHORUS_CELL || count <= 0. {
        return None;
    }
    let probability = (count / 16.).min(1.);
    for cz in z.div_euclid(4)..=(z + width - 1).div_euclid(4) {
        for cx in x.div_euclid(4)..=(x + width - 1).div_euclid(4) {
            let n = hash(seed, cx, cz);
            if (n as u32 as f64 / u32::MAX as f64) >= probability {
                continue;
            }
            let tx = cx * 4 + ((n >> 32) & 3) as i32;
            let tz = cz * 4 + ((n >> 34) & 3) as i32;
            if tx < x || tx >= x + width || tz < z || tz >= z + width {
                continue;
            }
            return Some(2 + ((n >> 40) % 4) as i32);
        }
    }
    None
}
impl Vegetation {
    pub(super) fn new(data: &GeneratorData, map: &[u16], seed: i64) -> Self {
        let mut trees = FxHashMap::default();
        let mut chorus = FxHashMap::default();
        for (index, &biome) in map.iter().enumerate() {
            let Ok(json) = serde_json::from_str::<Value>(
                data.biome_json(horizon_world::worldgen::biome::BiomeId(index as u32)),
            ) else {
                continue;
            };
            let mut list = Vec::new();
            let mut plant = None;
            if let Some(steps) = json["features"].as_array() {
                for step in steps {
                    if let Some(features) = step.as_array() {
                        for feature in features {
                            gather(data, feature, true, 1., 0, &mut list, &mut plant);
                        }
                    }
                }
            }
            if !list.is_empty() {
                trees.insert(biome, list);
            }
            if let Some(plant) = plant {
                chorus.insert(biome, plant);
            }
        }
        Self {
            trees,
            chorus,
            seed: seed as u64,
            registry: data.registry(),
        }
    }
    pub(super) fn apply(&self, column: &mut Column, x: i32, z: i32, width: i32, light: u16) {
        let Some(surface) = column.surface else {
            return;
        };
        if surface.water.is_some_and(|(y, _)| y >= surface.height) {
            return;
        }
        let ground = self
            .registry
            .state(surface.state)
            .map_or("", |s| s.name.as_str());
        if ground == "minecraft:end_stone" {
            self.apply_chorus(column, surface, x, z, width, light);
            return;
        }
        if !matches!(
            ground,
            "minecraft:grass_block"
                | "minecraft:dirt"
                | "minecraft:coarse_dirt"
                | "minecraft:podzol"
                | "minecraft:mycelium"
                | "minecraft:rooted_dirt"
                | "minecraft:moss_block"
        ) {
            return;
        }
        let Some(trees) = self.trees.get(&surface.biome) else {
            return;
        };
        let total: f64 = trees.iter().map(|tree| tree.count).sum();
        let choose = |n: u64| {
            let mut value = (n as u32 as f64 / u32::MAX as f64) * total;
            for tree in trees {
                value -= tree.count;
                if value <= 0. {
                    return tree;
                }
            }
            &trees[trees.len() - 1]
        };
        let mut best = None;
        if width >= 16 {
            let n = hash(self.seed, x.div_euclid(4), z.div_euclid(4));
            let tree = choose(n);
            let coverage = 1.
                - (-total * f64::from(tree.radius * tree.radius) * std::f64::consts::PI / 256.)
                    .exp();
            if coverage >= 0.25 {
                let height = tree.height + ((n >> 32) & 1) as i32;
                best = Some((
                    surface.height + height.max(4),
                    surface.height + height.max(4) - tree.radius * 2,
                    *tree,
                    false,
                ));
            }
        } else {
            let radius = trees.iter().map(|t| t.radius).max().unwrap_or(2);
            for cz in (z - radius).div_euclid(4)..=(z + radius).div_euclid(4) {
                for cx in (x - radius).div_euclid(4)..=(x + radius).div_euclid(4) {
                    let n = hash(self.seed, cx, cz);
                    if (n as u32 as f64 / u32::MAX as f64) >= (total / 16.).min(1.) {
                        continue;
                    }
                    let tree = *choose(n.rotate_left(21));
                    let tx = cx * 4 + ((n >> 32) & 3) as i32;
                    let tz = cz * 4 + ((n >> 34) & 3) as i32;
                    let d2 = (x - tx).pow(2) + (z - tz).pow(2);
                    if d2 > tree.radius.pow(2) {
                        continue;
                    }
                    let height =
                        tree.height + ((n >> 40) % (tree.variation.max(0) + 1) as u64) as i32;
                    let cap = ((tree.radius.pow(2) - d2) as f32).sqrt().ceil() as i32;
                    let top = surface.height + height + cap.min(2);
                    let bottom = surface.height + height - tree.radius;
                    if best.as_ref().is_none_or(|&(y, _, _, _)| top > y) {
                        best = Some((top, bottom, tree, x == tx && z == tz));
                    }
                }
            }
        }
        if let Some((top, bottom, tree, trunk)) = best {
            let bottom = bottom.max(surface.height + 1);
            if trunk {
                column.push(Span {
                    min: surface.height as i16,
                    max: bottom as i16,
                    state: tree.log,
                    biome: surface.biome,
                    light,
                });
            }
            column.push(Span {
                min: bottom as i16,
                max: top.max(bottom + 1) as i16,
                state: tree.leaves,
                biome: surface.biome,
                light,
            });
            column.surface = Some(Surface {
                height: top.max(bottom + 1),
                state: tree.leaves,
                ..surface
            });
        }
    }
    /// A chorus plant as its place-and-grow feature leaves it: a 2..=5 block
    /// stem under one fully grown flower, or a full-cell stand-in where the
    /// output cell covers several blocks.
    fn apply_chorus(
        &self,
        column: &mut Column,
        surface: Surface,
        x: i32,
        z: i32,
        width: i32,
        light: u16,
    ) {
        let Some(chorus) = self.chorus.get(&surface.biome) else {
            return;
        };
        let Some(height) = chorus_at(self.seed, chorus.count, x, z, width) else {
            return;
        };
        let base = surface.height;
        let top = base + height;
        column.push(Span {
            min: base as i16,
            max: top as i16,
            state: chorus.plant,
            biome: surface.biome,
            light,
        });
        column.push(Span {
            min: top as i16,
            max: (top + 1) as i16,
            state: chorus.flower,
            biome: surface.biome,
            light,
        });
        column.surface = Some(Surface {
            height: top + 1,
            state: chorus.flower,
            ..surface
        });
    }
    #[cfg(test)]
    pub(super) fn chorus_count(&self, biome: u16) -> Option<f64> {
        self.chorus.get(&biome).map(|chorus| chorus.count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chorus_density_and_height_follow_the_placements() {
        let mut plants = 0;
        for z in 0..256 {
            for x in 0..256 {
                if let Some(height) = chorus_at(7, 1., x, z, 1) {
                    plants += 1;
                    assert!((2..=5).contains(&height), "height {height}");
                }
            }
        }
        // One plant per chunk: 256² blocks hold 256 plants, within hash noise.
        assert!((180..=330).contains(&plants), "plants: {plants}");
        assert_eq!(chorus_at(7, 1., 0, 0, 1), chorus_at(7, 1., 0, 0, 1));
        assert_eq!(chorus_at(7, 1., 0, 0, 16), None);
        assert_eq!(chorus_at(7, 0., 0, 0, 1), None);

        let mut wide = 0;
        for z in (0..64).step_by(4) {
            for x in (0..64).step_by(4) {
                wide += usize::from(chorus_at(7, 1., x, z, 4).is_some());
            }
        }
        assert!((4..=40).contains(&wide), "4×4-cell plants: {wide}");
    }
}
