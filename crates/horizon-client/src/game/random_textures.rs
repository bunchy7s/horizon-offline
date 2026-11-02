//! OptiFine random entity textures: each mob's variants, chosen when this window first sees it
//! and checked again once a second, counted in this window's ticks so a frozen world still
//! checks (`horizon_client::mobs::random_textures`).

use super::Game;
use horizon_client::mobs::random_textures::{Seen, choose, seed};
use horizon_core::geometry::Vec3;
use horizon_optifine::random::Weather;

impl Game<'_> {
    pub(super) fn tick_random_textures(&mut self) {
        let res = self.res.clone();
        let textures = res
            .mob_data()
            .random_textures
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if textures.is_empty() {
            return;
        }
        for i in 0..self.mobs.len() {
            let m = &mut self.mobs[i];
            let id = m.id;
            if !m.random_textures.due(&textures, id) {
                continue;
            }
            let m = &self.mobs[i];
            let seed = seed(m, self.persist.uuids.get(&id).copied());
            let seen = (!m.random_textures.has_seen()).then(|| self.seen_at(m.position));
            choose(&mut self.mobs[i], &textures, res.mob_data(), seen, seed);
        }
    }

    /// What OptiFine reads of a mob's surroundings when it first appears.
    fn seen_at(&self, at: Vec3) -> Seen {
        let (x, y, z) = (
            at.x.floor() as i32,
            at.y.floor() as i32,
            at.z.floor() as i32,
        );
        let block = |y| {
            let state = self.pack.assets.registry.state(self.world.block(x, y, z))?;
            let properties = state
                .properties
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            Some((state.name.clone(), properties))
        };
        let weather = &self.world.weather;
        Seen {
            biome: self.biome_name_at(at),
            y,
            blocks: [block(y - 1), block(y)].into_iter().flatten().collect(),
            moon_phase: self.world.day_time.div_euclid(24_000).rem_euclid(8) as i32,
            day_time: self.world.day_time.rem_euclid(24_000) as i32,
            weather: if weather.thundering {
                Weather::Thunder
            } else if weather.raining {
                Weather::Rain
            } else {
                Weather::Clear
            },
        }
    }
}
