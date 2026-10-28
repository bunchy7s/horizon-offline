//! Biome colours from the pinned biome definitions and colormaps
//! (`Biome.getGrassColor`/`getFoliageColor`/..., `BlockTintSources`).
use crate::Result;
use horizon_core::{noise::SimplexNoise, position::BlockPos, random::LegacyRandom};
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrassModifier {
    None,
    DarkForest,
    Swamp,
}
#[derive(Clone, Debug)]
pub struct SpawnEntry {
    pub entity: String,
    pub weight: u32,
    pub min: u32,
    pub max: u32,
    /// A `ConstantInt` count (sampling draws nothing).
    pub constant: bool,
}
/// A `natural_mob_spawns` / `spawn_overrides` entry list.
pub fn parse_entries(list: &Value) -> Vec<SpawnEntry> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let count = &e["count"];
            let (min, max, constant) = match count {
                Value::Number(n) => {
                    let n = n.as_u64().unwrap_or(1) as u32;
                    (n, n, true)
                }
                c if c["type"].as_str() == Some("minecraft:constant") => {
                    let n = c["value"].as_u64().unwrap_or(1) as u32;
                    (n, n, true)
                }
                c => (
                    c["min_inclusive"].as_u64().unwrap_or(1) as u32,
                    c["max_inclusive"].as_u64().unwrap_or(1) as u32,
                    false,
                ),
            };
            Some(SpawnEntry {
                entity: e["type"].as_str()?.to_owned(),
                weight: e["weight"].as_u64().unwrap_or(1) as u32,
                min,
                max,
                constant,
            })
        })
        .collect()
}
/// `MobSpawnSettings.MobSpawnCost`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpawnCost {
    pub energy_budget: f64,
    pub charge: f64,
}
/// The `spawn_costs` of a `natural_mob_spawns` value.
pub fn parse_costs(costs: &Value) -> HashMap<String, SpawnCost> {
    costs
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, c)| {
            (
                k.clone(),
                SpawnCost {
                    energy_budget: c["energy_budget"].as_f64().unwrap_or(0.0),
                    charge: c["charge"].as_f64().unwrap_or(0.0),
                },
            )
        })
        .collect()
}

#[derive(Clone, Debug)]
pub struct BiomeInfo {
    /// `natural_mob_spawns` by category (creature, monster, ...).
    pub spawns: HashMap<String, Vec<SpawnEntry>>,
    /// `natural_mob_spawns` spawn costs by entity type.
    pub spawn_costs: HashMap<String, SpawnCost>,
    /// The `gameplay/creature_world_gen_spawn_probability` attribute.
    pub creature_probability: f32,
    /// The `gameplay/increased_fire_burnout` attribute (jungles, swamps, peaks,
    /// mushroom fields).
    pub increased_fire_burnout: bool,
    pub name: String,
    pub temperature: f32,
    /// `temperature_modifier: frozen` (the frozen oceans).
    pub frozen: bool,
    pub downfall: f32,
    pub precipitation: bool,
    pub grass: u32,
    pub modifier: GrassModifier,
    pub foliage: u32,
    pub dry_foliage: u32,
    pub water: u32,
    pub sky: Option<u32>,
    /// The biome's own fog colour attribute; otherwise the dimension's applies.
    pub fog: Option<u32>,
    pub water_fog: u32,
}
impl BiomeInfo {
    /// `Biome.getHeightAdjustedTemperature`: the frozen modifier, then cooler
    /// with height above `sea_level + 17`.
    pub fn temperature_at(&self, pos: BlockPos, sea_level: i32) -> f32 {
        use horizon_world::worldgen::biome_temperature::{
            BiomeTemperatureSettings, TemperatureModifier,
        };
        BiomeTemperatureSettings {
            base_temperature: self.temperature,
            modifier: if self.frozen {
                TemperatureModifier::Frozen
            } else {
                TemperatureModifier::None
            },
            has_precipitation: self.precipitation,
            downfall: self.downfall,
        }
        .height_adjusted(pos, sea_level)
    }
    /// `Biome.getPrecipitationAt` in the Overworld (sea level 63): None
    /// without precipitation, else whether it falls as snow
    /// (`getHeightAdjustedTemperature` below 0.15).
    pub fn snows_at(&self, x: i32, y: i32, z: i32) -> Option<bool> {
        self.precipitation
            .then(|| self.temperature_at(BlockPos::new(x, y, z), 63) < 0.15)
    }
}
pub struct Biomes {
    pub list: Vec<BiomeInfo>,
    pub index: HashMap<String, u16>,
    info_noise: SimplexNoise,
}
fn color(v: &Value) -> Option<u32> {
    match v {
        Value::String(s) => u32::from_str_radix(s.trim_start_matches('#'), 16)
            .ok()
            .map(|c| c & 0xFF_FFFF),
        Value::Number(n) => n.as_i64().map(|c| c as u32 & 0xFF_FFFF),
        _ => None,
    }
}
fn colormap(root: &Path, name: &str) -> Result<Vec<u32>> {
    let path = root.join(format!("assets/minecraft/textures/colormap/{name}.png"));
    let mut decoder = png::Decoder::new(std::io::BufReader::new(fs::File::open(path)?));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let mut bytes = vec![0; reader.output_buffer_size().ok_or("colormap too large")?];
    let info = reader.next_frame(&mut bytes)?;
    let channels = info.color_type.samples();
    Ok(bytes[..info.buffer_size()]
        .chunks_exact(channels)
        .map(|p| u32::from(p[0]) << 16 | u32::from(p[1]) << 8 | u32::from(p[2]))
        .collect())
}
/// `GrassColor.get`/`FoliageColor.get`: temperature and downfall index a 256x256 map.
fn sample(map: &[u32], temperature: f32, downfall: f32, fallback: u32) -> u32 {
    let temperature = f64::from(temperature.clamp(0.0, 1.0));
    let downfall = f64::from(downfall.clamp(0.0, 1.0)) * temperature;
    let x = ((1.0 - temperature) * 255.0) as i32;
    let y = ((1.0 - downfall) * 255.0) as i32;
    let index = (y << 8 | x) as usize;
    map.get(index).copied().unwrap_or(fallback)
}
impl Biomes {
    pub fn load(root: &Path) -> Result<Self> {
        let grass = colormap(root, "grass")?;
        let foliage = colormap(root, "foliage")?;
        let dry = colormap(root, "dry_foliage")?;
        let mut list = Vec::new();
        let mut names: Vec<_> = fs::read_dir(root.join("data/minecraft/worldgen/biome"))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect();
        names.sort();
        for path in names {
            let name = format!(
                "minecraft:{}",
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .ok_or("bad biome")?
            );
            let v: Value = serde_json::from_slice(&fs::read(&path)?)?;
            let effects = &v["effects"];
            let attributes = &v["attributes"];
            let temperature = v["temperature"].as_f64().unwrap_or(0.5) as f32;
            let downfall = v["downfall"].as_f64().unwrap_or(0.5) as f32;
            let modifier = match effects["grass_color_modifier"].as_str() {
                Some("dark_forest") => GrassModifier::DarkForest,
                Some("swamp") => GrassModifier::Swamp,
                _ => GrassModifier::None,
            };
            let mut spawns: HashMap<String, Vec<SpawnEntry>> = HashMap::new();
            let natural = &attributes["minecraft:gameplay/natural_mob_spawns"];
            // A modifier object ({argument, modifier: overlay}) or the plain value;
            // the overlay over the dimension's empty default is the biome's value.
            let natural = if natural["argument"].is_object() {
                &natural["argument"]
            } else {
                natural
            };
            let categories = &natural["spawns_by_category"];
            for (category, entries) in categories.as_object().into_iter().flatten() {
                let list = parse_entries(entries);
                spawns.insert(category.clone(), list);
            }
            list.push(BiomeInfo {
                spawns,
                spawn_costs: parse_costs(&natural["spawn_costs"]),
                creature_probability:
                    attributes["minecraft:gameplay/creature_world_gen_spawn_probability"]
                        .as_f64()
                        .unwrap_or(0.1) as f32,
                increased_fire_burnout: attributes["minecraft:gameplay/increased_fire_burnout"]
                    .as_bool()
                    .unwrap_or(false),
                temperature,
                frozen: v["temperature_modifier"].as_str() == Some("frozen"),
                downfall,
                precipitation: v["has_precipitation"].as_bool().unwrap_or(true),
                grass: color(&effects["grass_color"])
                    .unwrap_or_else(|| sample(&grass, temperature, downfall, 0xFF00FF)),
                modifier,
                foliage: color(&effects["foliage_color"])
                    .unwrap_or_else(|| sample(&foliage, temperature, downfall, 0x48B518)),
                dry_foliage: color(&effects["dry_foliage_color"])
                    .unwrap_or_else(|| sample(&dry, temperature, downfall, 0x5D4930)),
                water: color(&effects["water_color"]).unwrap_or(0x3F76E4),
                sky: color(&attributes["minecraft:visual/sky_color"]),
                fog: color(&attributes["minecraft:visual/fog_color"]),
                water_fog: color(&attributes["minecraft:visual/water_fog_color"])
                    .unwrap_or(0x050533),
                name,
            });
        }
        let index = list
            .iter()
            .enumerate()
            .map(|(i, b)| (b.name.clone(), i as u16))
            .collect();
        Ok(Self {
            list,
            index,
            info_noise: SimplexNoise::with_skipped_offsets(&mut LegacyRandom::new(2345), true),
        })
    }
    pub fn id(&self, name: &str) -> u16 {
        self.index
            .get(name)
            .or_else(|| self.index.get("minecraft:plains"))
            .copied()
            .unwrap_or(0)
    }
    pub fn get(&self, id: u16) -> &BiomeInfo {
        &self.list[usize::from(id).min(self.list.len() - 1)]
    }
    pub fn grass(&self, id: u16, x: f64, z: f64) -> u32 {
        let b = self.get(id);
        match b.modifier {
            GrassModifier::None => b.grass,
            GrassModifier::DarkForest => ((b.grass & 0xFE_FEFE) + 0x28_340A) >> 1,
            GrassModifier::Swamp => {
                if f64::from(self.info_noise.get2(x * 0.0225, z * 0.0225)) < -0.1 {
                    0x4C_763C
                } else {
                    0x6A_7039
                }
            }
        }
    }
}

/// A 24-bit colour as sRGB components in 0..1.
pub fn rgb(c: u32) -> [f32; 3] {
    [16, 8, 0].map(|shift| (c >> shift & 255) as f32 / 255.0)
}

/// `GaussianSampler.GAUSSIAN_SAMPLE_KERNEL`.
const GAUSSIAN_KERNEL: [f64; 7] = [0.0, 1.0, 4.0, 6.0, 4.0, 1.0, 0.0];

/// `EnvironmentAttributeProbe.tick`: `GaussianSampler.sample` over the noise
/// biomes (quart cells) around `position`, in blocks. Each biome comes once
/// with its summed weight, in the order it was first sampled, as
/// `SpatialAttributeInterpolator` keeps them (zero weights included).
pub fn gaussian_weights(
    position: [f64; 3],
    mut biome_at_quart: impl FnMut(i32, i32, i32) -> u16,
) -> Vec<(u16, f64)> {
    let p = position.map(|c| c * 0.25 - 0.5);
    let integral = p.map(|c| c.floor() as i32);
    let relative: [f64; 3] = std::array::from_fn(|i| p[i] - f64::from(integral[i]));
    let weight = |axis: usize, i: usize| {
        let (start, end) = (GAUSSIAN_KERNEL[i + 1], GAUSSIAN_KERNEL[i]);
        start + relative[axis] * (end - start)
    };
    let mut weights: Vec<(u16, f64)> = Vec::new();
    for z in 0..6 {
        let wz = weight(2, z);
        for x in 0..6 {
            let wx = weight(0, x);
            for y in 0..6 {
                let wy = weight(1, y);
                let biome = biome_at_quart(
                    integral[0] - 2 + x as i32,
                    integral[1] - 2 + y as i32,
                    integral[2] - 2 + z as i32,
                );
                let w = wx * wy * wz;
                match weights.iter_mut().find(|(b, _)| *b == biome) {
                    Some((_, total)) => *total += w,
                    None => weights.push((biome, w)),
                }
            }
        }
    }
    weights
}

/// `SpatialAttributeInterpolator.applyAttributeLayer` for an RGB colour
/// (`ARGB.srgbLerp`: a component-wise lerp of the sRGB values).
pub fn blend_rgb(weights: &[(u16, f64)], mut value: impl FnMut(u16) -> [f32; 3]) -> [f32; 3] {
    let mut result: Option<[f32; 3]> = None;
    let mut accumulated = 0.0;
    for &(biome, weight) in weights {
        let v = value(biome);
        accumulated += weight;
        result = Some(match result {
            None => v,
            Some(r) => {
                let t = (weight / accumulated) as f32;
                std::array::from_fn(|i| r[i] + (v[i] - r[i]) * t)
            }
        });
    }
    result.unwrap_or([0.0; 3])
}

#[cfg(test)]
mod blend_tests {
    use super::*;

    #[test]
    fn one_biome_keeps_its_colour() {
        let weights = gaussian_weights([10.3, 64.0, -7.9], |_, _, _| 4);
        assert_eq!(weights.len(), 1);
        // The kernel sums to 16 per axis whatever the offset.
        assert!((weights[0].1 - 16f64.powi(3)).abs() < 1e-9);
        assert_eq!(blend_rgb(&weights, |_| [0.2, 0.4, 0.6]), [0.2, 0.4, 0.6]);
    }

    #[test]
    fn colour_changes_gradually_across_a_boundary() {
        // Biome 1 for quart x >= 0 (block x >= 0), biome 0 below.
        let at = |x: f64| {
            let w = gaussian_weights([x, 40.0, 0.0], |qx, _, _| u16::from(qx >= 0));
            blend_rgb(&w, |b| if b == 1 { [1.0, 0.0, 0.0] } else { [0.0; 3] })[0]
        };
        assert_eq!(at(-12.0), 0.0);
        assert_eq!(at(12.0), 1.0);
        // Halfway at the boundary, rising steadily through it.
        assert!((at(0.0) - 0.5).abs() < 1e-5, "{}", at(0.0));
        let mut last = 0.0;
        for i in -16..=16 {
            let v = at(f64::from(i) * 0.5);
            assert!(v >= last - 1e-6, "{v} < {last} at {i}");
            last = v;
        }
        assert!(at(-2.0) > 0.0 && at(-2.0) < 0.5);
    }
}
