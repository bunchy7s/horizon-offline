//! The look of Horizon's graphics settings screens, in one place: sizes,
//! colours and the widgets they draw with. The screens (`graphics_ui`) only
//! lay out rows and handle input; a menu theme replaces this module.
use crate::{
    hud::{HudAtlas, HudBatch},
    lang,
    text::Text,
    ui,
};

/// What a setting costs in frame time, shown beside its description.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cost {
    None,
    Low,
    Medium,
    High,
    /// Turning it on makes frames faster (culling).
    Saves,
}
impl Cost {
    pub fn label(self) -> &'static str {
        match self {
            Cost::None => "No cost",
            Cost::Low => "Low cost",
            Cost::Medium => "Medium cost",
            Cost::High => "High cost",
            Cost::Saves => "Saves time",
        }
    }
}

pub struct Style {
    /// Width of a setting's button or slider.
    pub control_width: f32,
    /// Space between the control and its description.
    pub gap: f32,
    /// Widest description column; narrower on small screens.
    pub max_text_width: f32,
    /// Vertical distance between setting rows.
    pub row_height: f32,
    pub header_height: f32,
    /// Top and bottom bands holding the title and the buttons.
    pub band: f32,
    pub title_color: u32,
    pub header_color: u32,
    pub description_color: u32,
    /// Colour of each `Cost`, in its order.
    pub cost_colors: [u32; 5],
}

/// `hud::rgba` with full alpha, usable in constants.
const fn opaque(r: u8, g: u8, b: u8) -> u32 {
    0xFF00_0000 | (b as u32) << 16 | (g as u32) << 8 | r as u32
}

pub const STYLE: Style = Style {
    control_width: 150.0,
    gap: 8.0,
    max_text_width: 250.0,
    row_height: 25.0,
    header_height: 16.0,
    band: 33.0,
    title_color: u32::MAX,
    header_color: opaque(255, 255, 85),
    description_color: opaque(170, 170, 170),
    cost_colors: [
        opaque(120, 200, 120),
        opaque(170, 220, 120),
        opaque(240, 200, 90),
        opaque(240, 110, 90),
        opaque(85, 255, 85),
    ],
};

impl Style {
    /// Width of the description column on a screen `screen_width` wide.
    pub fn text_width(&self, screen_width: f32) -> f32 {
        (screen_width - self.control_width - self.gap - 24.0).clamp(120.0, self.max_text_width)
    }
    /// Left edge of a row's control, centring control and description.
    pub fn row_x(&self, screen_width: f32) -> f32 {
        let total = self.control_width + self.gap + self.text_width(screen_width);
        ((screen_width - total) / 2.0).floor()
    }
    pub fn background(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        top: f32,
        bottom: f32,
        in_world: bool,
    ) {
        let (w, h) = (batch.width, batch.height);
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, in_world);
        ui::list_background(batch, atlas, top, bottom, in_world);
    }
    pub fn frame(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        top: f32,
        bottom: f32,
        in_world: bool,
    ) {
        ui::separators(batch, atlas, top, bottom, in_world);
    }
    pub fn title(&self, batch: &mut HudBatch, atlas: &HudAtlas, text: &Text) {
        let w = batch.width;
        ui::centered_text(batch, atlas, text, 0.0, 12.0, w, self.title_color);
    }
    pub fn header(&self, batch: &mut HudBatch, atlas: &HudAtlas, text: &Text, x: f32, y: f32) {
        batch.text_styled(atlas, text, x, y + 4.0, self.header_color, true);
    }
    pub fn button(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        (x, y, w): (f32, f32, f32),
        label: &Text,
        hover: bool,
        active: bool,
    ) {
        ui::button(batch, atlas, x, y, w, 20.0, label, active, hover);
    }
    pub fn slider(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        (x, y, w): (f32, f32, f32),
        label: &Text,
        value: f32,
        hover: bool,
        active: bool,
        dragging: bool,
    ) {
        ui::slider(
            batch,
            atlas,
            x,
            y,
            w,
            label,
            value,
            hover && active,
            active,
            dragging && active,
        );
    }
    pub fn slider_ext(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        (x, y, w): (f32, f32, f32),
        label: &Text,
        value: f32,
        hover: bool,
        focused: bool,
        can_change_value: bool,
        active: bool,
        dragging: bool,
    ) {
        ui::slider_ext(
            batch,
            atlas,
            x,
            y,
            w,
            label,
            value,
            hover && active,
            focused,
            can_change_value,
            active,
            dragging && active,
        );
    }
    /// The cost tag and the description beside a control, wrapped to two lines.
    pub fn description(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        x: f32,
        y: f32,
        cost: Cost,
        text: &str,
    ) {
        let tag = Text::literal(lang::get(cost.label()));
        batch.text_styled(atlas, &tag, x, y, self.cost_colors[cost as usize], true);
        let indent = atlas.font.width_text(&tag) + 4.0;
        let text = lang::get(text);
        let lines = wrap(atlas, &text, self.text_width(batch.width), indent);
        for (i, line) in lines.iter().take(2).enumerate() {
            let lx = if i == 0 { x + indent } else { x };
            batch.text_styled(
                atlas,
                &Text::literal(line.clone()),
                lx,
                y + i as f32 * 10.0,
                self.description_color,
                true,
            );
        }
    }
}

/// Greedy word wrap; the first line is `indent` narrower.
fn wrap(atlas: &HudAtlas, text: &str, width: f32, indent: f32) -> Vec<String> {
    let mut lines = vec![String::new()];
    for word in text.split_whitespace() {
        let room = if lines.len() == 1 {
            width - indent
        } else {
            width
        };
        let line = lines.last().expect("one line");
        let candidate = if line.is_empty() {
            word.to_owned()
        } else {
            format!("{line} {word}")
        };
        if atlas.font.width(&candidate) <= room || line.is_empty() {
            *lines.last_mut().expect("one line") = candidate;
        } else {
            lines.push(word.to_owned());
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_cost_has_a_label_and_a_colour() {
        let all = [Cost::None, Cost::Low, Cost::Medium, Cost::High, Cost::Saves];
        assert_eq!(all.len(), STYLE.cost_colors.len());
        for (i, cost) in all.into_iter().enumerate() {
            assert_eq!(cost as usize, i);
            assert!(!cost.label().is_empty());
        }
    }
}
