//! Optional, texture-matched native overlays. No extra draw calls or world light changes.
use crate::sprites::Sprites;
use std::{collections::BTreeMap, path::Path};

pub const MAX: usize = 32;

/// Load only bounded, explicitly named emissive textures from the active pack.
pub fn load(root: &Path, sprites: &mut Sprites) {
    let mut names: Vec<_> = crate::atlas_sources::list_textures(root)
        .into_iter()
        .filter(|name| {
            name.split_once(':')
                .is_some_and(|(_, p)| p.starts_with("emissive/"))
        })
        .take(MAX)
        .collect();
    names.sort();
    for name in names {
        let file = crate::atlas_sources::texture_path(root, &name, "textures");
        if std::fs::metadata(file).is_ok_and(|m| m.len() <= 1 << 20) {
            let _ = sprites.insert(root, &name);
        }
    }
}

/// Only loaded base/overlay pairs are sent to the GPU; sorted for vertex lookup.
pub fn mappings<'a>(
    layers: &BTreeMap<String, u32>,
    requests: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Vec<[u32; 4]> {
    let mut pairs = BTreeMap::new();
    for (base, overlay) in requests.into_iter().take(MAX) {
        if let (Some(&b), Some(&o)) = (layers.get(base), layers.get(overlay)) {
            pairs.entry(b).or_insert(o);
        }
    }
    pairs.into_iter().map(|(b, o)| [b, o, 0, 0]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_existing_pairs_are_mapped() {
        let layers = [
            ("minecraft:block/ore".into(), 3),
            ("mine:emissive/ore".into(), 7),
        ]
        .into();
        assert_eq!(
            mappings(
                &layers,
                [
                    ("minecraft:block/ore", "mine:emissive/ore"),
                    ("missing", "mine:emissive/ore")
                ]
            ),
            vec![[3, 7, 0, 0]]
        );
    }
}
