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
    pub(super) fn gather_dynamic_lights(&mut self, camera: DVec3, alpha: f64) {
        let mut out = std::mem::take(&mut self.dynamic_lights);
        out.clear();
        if !self.settings.extras.dynamic_lights {
            self.dynamic_lights = out;
            return;
        }
        let lum = self.luminance();
        let held = |item: Option<&str>| item.map_or(0, |i| lum.of(i));
        let mut sources = Vec::new();
        if let Some(p) = &self.player
            && !p.spectator
        {
            let feet = self
                .previous_position
                .lerp(p.position, self.clock.camera_fraction());
            let level = held(self.stack_in(self.selected).map(|s| &*s.item))
                .max(held(self.stack_in(shield::OFFHAND_SLOT).map(|s| &*s.item)))
                .max(if p.fire_ticks > 0 { BURNING } else { 0 });
            // About where the hands are.
            sources.push(Source {
                at: DVec3::new(feet.x, feet.y + 1.2, feet.z),
                level,
            });
        }
        for m in self.mobs.iter().filter(|m| !m.removed) {
            let level = held(m.extra.main_hand.as_deref())
                .max(held(m.extra.off_hand.as_deref()))
                .max(if m.fire_ticks > 0 { BURNING } else { 0 });
            if level == 0 {
                continue;
            }
            let at = m.previous.lerp(m.position, alpha);
            sources.push(Source {
                at: DVec3::new(at.x, at.y + f64::from(m.height()) * 0.6, at.z),
                level,
            });
        }
        for item in &self.dropped {
            let level = lum
                .of(&item.stack.item)
                .max(if item.fire_ticks > 0 { BURNING } else { 0 });
            if level == 0 {
                continue;
            }
            let at = item.previous.lerp(item.position, alpha);
            sources.push(Source {
                at: DVec3::new(at.x, at.y + 0.25, at.z),
                level,
            });
        }
        dynamic_light::select(&sources, camera, &mut out);
        self.dynamic_lights = out;
    }
}
