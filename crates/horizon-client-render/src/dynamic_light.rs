//! Horizon Extras' dynamic lights (docs/extras.md): a torch, lantern or other light source held by
//! a player or mob, dropped as an item, or an entity on fire lights the blocks and entities
//! around it. Not in vanilla; off by default.
//!
//! Client-only and at render time: the light engine, chunk meshes and saves never see it. Each
//! frame the game picks the [`MAX`] sources nearest the camera; terrain.wgsl and entity.wgsl
//! raise a vertex's block light to `level - distance` (one level per block, as block light falls
//! off) when that is brighter. Off, the list is empty and the shaders skip the loop.
use crate::blocks::BlockFacts;
use glam::DVec3;
use horizon_world::registry::BlockRegistry;
use std::collections::HashMap;

/// Lights the shaders read: `Globals.dynamic_lights` in gfx.rs, terrain.wgsl and entity.wgsl.
pub const MAX: usize = 16;
/// Camera-relative position and radius; RGB intensity in a second vec4.
pub type Colored = [[f32; 4]; 2];

/// Nearest validated native visual lights; never enters gameplay lighting.
pub fn select_colored(
    sources: impl Iterator<Item = ([f64; 3], u32, f32, f32)>,
    camera: DVec3,
) -> Vec<Colored> {
    let mut nearest: Vec<_> = sources.collect();
    nearest.sort_by(|a, b| {
        (DVec3::from(a.0) - camera)
            .length_squared()
            .total_cmp(&(DVec3::from(b.0) - camera).length_squared())
    });
    nearest
        .into_iter()
        .take(MAX)
        .map(|(at, rgb, radius, intensity)| {
            let at = (DVec3::from(at) - camera).as_vec3();
            [
                [at.x, at.y, at.z, radius],
                [
                    ((rgb >> 16) & 255) as f32 / 255.0,
                    ((rgb >> 8) & 255) as f32 / 255.0,
                    (rgb & 255) as f32 / 255.0,
                    intensity,
                ],
            ]
        })
        .collect()
}
/// What an entity on fire gives off, as a lit campfire or fire block does.
pub const BURNING: u8 = 15;
/// Sources farther than this from the camera light nothing it can see well.
const REACH: f64 = 96.0;

/// How brightly each item glows when held or dropped.
#[derive(Debug, Default)]
pub struct Luminance(HashMap<String, u8>);

impl Luminance {
    /// A block item glows like its block's default state (`getLightEmission`): torches 14,
    /// lanterns and glowstone 15, a lit redstone torch 7; a furnace's item, unlit, not at all.
    pub fn new(facts: &BlockFacts, registry: &BlockRegistry) -> Self {
        let mut map = HashMap::new();
        for (id, state) in facts.states.iter().enumerate() {
            if state.emission == 0 || *state.item != *state.name {
                continue;
            }
            if registry.default_state(&state.name).map(|d| d.0 as usize) == Some(id) {
                map.insert(state.item.to_string(), state.emission);
            }
        }
        // A bucket of lava glows as lava does.
        map.insert("minecraft:lava_bucket".into(), 15);
        Self(map)
    }

    pub fn of(&self, item: &str) -> u8 {
        self.0.get(item).copied().unwrap_or(0)
    }

    #[cfg(test)]
    fn from_pairs(pairs: &[(&str, u8)]) -> Self {
        Self(pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect())
    }
}

/// One light this frame: where it is (world space) and its level, 1 to 15.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Source {
    pub at: DVec3,
    pub level: u8,
}

/// The [`MAX`] sources nearest `camera` within reach, as the shaders read them: camera-relative
/// position and level. Overwrites `out`.
pub fn select(sources: &[Source], camera: DVec3, out: &mut Vec<[f32; 4]>) {
    out.clear();
    let key = |s: &Source| s.at.distance_squared(camera);
    let mut near: Vec<&Source> = sources
        .iter()
        .filter(|s| s.level > 0 && key(s) <= REACH * REACH)
        .collect();
    near.sort_by(|a, b| key(a).total_cmp(&key(b)));
    out.extend(near.into_iter().take(MAX).map(|s| {
        let p = (s.at - camera).as_vec3();
        [p.x, p.y, p.z, f32::from(s.level.min(15))]
    }));
}

/// The block light a vertex at `pos` (camera-relative) gets from `lights`, 0 to 15: the WGSL
/// `dynamic_block_light` in Rust, for tests.
pub fn level_at(pos: [f32; 3], lights: &[[f32; 4]]) -> f32 {
    lights.iter().fold(0.0f32, |level, l| {
        let d =
            ((pos[0] - l[0]).powi(2) + (pos[1] - l[1]).powi(2) + (pos[2] - l[2]).powi(2)).sqrt();
        level.max(l[3] - d)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colored_sources_are_camera_relative_and_bounded() {
        let lights = select_colored(
            (0..20).map(|x| ([x as f64, 64.0, 0.0], 0xff8040, 5.0, 0.5)),
            DVec3::new(2.0, 64.0, 0.0),
        );
        assert_eq!(lights.len(), MAX);
        assert_eq!(lights[0][0][0], 0.0);
        assert_eq!(lights[0][1], [1.0, 128.0 / 255.0, 64.0 / 255.0, 0.5]);
    }

    #[test]
    fn nearest_sources_win() {
        let camera = DVec3::new(100.0, 64.0, 100.0);
        let mut sources: Vec<Source> = (0..40)
            .map(|i| Source {
                at: camera + DVec3::new(f64::from(i), 0.0, 0.0),
                level: 14,
            })
            .collect();
        sources.push(Source {
            at: camera + DVec3::new(0.5, 0.0, 0.0),
            level: 0,
        });
        sources.push(Source {
            at: camera + DVec3::new(0.0, 0.0, 200.0),
            level: 15,
        });
        let mut out = Vec::new();
        select(&sources, camera, &mut out);
        assert_eq!(out.len(), MAX);
        assert_eq!(out[0], [0.0, 0.0, 0.0, 14.0]);
        assert_eq!(out[MAX - 1][0], (MAX - 1) as f32);
        select(&[], camera, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn light_falls_off_a_level_per_block() {
        let torch = [[0.0, 0.0, 0.0, 14.0]];
        assert_eq!(level_at([0.0, 0.0, 0.0], &torch), 14.0);
        assert_eq!(level_at([3.0, 4.0, 0.0], &torch), 9.0);
        assert_eq!(level_at([20.0, 0.0, 0.0], &torch), 0.0);
        let two = [[0.0, 0.0, 0.0, 14.0], [10.0, 0.0, 0.0, 15.0]];
        assert_eq!(level_at([8.0, 0.0, 0.0], &two), 13.0);
        assert_eq!(level_at([0.0; 3], &[]), 0.0);
    }

    #[test]
    fn item_luminance() {
        let l = Luminance::from_pairs(&[("minecraft:torch", 14)]);
        assert_eq!(l.of("minecraft:torch"), 14);
        assert_eq!(l.of("minecraft:stone"), 0);
    }
}
