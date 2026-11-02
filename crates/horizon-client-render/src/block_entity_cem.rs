//! OptiFine custom entity models over the block entities' models (`block_entity_render`'s
//! `MODELS`): `chest.jem`, `shulker_box.jem`, `banner.jem`, `bell.jem`, `head_zombie.jem`... (the
//! "Block entity name" table of OptiFine's `cem_model.txt`, and Entity Model Features' one-file
//! `banner`), found and read as the mobs' are ([`horizon_optifine::cem::files`]). Each entry
//! replaces or joins the boxes of the vanilla part it names and takes its `scale`; animations
//! don't run yet. A `.jem`'s texture paints the models it reshapes instead of vanilla's when the
//! atlas has it.
//!
//! An entry's part is OptiFine's name for it, else the vanilla part's own name. Trapped and
//! ender chests take only their own file, and copper chests (no CEM name) keep vanilla's chest.
//! Lectern and enchanting-table books take theirs, else `book`. Signs, hanging signs and beds are
//! block models here, so their files are not read; copper golem statues have no CEM name.

use crate::block_entity_render::ModelPart;
use horizon_optifine::cem::{
    files::{atlas_name, jem_files, load_jem},
    model::{Bone, EntityModel},
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::Path,
};

/// One model the renderer draws, the CEM names that reshape it, best first, and OptiFine's
/// names for its vanilla parts.
struct Target {
    key: &'static str,
    /// The vanilla model it starts from: `key` itself, or the one a block the renderer tells
    /// apart shares (a trapped chest's is the chest's).
    base: &'static str,
    names: &'static [&'static str],
    parts: &'static [(&'static str, &'static str)],
}

const CHEST: &[(&str, &str)] = &[
    ("root", "root"),
    ("base", "bottom"),
    ("lid", "lid"),
    ("knob", "lock"),
];
const SHULKER: &[(&str, &str)] = &[("root", "root"), ("base", "base"), ("lid", "lid")];
const BANNER: &[(&str, &str)] = &[("stand", "pole"), ("top", "bar")];
const FLAG: &[(&str, &str)] = &[("slate", "flag")];
const HEAD: &[(&str, &str)] = &[("root", "root"), ("head", "head")];
const DRAGON: &[(&str, &str)] = &[("root", "root"), ("head", "head"), ("jaw", "jaw")];
const PIGLIN: &[(&str, &str)] = &[
    ("root", "root"),
    ("head", "head"),
    ("left_ear", "left_ear"),
    ("right_ear", "right_ear"),
];
const BELL: &[(&str, &str)] = &[
    ("root", "root"),
    ("body", "bell_body"),
    ("base", "bell_base"),
];
const BOOK: &[(&str, &str)] = &[
    ("root", "root"),
    ("cover_right", "right_lid"),
    ("cover_left", "left_lid"),
    ("pages_right", "right_pages"),
    ("pages_left", "left_pages"),
    ("flipping_page_right", "flip_page1"),
    ("flipping_page_left", "flip_page2"),
    ("book_spine", "seam"),
];
const POT_BASE: &[(&str, &str)] = &[("neck", "neck"), ("top", "top"), ("bottom", "bottom")];
const POT_SIDES: &[(&str, &str)] = &[
    ("front", "front"),
    ("back", "back"),
    ("left", "left"),
    ("right", "right"),
];

const fn model(
    key: &'static str,
    names: &'static [&'static str],
    parts: &'static [(&'static str, &'static str)],
) -> Target {
    Target {
        key,
        base: key,
        names,
        parts,
    }
}
const fn copy(
    key: &'static str,
    base: &'static str,
    names: &'static [&'static str],
    parts: &'static [(&'static str, &'static str)],
) -> Target {
    Target {
        key,
        base,
        names,
        parts,
    }
}

/// In order: a copy comes after its base.
const TARGETS: &[Target] = &[
    model("chest", &["chest"], CHEST),
    model("double_chest_left", &["chest_left"], CHEST),
    model("double_chest_right", &["chest_right"], CHEST),
    copy("trapped_chest", "chest", &["trapped_chest"], CHEST),
    copy(
        "trapped_chest_left",
        "double_chest_left",
        &["trapped_chest_left"],
        CHEST,
    ),
    copy(
        "trapped_chest_right",
        "double_chest_right",
        &["trapped_chest_right"],
        CHEST,
    ),
    copy("ender_chest", "chest", &["ender_chest"], CHEST),
    model("shulker_box", &["shulker_box"], SHULKER),
    model("standing_banner", &["banner_base", "banner"], BANNER),
    model("wall_banner", &["banner_base", "banner"], BANNER),
    model("standing_banner_flag", &["banner_flag", "banner"], FLAG),
    model("wall_banner_flag", &["banner_flag", "banner"], FLAG),
    model("skeleton_skull", &["head_skeleton"], HEAD),
    model("wither_skeleton_skull", &["head_wither_skeleton"], HEAD),
    model("zombie_head", &["head_zombie"], HEAD),
    model("creeper_head", &["head_creeper"], HEAD),
    model("dragon_skull", &["head_dragon"], DRAGON),
    model("piglin_head", &["head_piglin"], PIGLIN),
    model("player_head", &["head_player"], HEAD),
    model("bell", &["bell"], BELL),
    model("book", &["enchanting_book", "book"], BOOK),
    copy("lectern_book", "book", &["lectern_book", "book"], BOOK),
    model("conduit_shell", &["conduit"], &[("base", "shell")]),
    model("conduit_cage", &["conduit"], &[("cage", "shell")]),
    model("conduit_wind", &["conduit"], &[("wind", "wind")]),
    model("conduit_eye", &["conduit"], &[("eye", "eye")]),
    model("decorated_pot_base", &["decorated_pot"], POT_BASE),
    model("decorated_pot_sides", &["decorated_pot"], POT_SIDES),
];

/// Every model, reshaped or vanilla's, and the atlas names of the `.jem` textures by model.
pub type Reshaped = (
    HashMap<&'static str, ModelPart>,
    HashMap<&'static str, String>,
);

/// The models the packs under `root` reshape, over `vanilla`'s; None when no block entity has
/// a `.jem` there.
pub fn reshape(root: &Path, vanilla: &HashMap<&'static str, ModelPart>) -> Option<Reshaped> {
    let (reshaped, unused) = reshape_all(root, vanilla)?;
    for (path, part) in unused {
        log::warn!("Horizon: custom entity model {path}: no part {part}");
    }
    Some(reshaped)
}

/// [`reshape`]'s models, and each file's entries whose part none of the models it reshapes has.
fn reshape_all(
    root: &Path,
    vanilla: &HashMap<&'static str, ModelPart>,
) -> Option<(Reshaped, Vec<(String, String)>)> {
    let files = jem_files(root);
    let named = |t: &Target| t.names.iter().any(|n| files.contains_key(*n));
    if !TARGETS.iter().any(named) {
        return None;
    }
    let mut models = vanilla.clone();
    let mut textures = HashMap::new();
    let mut reshaped = HashSet::new();
    // Each file parsed once; which of its entries found a part in some model.
    let mut parsed: HashMap<&str, Option<EntityModel>> = HashMap::new();
    let mut used: BTreeMap<&str, Vec<bool>> = BTreeMap::new();
    for t in TARGETS {
        let Some(model) = vanilla.get(t.base) else {
            continue;
        };
        let found = t
            .names
            .iter()
            .flat_map(|name| files.get(*name).into_iter().flatten())
            .map(String::as_str)
            .find(|&path| {
                parsed
                    .entry(path)
                    .or_insert_with(|| load_jem(root, path))
                    .is_some()
            });
        let Some(path) = found else {
            // A copy of a reshaped base that has no file of its own stays vanilla.
            if t.key != t.base && reshaped.contains(t.base) {
                models.insert(t.key, model.clone());
            }
            continue;
        };
        let Some(Some(jem)) = parsed.get(path) else {
            continue;
        };
        let mut out = model.clone();
        let hits = apply(&mut out, jem, t.parts);
        // A file shared by several models (the conduit's, the pot's) paints only the models
        // it reshapes: vanilla geometry keeps its own texture's layout.
        let reshapes = hits.iter().any(|hit| *hit);
        let marks = used
            .entry(path)
            .or_insert_with(|| vec![false; jem.parts.len()]);
        for (mark, hit) in marks.iter_mut().zip(hits) {
            *mark |= hit;
        }
        models.insert(t.key, out);
        reshaped.insert(t.key);
        if reshapes && let Some(texture) = jem.texture.as_deref().and_then(atlas_name) {
            textures.insert(t.key, texture);
        }
    }
    let mut unused = Vec::new();
    for (path, marks) in &used {
        let Some(Some(jem)) = parsed.get(path) else {
            continue;
        };
        for (p, _) in jem.parts.iter().zip(marks).filter(|(_, hit)| !**hit) {
            unused.push(((*path).to_owned(), p.part.clone()));
        }
    }
    Some(((models, textures), unused))
}

/// `jem`'s entries on `model`, OptiFine's part names read through `parts`, else as the vanilla
/// part's own name: a replacing entry clears its part's boxes, and each joins the part as a child
/// `cem#<n>`. Which entries found their part.
fn apply(model: &mut ModelPart, jem: &EntityModel, parts: &[(&str, &str)]) -> Vec<bool> {
    let mut count = 0;
    let mut hits = Vec::with_capacity(jem.parts.len());
    for p in &jem.parts {
        let mapped = parts.iter().find(|(of, _)| *of == p.part).map(|&(_, v)| v);
        let target = match mapped {
            Some(vanilla) if find_mut(model, vanilla).is_some() => vanilla,
            _ => p.part.as_str(),
        };
        let Some(node) = find_mut(model, target) else {
            hits.push(false);
            continue;
        };
        if !p.attach {
            node.polygons.clear();
        }
        let (name, mut bone) = part(&p.bone, &mut count);
        bone.pose[6..9].fill(p.scale);
        node.children.push((name, bone));
        hits.push(true);
    }
    hits
}

/// The model itself for `root`, else its first part named `name`, depth first.
fn find_mut<'a>(model: &'a mut ModelPart, name: &str) -> Option<&'a mut ModelPart> {
    fn child<'a>(part: &'a mut ModelPart, name: &str) -> Option<&'a mut ModelPart> {
        part.children.iter_mut().find_map(|(n, c)| {
            if n.as_str() == name {
                Some(c)
            } else {
                child(c, name)
            }
        })
    }
    if name == "root" {
        return Some(model);
    }
    child(model, name)
}

/// A baked bone as a model part, named `cem#<n>`: unique in the model, so no vanilla pose
/// change by name reaches it.
fn part(bone: &Bone, count: &mut usize) -> (String, ModelPart) {
    let name = format!("cem#{count}");
    *count += 1;
    let [x, y, z] = bone.pivot;
    let [rx, ry, rz] = bone.rotation;
    let out = ModelPart {
        pose: [x, y, z, rx, ry, rz, 1.0, 1.0, 1.0],
        polygons: bone.quads.iter().map(|q| q.vertices).collect(),
        children: bone.children.iter().map(|c| part(c, count)).collect(),
    };
    (name, out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::{collections::BTreeSet, fs};

    /// A part with one face and `children`.
    pub(crate) fn plane(children: Vec<(&str, ModelPart)>) -> ModelPart {
        ModelPart {
            pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            polygons: vec![[[0.0; 5]; 4]],
            children: children
                .into_iter()
                .map(|(n, c)| (n.to_owned(), c))
                .collect(),
        }
    }
    pub(crate) fn chest() -> ModelPart {
        let mut root = plane(vec![
            ("bottom", plane(vec![])),
            ("lid", plane(vec![])),
            ("lock", plane(vec![])),
        ]);
        root.polygons.clear();
        root
    }
    pub(crate) const BOX: &str =
        r#""boxes": [{"coordinates": [0, 0, 0, 1, 1, 1], "textureOffset": [0, 0]}]"#;
    /// A fresh pack folder named for `test`, holding `files` under `assets/minecraft/`.
    pub(crate) fn pack(test: &str, files: &[(&str, String)]) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("horizon-be-cem-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for (path, text) in files {
            let path = root.join("assets/minecraft").join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        root
    }
    /// A `.jem` replacing each of `parts` with one box.
    fn jem(parts: &[&str]) -> String {
        let entries: Vec<_> = parts
            .iter()
            .map(|p| format!(r#"{{"part": "{p}", {BOX}}}"#))
            .collect();
        format!(r#"{{"models": [{}]}}"#, entries.join(", "))
    }
    /// A `.jem` replacing each of `parts` with a box of its own: the `i`th is `i + 1` pixels
    /// deep, so a part that took another entry's box shows.
    fn jem_sized(parts: &[&str]) -> String {
        let entries: Vec<_> = parts
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let depth = i + 1;
                format!(
                    r#"{{"part": "{p}", "boxes": [{{"coordinates": [0, 0, 0, 1, 1, {depth}], "textureOffset": [0, 0]}}]}}"#
                )
            })
            .collect();
        format!(r#"{{"models": [{}]}}"#, entries.join(", "))
    }
    /// How deep (z) the custom box under `model`'s part `name` is.
    fn depth(model: &ModelPart, name: &str) -> f32 {
        let part = model.child(name).unwrap();
        let zs = part
            .children
            .iter()
            .flat_map(|(_, c)| c.polygons.iter().flatten().map(|v| v[2]));
        let (lo, hi) = zs.fold((f32::MAX, f32::MIN), |(lo, hi), z| (lo.min(z), hi.max(z)));
        hi - lo
    }
    /// Whether `model`'s part `name` was replaced by one custom box: its own face gone, and one
    /// child of a box's six faces.
    fn replaced(model: &ModelPart, name: &str) -> bool {
        let part = model.child(name).unwrap();
        match part.children.as_slice() {
            [(_, only)] => part.polygons.is_empty() && only.polygons.len() == 6,
            _ => false,
        }
    }

    #[test]
    fn every_optifine_part_name_lands_on_its_vanilla_part() {
        // File, OptiFine's part, the model it reshapes, vanilla's part there.
        let cases = [
            ("chest", "base", "chest", "bottom"),
            ("chest", "lid", "chest", "lid"),
            ("chest", "knob", "chest", "lock"),
            ("chest_left", "base", "double_chest_left", "bottom"),
            ("chest_left", "lid", "double_chest_left", "lid"),
            ("chest_left", "knob", "double_chest_left", "lock"),
            ("chest_right", "base", "double_chest_right", "bottom"),
            ("chest_right", "lid", "double_chest_right", "lid"),
            ("chest_right", "knob", "double_chest_right", "lock"),
            ("trapped_chest", "base", "trapped_chest", "bottom"),
            ("trapped_chest", "knob", "trapped_chest", "lock"),
            ("trapped_chest_left", "base", "trapped_chest_left", "bottom"),
            ("trapped_chest_left", "knob", "trapped_chest_left", "lock"),
            (
                "trapped_chest_right",
                "base",
                "trapped_chest_right",
                "bottom",
            ),
            ("trapped_chest_right", "knob", "trapped_chest_right", "lock"),
            ("ender_chest", "base", "ender_chest", "bottom"),
            ("ender_chest", "knob", "ender_chest", "lock"),
            ("shulker_box", "base", "shulker_box", "base"),
            ("shulker_box", "lid", "shulker_box", "lid"),
            ("banner", "stand", "standing_banner", "pole"),
            ("banner", "top", "standing_banner", "bar"),
            ("banner", "top", "wall_banner", "bar"),
            ("banner", "slate", "standing_banner_flag", "flag"),
            ("banner", "slate", "wall_banner_flag", "flag"),
            ("head_skeleton", "head", "skeleton_skull", "head"),
            (
                "head_wither_skeleton",
                "head",
                "wither_skeleton_skull",
                "head",
            ),
            ("head_zombie", "head", "zombie_head", "head"),
            ("head_creeper", "head", "creeper_head", "head"),
            ("head_dragon", "head", "dragon_skull", "head"),
            ("head_dragon", "jaw", "dragon_skull", "jaw"),
            ("head_piglin", "head", "piglin_head", "head"),
            ("head_piglin", "left_ear", "piglin_head", "left_ear"),
            ("head_piglin", "right_ear", "piglin_head", "right_ear"),
            ("head_player", "head", "player_head", "head"),
            ("bell", "body", "bell", "bell_body"),
            ("bell", "base", "bell", "bell_base"),
            ("enchanting_book", "cover_right", "book", "right_lid"),
            ("enchanting_book", "cover_left", "book", "left_lid"),
            ("enchanting_book", "pages_right", "book", "right_pages"),
            ("enchanting_book", "pages_left", "book", "left_pages"),
            (
                "enchanting_book",
                "flipping_page_right",
                "book",
                "flip_page1",
            ),
            (
                "enchanting_book",
                "flipping_page_left",
                "book",
                "flip_page2",
            ),
            ("enchanting_book", "book_spine", "book", "seam"),
            ("lectern_book", "cover_right", "lectern_book", "right_lid"),
            ("lectern_book", "pages_left", "lectern_book", "left_pages"),
            (
                "lectern_book",
                "flipping_page_right",
                "lectern_book",
                "flip_page1",
            ),
            ("lectern_book", "book_spine", "lectern_book", "seam"),
            ("conduit", "base", "conduit_shell", "shell"),
            ("conduit", "cage", "conduit_cage", "shell"),
            ("conduit", "wind", "conduit_wind", "wind"),
            ("conduit", "eye", "conduit_eye", "eye"),
            ("decorated_pot", "neck", "decorated_pot_base", "neck"),
            ("decorated_pot", "top", "decorated_pot_base", "top"),
            ("decorated_pot", "bottom", "decorated_pot_base", "bottom"),
            ("decorated_pot", "front", "decorated_pot_sides", "front"),
            ("decorated_pot", "back", "decorated_pot_sides", "back"),
            ("decorated_pot", "left", "decorated_pot_sides", "left"),
            ("decorated_pot", "right", "decorated_pot_sides", "right"),
        ];
        // One file per name with each of its parts, and a stub of each model holding its parts
        // (chest models all of theirs, so a misplaced entry shows).
        let mut files: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        let mut trees: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for (file, part, key, vanilla) in cases {
            files.entry(file).or_default().insert(part);
            trees.entry(key).or_default().insert(vanilla);
        }
        for key in ["chest", "double_chest_left", "double_chest_right"] {
            trees
                .entry(key)
                .or_default()
                .extend(["bottom", "lid", "lock"]);
        }
        let files_by_name = files.clone();
        let files: Vec<_> = files
            .iter()
            .map(|(file, parts)| {
                let parts: Vec<_> = parts.iter().copied().collect();
                (format!("optifine/cem/{file}.jem"), jem_sized(&parts))
            })
            .collect();
        let files: Vec<_> = files.iter().map(|(p, t)| (p.as_str(), t.clone())).collect();
        let root = pack("parts", &files);
        let vanilla: HashMap<&'static str, ModelPart> = trees
            .iter()
            .map(|(key, parts)| {
                let parts = parts.iter().map(|p| (*p, plane(vec![]))).collect();
                (*key, plane(parts))
            })
            .collect();
        let result = reshape_all(&root, &vanilla);
        fs::remove_dir_all(&root).unwrap();
        let ((models, _), unused) = result.unwrap();
        for (file, part, key, vanilla) in cases {
            assert!(
                replaced(&models[key], vanilla),
                "{file}.jem {part}: {key} {vanilla}"
            );
            // The box is this entry's own: swapped names (left and right, body and base) show.
            let own = files_by_name[file].iter().position(|p| *p == part).unwrap() + 1;
            assert_eq!(
                depth(&models[key], vanilla),
                own as f32,
                "{file}.jem {part}: {key} {vanilla}"
            );
        }
        assert_eq!(unused, Vec::new());
    }

    #[test]
    fn a_chest_jem_reshapes_the_chest_alone_and_warns_of_entries_without_a_part() {
        let root = pack(
            "chest",
            &[(
                "optifine/cem/chest.jem",
                format!(
                    r#"{{"texture": "textures/entity/chest/fancy.png", "models": [
                        {{"part": "lid", {BOX}}},
                        {{"part": "knob", "attach": true, "scale": 2, {BOX}}},
                        {{"part": "bottom", {BOX}}},
                        {{"part": "tail", {BOX}}}]}}"#
                ),
            )],
        );
        let vanilla: HashMap<_, _> = [("chest", chest()), ("double_chest_left", chest())].into();
        let result = reshape_all(&root, &vanilla);
        fs::remove_dir_all(&root).unwrap();
        let ((models, textures), unused) = result.unwrap();
        let chest = &models["chest"];
        // Replaced: the lid's own face goes and the custom box (6 faces) joins it.
        assert!(replaced(chest, "lid"));
        // `bottom` is no OptiFine name but vanilla's own: it reaches the part as well.
        assert!(replaced(chest, "bottom"));
        // Attached: the lock keeps its face beside the scaled box.
        let lock = chest.child("lock").unwrap();
        assert_eq!((lock.polygons.len(), lock.children.len()), (1, 1));
        assert_eq!(lock.children[0].1.pose[6..], [2.0, 2.0, 2.0]);
        // `tail` names no chest part: the one warning.
        assert_eq!(
            unused,
            [(
                "assets/minecraft/optifine/cem/chest.jem".to_owned(),
                "tail".to_owned()
            )]
        );
        // The double chest has no file, nor do trapped and ender chests: vanilla's own, and
        // only the chest takes the texture.
        for key in ["double_chest_left", "trapped_chest", "ender_chest"] {
            let lid = models[key].child("lid").unwrap();
            assert_eq!((lid.polygons.len(), lid.children.len()), (1, 0), "{key}");
        }
        assert_eq!(
            textures,
            HashMap::from([("chest", "entity/chest/fancy".to_owned())])
        );
    }

    #[test]
    fn banners_split_one_file_and_books_fall_back() {
        let root = pack(
            "banner",
            &[
                ("optifine/cem/banner.jem", jem(&["stand", "top", "slate"])),
                ("emf/cem/enchanting_book.jem", jem(&["cover_left"])),
            ],
        );
        let banner = plane(vec![("pole", plane(vec![])), ("bar", plane(vec![]))]);
        let wall = plane(vec![("bar", plane(vec![]))]);
        let flag = plane(vec![("flag", plane(vec![]))]);
        let book = plane(vec![("left_lid", plane(vec![]))]);
        let vanilla: HashMap<_, _> = [
            ("standing_banner", banner),
            ("wall_banner", wall),
            ("wall_banner_flag", flag),
            ("book", book),
        ]
        .into();
        let result = reshape_all(&root, &vanilla);
        fs::remove_dir_all(&root).unwrap();
        let ((models, textures), unused) = result.unwrap();
        assert!(replaced(&models["standing_banner"], "pole"));
        assert!(replaced(&models["standing_banner"], "bar"));
        assert!(replaced(&models["wall_banner"], "bar"));
        assert!(replaced(&models["wall_banner_flag"], "flag"));
        assert!(replaced(&models["book"], "left_lid"));
        // The lectern's book has no file of its own nor `book.jem`: vanilla's.
        let lectern = models["lectern_book"].child("left_lid").unwrap();
        assert_eq!((lectern.polygons.len(), lectern.children.len()), (1, 0));
        assert!(textures.is_empty());
        // Each entry found its part in one of the models: no warning.
        assert_eq!(unused, Vec::new());
    }

    #[test]
    fn book_jem_reshapes_the_lectern_book_without_its_own_file() {
        let root = pack("book", &[("optifine/cem/book.jem", jem(&["cover_left"]))]);
        let vanilla: HashMap<_, _> = [("book", plane(vec![("left_lid", plane(vec![]))]))].into();
        let result = reshape(&root, &vanilla);
        fs::remove_dir_all(&root).unwrap();
        let (models, _) = result.unwrap();
        assert!(replaced(&models["book"], "left_lid"));
        assert!(replaced(&models["lectern_book"], "left_lid"));
    }

    #[test]
    fn a_shared_file_paints_only_the_models_it_reshapes() {
        // The conduit's file reshapes its eye alone: the shell, cage and wind keep their
        // vanilla geometry and so their own textures.
        let file = format!(
            r#"{{"texture": "textures/entity/conduit/custom.png", "models": [{{"part": "eye", {BOX}}}]}}"#
        );
        let root = pack("conduit", &[("optifine/cem/conduit.jem", file)]);
        let vanilla: HashMap<_, _> = [
            "conduit_shell",
            "conduit_cage",
            "conduit_wind",
            "conduit_eye",
        ]
        .map(|key| {
            let part = if key == "conduit_eye" { "eye" } else { "shell" };
            (key, plane(vec![(part, plane(vec![]))]))
        })
        .into();
        let result = reshape(&root, &vanilla);
        fs::remove_dir_all(&root).unwrap();
        let (models, textures) = result.unwrap();
        assert!(replaced(&models["conduit_eye"], "eye"));
        let painted: BTreeSet<_> = textures.keys().copied().collect();
        assert_eq!(painted, BTreeSet::from(["conduit_eye"]));
        assert_eq!(textures["conduit_eye"], "entity/conduit/custom");
    }
}
