//! CreateFlatWorldScreen and PresetFlatWorldScreen: the Superflat layer list,
//! the preset chooser and the settings they hand back to Create New World.
use super::{Button, Title};
use crate::{
    hud::{HudAtlas, HudBatch, rgba},
    item_render::{self, ItemRenderState, ItemRenderer, NoHost},
    items::ItemStack,
    lang,
    text::{Text, format_translation},
    ui,
};
use horizon_assets::item_model::{DisplayContext, NoEnv};
use horizon_world::nbt::Tag;
use serde_json::Value;
use std::{collections::BTreeSet, path::Path};

/// DimensionType.Y_SIZE: the most blocks a flat generator takes.
const Y_SIZE: i32 = 4064;
/// The list's row width (ObjectSelectionList.getRowWidth).
const ROW_WIDTH: f32 = 220.0;
/// The header row above the layers.
const HEADER: f32 = 14.0;
/// A layer or preset row.
const LAYER_ROW: f32 = 20.0;
const PRESET_ROW: f32 = 24.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlatLayer {
    /// A block id such as `minecraft:dirt`.
    pub block: String,
    /// How many blocks tall this layer is (0..=4064).
    pub height: i32,
}

/// FlatLevelGeneratorSettings: the layers (bottom to top), the biome and the
/// decorations CreateFlatWorldScreen edits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlatSettings {
    pub biome: String,
    pub features: bool,
    pub lakes: bool,
    pub layers: Vec<FlatLayer>,
    pub structure_overrides: Vec<String>,
}
impl Default for FlatSettings {
    fn default() -> Self {
        // worldgen/world_preset/flat.json's overworld.
        Self {
            biome: "minecraft:plains".into(),
            features: false,
            lakes: false,
            layers: vec![
                FlatLayer {
                    block: "minecraft:bedrock".into(),
                    height: 1,
                },
                FlatLayer {
                    block: "minecraft:dirt".into(),
                    height: 2,
                },
                FlatLayer {
                    block: "minecraft:grass_block".into(),
                    height: 1,
                },
            ],
            structure_overrides: vec!["minecraft:strongholds".into(), "minecraft:villages".into()],
        }
    }
}
impl FlatSettings {
    /// A saved world's flat generator settings, for Re-Create.
    pub fn from_world(dir: &Path) -> Option<Self> {
        let settings = crate::save::world_gen_settings(dir)?;
        let generator = settings
            .get("dimensions")?
            .get("minecraft:overworld")?
            .get("generator")?;
        if crate::save::text(generator.get("type"))?.trim_start_matches("minecraft:") != "flat" {
            return None;
        }
        Self::from_tag(generator.get("settings")?)
    }
    pub fn from_tag(tag: &Tag) -> Option<Self> {
        let Some(Tag::List(list)) = tag.get("layers") else {
            return None;
        };
        let layers: Vec<FlatLayer> = list
            .iter()
            .filter_map(|layer| {
                Some(FlatLayer {
                    block: crate::save::text(layer.get("block"))?,
                    height: crate::save::number(layer.get("height"))?.clamp(0, Y_SIZE as i64)
                        as i32,
                })
            })
            .collect();
        // `layers: []` is a valid void flat world; a list whose entries all
        // fail to parse is not.
        if layers.is_empty() && !list.is_empty() {
            return None;
        }
        let flag = |key: &str, default: bool| {
            crate::save::number(tag.get(key)).map_or(default, |value| value != 0)
        };
        let structure_overrides = match tag.get("structure_overrides") {
            Some(Tag::String(text)) => vec![text.to_string_lossy()],
            Some(Tag::List(list)) => list
                .iter()
                .filter_map(|value| crate::save::text(Some(value)))
                .collect(),
            _ => Vec::new(),
        };
        Some(Self {
            biome: crate::save::text(tag.get("biome")).unwrap_or_else(|| "minecraft:plains".into()),
            features: flag("features", false),
            lakes: flag("lakes", false),
            layers,
            structure_overrides,
        })
    }
    /// FlatLevelGeneratorSettings.CODEC, as world_gen_settings stores it.
    pub fn to_tag(&self) -> Tag {
        let string = |text: &str| Tag::String(text.into());
        Tag::compound([
            ("biome", string(&self.biome)),
            ("features", Tag::Byte(i8::from(self.features))),
            ("lakes", Tag::Byte(i8::from(self.lakes))),
            (
                "layers",
                Tag::List(
                    self.layers
                        .iter()
                        .map(|layer| {
                            Tag::compound([
                                ("block", string(&layer.block)),
                                ("height", Tag::Int(layer.height)),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "structure_overrides",
                Tag::List(
                    self.structure_overrides
                        .iter()
                        .map(|id| string(id))
                        .collect(),
                ),
            ),
        ])
    }
    /// `CreateFlatWorldScreen.save`: the text the preset screen shows.
    pub fn to_share(&self) -> String {
        let layers = self
            .layers
            .iter()
            .map(|layer| {
                if layer.height == 1 {
                    layer.block.clone()
                } else {
                    format!("{}*{}", layer.height, layer.block)
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        format!("{layers};{}", self.biome)
    }
    /// `PresetFlatWorldScreen.fromString`: `layers;biome`, keeping the
    /// decorations the screen was opened with. `None` for a malformed list,
    /// as vanilla falls back to the preset's own settings.
    pub fn parse(
        text: &str,
        blocks: &BTreeSet<String>,
        biomes: &BTreeSet<String>,
        base: &Self,
    ) -> Option<Self> {
        let (layer_text, biome_text) = text.split_once(';').unwrap_or((text, ""));
        let mut layers = Vec::new();
        let mut total = 0;
        for part in layer_text.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let (height, block) = match part.split_once('*') {
                Some((height, block)) => (height.trim().parse::<i32>().ok()?, block.trim()),
                None => (1, part),
            };
            let block = namespaced(block);
            if !blocks.is_empty() && !blocks.contains(&block) {
                return None;
            }
            let height = height.max(0).min(Y_SIZE - total);
            if height == 0 {
                continue;
            }
            total += height;
            layers.push(FlatLayer { block, height });
        }
        if layers.is_empty() {
            return None;
        }
        let mut biome = namespaced(biome_text.trim());
        if biome == "minecraft:" || biome.is_empty() {
            biome = "minecraft:plains".into();
        }
        if !biomes.is_empty() && !biomes.contains(&biome) {
            biome = "minecraft:plains".into();
        }
        Some(Self {
            biome,
            layers,
            ..base.clone()
        })
    }
}
/// `Identifier.parse`: bare names get the default namespace.
fn namespaced(id: &str) -> String {
    if id.is_empty() || id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Page {
    Layers,
    Presets,
}

#[derive(Clone, Debug)]
pub struct State {
    pub(super) page: Page,
    pub(super) settings: FlatSettings,
    /// The preset screen's share box (`createWorld.customize.presets.share`).
    pub(super) share: String,
    pub(super) share_focused: bool,
    /// The selected layer, in the list's top-first order.
    pub(super) selected: Option<usize>,
    pub(super) scroll: ui::Scroll,
    preset_selected: Option<usize>,
    preset_scroll: ui::Scroll,
    presets: Vec<Preset>,
    blocks: BTreeSet<String>,
    biomes: BTreeSet<String>,
}
#[derive(Clone, Debug)]
struct Preset {
    id: String,
    display: String,
    settings: FlatSettings,
}
impl State {
    /// The layers the editor holds, for the control socket's state.
    pub fn settings(&self) -> &FlatSettings {
        &self.settings
    }
    pub(super) fn new(root: &Path, current: Option<FlatSettings>) -> Self {
        let settings = current.unwrap_or_default();
        let (presets, blocks, biomes) = load(root);
        Self {
            page: Page::Layers,
            share: settings.to_share(),
            settings,
            share_focused: false,
            selected: None,
            scroll: ui::Scroll::default(),
            preset_selected: None,
            preset_scroll: ui::Scroll::default(),
            presets,
            blocks,
            biomes,
        }
    }
    pub(super) fn open_presets(&mut self) {
        self.page = Page::Presets;
        self.preset_selected = None;
        self.preset_scroll = ui::Scroll::default();
        self.share = self.settings.to_share();
        self.share_focused = false;
    }
    pub(super) fn back_to_layers(&mut self) {
        self.page = Page::Layers;
        self.share_focused = false;
    }
    pub(super) fn remove_selected(&mut self) {
        let Some(row) = self.selected else {
            return;
        };
        let index = self.settings.layers.len().wrapping_sub(1 + row);
        if index < self.settings.layers.len() {
            self.settings.layers.remove(index);
        }
        self.selected = None;
    }
    /// PresetFlatWorldScreen's Select: parse the share text into the settings
    /// the Create New World screen keeps.
    pub(super) fn apply_share(&mut self) {
        // PresetFlatWorldScreen keeps the selected preset's decorations while
        // the share text supplies its layers and biome.
        let base = self
            .preset_selected
            .and_then(|index| self.presets.get(index))
            .map(|preset| preset.settings.clone())
            .unwrap_or_else(|| self.settings.clone());
        // A malformed list falls back to FlatLevelGeneratorSettings.getDefault.
        let settings =
            FlatSettings::parse(&self.share, &self.blocks, &self.biomes, &base).unwrap_or_default();
        self.settings = settings;
        self.share = self.settings.to_share();
        self.page = Page::Layers;
        self.share_focused = false;
        self.selected = None;
    }
    fn select_preset(&mut self, index: usize) {
        let Some(preset) = self.presets.get(index) else {
            return;
        };
        self.preset_selected = Some(index);
        self.share = preset.settings.to_share();
        self.share_focused = false;
    }
    fn selecting(&self) -> Option<usize> {
        self.preset_selected
    }
}

/// `<root>/data/<namespace>/worldgen/flat_level_generator_preset/<path>.json`,
/// in the order of the `visible` tag, plus the block and biome ids the share
/// text is checked against.
fn load(root: &Path) -> (Vec<Preset>, BTreeSet<String>, BTreeSet<String>) {
    let mut presets = Vec::new();
    let tag_path =
        root.join("data/minecraft/tags/worldgen/flat_level_generator_preset/visible.json");
    if let Ok(text) = std::fs::read_to_string(tag_path)
        && let Ok(Value::Object(tag)) = serde_json::from_str(&text)
        && let Some(Value::Array(ids)) = tag.get("values")
    {
        for id in ids.iter().filter_map(Value::as_str) {
            let Some((namespace, path)) = id.split_once(':') else {
                continue;
            };
            let path = root.join(format!(
                "data/{namespace}/worldgen/flat_level_generator_preset/{path}.json"
            ));
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            let Some(settings) = preset_settings(&value) else {
                continue;
            };
            presets.push(Preset {
                id: id.to_owned(),
                display: value["display"]
                    .as_str()
                    .unwrap_or("minecraft:grass_block")
                    .to_owned(),
                settings,
            });
        }
    }
    let blocks = registry_ids(root, "minecraft:block");
    let biomes = json_keys(&root.join("worldgen/biomes.json"));
    (presets, blocks, biomes)
}
fn preset_settings(value: &Value) -> Option<FlatSettings> {
    let settings = value.get("settings")?;
    let layers = settings
        .get("layers")?
        .as_array()?
        .iter()
        .filter_map(|layer| {
            Some(FlatLayer {
                block: layer.get("block")?.as_str()?.to_owned(),
                height: layer.get("height")?.as_i64()? as i32,
            })
        })
        .collect::<Vec<_>>();
    if layers.is_empty() {
        return None;
    }
    let structure_overrides = match settings.get("structure_overrides") {
        Some(Value::String(id)) => vec![id.clone()],
        Some(Value::Array(ids)) => ids
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    };
    Some(FlatSettings {
        biome: settings
            .get("biome")
            .and_then(Value::as_str)
            .unwrap_or("minecraft:plains")
            .to_owned(),
        features: settings
            .get("features")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        lakes: settings
            .get("lakes")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        layers,
        structure_overrides,
    })
}
/// The ids of a registry in the prepared `registries.json`.
fn registry_ids(root: &Path, registry: &str) -> BTreeSet<String> {
    let Ok(text) = std::fs::read_to_string(root.join("registries.json")) else {
        return BTreeSet::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return BTreeSet::new();
    };
    value
        .get(registry)
        .and_then(|registry| registry.get("entries"))
        .and_then(Value::as_object)
        .map(|entries| entries.keys().cloned().collect())
        .unwrap_or_default()
}
fn json_keys(path: &Path) -> BTreeSet<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeSet::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return BTreeSet::new();
    };
    value
        .as_object()
        .map(|entries| entries.keys().cloned().collect())
        .unwrap_or_default()
}

/// CreateFlatWorldScreen's list and the preset screen's list.
fn list_area(state: &State, size: (f32, f32)) -> (f32, f32, f32, f32) {
    let (w, h) = (size.0.ceil(), size.1.ceil());
    let (top, bottom) = match state.page {
        Page::Layers => (43.0, h - 60.0),
        Page::Presets => (80.0, h - 37.0),
    };
    let left = ((w - ROW_WIDTH) * 0.5).floor();
    let rows = match state.page {
        Page::Layers => HEADER + state.settings.layers.len() as f32 * LAYER_ROW + 4.0,
        Page::Presets => state.presets.len() as f32 * PRESET_ROW + 4.0,
    };
    (top, bottom, left, rows)
}

pub(super) fn buttons(w: f32, h: f32, state: &State) -> Vec<Button> {
    let b = |x: f32, y: f32, w: f32, key: &str, fallback: &str, enabled: bool, id: u8| Button {
        x,
        y,
        w,
        label: lang::get_or(key, fallback),
        enabled,
        id,
        icon: None,
        trailing: None,
    };
    match state.page {
        Page::Layers => {
            let x = ((w - 308.0) * 0.5).trunc();
            vec![
                b(
                    x,
                    h - 52.0,
                    150.0,
                    "createWorld.customize.flat.removeLayer",
                    "Remove Layer",
                    state.selected.is_some(),
                    70,
                ),
                b(
                    x + 158.0,
                    h - 52.0,
                    150.0,
                    "createWorld.customize.presets",
                    "Presets",
                    true,
                    71,
                ),
                b(x, h - 28.0, 150.0, "gui.done", "Done", true, 72),
                b(x + 158.0, h - 28.0, 150.0, "gui.cancel", "Cancel", true, 73),
            ]
        }
        Page::Presets => vec![
            b(
                w / 2.0 - 155.0,
                h - 28.0,
                150.0,
                "createWorld.customize.presets.select",
                "Select",
                state.selecting().is_some() || state.share.len() > 1,
                74,
            ),
            b(
                w / 2.0 + 5.0,
                h - 28.0,
                150.0,
                "gui.cancel",
                "Cancel",
                true,
                75,
            ),
        ],
    }
}

/// The layer under the pointer (top-first), and whether the pointer is on the
/// list at all.
fn layer_row(state: &State, cursor: (f32, f32), size: (f32, f32)) -> Option<usize> {
    let (top, bottom, _, _) = list_area(state, size);
    if cursor.1 < top || cursor.1 >= bottom {
        return None;
    }
    let y = cursor.1 - top - 2.0 + state.scroll.amount;
    if y < HEADER {
        return None;
    }
    let row = ((y - HEADER) / LAYER_ROW).floor() as usize;
    (row < state.settings.layers.len()).then_some(row)
}
fn preset_row(state: &State, cursor: (f32, f32), size: (f32, f32)) -> Option<usize> {
    let (top, bottom, _, _) = list_area(state, size);
    if cursor.1 < top || cursor.1 >= bottom {
        return None;
    }
    let row = ((cursor.1 - top - 2.0 + state.preset_scroll.amount) / PRESET_ROW).floor() as usize;
    (row < state.presets.len()).then_some(row)
}

/// A press on the list, the share box or its scrollbar.
pub(super) fn click(state: &mut State, cursor: (f32, f32), size: (f32, f32)) {
    let (top, bottom, left, content) = list_area(state, size);
    let bar = left + ROW_WIDTH + 6.0;
    match state.page {
        Page::Layers => {
            if state.scroll.press(cursor, bar, top, bottom, content) {
                return;
            }
            state.selected = layer_row(state, cursor, size);
        }
        Page::Presets => {
            let field = share_field(size);
            state.share_focused = ui::hovered(cursor, field[0], field[1], field[2], field[3]);
            if state.preset_scroll.press(cursor, bar, top, bottom, content) {
                return;
            }
            if let Some(row) = preset_row(state, cursor, size) {
                state.select_preset(row);
            }
        }
    }
}
fn share_field(size: (f32, f32)) -> [f32; 4] {
    [50.0, 40.0, (size.0.ceil() - 100.0).max(1.0), 20.0]
}

pub(super) fn wheel(state: &mut State, delta: f32, cursor: (f32, f32), size: (f32, f32)) {
    let (top, bottom, _, content) = list_area(state, size);
    if cursor.1 < top || cursor.1 >= bottom {
        return;
    }
    match state.page {
        Page::Layers => state.scroll.wheel(delta, LAYER_ROW, top, bottom, content),
        Page::Presets => state
            .preset_scroll
            .wheel(delta, PRESET_ROW, top, bottom, content),
    }
}
pub(super) fn drag(state: &mut State, cursor: (f32, f32), size: (f32, f32)) {
    let (top, bottom, _, content) = list_area(state, size);
    match state.page {
        Page::Layers => state.scroll.drag(cursor.1, top, bottom, content),
        Page::Presets => state.preset_scroll.drag(cursor.1, top, bottom, content),
    }
}
pub(super) fn release(state: &mut State) {
    state.scroll.release();
    state.preset_scroll.release();
}

pub(super) fn draw(
    title: &Title,
    atlas: &HudAtlas,
    items: Option<&ItemRenderer>,
    batch: &mut HudBatch,
    cursor: (f32, f32),
    state: &State,
) {
    let (w, h) = (batch.width.ceil(), batch.height.ceil());
    ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
    match state.page {
        Page::Layers => draw_layers(title, atlas, items, batch, cursor, state, (w, h)),
        Page::Presets => draw_presets(title, atlas, items, batch, cursor, state, (w, h)),
    }
}

fn draw_layers(
    _title: &Title,
    atlas: &HudAtlas,
    items: Option<&ItemRenderer>,
    batch: &mut HudBatch,
    cursor: (f32, f32),
    state: &State,
    (w, h): (f32, f32),
) {
    super::centered(
        atlas,
        batch,
        &lang::get_or(
            "createWorld.customize.flat.title",
            "Superflat Customization",
        ),
        8.0,
        u32::MAX,
    );
    let (top, bottom, left, content) = list_area(state, (w, h));
    ui::list_background(batch, atlas, top, bottom, false);
    crate::ui::separators(batch, atlas, top, bottom, false);
    let first = batch.vertices.len();
    let mut pending = Vec::new();
    let right = left + ROW_WIDTH;
    batch.text_styled(
        atlas,
        &Text::translate("createWorld.customize.flat.tile", &[]).underlined(),
        left + 2.0,
        top + 3.0,
        u32::MAX,
        true,
    );
    let height_header = Text::translate("createWorld.customize.flat.height", &[]).underlined();
    let hw = HudBatch::text_width(atlas, &height_header.plain());
    batch.text_styled(
        atlas,
        &height_header,
        right - hw - 2.0,
        top + 3.0,
        u32::MAX,
        true,
    );
    let offset = state.scroll.amount.trunc();
    let selected = state.selected;
    // DetailsList: its header row ("Layer Material. Height"), then the layers
    // top first, each "Selected: <item>. Height. <height>".
    crate::widgets::narratable(|| {
        use crate::widgets::{ListKind, Narratable, Row, join_for_narration as join};
        let tr = |key: &str| Text::translate(key, &[]).plain();
        let height = tr("createWorld.customize.flat.height");
        let row = |i: usize| {
            let narration = match i.checked_sub(1) {
                None => join(&[&tr("createWorld.customize.flat.tile"), &height]),
                Some(layer) => {
                    let layer = &state.settings.layers[state.settings.layers.len() - 1 - layer];
                    let item = Text::translate(
                        "narrator.select",
                        &[Text::literal(item_name(&layer.block))],
                    )
                    .plain();
                    join(&[&item, &height, &layer.height.to_string()])
                }
            };
            Row::new(i, narration)
        };
        let over_header = cursor.1 >= top
            && cursor.1 < bottom
            && cursor.1 - top - 2.0 + state.scroll.amount < HEADER;
        let hovered = if over_header {
            Some(0)
        } else {
            layer_row(state, cursor, (w, h)).map(|r| r + 1)
        };
        Narratable::list(
            ListKind::Object,
            state.settings.layers.len() + 1,
            hovered.map(row),
            selected.map(|r| row(r + 1)),
            [0.0, top, w, bottom - top],
        )
        .key(crate::widgets::key_of(&"create_world/flat/layers"))
    });
    for (row, layer) in state.settings.layers.iter().rev().enumerate() {
        let y = top + 2.0 + HEADER + row as f32 * LAYER_ROW - offset;
        let target = [left - 2.0, y - 1.0, ROW_WIDTH + 4.0, LAYER_ROW];
        let hovered = ui::hovered(cursor, target[0], target[1], target[2], target[3]);
        if selected == Some(row) {
            batch.fill(
                atlas,
                target[0],
                target[1],
                target[2],
                target[3],
                rgba(255, 255, 255, 70),
            );
        } else if hovered {
            batch.fill(
                atlas,
                target[0],
                target[1],
                target[2],
                target[3],
                rgba(255, 255, 255, 25),
            );
        }
        slot(
            atlas,
            items,
            &mut pending,
            batch,
            &layer.block,
            left + 2.0,
            y + 1.0,
        );
        batch.text(
            atlas,
            &item_name(&layer.block),
            left + 2.0 + 18.0 + 5.0,
            y + 5.0,
            u32::MAX,
            true,
        );
        let (key, fallback) = if row == 0 {
            ("createWorld.customize.flat.layer.top", "Top - %s")
        } else if row + 1 == state.settings.layers.len() {
            ("createWorld.customize.flat.layer.bottom", "Bottom - %s")
        } else {
            ("createWorld.customize.flat.layer", "%s")
        };
        let label = format_translation(
            &lang::get_or(key, fallback),
            &[Text::literal(layer.height.to_string())],
        )
        .plain();
        let text_width = HudBatch::text_width(atlas, &label);
        batch.text(
            atlas,
            &label,
            right - text_width - 2.0,
            y + 5.0,
            u32::MAX,
            true,
        );
    }
    batch.clip_since(first, 0.0, top, w, bottom);
    draw_items(atlas, items, batch, pending, [0.0, top, w, bottom]);
    ui::scrollbar(
        batch,
        atlas,
        left + ROW_WIDTH + 6.0,
        top,
        bottom,
        content,
        state.scroll.amount,
    );
}

fn draw_presets(
    title: &Title,
    atlas: &HudAtlas,
    items: Option<&ItemRenderer>,
    batch: &mut HudBatch,
    cursor: (f32, f32),
    state: &State,
    (w, h): (f32, f32),
) {
    super::centered(
        atlas,
        batch,
        &lang::get_or("createWorld.customize.presets.title", "Select a Preset"),
        8.0,
        u32::MAX,
    );
    batch.text(
        atlas,
        &lang::get_or(
            "createWorld.customize.presets.share",
            "Want to share your preset with someone? Use the box below!",
        ),
        51.0,
        30.0,
        rgba(160, 160, 160, 255),
        true,
    );
    // The share box: EditBox at 50,40, width - 100 by 20.
    let field = share_field((w, h));
    title.caret.borrow_mut().draw(
        batch,
        atlas,
        (super::FLAT_SHARE, &state.share, 1230),
        field,
        state.share_focused,
        title.started,
    );
    crate::widgets::name_last_edit_box(|| {
        Text::translate("createWorld.customize.presets.share", &[]).plain()
    });
    batch.text(
        atlas,
        &lang::get_or(
            "createWorld.customize.presets.list",
            "Alternatively, here are some we made earlier!",
        ),
        51.0,
        68.0,
        rgba(160, 160, 160, 255),
        true,
    );
    let (top, bottom, left, content) = list_area(state, (w, h));
    ui::list_background(batch, atlas, top, bottom, false);
    crate::ui::separators(batch, atlas, top, bottom, false);
    let first = batch.vertices.len();
    let mut pending = Vec::new();
    let offset = state.preset_scroll.amount.trunc();
    // PresetsList: "Selected: <preset>".
    crate::widgets::narratable(|| {
        use crate::widgets::{ListKind, Narratable, Row};
        let row = |i: usize| {
            let name = Text::literal(preset_name(&state.presets[i].id));
            Row::new(i, Text::translate("narrator.select", &[name]).plain())
        };
        Narratable::list(
            ListKind::Object,
            state.presets.len(),
            preset_row(state, cursor, (w, h)).map(row),
            state.preset_selected.map(row),
            [0.0, top, w, bottom - top],
        )
        .key(crate::widgets::key_of(&"create_world/flat/presets"))
    });
    for (row, preset) in state.presets.iter().enumerate() {
        let y = top + 2.0 + row as f32 * PRESET_ROW - offset;
        let target = [left - 2.0, y - 1.0, ROW_WIDTH + 4.0, PRESET_ROW];
        let hovered = ui::hovered(cursor, target[0], target[1], target[2], target[3]);
        if state.preset_selected == Some(row) {
            batch.fill(
                atlas,
                target[0],
                target[1],
                target[2],
                target[3],
                rgba(255, 255, 255, 70),
            );
        } else if hovered {
            batch.fill(
                atlas,
                target[0],
                target[1],
                target[2],
                target[3],
                rgba(255, 255, 255, 25),
            );
        }
        slot(
            atlas,
            items,
            &mut pending,
            batch,
            &preset.display,
            left + 2.0,
            y + 2.0,
        );
        batch.text(
            atlas,
            &preset_name(&preset.id),
            left + 2.0 + 18.0 + 5.0,
            y + 7.0,
            u32::MAX,
            true,
        );
    }
    batch.clip_since(first, 0.0, top, w, bottom);
    draw_items(atlas, items, batch, pending, [0.0, top, w, bottom]);
    ui::scrollbar(
        batch,
        atlas,
        left + ROW_WIDTH + 6.0,
        top,
        bottom,
        content,
        state.preset_scroll.amount,
    );
}

/// The 18x18 slot behind a layer's item icon (`container/slot`), then the item
/// (`GuiGraphics.renderItem`): its GUI model, so a block shows as the isometric
/// block. Without item models, or for an id with none, the flat sprite stands in.
fn slot(
    atlas: &HudAtlas,
    items: Option<&ItemRenderer>,
    pending: &mut Vec<(ItemRenderState, f32, f32)>,
    batch: &mut HudBatch,
    id: &str,
    x: f32,
    y: f32,
) {
    if let Some(rect) = atlas.sprites.get("gui/sprites/container/slot").copied() {
        batch.rect_uv(x, y, 18.0, 18.0, rect, u32::MAX);
    } else {
        batch.fill(atlas, x, y, 18.0, 18.0, rgba(139, 139, 139, 255));
        batch.fill(atlas, x + 1.0, y + 1.0, 16.0, 16.0, rgba(55, 55, 55, 255));
    }
    if let Some(renderer) = items {
        let stack = ItemStack::new(&layer_item(id), 1);
        let state = renderer.resolve(
            &item_render::stack_view(&stack),
            DisplayContext::Gui,
            None,
            None,
            0,
            &NoEnv,
        );
        if !state.is_empty() {
            pending.push((state, x + 1.0, y + 1.0));
            return;
        }
    }
    if let Some(icon) = layer_icon(atlas, id) {
        batch.sprite_scaled(atlas, &icon, x + 1.0, y + 1.0, 16.0, 16.0, u32::MAX);
    }
}
/// The item models of a list's rows, drawn after the list's own quads are cut
/// to its area (`clip_since` keeps or drops a slanted quad whole): each is
/// clipped exactly to the list, as vanilla's scissor does.
fn draw_items(
    atlas: &HudAtlas,
    renderer: Option<&ItemRenderer>,
    batch: &mut HudBatch,
    pending: Vec<(ItemRenderState, f32, f32)>,
    scissor: [f32; 4],
) {
    let Some(renderer) = renderer else { return };
    for (state, x, y) in &pending {
        item_render::gui::draw_scissored(
            state,
            renderer,
            &atlas.sprites,
            &NoHost,
            batch,
            *x,
            *y,
            scissor,
        );
    }
}
/// The item a layer shows: `FlatLayerInfo` draws `block.asItem()`, and the
/// fluids' items are their buckets.
fn layer_item(id: &str) -> String {
    match id.strip_prefix("minecraft:").unwrap_or(id) {
        "water" => "minecraft:water_bucket".into(),
        "lava" => "minecraft:lava_bucket".into(),
        name => format!("minecraft:{name}"),
    }
}
fn layer_icon(atlas: &HudAtlas, id: &str) -> Option<String> {
    let name = id.strip_prefix("minecraft:").unwrap_or(id);
    let candidates: Vec<String> = if name == "water" {
        vec!["item/water_bucket".into()]
    } else if name == "lava" {
        vec!["item/lava_bucket".into()]
    } else {
        vec![
            format!("item/{name}"),
            format!("block/{name}"),
            format!("block/{name}_side"),
            format!("block/{name}_top"),
        ]
    };
    candidates
        .into_iter()
        .find(|sprite| atlas.sprites.contains_key(sprite))
}
fn item_name(id: &str) -> String {
    let name = id.strip_prefix("minecraft:").unwrap_or(id);
    if name == "water" {
        return lang::get_or("item.minecraft.water_bucket", "Water Bucket");
    }
    if name == "lava" {
        return lang::get_or("item.minecraft.lava_bucket", "Lava Bucket");
    }
    for key in [
        format!("block.minecraft.{name}"),
        format!("item.minecraft.{name}"),
    ] {
        if lang::has(&key) {
            return lang::get(&key);
        }
    }
    pretty(name)
}
fn preset_name(id: &str) -> String {
    let key = format!("flat_world_preset.{}", id.replace(':', "."));
    lang::get_or(&key, &pretty(id.strip_prefix("minecraft:").unwrap_or(id)))
}
fn pretty(name: &str) -> String {
    name.split('_')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::layer_item;

    #[test]
    fn a_layer_shows_the_item_of_its_block() {
        assert_eq!(layer_item("minecraft:grass_block"), "minecraft:grass_block");
        assert_eq!(layer_item("dirt"), "minecraft:dirt");
        assert_eq!(layer_item("minecraft:water"), "minecraft:water_bucket");
        assert_eq!(layer_item("minecraft:lava"), "minecraft:lava_bucket");
    }
}
