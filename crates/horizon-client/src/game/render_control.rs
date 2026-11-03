//! Control-socket commands for inspecting rendering: place blocks without
//! updates (`setblock`, `fill`) to build test scenes.
use super::Game;
use horizon_client::Result;
use horizon_core::random::RandomSource;
use horizon_world::registry::BlockStateId;
use serde_json::{Value, json};

pub const HELP: &[&str] = &[
    "setblock X Y Z STATE           set a block state (name[prop=value,...]) without updates",
    "fill X1 Y1 Z1 X2 Y2 Z2 STATE   fill a box with a block state without updates",
    "particle NAME X Y Z [DX DY DZ] [COUNT]  spawn particles (vanilla /particle spread)",
    "signedit X Y Z [front|back]    open the sign editor for the sign at X Y Z",
    "section-draws [flat|indirect]  how terrain sections are drawn (A/B checks); replies the mode",
    "section-cull [on|off]          occlusion culling of solid sections (A/B checks); replies the mode",
    "section-sides [on|off]         leave out solid quads facing away from the eye (A/B checks); replies the mode",
    "gpu-culling [on|off]           list and draw the near solid sections on the GPU where the device can (A/B checks); replies the mode",
];

impl Game<'_> {
    /// Parse `name[k=v,...]` into a registry state (defaults for missing properties).
    fn parse_state(&self, text: &str) -> Result<BlockStateId> {
        parse_state(self.pack.assets.registry, text)
    }
    pub(super) fn render_command(&mut self, words: &[&str]) -> Result<Option<Value>> {
        let int = |i: usize| -> Result<i32> {
            Ok(words
                .get(i)
                .ok_or_else(|| format!("missing argument {i}"))?
                .parse::<i32>()?)
        };
        match words[0] {
            "setblock" => {
                let state = self.parse_state(words.get(4).ok_or("missing state")?)?;
                self.world.set(int(1)?, int(2)?, int(3)?, state)?;
                Ok(Some(json!({ "ok": true, "state": state.0 })))
            }
            "fill" => {
                let state = self.parse_state(words.get(7).ok_or("missing state")?)?;
                let (a, b) = ([int(1)?, int(2)?, int(3)?], [int(4)?, int(5)?, int(6)?]);
                let mut count = 0;
                for x in a[0].min(b[0])..=a[0].max(b[0]) {
                    for y in a[1].min(b[1])..=a[1].max(b[1]) {
                        for z in a[2].min(b[2])..=a[2].max(b[2]) {
                            self.world.set(x, y, z, state)?;
                            count += 1;
                        }
                    }
                }
                Ok(Some(json!({ "ok": true, "blocks": count })))
            }
            "particle" => {
                use horizon_client::particles::{ParticleOptions, ParticleType};
                let ty = ParticleType::from_name(words.get(1).ok_or("missing particle")?)
                    .ok_or("unknown particle type")?;
                let num = |i: usize, default: f64| -> Result<f64> {
                    Ok(words.get(i).map_or(Ok(default), |w| w.parse::<f64>())?)
                };
                let at = [num(2, 0.0)?, num(3, 0.0)?, num(4, 0.0)?];
                let delta = [num(5, 0.0)?, num(6, 0.0)?, num(7, 0.0)?];
                let count = num(8, 1.0)? as u32;
                let options = match ty {
                    ParticleType::Dust => ParticleOptions::dust([1.0, 0.0, 0.0], 1.0),
                    ParticleType::Block
                    | ParticleType::FallingDust
                    | ParticleType::DustPillar
                    | ParticleType::BlockMarker => {
                        ParticleOptions::block(ty, "block/stone".into(), 0x808080)
                    }
                    ParticleType::Item => self.item_particle(
                        &horizon_client::items::ItemStack::new("minecraft:apple", 1),
                    ),
                    ParticleType::ItemSlime
                    | ParticleType::ItemSnowball
                    | ParticleType::ItemCobweb => {
                        self.ensure_item_particle_sprites();
                        ParticleOptions::simple(ty)
                    }
                    _ => ParticleOptions::simple(ty),
                };
                // ServerLevel.sendParticles: count 0 uses the delta as velocity.
                if count == 0 {
                    self.particles.create(&options, at, delta);
                }
                for _ in 0..count {
                    let r = self.particles.random();
                    let p = [
                        at[0] + r.next_gaussian() * delta[0],
                        at[1] + r.next_gaussian() * delta[1],
                        at[2] + r.next_gaussian() * delta[2],
                    ];
                    self.particles.create(&options, p, [0.0; 3]);
                }
                Ok(Some(json!({ "ok": true, "live": self.particles.len() })))
            }
            "signedit" => {
                let key = (int(1)?, int(2)?, int(3)?);
                let name = self
                    .res
                    .facts()
                    .get(self.world.block(key.0, key.1, key.2))
                    .name
                    .clone();
                let entity = self
                    .new_block_entity(&name)
                    .filter(|e| matches!(e, horizon_client::containers::BlockEntity::Sign(_)))
                    .ok_or("no sign there")?;
                self.world.block_entities.get_or_insert_with(key, || entity);
                self.open_sign_editor(key, words.get(4) != Some(&"back"));
                Ok(Some(json!({ "ok": true })))
            }
            other => Err(format!("unknown render command {other}").into()),
        }
    }
}

/// Parse `name[k=v,...]` into a registry state (defaults for missing properties).
pub(super) fn parse_state(
    registry: &horizon_world::registry::BlockRegistry,
    text: &str,
) -> Result<BlockStateId> {
    let (name, props) = match text.split_once('[') {
        Some((n, rest)) => (n, rest.trim_end_matches(']')),
        None => (text, ""),
    };
    let name = if name.contains(':') {
        name.to_owned()
    } else {
        format!("minecraft:{name}")
    };
    let default = registry
        .default_state(&name)
        .ok_or_else(|| format!("unknown block {name}"))?;
    let mut state = registry
        .state(default)
        .ok_or("missing default state")?
        .clone();
    for pair in props.split(',').filter(|p| !p.is_empty()) {
        let (k, v) = pair
            .split_once('=')
            .ok_or_else(|| format!("bad property {pair}"))?;
        if !state.properties.contains_key(k) {
            return Err(format!("{name} has no property {k}").into());
        }
        state.properties.insert(k.to_owned(), v.to_owned());
    }
    Ok(registry.resolve(&state)?)
}
