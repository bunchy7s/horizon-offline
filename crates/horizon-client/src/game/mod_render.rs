//! Capability `render` (docs/mods.md): the shapes mods keep on the HUD,
//! drawn by the game's own HUD renderer, and their camera shakes. Mods
//! never supply shader code. With no mod drawing, each hook here returns
//! after one check.
use super::Game;
use horizon_client::hud::HudBatch;
#[cfg(feature = "mods")]
use {
    horizon_client::{hud::rgba, items::ItemStack, text::Text, ui},
    horizon_mods::Shape,
    std::time::Instant,
};

/// The camera's jolts from `render.shake`, while any is fading.
#[cfg(feature = "mods")]
#[derive(Default)]
pub(super) struct Shake {
    jolts: Vec<Jolt>,
}

#[cfg(feature = "mods")]
struct Jolt {
    start: Instant,
    strength: f32,
    seconds: f32,
}

/// The HUD batch's colour (ABGR) from a mod's 0xAARRGGBB.
#[cfg(feature = "mods")]
fn argb(color: u32) -> u32 {
    let [a, r, g, b] = color.to_be_bytes();
    rgba(r, g, b, a)
}

/// The view's turn at full strength, in degrees: yaw, then pitch.
#[cfg(feature = "mods")]
const SHAKE_DEGREES: (f32, f32) = (3.0, 2.0);

#[cfg(feature = "mods")]
impl Shake {
    pub(super) fn add(&mut self, strength: f32, seconds: f32) {
        self.jolts.push(Jolt {
            start: Instant::now(),
            strength,
            seconds,
        });
    }
    /// How far the view turns now (yaw, pitch degrees), `scale` being the
    /// player's Distortion Effects; fades jolts out and forgets ended ones.
    fn offset(&mut self, now: Instant, scale: f32) -> (f32, f32) {
        self.jolts
            .retain(|j| now.duration_since(j.start).as_secs_f32() < j.seconds);
        let mut strength = 0.0;
        for j in &self.jolts {
            let left = 1.0 - now.duration_since(j.start).as_secs_f32() / j.seconds;
            strength += j.strength * left * left;
        }
        let strength = strength.min(1.0) * scale.clamp(0.0, 1.0);
        if strength <= 0.0 {
            return (0.0, 0.0);
        }
        // Smooth noise: sines at unrelated rates, so it never repeats visibly.
        let t = self
            .jolts
            .first()
            .map_or(0.0, |j| now.duration_since(j.start).as_secs_f32());
        let wave = |a: f32, b: f32, c: f32| {
            ((t * a).sin() * 0.5 + (t * b).sin() * 0.3 + (t * c).sin() * 0.2).clamp(-1.0, 1.0)
        };
        (
            strength * SHAKE_DEGREES.0 * wave(23.0, 37.3, 51.7),
            strength * SHAKE_DEGREES.1 * wave(29.1, 43.9, 61.3),
        )
    }
}

#[cfg(not(feature = "mods"))]
impl Game<'_> {
    pub(super) fn draw_mod_shapes(&self, _batch: &mut HudBatch) {}
    pub(super) fn shaken_view(&mut self, yaw: f32, pitch: f32) -> (f32, f32) {
        (yaw, pitch)
    }
}

#[cfg(feature = "mods")]
impl Game<'_> {
    /// The running mods' HUD shapes, in order, over the HUD.
    pub(super) fn draw_mod_shapes(&self, batch: &mut HudBatch) {
        for shape in self.mods.mods.hud_shapes() {
            self.draw_shapes(batch, std::slice::from_ref(shape), (0.0, 0.0));
        }
    }
    /// Shapes with their GUI coordinates moved by `(ox, oy)`.
    pub(super) fn draw_shapes(&self, batch: &mut HudBatch, shapes: &[Shape], (ox, oy): (f32, f32)) {
        let atlas = &self.gui.atlas;
        for shape in shapes {
            match shape {
                Shape::Fill(f) => {
                    let r = f.rect;
                    batch.fill(atlas, ox + r.x, oy + r.y, r.w, r.h, argb(f.color));
                }
                Shape::Quad(q) => {
                    let (a, b, c, d) = q.corners;
                    let corners = [a, b, c, d].map(|p| [ox + p.x, oy + p.y]);
                    batch.fill_quad(atlas, corners, argb(q.color));
                }
                Shape::Sprite(s) => {
                    let r = s.rect;
                    let name = s.sprite.strip_prefix("minecraft:").unwrap_or(&s.sprite);
                    let name = format!("gui/sprites/{name}");
                    // As vanilla's blitSprite: a frame or a button keeps its border.
                    ui::blit_sprite(
                        batch,
                        atlas,
                        &name,
                        ox + r.x,
                        oy + r.y,
                        r.w,
                        r.h,
                        argb(s.color),
                    );
                }
                Shape::Item(i) => {
                    if self.res.items().get(&i.item).is_none() {
                        continue;
                    }
                    let start = batch.vertices.len();
                    let stack = ItemStack::new(i.item.as_str(), 1);
                    let (x, y) = (ox + i.at.x, oy + i.at.y);
                    self.draw_item_model(batch, &stack, x, y);
                    batch.scale_since(start, x, y, i.scale, i.scale);
                }
                Shape::Text(t) => {
                    let text = Text::literal(t.text.clone());
                    let centre = if t.centered {
                        (HudBatch::text_width(atlas, &t.text) / 2.0).floor()
                    } else {
                        0.0
                    };
                    let x = ox + t.at.x - centre;
                    batch.text_styled(
                        atlas,
                        &text,
                        x,
                        oy + t.at.y,
                        argb(t.color | 0xFF00_0000),
                        t.shadow,
                    );
                }
            }
        }
    }
    /// The view's yaw and pitch after the mods' camera shakes; unchanged
    /// while none is fading.
    pub(super) fn shaken_view(&mut self, yaw: f32, pitch: f32) -> (f32, f32) {
        if self.mods.shake.jolts.is_empty() {
            return (yaw, pitch);
        }
        let scale = self.settings.file.double("screenEffectScale") as f32;
        let (dy, dp) = self.mods.shake.offset(Instant::now(), scale);
        (yaw + dy, (pitch + dp).clamp(-90.0, 90.0))
    }
}

#[cfg(all(test, feature = "mods"))]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_jolt_fades_out_and_distortion_scales_it() {
        let mut shake = Shake::default();
        shake.add(1.0, 2.0);
        let start = shake.jolts[0].start;
        let at = |s: f32| start + Duration::from_secs_f32(s);
        let size = |(y, p): (f32, f32)| y.abs() + p.abs();
        // Off with Distortion Effects at 0, inside the limits otherwise.
        assert_eq!(shake.offset(at(0.3), 0.0), (0.0, 0.0));
        let (y, p) = shake.offset(at(0.3), 1.0);
        assert!(y.abs() <= SHAKE_DEGREES.0 && p.abs() <= SHAKE_DEGREES.1);
        // Fading: the largest turn over a window shrinks.
        let mut peak = |from: f32| {
            (0..50)
                .map(|i| size(shake.offset(at(from + i as f32 * 0.002), 1.0)))
                .fold(0.0, f32::max)
        };
        let early = peak(0.1);
        let late = peak(1.6);
        assert!(late < early, "{late} !< {early}");
        // Gone once it ends.
        assert_eq!(shake.offset(at(2.1), 1.0), (0.0, 0.0));
        assert!(shake.jolts.is_empty());
    }
}
