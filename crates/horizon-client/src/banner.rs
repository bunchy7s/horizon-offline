//! Banner patterns: the `banner_pattern` registry and its tags, the
//! `banner_patterns` layers carried by banner items, shields and placed
//! banners, and the loom rules of `LoomMenu` (selectable patterns, the
//! six-layer limit and the result stack).
use crate::{block_entity_render::DYES, items::ItemStack, items::Items};
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path, sync::Arc};

pub use horizon_items::banner_layer::{BannerLayer, layers_from_nbt, layers_to_nbt};

/// Layers a loom may stack on a banner (`LoomMenu`'s `hasMaxPatterns`).
pub const MAX_LAYERS: usize = 6;

/// One `banner_pattern` registry entry.
#[derive(Clone, Debug)]
pub struct Pattern {
    pub id: Arc<str>,
    /// `asset_id` path, the texture under `entity/banner` and `entity/shield`.
    pub asset: String,
    pub translation_key: String,
}

/// The pattern registry, its tags and the item components the loom reads.
#[derive(Default)]
pub struct Banners {
    patterns: Vec<Pattern>,
    /// `#minecraft:no_item_required`, in tag order.
    no_item_required: Vec<Arc<str>>,
    /// Item -> `dye` component.
    dyes: HashMap<Arc<str>, Arc<str>>,
    /// Item -> `provides_banner_patterns`, in tag order.
    provided: HashMap<Arc<str>, Vec<Arc<str>>>,
}

fn read_tag(root: &Path, name: &str, depth: u32) -> Vec<Arc<str>> {
    if depth > 8 {
        return Vec::new();
    }
    let path = root.join(format!(
        "data/minecraft/tags/banner_pattern/{}.json",
        name.trim_start_matches("minecraft:")
    ));
    let Some(v) = fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for value in v["values"].as_array().into_iter().flatten() {
        let Some(s) = value.as_str().or_else(|| value["id"].as_str()) else {
            continue;
        };
        match s.strip_prefix('#') {
            Some(tag) => out.extend(read_tag(root, tag, depth + 1)),
            None => out.push(Arc::from(s)),
        }
    }
    out
}

impl Banners {
    pub fn load(root: &Path) -> Self {
        let mut banners = Self::default();
        let mut files: Vec<_> = fs::read_dir(root.join("data/minecraft/banner_pattern"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect();
        files.sort();
        for path in files {
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Some(v) = fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            else {
                continue;
            };
            let asset = v["asset_id"]
                .as_str()
                .unwrap_or(stem)
                .trim_start_matches("minecraft:")
                .to_owned();
            banners.patterns.push(Pattern {
                id: Arc::from(format!("minecraft:{stem}").as_str()),
                asset,
                translation_key: v["translation_key"].as_str().unwrap_or_default().to_owned(),
            });
        }
        banners.no_item_required = read_tag(root, "no_item_required", 0);
        if let Some(raw) = fs::read(root.join("items.json")).ok().and_then(|b| {
            serde_json::from_slice::<std::collections::BTreeMap<String, Value>>(&b).ok()
        }) {
            for (item, c) in raw {
                let id: Arc<str> = Arc::from(item.as_str());
                if let Some(dye) = c["minecraft:dye"].as_str() {
                    banners.dyes.insert(id.clone(), Arc::from(dye));
                }
                match &c["minecraft:provides_banner_patterns"] {
                    Value::String(tag) if tag.starts_with('#') => {
                        banners.provided.insert(id, read_tag(root, &tag[1..], 0));
                    }
                    Value::String(single) => {
                        banners
                            .provided
                            .insert(id, vec![Arc::from(single.as_str())]);
                    }
                    _ => {}
                }
            }
        }
        banners
    }
    pub fn pattern(&self, id: &str) -> Option<&Pattern> {
        self.patterns.iter().find(|p| &*p.id == id)
    }
    /// `Sheets.getBannerSprite` texture name of a pattern.
    pub fn banner_texture(&self, id: &str) -> String {
        match self.pattern(id) {
            Some(p) => format!("entity/banner/{}", p.asset),
            None => format!("entity/banner/{}", id.trim_start_matches("minecraft:")),
        }
    }
    /// `Sheets.getShieldSprite` texture name of a pattern.
    pub fn shield_texture(&self, id: &str) -> String {
        match self.pattern(id) {
            Some(p) => format!("entity/shield/{}", p.asset),
            None => format!("entity/shield/{}", id.trim_start_matches("minecraft:")),
        }
    }
    /// `BannerPatternLayers.Layer.description` translation key.
    pub fn layer_key(&self, layer: &BannerLayer) -> String {
        let base = self.pattern(&layer.pattern).map_or_else(
            || {
                format!(
                    "block.minecraft.banner.{}",
                    layer.pattern.trim_start_matches("minecraft:")
                )
            },
            |p| p.translation_key.clone(),
        );
        format!("{base}.{}", layer.color)
    }
    /// `LoomMenu.isDyeItem`: the dye colour of a loom dye.
    pub fn loom_dye(&self, items: &Items, stack: &ItemStack) -> Option<Arc<str>> {
        let tagged = items
            .item_tags
            .get("minecraft:loom_dyes")
            .is_some_and(|t| t.contains(&*stack.item));
        tagged
            .then(|| self.dyes.get(&stack.item).cloned())
            .flatten()
    }
    /// `LoomMenu.isPatternItem`.
    pub fn is_pattern_item(&self, items: &Items, stack: &ItemStack) -> bool {
        items
            .item_tags
            .get("minecraft:loom_patterns")
            .is_some_and(|t| t.contains(&*stack.item))
            && self.provided.contains_key(&stack.item)
    }
    /// `LoomMenu.getSelectablePatterns`.
    pub fn selectable(&self, pattern_item: Option<&ItemStack>) -> Vec<Arc<str>> {
        match pattern_item {
            None => self.no_item_required.clone(),
            Some(stack) => self.provided.get(&stack.item).cloned().unwrap_or_default(),
        }
    }
}

/// `BannerItem`: a banner's dye colour from its item id.
pub fn banner_color(item: &str) -> Option<&'static str> {
    let name = item.strip_prefix("minecraft:")?.strip_suffix("_banner")?;
    DYES.iter().map(|d| d.0).find(|d| *d == name)
}
/// `DyeColor.getTextureDiffuseColor` (RGB) by dye name.
pub fn dye_rgb(name: &str) -> u32 {
    DYES.iter().find(|d| d.0 == name).map_or(0xFFFFFF, |d| d.1)
}
/// The layers of a banner or shield stack.
pub fn layers(stack: &ItemStack) -> &[BannerLayer] {
    stack.data().map_or(&[], |d| &d.banner_patterns)
}
/// The ominous banner's component patch hides its pattern tooltip.
pub fn tooltip_layers(stack: &ItemStack) -> &[BannerLayer] {
    if stack.data().is_some_and(|d| d.ominous_banner) {
        &[]
    } else {
        layers(stack)
    }
}
/// `LoomMenu.setupResultSlot`: one banner with the pattern added in the dye colour.
pub fn loom_result(banner: &ItemStack, dye: &str, pattern: &str) -> ItemStack {
    let mut result = ItemStack {
        count: 1,
        ..banner.clone()
    };
    result
        .data_mut()
        .banner_patterns
        .push(BannerLayer::new(pattern, dye));
    result
}
/// Whether the banner already carries the loom's limit.
pub fn has_max_layers(banner: &ItemStack) -> bool {
    layers(banner).len() >= MAX_LAYERS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn banners() -> Banners {
        let mut b = Banners {
            no_item_required: vec![Arc::from("minecraft:stripe_top")],
            ..Default::default()
        };
        b.provided.insert(
            Arc::from("minecraft:flower_banner_pattern"),
            vec![Arc::from("minecraft:flower")],
        );
        b
    }
    #[test]
    fn selectable_follow_the_pattern_item() {
        let b = banners();
        assert_eq!(b.selectable(None).len(), 1);
        let item = ItemStack::new("minecraft:flower_banner_pattern", 1);
        assert_eq!(&*b.selectable(Some(&item))[0], "minecraft:flower");
    }
    #[test]
    fn banner_colour_comes_from_the_item() {
        assert_eq!(
            banner_color("minecraft:light_gray_banner"),
            Some("light_gray")
        );
        assert_eq!(banner_color("minecraft:white_banner"), Some("white"));
        assert_eq!(banner_color("minecraft:white_wool"), None);
    }
    #[test]
    fn loom_adds_a_layer_up_to_the_limit() {
        let mut banner = ItemStack::new("minecraft:white_banner", 3);
        for i in 0..MAX_LAYERS {
            assert!(!has_max_layers(&banner));
            banner = loom_result(&banner, "red", "minecraft:cross");
            assert_eq!(banner.count, 1);
            assert_eq!(layers(&banner).len(), i + 1);
        }
        assert!(has_max_layers(&banner));
    }
    #[test]
    fn ominous_banner_hides_tooltip_layers_but_keeps_its_patterns() {
        let mut banner = crate::mobs::raider::ominous_banner();
        (banner.damage, banner.extra) = crate::item_data::decode_components(
            crate::item_data::encode_components(&banner).as_ref(),
        );
        assert_eq!(
            layers(&banner).len(),
            crate::item_data::OMINOUS_LAYERS.len()
        );
        assert!(tooltip_layers(&banner).is_empty());
        banner.data_mut().custom_name =
            Some(horizon_world::nbt::Tag::String("Captain's Standard".into()));
        assert!(tooltip_layers(&banner).is_empty());
        banner.data_mut().ominous_banner = false;
        assert_eq!(tooltip_layers(&banner), layers(&banner));
        assert!(!tooltip_layers(&banner).is_empty());
    }

    #[test]
    fn layers_round_trip_through_nbt() {
        let l = vec![
            BannerLayer::new("minecraft:flower", "red"),
            BannerLayer::new("minecraft:border", "black"),
        ];
        assert_eq!(layers_from_nbt(Some(&layers_to_nbt(&l))), l);
    }
}
