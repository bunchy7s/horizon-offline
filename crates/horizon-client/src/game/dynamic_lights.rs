//! Horizon Extras' dynamic lights (`horizon_client::dynamic_light`): the light sources near the
//! camera this frame. Nothing is gathered while the extra is off.
use super::{Game, shield};
use glam::DVec3;
use horizon_client::dynamic_light::{self, BURNING, Luminance, Source};

impl Game<'_> {
    /// Fills `dynamic_lights` for the frame: the player's and mobs' held items and fire, and
    /// glowing dropped items, the nearest [`dynamic_light::MAX`] of them.
    /// How brightly each item glows, built on first use.
    pub(super) fn luminance(&self) -> &Luminance {
        self.luminance
            .get_or_init(|| Luminance::new(self.res.facts(), self.pack.assets.registry))
    }
    /// The light sources this frame while the extra is on: the player's and mobs' held items and
    /// fire, and glowing dropped items, each with the item that gives the light (none for fire).
    pub(super) fn dynamic_light_sources(&self, alpha: f64) -> Vec<(Source, Option<String>)> {
        let mut sources = Vec::new();
        if !self.settings.extras.dynamic_lights {
            return sources;
        }
        let lum = self.luminance();
        // The brighter of two hands with what it holds, or fire if it outshines both.
        let brighter = |items: [Option<&str>; 2], on_fire: bool| {
            let (level, item) = items
                .into_iter()
                .flatten()
                .map(|i| (lum.of(i), Some(i)))
                .max_by_key(|(level, _)| *level)
                .unwrap_or((0, None));
            if on_fire && BURNING > level {
                (BURNING, None)
            } else {
                // Only a lit source names its item: nothing is allocated for a dark one.
                (level, item.filter(|_| level > 0).map(str::to_owned))
            }
        };
        if let Some(p) = &self.player
            && !p.spectator
        {
            let feet = self
                .previous_position
                .lerp(p.position, self.clock.camera_fraction());
            let (level, item) = brighter(
                [
                    self.stack_in(self.selected).map(|s| &*s.item),
                    self.stack_in(shield::OFFHAND_SLOT).map(|s| &*s.item),
                ],
                p.fire_ticks > 0,
            );
            // About where the hands are.
            sources.push((
                Source {
                    at: DVec3::new(feet.x, feet.y + 1.2, feet.z),
                    level,
                },
                item,
            ));
        }
        for m in self.mobs.iter().filter(|m| !m.removed) {
            let (level, item) = brighter(
                [m.extra.main_hand.as_deref(), m.extra.off_hand.as_deref()],
                m.fire_ticks > 0,
            );
            if level == 0 {
                continue;
            }
            let at = m.previous.lerp(m.position, alpha);
            sources.push((
                Source {
                    at: DVec3::new(at.x, at.y + f64::from(m.height()) * 0.6, at.z),
                    level,
                },
                item,
            ));
        }
        for dropped in &self.dropped {
            let (level, item) =
                brighter([Some(&*dropped.stack.item), None], dropped.fire_ticks > 0);
            if level == 0 {
                continue;
            }
            let at = dropped.previous.lerp(dropped.position, alpha);
            sources.push((
                Source {
                    at: DVec3::new(at.x, at.y + 0.25, at.z),
                    level,
                },
                item,
            ));
        }
        sources
    }
    /// Fills `dynamic_lights` for the frame from [`Self::dynamic_light_sources`], the nearest
    /// [`dynamic_light::MAX`] of them.
    pub(super) fn gather_dynamic_lights(&mut self, camera: DVec3, alpha: f64) {
        let mut out = std::mem::take(&mut self.dynamic_lights);
        out.clear();
        let sources: Vec<Source> = self
            .dynamic_light_sources(alpha)
            .into_iter()
            .map(|(s, _)| s)
            .collect();
        dynamic_light::select(&sources, camera, &mut out);
        self.dynamic_lights = out;
    }
}
