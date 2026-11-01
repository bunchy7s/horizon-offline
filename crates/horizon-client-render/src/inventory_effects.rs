//! `EffectsInInventory`: active effect panels beside survival and creative
//! inventories, with compact icons when there is little room on the right.
use std::borrow::Cow;

use crate::{
    effects::EffectInstance,
    hud::{HudAtlas, HudBatch},
    hud_overlays::{EffectIcon, compare_effects, java_sort},
};

#[derive(Debug, PartialEq)]
pub struct EffectPanel {
    pub icon: EffectIcon,
    pub name: String,
    pub duration: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub spacing: i32,
}

/// Lay out panels using the container's actual origin (including recipe book
/// shifts). Vanilla hides them when fewer than 32 GUI pixels remain.
pub fn panels(
    screen_width: i32,
    origin: [i32; 2],
    image_width: i32,
    effects: &[EffectInstance],
    tick_rate: f32,
    translate: &dyn Fn(&str) -> Option<String>,
    text_width: &dyn Fn(&str) -> i32,
) -> Vec<EffectPanel> {
    let x = origin[0] + image_width + 2;
    let available = screen_width - x;
    if effects.is_empty() || available < 32 {
        return Vec::new();
    }
    let max_width = if available >= 120 { available - 7 } else { 32 };
    let spacing = if effects.len() > 5 {
        132 / (effects.len() as i32 - 1)
    } else {
        33
    };
    let mut sorted: Vec<_> = effects.iter().map(|e| (e, EffectIcon::from(e))).collect();
    java_sort(&mut sorted, |a, b| compare_effects(&a.1, &b.1));
    sorted
        .into_iter()
        .enumerate()
        .map(|(i, (effect, icon))| {
            let mut name = translate(&horizon_client_core::potions::description_id(effect.id))
                .unwrap_or_else(|| effect.id.to_owned());
            if (1..=9).contains(&effect.amplifier) {
                let level = effect.amplifier + 1;
                let suffix = translate(&format!("enchantment.level.{level}"))
                    .unwrap_or_else(|| crate::enchant::level_name(level as u32));
                name = format!("{name} {suffix}");
            }
            let duration = if effect.duration == -1 {
                translate("effect.duration.infinite").unwrap_or_else(|| "∞".to_owned())
            } else {
                // StringUtil.formatTickDuration uses the level's current rate.
                crate::brewing::format_tick_duration(effect.duration, tick_rate)
            };
            let width = max_width.min(39 + text_width(&name).max(text_width(&duration)));
            EffectPanel {
                icon,
                name,
                duration,
                x,
                y: origin[1] + i as i32 * spacing,
                width,
                spacing,
            }
        })
        .collect()
}

/// The inventory effect background has a four-pixel nine-slice border. Since
/// its height stays 32, only horizontal slicing/tiling is needed.
fn background(batch: &mut HudBatch, atlas: &HudAtlas, panel: &EffectPanel) {
    let name = if panel.icon.ambient {
        "gui/sprites/container/inventory/effect_background_ambient"
    } else {
        "gui/sprites/container/inventory/effect_background"
    };
    let Some(rect) = atlas.sprites.get(name).copied() else {
        return;
    };
    let mut strip = |dx: i32, sx: i32, width: i32| {
        let mut r = rect;
        let du = rect.u1 - rect.u0;
        r.u0 = rect.u0 + du * sx as f32 / 32.0;
        r.u1 = r.u0 + du * width as f32 / 32.0;
        batch.rect_uv(
            (panel.x + dx) as f32,
            panel.y as f32,
            width as f32,
            32.0,
            r,
            u32::MAX,
        );
    };
    strip(0, 0, 4);
    let mut dx = 4;
    while dx < panel.width - 4 {
        let width = 24.min(panel.width - 4 - dx);
        strip(dx, 4, width);
        dx += width;
    }
    strip(panel.width - 4, 28, 4);
}

/// Paint panels and return the full name/duration tooltip for a hovered compact
/// or clipped panel. Its hover height follows the spacing of overlapping rows.
pub fn paint(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    panels: &[EffectPanel],
    cursor: [i32; 2],
) -> Option<[(String, u32); 2]> {
    let mut tooltip = None;
    let mut icon = String::new();
    for panel in panels {
        background(batch, atlas, panel);
        let text_width = panel.width - 39;
        let clipped = atlas.font.width(&panel.name).ceil() as i32 > text_width;
        if text_width > 0 {
            let shown: Cow<'_, str> = if clipped {
                let available = text_width as f32 - atlas.font.width("...");
                let count = atlas.font.fit(&panel.name, available);
                let text: String = panel.name.chars().take(count).collect();
                Cow::Owned(format!("{text}..."))
            } else {
                Cow::Borrowed(panel.name.as_str())
            };
            batch.text(
                atlas,
                &shown,
                (panel.x + 32) as f32,
                (panel.y + 7) as f32,
                u32::MAX,
                true,
            );
            batch.text(
                atlas,
                &panel.duration,
                (panel.x + 32) as f32,
                (panel.y + 16) as f32,
                0xff808080,
                true,
            );
        }
        icon.clear();
        horizon_client_core::potions::push_sprite(&mut icon, &panel.icon.id);
        if let Some(r) = atlas.sprite_or_missing(&icon) {
            let (x, y) = ((panel.x + 7) as f32, (panel.y + 7) as f32);
            batch.rect_uv(x, y, 18.0, 18.0, r, u32::MAX);
        }
        if (text_width <= 0 || clipped)
            && cursor[0] >= panel.x
            && cursor[0] <= panel.x + panel.width
            && cursor[1] >= panel.y
            && cursor[1] <= panel.y + panel.spacing
        {
            tooltip = Some([
                (panel.name.clone(), u32::MAX),
                (panel.duration.clone(), u32::MAX),
            ]);
        }
    }
    tooltip
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(width: i32, effects: &[EffectInstance]) -> Vec<EffectPanel> {
        panels(
            width,
            [72, 37],
            176,
            effects,
            20.0,
            &|key| match key {
                "effect.minecraft.levitation" => Some("Levitation".into()),
                "effect.minecraft.speed" => Some("Speed".into()),
                _ => None,
            },
            &|text| text.chars().count() as i32 * 6,
        )
    }

    #[test]
    fn inventory_labels_levels_and_durations() {
        let effects = [
            EffectInstance::new("levitation", -1, 0),
            EffectInstance::new("speed", 450, 1),
        ];
        let p = layout(480, &effects);
        assert_eq!(p.len(), 2);
        // Natural order puts shorter durations before infinite effects.
        assert_eq!(
            (&p[0].name, &p[0].duration),
            (&"Speed II".into(), &"00:22".into())
        );
        assert_eq!(
            (&p[1].name, &p[1].duration),
            (&"Levitation".into(), &"∞".into())
        );
        assert_eq!((p[0].x, p[0].y, p[0].width), (250, 37, 87));
        assert_eq!((p[1].x, p[1].y, p[1].width), (250, 70, 99));
    }

    #[test]
    fn narrow_windows_hide_or_compact_the_panels() {
        let effects = [EffectInstance::new("levitation", -1, 0)];
        assert!(layout(281, &effects).is_empty());
        assert_eq!(layout(282, &effects)[0].width, 32);
        assert_eq!(layout(369, &effects)[0].width, 32);
        assert_eq!(layout(370, &effects)[0].width, 99);
        assert!(layout(480, &[]).is_empty());
    }

    #[test]
    fn crowded_effects_overlap_and_preserve_ambient_order() {
        let mut effects: Vec<_> = (0..7)
            .map(|i| EffectInstance::new("speed", (i + 1) * 100, 0))
            .collect();
        effects[0].ambient = true;
        let p = layout(480, &effects);
        assert_eq!(p[0].icon.duration, 200);
        assert!(p.last().unwrap().icon.ambient);
        assert_eq!(
            p.iter().map(|p| p.y).collect::<Vec<_>>(),
            [37, 59, 81, 103, 125, 147, 169]
        );
    }

    #[test]
    fn creative_origin_recipe_book_shift_and_tick_rate() {
        let effects = [EffectInstance::new("speed", 72000, 10)];
        let p = panels(640, [222, 43], 195, &effects, 10.0, &|_| None, &|_| 60);
        assert_eq!((p[0].x, p[0].y), (419, 43));
        assert_eq!(p[0].name, "speed");
        assert_eq!(p[0].duration, "02:00:00");
        let p = panels(480, [149, 37], 176, &effects, 20.0, &|_| None, &|_| 150);
        assert_eq!((p[0].x, p[0].width), (327, 146));
    }
}
