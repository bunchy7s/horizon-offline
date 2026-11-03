//! `.jem` entity models and the `.jpm` part models they load, baked into the geometry vanilla's
//! `ModelPart` draws: quads in model pixels on vanilla's axes (y down), UVs a fraction of the
//! texture.
//!
//! The files place parts the way Blockbench's OptiFine export writes them, and the way EMF (and
//! OptiFine) build them: `invertAxis` names the axes that point the other way from vanilla's
//! (`"xy"` in practice), and **every** bone — a part or a submodel — sits at its own
//! `translate` (negated on the inverted axes), its boxes at the coordinates the file gives
//! (mirrored on them), its submodels below it at their own translates. A part keeps the pivot
//! of the vanilla part it replaces (OptiDocs, "Limitations: pivot points"): its bone rides as
//! a child of the vanilla part, `translate` away from its pivot, and an export that writes the
//! part's boxes where the model has them (the ground at y 0) has `translate` at minus the
//! part's pivot, so they land back around it.
//!
//! The bones' poses are what the pack's animations write: `sub.ty` replaces the pose the bone
//! starts at, which is its `translate` alone — never `translate` plus whatever its parent
//! carries. Folding a parent's translate into a child's pivot places the geometry the same at
//! rest, but every animation write then moves it by the folded amount: Fresh Animations parks
//! a cow's `head2` at `ty = -20` under a `body` part whose translate is 14, and the head would
//! fly 14 pixels off the body the moment the pack's first frame runs. EMF's
//! `EMFModelPartCustom` (the part's x, y, z are its `translate`) with `EMFPartData.prepare`
//! (the translate negated per inverted axis, the rotate negated about its own inverted axis
//! only, `scale` per bone, `attachments` offsets flipped) is the reference.

use super::json::{Json, get};
use crate::paths;
use std::{collections::HashMap, rc::Rc};

/// One face of a box, as `horizon_client::mobs::Polygon` holds it: `[x, y, z, u, v]` vertices
/// in model pixels, UV a fraction of the texture, and the outward normal.
#[derive(Clone, Debug, PartialEq)]
pub struct Quad {
    pub normal: [f32; 3],
    pub vertices: [[f32; 5]; 4],
}

/// A baked part model or submodel: a `ModelPart` with its boxes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bone {
    pub id: Option<String>,
    /// `ModelPart` x, y, z: the pivot in the parent's frame, vanilla pixels. A part's bone
    /// hangs at its `translate` below the vanilla part's own pivot; a submodel's at its own
    /// `translate` below its parent bone.
    pub pivot: [f32; 3],
    /// `ModelPart` xRot, yRot, zRot in radians, on vanilla's axes.
    pub rotation: [f32; 3],
    /// `ModelPart` xScale, yScale, zScale: `scale`, a number for all three axes or an array of
    /// three (EMF's own field is a number; an array is wider than it, not narrower).
    pub scale: [f32; 3],
    /// The `attachments` this declaring bone carries: an item anchor's name and its offset,
    /// flipped on this bone's inverted axes (EMF's `EMFPartData.getAttachments`).
    pub attachments: Vec<(String, [f32; 3])>,
    pub quads: Vec<Quad>,
    pub children: Vec<Bone>,
}

/// One entry of a `.jem`'s `models`.
#[derive(Clone, Debug, PartialEq)]
pub struct PartModel {
    /// The OptiFine part name it attaches to (`head`, `leg1`...).
    pub part: String,
    /// True: drawn beside the vanilla part's own boxes; false: in place of them.
    pub attach: bool,
    /// `scale`: the render scale of the part.
    pub scale: f32,
    /// `animations`: target and expression, in file order (later ones see earlier results).
    pub animations: Vec<(String, String)>,
    pub bone: Bone,
}

/// A parsed `.jem`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntityModel {
    /// The texture every part draws with; None: the vanilla texture.
    pub texture: Option<String>,
    pub parts: Vec<PartModel>,
    /// What was left out and why: entries whose `.jpm` is missing or broken, a `baseId` naming
    /// no model, what is past the limits, per-part textures and sprites (not drawn yet).
    pub warnings: Vec<String>,
}

/// OptiFine's texture size when a model gives none.
const DEFAULT_TEXTURE_SIZE: [f32; 2] = [64.0, 32.0];
/// What one `.jem` may make the game build, so a pack can't make loading run out of memory or
/// time: its entries, the work of baking them (see [`Bake`]), and the bytes of the `.jem` and
/// every `.jpm` it loads, together.
const MAX_ENTRIES: usize = 1024;
const MAX_NODES: usize = 1 << 16;
/// The bytes a `.jem` and its `.jpm` files may hold together: [`parse_jem`]'s `read` should
/// refuse a larger file without reading it.
pub const MAX_BYTES: usize = 4 << 20;

/// Keys looked up in a part definition.
trait Def {
    fn key(&self, key: &str) -> Option<&Json>;
}
impl Def for [(String, Json)] {
    fn key(&self, key: &str) -> Option<&Json> {
        get(self, key)
    }
}
/// An object's pairs and, for each key, the position of its last pair.
struct Layer {
    pairs: Vec<(String, Json)>,
    index: HashMap<String, usize>,
}
impl Layer {
    fn new(pairs: Vec<(String, Json)>) -> Rc<Layer> {
        let index = pairs.iter().enumerate().map(|(i, (k, _))| (k.clone(), i));
        Rc::new(Layer {
            index: index.collect(),
            pairs,
        })
    }
}
/// An entry's keys: its own, then its `.jpm`'s, then its `baseId` entry's, each layer over the
/// next. Layers are shared, so inheriting a big model copies nothing.
#[derive(Clone, Default)]
struct Layers(Vec<Rc<Layer>>);
impl Def for Layers {
    fn key(&self, key: &str) -> Option<&Json> {
        self.0
            .iter()
            .find_map(|layer| Some(&layer.pairs[*layer.index.get(key)?].1))
    }
}

/// What a file may still build: `left` units of work. Entries can share a `.jpm` or a base, so
/// what is charged is what each one walks and copies, not what the files hold: a bone costs
/// one, a submodel also its keys (each lookup walks them), a part also its layers (each lookup
/// tries them); a box its faces or its keys, whichever is more (at least one); any other item
/// of `boxes` or `submodels` one; an item of `animations` its pairs (at least one), and a pair
/// kept one more for a number or boolean. On top of that every string read or copied out of the
/// files (part names, ids, animation targets and expressions, axes, numbers written as text)
/// costs one per started 64 bytes, each time. `cut`: something was left out for want of them;
/// `unread`: a part or submodel has a texture or sprites of its own.
struct Bake {
    left: usize,
    cut: bool,
    unread: bool,
}
impl Bake {
    /// Takes `cost`; when less is left, takes the rest, notes the cut and says false.
    fn spend(&mut self, cost: usize) -> bool {
        let Some(left) = self.left.checked_sub(cost) else {
            (self.left, self.cut) = (0, true);
            return false;
        };
        self.left = left;
        true
    }
}

/// What reading or copying a string of the files costs.
fn text_cost(s: &str) -> usize {
    s.len().div_ceil(64)
}

/// A string value, paid for; None also when it doesn't fit what is left.
fn string<'a>(v: Option<&'a Json>, bake: &mut Bake) -> Option<&'a str> {
    v?.as_str().filter(|s| bake.spend(text_cost(s)))
}

/// Parses the `.jem` at resource path `path` (`assets/minecraft/optifine/cem/zombie.jem`).
/// `read` returns the text of a resource path (the `.jpm` files `model` names), None when missing;
/// it should return None for a file larger than [`MAX_BYTES`] rather than read it, as the `.jem`
/// and the `.jpm` files it loads may not pass that together. An error means the file as a whole
/// can't be read; a broken entry only adds a warning.
pub fn parse_jem(
    path: &str,
    text: &str,
    read: &mut dyn FnMut(&str) -> Option<String>,
) -> Result<EntityModel, String> {
    if text.len() > MAX_BYTES {
        return Err(format!("{path}: larger than {MAX_BYTES} bytes"));
    }
    let root = Json::parse(text).map_err(|e| format!("{path}: {e}"))?;
    let mut bake = Bake {
        left: MAX_NODES,
        cut: false,
        unread: false,
    };
    let size = size2(root.get("textureSize"), &mut bake).unwrap_or(DEFAULT_TEXTURE_SIZE);
    let texture = root
        .get("texture")
        .and_then(Json::as_str)
        .and_then(|t| texture(path, t));
    let mut out = EntityModel {
        texture,
        ..EntityModel::default()
    };
    let mut jpms: HashMap<String, Result<Rc<Layer>, String>> = HashMap::new();
    let mut bytes = text.len();
    // The first entry with an id is the one `baseId` finds.
    let mut by_id: HashMap<String, Layers> = HashMap::new();
    for (n, entry) in objects(root.get("models")).enumerate() {
        if n == MAX_ENTRIES {
            out.warnings
                .push(format!("{path}: more than {MAX_ENTRIES} models"));
            break;
        }
        let mut layers = Layers(vec![Layer::new(entry.to_vec())]);
        // `model`: the part model in a `.jpm`, under this entry's own keys.
        if let Some(model) = get(entry, "model") {
            let jpm = model
                .as_str()
                .ok_or_else(|| format!("{path}: a model that isn't a file name"))
                .and_then(|m| jpm(path, m, read, &mut jpms, &mut bytes));
            match jpm {
                Ok(jpm) => layers.0.push(jpm),
                Err(e) => {
                    out.warnings.push(e);
                    continue;
                }
            }
        }
        // `baseId`: the named entry's properties, under this one's and its `.jpm`'s.
        if let Some(base) = get(entry, "baseId").and_then(Json::as_str) {
            match by_id.get(base) {
                // A base sharing this entry's `.jpm` adds it once: of this entry's layers only
                // that one can be among the base's.
                Some(parent) => {
                    let own = layers.0.get(1).cloned();
                    let shared = |l: &&Rc<Layer>| own.as_ref().is_some_and(|o| Rc::ptr_eq(o, l));
                    layers
                        .0
                        .extend(parent.0.iter().filter(|l| !shared(l)).cloned());
                }
                None => out
                    .warnings
                    .push(format!("{path}: no model {base} for baseId")),
            }
        }
        // An id is hashed, maybe from a file other entries share: it is paid for each time.
        if let Some(id) = string(layers.key("id"), &mut bake)
            && !by_id.contains_key(id)
        {
            by_id.insert(id.to_owned(), layers.clone());
        }
        let Some(part) = layers.key("part").and_then(Json::as_str) else {
            continue;
        };
        // Past the limits a part is left out, not emptied or frozen: the vanilla part stays. Its
        // name, the layers each key is looked up through, its animations, all of them, and its
        // bone must fit before it is baked.
        let earlier = std::mem::replace(&mut bake.cut, false);
        let attach = boolean(layers.key("attach"), &mut bake).unwrap_or(false);
        let scale = num(layers.key("scale"), &mut bake).unwrap_or(1.0);
        if !bake.spend(text_cost(part) + layers.0.len()) {
            continue;
        }
        let Some((animations, cost)) = animations(layers.key("animations"), bake.left) else {
            bake.cut = true;
            continue;
        };
        bake.left -= cost;
        let bone = bone(&layers, size, &mut bake);
        let emptied = bake.cut && !has_quads(&bone);
        bake.cut |= earlier;
        if emptied {
            continue;
        }
        out.parts.push(PartModel {
            part: part.to_owned(),
            attach,
            scale,
            animations,
            bone,
        });
    }
    if bake.cut {
        out.warnings.push(format!(
            "{path}: more than {MAX_NODES} bones, boxes, faces and animations, the rest left out"
        ));
    }
    if bake.unread {
        out.warnings.push(format!(
            "{path}: per-part textures and sprites aren't drawn yet"
        ));
    }
    Ok(out)
}

/// The `.jpm` that `model` (in the `.jem` at `path`) names, read and parsed once per file.
/// `bytes`: what the model's files read so far hold, refused ones included.
fn jpm(
    path: &str,
    model: &str,
    read: &mut dyn FnMut(&str) -> Option<String>,
    cache: &mut HashMap<String, Result<Rc<Layer>, String>>,
    bytes: &mut usize,
) -> Result<Rc<Layer>, String> {
    let Some(file) = paths::resolve(path, model) else {
        return Err(format!("{path}: {model} is outside the pack"));
    };
    cache
        .entry(file.clone())
        .or_insert_with(|| {
            let text = read(&file).ok_or_else(|| format!("{path}: missing {file}"))?;
            *bytes = bytes.saturating_add(text.len());
            if *bytes > MAX_BYTES {
                return Err(format!(
                    "{file}: the model's files are larger than {MAX_BYTES} bytes together"
                ));
            }
            match Json::parse(&text).map_err(|e| format!("{file}: {e}"))? {
                Json::Obj(pairs) => Ok(Layer::new(pairs)),
                _ => Err(format!("{file}: not an object")),
            }
        })
        .clone()
}

/// An entry's animations and what they cost, None when they don't fit in `left` with one to
/// spare for the bone. Every pair costs one, skipped or not, and every other item one; a pair
/// kept also its target and expression, paid before they are copied.
fn animations(v: Option<&Json>, left: usize) -> Option<(Vec<(String, String)>, usize)> {
    let mut out = Vec::new();
    let mut cost = 0;
    for item in items(v) {
        let pairs = item.as_object().unwrap_or_default();
        cost += pairs.len().max(1);
        if cost >= left {
            return None;
        }
        for (k, v) in pairs {
            cost += text_cost(k)
                + match v {
                    Json::Str(s) => text_cost(s),
                    Json::Num(_) | Json::Bool(_) => 1,
                    _ => continue,
                };
            if cost >= left {
                return None;
            }
            let expr = match v {
                Json::Str(s) => s.clone(),
                Json::Num(n) => n.to_string(),
                Json::Bool(b) => b.to_string(),
                _ => continue,
            };
            out.push((k.clone(), expr));
        }
    }
    (cost < left).then_some((out, cost))
}

fn has_quads(bone: &Bone) -> bool {
    !bone.quads.is_empty() || bone.children.iter().any(has_quads)
}

/// The items of an array value.
fn items(v: Option<&Json>) -> impl Iterator<Item = &Json> {
    v.and_then(Json::as_array).into_iter().flatten()
}

/// The objects of an array value.
fn objects(v: Option<&Json>) -> impl Iterator<Item = &[(String, Json)]> {
    items(v).filter_map(Json::as_object)
}

/// A texture value as a resource path with its `.png`; None for one outside the pack.
fn texture(path: &str, value: &str) -> Option<String> {
    let resolved = paths::resolve(path, value)?;
    Some(if resolved.ends_with(".png") {
        resolved
    } else {
        resolved + ".png"
    })
}

/// A finite number, also when written as a string (paid for).
fn num(v: Option<&Json>, bake: &mut Bake) -> Option<f32> {
    let n = match v? {
        Json::Num(n) => *n as f32,
        Json::Str(_) => string(v, bake)?.trim().parse().ok()?,
        _ => return None,
    };
    n.is_finite().then_some(n)
}

fn boolean(v: Option<&Json>, bake: &mut Bake) -> Option<bool> {
    match v? {
        Json::Bool(b) => Some(*b),
        Json::Str(_) => string(v, bake)?.trim().parse().ok(),
        _ => None,
    }
}

fn nums<const N: usize>(v: Option<&Json>, bake: &mut Bake) -> Option<[f32; N]> {
    let a = v?.as_array()?;
    (a.len() >= N).then(|| std::array::from_fn(|i| num(a.get(i), bake).unwrap_or(0.0)))
}

fn size2(v: Option<&Json>, bake: &mut Bake) -> Option<[f32; 2]> {
    nums::<2>(v, bake).filter(|s| s[0] > 0.0 && s[1] > 0.0)
}

/// Bakes a part model definition, a part or a submodel the same way: the bone sits at its own
/// `translate` (negated on the inverted axes), its boxes at the file's coordinates mirrored on
/// them, its submodels at their own translates below it — the poses the pack's animation
/// writes replace. The bone costs one of `bake.left`, which the caller makes sure is left, as
/// it pays for the walk of its keys; its strings, boxes and submodels what [`Bake`] says.
fn bone(def: &(impl Def + ?Sized), size: [f32; 2], bake: &mut Bake) -> Bone {
    bake.left -= 1;
    bake.unread |= def.key("texture").is_some() || def.key("sprites").is_some();
    let axes = string(def.key("invertAxis"), bake).unwrap_or("");
    let invert = [axes.contains('x'), axes.contains('y'), axes.contains('z')];
    let sign = |i: usize| if invert[i] { -1.0 } else { 1.0 };
    let size = size2(def.key("textureSize"), bake).unwrap_or(size);
    let mirror = string(def.key("mirrorTexture"), bake).unwrap_or("");
    let translate = nums::<3>(def.key("translate"), bake).unwrap_or_default();
    let translate: [f32; 3] = std::array::from_fn(|i| sign(i) * translate[i]);
    // A rotation seen through flipped axes: about a flipped axis it turns the other way, and
    // that is all — EMF negates per axis, and an odd number of flips is not a mirror to turn
    // every rotation around.
    let rotate = nums::<3>(def.key("rotate"), bake).unwrap_or_default();
    // `scale`: a number for all three axes, or an array of three.
    let scale = match def.key("scale") {
        Some(Json::Num(s)) => {
            let s = *s as f32;
            [if s.is_finite() { s } else { 1.0 }; 3]
        }
        Some(Json::Arr(a)) if a.len() >= 3 => {
            std::array::from_fn(|i| num(a.get(i), bake).unwrap_or(1.0))
        }
        _ => [1.0; 3],
    };
    // `attachments`: the item anchors this declaring bone carries (`right_handheld_item`,
    // `fox_item`...), each offset flipped on the inverted axes, as EMF's part data does —
    // the file's frame is y up, the baked tree's y down.
    let attachments = def
        .key("attachments")
        .and_then(Json::as_object)
        .map(|pairs| {
            pairs
                .iter()
                .filter_map(|(name, v)| {
                    let at = nums::<3>(Some(v), bake)?;
                    Some((name.clone(), std::array::from_fn(|i| sign(i) * at[i])))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut out = Bone {
        id: string(def.key("id"), bake).map(str::to_owned),
        pivot: translate,
        rotation: std::array::from_fn(|i| sign(i) * rotate[i].to_radians()),
        scale,
        attachments,
        ..Bone::default()
    };
    let shape = Shape {
        invert,
        mirror_u: mirror.contains('u'),
        mirror_v: mirror.contains('v'),
        size,
    };
    for b in items(def.key("boxes")) {
        if bake.left == 0 {
            bake.cut = true;
            break;
        }
        let Some(b) = b.as_object() else {
            bake.left -= 1;
            continue;
        };
        // Baking a box reads through its keys: it costs them, so junk keys can't be read again
        // by every entry that inherits the box.
        if b.len() > bake.left {
            bake.left = 0;
            bake.cut = true;
            break;
        }
        // Its strings are paid as they are read; one that doesn't fit leaves nothing for faces.
        let before = out.quads.len();
        shape.bake(b, &mut out.quads, bake);
        let made = out.quads.len() - before;
        if made > bake.left {
            out.quads.truncate(before + bake.left);
            bake.cut = true;
        }
        bake.left = bake.left.saturating_sub(made.max(b.len()).max(1));
    }
    let one = def.key("submodel").filter(|s| s.as_object().is_some());
    for sub in one.into_iter().chain(items(def.key("submodels"))) {
        if bake.left == 0 {
            bake.cut = true;
            break;
        }
        let Some(sub) = sub.as_object() else {
            bake.left -= 1;
            continue;
        };
        // Its keys are looked up one by one: it costs them, with one left for its bone.
        if sub.len() >= bake.left {
            bake.left = 0;
            bake.cut = true;
            break;
        }
        bake.left -= sub.len();
        out.children.push(bone(sub, size, bake));
    }
    out
}

struct Shape {
    invert: [bool; 3],
    mirror_u: bool,
    mirror_v: bool,
    size: [f32; 2],
}

/// A box's faces in `ModelPart.Cube`'s order of corners, keyed by the per-face UV name for the
/// files' frame (up is vanilla's -y, north -z). `uvWest` is the min-x face and `uvEast` the
/// max-x one, as in `ModelPart.Cube` and EMF (whose west face takes the middle row's first
/// strip, its east the third): packs name the sides by the model's own frame, and Fresh
/// Animations draws 54 models' side art through these two keys.
const FACES: [(&str, &str, [usize; 4], [f32; 3]); 6] = [
    ("uvUp", "", [5, 4, 0, 1], [0.0, -1.0, 0.0]),
    ("uvDown", "", [2, 3, 7, 6], [0.0, 1.0, 0.0]),
    ("uvWest", "uvLeft", [0, 4, 7, 3], [-1.0, 0.0, 0.0]),
    ("uvNorth", "uvFront", [1, 0, 3, 2], [0.0, 0.0, -1.0]),
    ("uvEast", "uvRight", [5, 1, 2, 6], [1.0, 0.0, 0.0]),
    ("uvSouth", "uvBack", [4, 5, 6, 7], [0.0, 0.0, 1.0]),
];
/// `ModelPart.Cube`'s corners: which of x, y, z take the box's max.
const CORNER: [[bool; 3]; 8] = [
    [false, false, false],
    [true, false, false],
    [true, true, false],
    [false, true, false],
    [false, false, true],
    [true, false, true],
    [true, true, true],
    [false, true, true],
];

impl Shape {
    /// Adds the box's faces to `out`; none when its corners or UVs aren't finite.
    fn bake(&self, b: &[(String, Json)], out: &mut Vec<Quad>, bake: &mut Bake) {
        let start = out.len();
        let Some(c) = nums::<6>(get(b, "coordinates"), bake) else {
            return;
        };
        // `sizeAddX/Y/Z` over `sizesAdd` over `sizeAdd`.
        let all = num(get(b, "sizeAdd"), bake).unwrap_or(0.0);
        let mut grow = nums::<3>(get(b, "sizesAdd"), bake).unwrap_or([all; 3]);
        for (i, key) in ["sizeAddX", "sizeAddY", "sizeAddZ"].iter().enumerate() {
            if let Some(g) = num(get(b, key), bake) {
                grow[i] = g;
            }
        }
        let dims = [c[3], c[4], c[5]];
        let mut lo = [0.0; 3];
        let mut hi = [0.0; 3];
        for i in 0..3 {
            let min = if self.invert[i] {
                -(c[i] + dims[i])
            } else {
                c[i]
            };
            lo[i] = min - grow[i];
            hi[i] = min + dims[i] + grow[i];
        }
        let boxed = nums::<2>(get(b, "textureOffset"), bake);
        // A box-UV box mirrored in u is `ModelPart.Cube` with `mirror`: its x sides trade
        // places, and each face winds the other way to keep facing out. `mirrorTexture` is
        // ignored on a box that names its own face rects — EMF hands its custom-UV cube no
        // mirror flags ("it seems optifine ignores these flags on custom uv's") — so Fresh
        // Animations' flat eye quads on mirrored parts stay where the pack drew them.
        let box_mirror = boxed.is_some() && self.mirror_u;
        let box_mirror_v = boxed.is_some() && self.mirror_v;
        if box_mirror {
            std::mem::swap(&mut lo[0], &mut hi[0]);
        }
        let corner = |i: usize| -> [f32; 3] {
            std::array::from_fn(|a| if CORNER[i][a] { hi[a] } else { lo[a] })
        };
        let rects = match boxed {
            Some(t) => box_rects(t, dims).map(Some),
            None => FACES.map(|(key, alias, ..)| {
                nums::<4>(get(b, key).or_else(|| get(b, alias)), bake)
                    .filter(|r| r[0] != r[2] || r[1] != r[3])
            }),
        };
        for ((_, _, corners, normal), rect) in FACES.iter().zip(rects) {
            let Some(mut r) = rect else {
                continue;
            };
            if box_mirror_v {
                r.swap(1, 3);
            }
            let [u1, v1, u2, v2] = r;
            let uv = [(u2, v1), (u1, v1), (u1, v2), (u2, v2)];
            let mut vertices: [[f32; 5]; 4] = std::array::from_fn(|k| {
                let [x, y, z] = corner(corners[k]);
                [x, y, z, uv[k].0 / self.size[0], uv[k].1 / self.size[1]]
            });
            if vertices.iter().flatten().any(|x| !x.is_finite()) {
                out.truncate(start);
                return;
            }
            let mut normal = *normal;
            if box_mirror {
                vertices.reverse();
                normal[0] = -normal[0];
            }
            out.push(Quad { normal, vertices });
        }
    }
}

/// `ModelPart.Cube`'s box UV: the faces' `[u1, v1, u2, v2]` around `textureOffset`, in
/// `FACES` order (top strip: up then down; then east, north, west, south).
fn box_rects([u, v]: [f32; 2], [w, h, d]: [f32; 3]) -> [[f32; 4]; 6] {
    let (u1, u2, u3, u4, u5) = (
        u + d,
        u + d + w,
        u + d + w + w,
        u + d + w + d,
        u + 2.0 * (d + w),
    );
    let (v1, v2) = (v + d, v + d + h);
    [
        [u1, v, u2, v1],
        [u2, v1, u3, v],
        [u, v1, u1, v2],
        [u1, v1, u2, v2],
        [u2, v1, u4, v2],
        [u4, v1, u5, v2],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const JEM: &str = "assets/minecraft/optifine/cem/zombie.jem";

    fn parse(text: &str) -> EntityModel {
        parse_jem(JEM, text, &mut |_| None).unwrap()
    }
    fn face(bone: &Bone, normal: [f32; 3]) -> &Quad {
        bone.quads.iter().find(|q| q.normal == normal).unwrap()
    }
    fn positions(q: &Quad) -> [[f32; 3]; 4] {
        q.vertices.map(|v| [v[0], v[1], v[2]])
    }

    // The zombie's head and right leg as an export writes them: vanilla's head has its pivot at
    // 0 and a box from (-4, -8, -4); the right leg its pivot at (-1.9, 12, 0) and a box from
    // (-2, 0, -2), 4 x 12 x 4.
    const ZOMBIE: &str = r#"{
        "textureSize": [64, 64],
        "models": [
            {"part": "head", "id": "head", "invertAxis": "xy", "translate": [0, -24, 0],
             "boxes": [{"coordinates": [-4, 24, -4, 8, 8, 8], "textureOffset": [0, 0]}]},
            {"part": "right_leg", "id": "right_leg", "invertAxis": "xy", "translate": [-1.9, -12, 0],
             "boxes": [{"coordinates": [-0.1, 0, -2, 4, 12, 4], "textureOffset": [0, 16]}]}
        ]
    }"#;

    #[test]
    fn parts_land_where_vanilla_has_them_relative_to_their_pivots() {
        let model = parse(ZOMBIE);
        // The bone hangs at its translate (minus the part's pivot, the file's y up against
        // vanilla's y down) and the box at the file's coordinates mirrored, the two landing
        // the geometry back around the vanilla pivot.
        let head = &model.parts[0].bone;
        assert_eq!(head.pivot, [0.0, 24.0, 0.0]);
        // The face, at the head's front (-z), with the skin's face square (8, 8)-(16, 16):
        // 8 pixels above the pivot, as vanilla has it.
        let north = face(head, [0.0, 0.0, -1.0]);
        let net = |q: &Quad| {
            q.vertices.map(|v| {
                [
                    v[0] + head.pivot[0],
                    v[1] + head.pivot[1],
                    v[2] + head.pivot[2],
                ]
            })
        };
        assert_eq!(
            net(north),
            [
                [4.0, -8.0, -4.0],
                [-4.0, -8.0, -4.0],
                [-4.0, 0.0, -4.0],
                [4.0, 0.0, -4.0]
            ]
        );
        let uv = north.vertices.map(|v| [v[3] * 64.0, v[4] * 64.0]);
        assert_eq!(uv, [[16.0, 8.0], [8.0, 8.0], [8.0, 16.0], [16.0, 16.0]]);
        // The leg's box spans x -2..2 and y 0..12 around the vanilla pivot: the bone at its
        // translate and the box mirrored below it, summed here as they draw.
        let leg = &model.parts[1].bone;
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for v in leg.quads.iter().flat_map(|q| q.vertices) {
            for a in 0..3 {
                lo[a] = lo[a].min(v[a] + leg.pivot[a]);
                hi[a] = hi[a].max(v[a] + leg.pivot[a]);
            }
        }
        let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5);
        assert!(
            close(lo, [-2.0, 0.0, -2.0]) && close(hi, [2.0, 12.0, 2.0]),
            "{lo:?} {hi:?}"
        );
        // Leg texture: the top at (4, 16)-(8, 20), the west side (vanilla +x) at (8, 20)-(12, 32).
        let uv = |q: &Quad| q.vertices.map(|v| [v[3] * 64.0, v[4] * 64.0]);
        assert_eq!(uv(face(leg, [0.0, -1.0, 0.0]))[1], [4.0, 16.0]);
        assert_eq!(uv(face(leg, [0.0, -1.0, 0.0]))[3], [8.0, 20.0]);
        assert_eq!(uv(face(leg, [1.0, 0.0, 0.0]))[1], [8.0, 20.0]);
        assert_eq!(uv(face(leg, [1.0, 0.0, 0.0]))[3], [12.0, 32.0]);
    }

    #[test]
    fn faces_wind_outward_mirrored_or_not() {
        for mirror in ["", "u"] {
            let model = parse(&format!(
                r#"{{"models": [{{"part": "body", "mirrorTexture": "{mirror}",
                    "boxes": [{{"coordinates": [1, 2, 3, 4, 5, 6], "textureOffset": [0, 0]}}]}}]}}"#
            ));
            let quads = &model.parts[0].bone.quads;
            assert_eq!(quads.len(), 6);
            let mut normals: Vec<[f32; 3]> = quads.iter().map(|q| q.normal).collect();
            normals.sort_by(|a, b| a.partial_cmp(b).unwrap());
            normals.dedup();
            assert_eq!(normals.len(), 6, "{mirror}: every side once");
            for q in quads {
                let [a, b, c, _] = positions(q);
                let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                let cross = [
                    e1[1] * e2[2] - e1[2] * e2[1],
                    e1[2] * e2[0] - e1[0] * e2[2],
                    e1[0] * e2[1] - e1[1] * e2[0],
                ];
                let dot: f32 = cross.iter().zip(q.normal).map(|(x, n)| x * n).sum();
                assert!(dot > 0.0, "{mirror}: {q:?}");
                // The face sits on the side its normal points to (box 1..5, 2..7, 3..9).
                let axis = q.normal.iter().position(|n| *n != 0.0).unwrap();
                let side = if q.normal[axis] > 0.0 {
                    [5.0, 7.0, 9.0]
                } else {
                    [1.0, 2.0, 3.0]
                };
                assert!(
                    q.vertices.iter().all(|v| v[axis] == side[axis]),
                    "{mirror}: {q:?}"
                );
            }
        }
    }

    #[test]
    fn mirrored_box_puts_the_first_strip_on_the_other_side() {
        let model = parse(
            r#"{"models": [{"part": "body", "mirrorTexture": "u",
                "boxes": [{"coordinates": [0, 0, 0, 2, 2, 2], "textureOffset": [0, 0]}]}]}"#,
        );
        // Unmirrored, (0, 2)-(2, 4) is the -x side; mirrored it is the +x side.
        let east = face(&model.parts[0].bone, [1.0, 0.0, 0.0]);
        let us: Vec<f32> = east.vertices.iter().map(|v| v[3] * 64.0).collect();
        assert!(us.iter().all(|u| (0.0..=2.0).contains(u)), "{us:?}");
    }

    #[test]
    fn submodels_pivot_in_their_parents_frame_and_rotate_on_vanilla_axes() {
        // A brim on the head pivoting at the head's pivot, and a feather 2 px above it.
        let model = parse(
            r#"{"models": [{"part": "head", "invertAxis": "xy", "translate": [0, -24, 0],
                "submodels": [{"id": "brim", "invertAxis": "xy", "translate": [0, 24, 0],
                    "rotate": [90, 0, 45],
                    "boxes": [{"coordinates": [-1, 0, -1, 2, 2, 2], "textureOffset": [0, 0]}],
                    "submodels": [{"id": "feather", "invertAxis": "xy", "translate": [0, 2, 0]}]}]}]}"#,
        );
        // The part's bone at its translate, the submodel's at its own, alone: what a
        // `brim.ty` write replaces, whatever the parent carries.
        assert_eq!(model.parts[0].bone.pivot, [0.0, 24.0, 0.0]);
        let brim = &model.parts[0].bone.children[0];
        assert_eq!(brim.id.as_deref(), Some("brim"));
        assert_eq!(brim.pivot, [0.0, -24.0, 0.0]);
        let r = std::f32::consts::FRAC_PI_2;
        assert_eq!(brim.rotation, [-r, 0.0, r / 2.0]);
        let ys: Vec<f32> = brim
            .quads
            .iter()
            .flat_map(|q| q.vertices.map(|v| v[1]))
            .collect();
        assert!(ys.iter().all(|y| (-2.0..=0.0).contains(y)), "{ys:?}");
        assert_eq!(brim.children[0].pivot, [0.0, -2.0, 0.0]);
    }

    fn uvs(q: &Quad, w: f32, h: f32) -> [[f32; 2]; 4] {
        q.vertices.map(|v| [v[3] * w, v[4] * h])
    }
    fn bounds(bone: &Bone) -> ([f32; 3], [f32; 3]) {
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for v in bone.quads.iter().flat_map(|q| q.vertices) {
            for a in 0..3 {
                lo[a] = lo[a].min(v[a]);
                hi[a] = hi[a].max(v[a]);
            }
        }
        (lo, hi)
    }

    #[test]
    fn each_inverted_axis_flips_position_and_rotation() {
        let part = |axes: &str| {
            parse(&format!(
                r#"{{"models": [{{"part": "body", "invertAxis": "{axes}",
                    "translate": [1, 2, 3], "rotate": [30, 60, 90],
                    "boxes": [{{"coordinates": [1, 2, 3, 1, 1, 1], "textureOffset": [0, 0]}}]}}]}}"#
            ))
            .parts
            .remove(0)
            .bone
        };
        // The geometry lands where it did: the bone at its translate, the box below it, the
        // translate never doubled in. The bones' poses are their own translates.
        let net = |b: &Bone| {
            let mut lo = [f32::MAX; 3];
            let mut hi = [f32::MIN; 3];
            for v in b.quads.iter().flat_map(|q| q.vertices) {
                for a in 0..3 {
                    lo[a] = lo[a].min(v[a] + b.pivot[a]);
                    hi[a] = hi[a].max(v[a] + b.pivot[a]);
                }
            }
            (lo, hi)
        };
        assert_eq!(net(&part("xyz")), ([-3.0, -5.0, -7.0], [-2.0, -4.0, -6.0]));
        assert_eq!(net(&part("")), ([2.0, 4.0, 6.0], [3.0, 5.0, 7.0]));
        assert_eq!(part("").pivot, [1.0, 2.0, 3.0]);
        assert_eq!(part("xyz").pivot, [-1.0, -2.0, -3.0]);
        // Degrees to radians; a flipped axis turns the other way, and only that axis: an odd
        // number of flips is not a mirror (EMF negates each axis on its own).
        let deg = |r: [f32; 3]| r.map(|a| (a.to_degrees() * 1000.0).round() / 1000.0);
        assert_eq!(deg(part("").rotation), [30.0, 60.0, 90.0]);
        assert_eq!(deg(part("xy").rotation), [-30.0, -60.0, 90.0]);
        assert_eq!(deg(part("xyz").rotation), [-30.0, -60.0, -90.0]);
        assert_eq!(deg(part("x").rotation), [-30.0, 60.0, 90.0]);
    }

    #[test]
    fn per_face_uvs_by_name_and_mirrored_boxes_keep_their_rects() {
        let face_of = |key: &str, mirror: &str| {
            let model = parse(&format!(
                r#"{{"models": [{{"part": "body", "textureSize": [16, 16], "mirrorTexture": "{mirror}",
                    "boxes": [{{"coordinates": [0, 0, 0, 2, 2, 2], "{key}": [0, 0, 2, 2]}}]}}]}}"#
            ));
            let quads = &model.parts[0].bone.quads;
            assert_eq!(quads.len(), 1, "{key}");
            quads[0].clone()
        };
        // `uvEast` is the max-x face and `uvWest` the min-x one, as `ModelPart.Cube` and
        // EMF name them: Fresh Animations draws its side art by these names.
        let east = face_of("uvEast", "");
        assert_eq!(east.normal, [1.0, 0.0, 0.0]);
        assert!(east.vertices.iter().all(|v| v[0] == 2.0));
        assert_eq!(
            uvs(&east, 16.0, 16.0),
            [[2.0, 0.0], [0.0, 0.0], [0.0, 2.0], [2.0, 2.0]]
        );
        // `uvRight` is `uvEast`'s other name; `mirrorTexture` is ignored on a box that
        // names its own face rects (EMF hands its custom-UV cube no mirror flags), so the
        // mirrored variant keeps the pack's rect.
        assert_eq!(
            uvs(&face_of("uvRight", "uv"), 16.0, 16.0),
            [[2.0, 0.0], [0.0, 0.0], [0.0, 2.0], [2.0, 2.0]]
        );
        let west = face_of("uvWest", "");
        assert_eq!(west.normal, [-1.0, 0.0, 0.0]);
        assert!(west.vertices.iter().all(|v| v[0] == 0.0));
    }

    #[test]
    fn size_adds_grow_positions_per_axis() {
        let model = parse(
            r#"{"models": [{"part": "body", "boxes": [{"coordinates": [0, 0, 0, 2, 2, 2],
                "textureOffset": [0, 0], "sizeAdd": 9, "sizesAdd": [1, 0, 0], "sizeAddY": 0.5}]}]}"#,
        );
        let bone = &model.parts[0].bone;
        assert_eq!(bounds(bone), ([-1.0, -0.5, 0.0], [3.0, 2.5, 2.0]));
        // The UVs keep the box's own size: the north face spans u 2..4 of the default 64 x 32.
        assert_eq!(
            uvs(face(bone, [0.0, 0.0, -1.0]), 64.0, 32.0),
            [[4.0, 2.0], [2.0, 2.0], [2.0, 4.0], [4.0, 4.0]]
        );
    }

    #[test]
    fn texture_size_reaches_a_single_submodel() {
        let model = parse(
            r#"{"models": [{"part": "head", "invertAxis": "xy", "translate": [0, -24, 0], "textureSize": [16, 16],
                "submodel": {"boxes": [{"coordinates": [0, 0, 0, 1, 1, 1], "uvUp": [0, 0, 1, 1]}]}}]}"#,
        );
        let child = &model.parts[0].bone.children[0];
        assert_eq!(child.pivot, [0.0, 0.0, 0.0]);
        let max_u = child.quads[0]
            .vertices
            .iter()
            .map(|v| v[3])
            .fold(0.0, f32::max);
        assert_eq!(max_u, 1.0 / 16.0);
    }

    #[test]
    fn animations_keep_file_order() {
        let model = parse(
            r#"{"models": [{"part": "head", "attach": "true",
                "animations": [{"var.a": "1", "head.rx": "var.a"}, {"this.tx": 0}]}]}"#,
        );
        let head = &model.parts[0];
        assert!(head.attach);
        assert_eq!(
            head.animations,
            [("var.a", "1"), ("head.rx", "var.a"), ("this.tx", "0")]
                .map(|(a, b)| (a.to_owned(), b.to_owned()))
        );
    }

    #[test]
    fn part_files_and_bases() {
        let file = |boxes: &str| {
            format!(
                r#"{{"textureSize": [32, 32], "boxes": [{{"coordinates": [{boxes}], "uvFront": [0, 0, 4, 2]}}]}}"#
            )
        };
        let mut asked = Vec::new();
        let model = parse_jem(
            JEM,
            r#"{"texture": "textures/entity/zombie/green", "models": [
                {"id": "base", "part": "head", "model": "hat.jpm", "scale": 1.5},
                {"baseId": "base", "part": "headwear", "attach": true},
                {"baseId": "base", "part": "body", "model": "./parts/vest.jpm"},
                {"part": "left_arm", "model": "hat.jpm"}
            ]}"#,
            &mut |p| {
                asked.push(p.to_owned());
                Some(file(if p.ends_with("vest.jpm") {
                    "0, 0, 0, 8, 8, 8"
                } else {
                    "0, 0, 0, 4, 2, 4"
                }))
            },
        )
        .unwrap();
        // Each file is read once, however many entries name it.
        assert_eq!(
            asked,
            [
                "assets/minecraft/optifine/cem/hat.jpm",
                "assets/minecraft/optifine/cem/parts/vest.jpm"
            ]
        );
        assert_eq!(
            model.texture.as_deref(),
            Some("assets/minecraft/textures/entity/zombie/green.png")
        );
        let [head, headwear, body, arm] = &model.parts[..] else {
            panic!("{:?}", model.parts);
        };
        assert_eq!((head.scale, head.attach), (1.5, false));
        assert_eq!(head.bone.quads.len(), 1);
        assert_eq!(bounds(&head.bone), ([0.0, 0.0, 0.0], [4.0, 2.0, 0.0]));
        // The second entry inherits the first's part model under its own part and attach.
        assert_eq!(
            (headwear.part.as_str(), headwear.attach, headwear.scale),
            ("headwear", true, 1.5)
        );
        assert_eq!(headwear.bone.quads, head.bone.quads);
        // Its own part file wins over the inherited one.
        assert_eq!(bounds(&body.bone), ([0.0, 0.0, 0.0], [8.0, 8.0, 0.0]));
        assert_eq!(arm.bone.quads, head.bone.quads);
        assert!(model.warnings.is_empty(), "{:?}", model.warnings);
    }

    #[test]
    fn broken_entries_are_left_out_with_a_warning() {
        let mut asked = Vec::new();
        let model = parse_jem(
            JEM,
            "\u{feff}{\"models\": [ // a hand-edited file\n
                {\"part\": \"head\", \"model\": \"missing.jpm\"},
                {\"part\": \"head\", \"model\": \"../../../../etc/passwd\"},
                {\"part\": \"body\", \"baseId\": \"nope\", \"boxes\": [{\"coordinates\": [0, 0, 0, 1, 1, 1], \"textureOffset\": [0, 0]}]},
                {\"part\": \"tail\", \"scale\": \"NaN\"},
            ]}",
            &mut |p| {
                asked.push(p.to_owned());
                None
            },
        )
        .unwrap();
        assert_eq!(asked, ["assets/minecraft/optifine/cem/missing.jpm"]);
        let names: Vec<&str> = model.parts.iter().map(|p| p.part.as_str()).collect();
        assert_eq!(names, ["body", "tail"]);
        assert_eq!(model.parts[0].bone.quads.len(), 6);
        assert_eq!(model.parts[1].scale, 1.0);
        assert_eq!(model.warnings.len(), 3, "{:?}", model.warnings);
        assert!(model.warnings[1].contains("outside the pack"));
        assert!(parse_jem(JEM, "{", &mut |_| None).is_err());
    }

    #[test]
    fn a_file_builds_at_most_its_limits() {
        // Faces: the part and its bone, then boxes of six, the last one cut to the faces left.
        let one = r#"{"coordinates": [0, 0, 0, 1, 1, 1], "textureOffset": [0, 0]}"#;
        let boxes = vec![one; MAX_NODES / 6 + 10].join(",");
        let model = parse(&format!(
            r#"{{"models": [{{"part": "body", "boxes": [{boxes}]}}]}}"#
        ));
        let quads = model.parts[0].bone.quads.len();
        assert!((MAX_NODES - 8..MAX_NODES).contains(&quads), "{quads}");
        assert_eq!(model.warnings.len(), 1);
        // Bones count too: empty submodels can't multiply for free. A part after the cut is left
        // out rather than replacing its vanilla part with nothing.
        let empty = vec!["{}"; MAX_NODES].join(",");
        let model = parse(&format!(
            r#"{{"models": [{{"part": "body", "boxes": [{one}], "submodels": [{empty}]}},
                {{"part": "head"}}]}}"#
        ));
        assert_eq!(model.parts.len(), 1);
        let children = model.parts[0].bone.children.len();
        assert!(children > 0 && children < MAX_NODES, "{children}");
        assert_eq!(model.warnings.len(), 1);
        // Boxes without faces count, and so do items that aren't boxes; a part they cut down to
        // nothing is left out too. A box of two keys costs both.
        for (junk, n) in [
            ("{}", MAX_NODES),
            ("0", MAX_NODES),
            (r#"{"a": 1, "b": 2}"#, MAX_NODES / 2 + 1),
        ] {
            let model = parse(&format!(
                r#"{{"models": [{{"part": "body", "boxes": [{}, {one}]}}, {{"part": "head"}}]}}"#,
                vec![junk; n].join(",")
            ));
            assert!(model.parts.is_empty(), "{junk}: {:?}", model.parts);
            assert_eq!(model.warnings.len(), 1, "{junk}");
        }
        // So do submodels that aren't objects.
        let model = parse(&format!(
            r#"{{"models": [{{"part": "body", "submodels": [{}, {{"boxes": [{one}]}}]}}]}}"#,
            vec!["[]"; MAX_NODES].join(",")
        ));
        assert!(model.parts.is_empty(), "{:?}", model.parts);
        // A box's keys cost even when it has no faces, so one of junk keys spends what is left
        // and isn't read again by the entries that inherit it.
        let keys: Vec<String> = (0..MAX_NODES).map(|i| format!(r#""k{i}": 0"#)).collect();
        let model = parse(&format!(
            r#"{{"models": [{{"id": "a", "boxes": [{{{}}}]}}, {{"part": "head", "baseId": "a"}},
                {{"part": "body", "boxes": [{one}]}}]}}"#,
            keys.join(",")
        ));
        assert!(model.parts.is_empty(), "{:?}", model.parts);
        assert_eq!(model.warnings.len(), 1);
        // Animations: an entry whose list doesn't fit what is left is left out whole, before it
        // spends anything, rather than drawn frozen. Pairs skipped as not scalar count too, and
        // so do items that aren't objects.
        let pairs = |pair: &str| format!("{{{}}}", vec![pair; MAX_NODES].join(","));
        for list in [
            pairs(r#""var.a": "1""#),
            pairs(r#""var.a": [1]"#),
            vec!["0"; MAX_NODES].join(","),
        ] {
            let model = parse(&format!(
                r#"{{"models": [{{"part": "body", "boxes": [{one}], "animations": [{list}]}},
                    {{"part": "head", "boxes": [{one}]}}]}}"#
            ));
            let names: Vec<&str> = model.parts.iter().map(|p| p.part.as_str()).collect();
            assert_eq!(names, ["head"], "{}", &list[..20]);
            assert_eq!(model.parts[0].bone.quads.len(), 6);
            assert_eq!(model.warnings.len(), 1);
        }
        // Entries count whether or not they name a part.
        let ids = vec![r#"{"id": "x"}"#; MAX_ENTRIES].join(",");
        let model = parse(&format!(r#"{{"models": [{ids}, {{"part": "late"}}]}}"#));
        assert!(model.parts.is_empty());
        assert!(model.warnings[0].contains("models"));
    }

    #[test]
    fn the_last_nodes_still_take_a_box_and_using_them_all_is_no_cut() {
        // The first part spends all but `left`: its name, its layer, its bone and its empty
        // submodels. The second costs three the same way, then its box: one face and two keys
        // cost two.
        let after = |left: usize, faces: &str| {
            let empty = vec!["{}"; MAX_NODES - left - 3].join(",");
            parse(&format!(
                r#"{{"models": [{{"part": "body", "submodels": [{empty}]}},
                    {{"part": "head", "boxes": [{{"coordinates": [0, 0, 0, 1, 1, 1], {faces}}}]}}]}}"#
            ))
        };
        // One face, with 5 left: it fits exactly.
        let model = after(5, r#""uvUp": [0, 0, 1, 1]"#);
        assert_eq!(model.parts[1].bone.quads.len(), 1);
        assert!(model.warnings.is_empty(), "{:?}", model.warnings);
        // Six faces with 6 left: the part keeps three of them.
        let model = after(6, r#""textureOffset": [0, 0]"#);
        assert_eq!(model.parts[1].bone.quads.len(), 3);
        assert_eq!(model.warnings.len(), 1);
        // With 3 left the bone takes the last and its box is cut: the part is left out, not
        // emptied.
        let model = after(3, r#""textureOffset": [0, 0]"#);
        let names: Vec<&str> = model.parts.iter().map(|p| p.part.as_str()).collect();
        assert_eq!(names, ["body"]);
        assert_eq!(model.warnings.len(), 1);
    }

    #[test]
    fn a_key_repeated_in_an_entry_takes_its_last_value() {
        let model = parse(
            r#"{"models": [{"part": "head", "textureSize": [16, 16], "textureSize": [32, 32],
                "boxes": [{"coordinates": [0, 0, 0, 1, 1, 1], "uvUp": [0, 0, 1, 1]}]}]}"#,
        );
        let max_u = model.parts[0].bone.quads[0]
            .vertices
            .iter()
            .map(|v| v[3])
            .fold(0.0, f32::max);
        assert_eq!(max_u, 1.0 / 32.0);
    }

    #[test]
    fn boxes_off_the_numbers_are_skipped() {
        // The first box's corners overflow, the second's UVs (u runs to twice its depth).
        let model = parse(
            r#"{"models": [{"part": "head", "boxes": [
                {"coordinates": [3e38, 0, 0, 3e38, 1, 1], "textureOffset": [0, 0]},
                {"coordinates": [0, 0, 0, 1, 1, 3e38], "textureOffset": [0, 0]},
                {"coordinates": [0, 0, 0, 1, 1, 1], "textureOffset": [0, 0]}]}]}"#,
        );
        let quads = &model.parts[0].bone.quads;
        assert_eq!(quads.len(), 6);
        assert!(
            quads
                .iter()
                .flat_map(|q| q.vertices)
                .flatten()
                .all(f32::is_finite)
        );
    }

    #[test]
    fn files_over_the_byte_limit() {
        // Valid JSON, so only its size can refuse it.
        let padded = |json: &str, len: usize| format!("{json}{}", " ".repeat(len - json.len()));
        let big = padded(r#"{"models": []}"#, MAX_BYTES + 1);
        let err = parse_jem(JEM, &big, &mut |_| None).unwrap_err();
        assert!(err.contains("larger"), "{err}");
        let big = padded("{}", MAX_BYTES + 1);
        let model = parse_jem(
            JEM,
            r#"{"models": [{"part": "head", "model": "big.jpm"}, {"part": "body"}]}"#,
            &mut |_| Some(big.clone()),
        )
        .unwrap();
        assert_eq!(model.parts.len(), 1);
        assert_eq!(model.parts[0].part, "body");
        assert_eq!(model.warnings.len(), 1);
        assert!(model.warnings[0].contains("larger"), "{:?}", model.warnings);
        // The files count together: two that fit alone don't fit both, and one read again from
        // the cache counts once.
        let half = padded(
            r#"{"boxes": [{"coordinates": [0, 0, 0, 1, 1, 1], "textureOffset": [0, 0]}]}"#,
            MAX_BYTES / 2 + 1,
        );
        let model = parse_jem(
            JEM,
            r#"{"models": [{"part": "head", "model": "a.jpm"}, {"part": "body", "model": "b.jpm"},
                {"part": "tail", "model": "a.jpm"}]}"#,
            &mut |_| Some(half.clone()),
        )
        .unwrap();
        let names: Vec<&str> = model.parts.iter().map(|p| p.part.as_str()).collect();
        assert_eq!(names, ["head", "tail"]);
        assert_eq!(model.warnings.len(), 1);
        assert!(model.warnings[0].contains("larger"), "{:?}", model.warnings);
        // The `.jem`'s own bytes count with them.
        let most = padded("{}", MAX_BYTES - 8);
        let model = parse_jem(
            JEM,
            r#"{"models": [{"part": "head", "model": "a.jpm"}]}"#,
            &mut |_| Some(most.clone()),
        )
        .unwrap();
        assert!(model.parts.is_empty(), "{:?}", model.parts);
        assert!(model.warnings[0].contains("larger"), "{:?}", model.warnings);
    }

    #[test]
    fn entries_sharing_a_part_file_pay_for_what_each_reads() {
        let jem: Vec<String> = (0..MAX_ENTRIES)
            .map(|i| format!(r#"{{"part": "p{i}", "model": "a.jpm"}}"#))
            .collect();
        let jem = format!(r#"{{"models": [{}]}}"#, jem.join(","));
        let one = r#"{"coordinates": [0, 0, 0, 1, 1, 1], "textureOffset": [0, 0]}"#;
        // Each part copies the expression it inherits, so only a few of a megabyte fit.
        let jpm = format!(
            r#"{{"boxes": [{one}], "animations": [{{"this.rx": "{}"}}]}}"#,
            "1".repeat(1 << 20)
        );
        let model = parse_jem(JEM, &jem, &mut |_| Some(jpm.clone())).unwrap();
        let copied: usize = model
            .parts
            .iter()
            .flat_map(|p| &p.animations)
            .map(|(_, e)| e.len())
            .sum();
        assert!(!model.parts.is_empty() && copied <= MAX_BYTES, "{copied}");
        assert_eq!(model.warnings.len(), 1, "{:?}", model.warnings);
        assert!(model.warnings[0].contains("left out"));
        // Each part looks keys up through its submodel's, so junk ones can't be read by all.
        let keys: Vec<String> = (0..200_000).map(|i| format!(r#""k{i}": 0"#)).collect();
        let jpm = format!(
            r#"{{"submodel": {{{}, "boxes": [{one}]}}}}"#,
            keys.join(",")
        );
        let model = parse_jem(JEM, &jem, &mut |_| Some(jpm.clone())).unwrap();
        assert!(model.parts.len() < MAX_ENTRIES);
        assert_eq!(model.warnings.len(), 1, "{:?}", model.warnings);
        assert!(model.warnings[0].contains("left out"));
    }

    #[test]
    fn a_missing_file_is_asked_for_once_and_warned_per_entry() {
        let mut asked = 0;
        let model = parse_jem(
            JEM,
            r#"{"models": [{"part": "head", "model": "gone.jpm"}, {"part": "body", "model": "gone.jpm"}]}"#,
            &mut |_| {
                asked += 1;
                None
            },
        )
        .unwrap();
        assert_eq!(asked, 1);
        assert!(model.parts.is_empty());
        assert_eq!(model.warnings.len(), 2, "{:?}", model.warnings);
    }

    #[test]
    fn own_keys_win_over_the_part_file_also_in_a_base() {
        // The part file's box spans u 0..4; at its own 32 wide that would end at 4/32.
        let jpm = r#"{"textureSize": [32, 32],
            "boxes": [{"coordinates": [0, 0, 0, 4, 2, 0], "uvFront": [0, 0, 4, 2]}]}"#;
        let model = parse_jem(
            JEM,
            r#"{"models": [
                {"part": "head", "model": "a.jpm", "textureSize": [64, 64]},
                {"id": "base", "model": "a.jpm", "textureSize": [64, 64]},
                {"part": "body", "baseId": "base"}
            ]}"#,
            &mut |_| Some(jpm.to_owned()),
        )
        .unwrap();
        assert_eq!(model.parts.len(), 2);
        for part in &model.parts {
            let max_u = part.bone.quads[0]
                .vertices
                .iter()
                .map(|v| v[3])
                .fold(0.0, f32::max);
            assert_eq!(max_u, 4.0 / 64.0, "{}", part.part);
        }
    }

    #[test]
    fn bases_chain_and_the_first_id_wins() {
        let model = parse(
            r#"{"models": [
                {"id": "a", "scale": 2, "boxes": [{"coordinates": [0, 0, 0, 1, 1, 1], "textureOffset": [0, 0]}]},
                {"id": "a", "scale": 9},
                {"id": "b", "baseId": "a", "attach": true},
                {"baseId": "b", "part": "head"}
            ]}"#,
        );
        let head = &model.parts[0];
        assert_eq!(
            (head.scale, head.attach, head.bone.quads.len()),
            (2.0, true, 6)
        );
    }

    #[test]
    fn a_submodels_pose_is_its_own_translate_whatever_the_parent_carries() {
        // Fresh Animations parks a cow's `head2` at `head2.ty = -20` under a `body` part
        // whose translate is 14: every bone's pose is its own translate alone, or each
        // animation write moves the geometry by the parent's amount and the head flies off
        // the body on the pack's first frame.
        let model = parse(
            r#"{"models": [{"part": "body", "invertAxis": "xy", "translate": [0, -14, -1],
                "submodels": [{"id": "head2", "invertAxis": "xy", "translate": [0, 20, -8],
                    "boxes": [{"coordinates": [-4, 14, -4, 8, 8, 8], "textureOffset": [0, 0]}],
                    "submodels": [{"id": "snout", "invertAxis": "xy", "translate": [0, -2, -6]}]}]}]}"#,
        );
        let body = &model.parts[0].bone;
        assert_eq!(body.pivot, [0.0, 14.0, -1.0]);
        let head = &body.children[0];
        assert_eq!(head.pivot, [0.0, -20.0, -8.0]);
        assert_eq!(head.children[0].pivot, [0.0, 2.0, -6.0]);
        // The chain of pivots lands the head where the model has it: 6 up and 9 forward of
        // the vanilla body's pivot, the box hanging 22..14 below the head's own pivot.
        assert_eq!(
            [
                body.pivot[0] + head.pivot[0],
                body.pivot[1] + head.pivot[1],
                body.pivot[2] + head.pivot[2]
            ],
            [0.0, -6.0, -9.0]
        );
        let lo = head
            .quads
            .iter()
            .flat_map(|q| q.vertices)
            .map(|v| v[1])
            .fold(f32::MAX, f32::min);
        let hi = head
            .quads
            .iter()
            .flat_map(|q| q.vertices)
            .map(|v| v[1])
            .fold(f32::MIN, f32::max);
        assert!(
            (lo - -22.0).abs() < 1e-5 && (hi - -14.0).abs() < 1e-5,
            "{lo} {hi}"
        );
    }

    #[test]
    fn bones_carry_scale_and_attachments() {
        let model = parse(
            r#"{"models": [{"part": "head", "invertAxis": "xy", "scale": 1.5,
                "attachments": {"right_handheld_item": [1, -2, 3]},
                "submodels": [{"id": "arm", "invertAxis": "xyz", "scale": [1, 2, 4],
                    "attachments": {"fox_item": [0, 6, -1]}}]}]}"#,
        );
        assert_eq!(model.parts[0].bone.scale, [1.5; 3]);
        // The offsets flip on the declaring bone's inverted axes.
        assert_eq!(
            model.parts[0].bone.attachments,
            [("right_handheld_item".to_owned(), [-1.0, 2.0, 3.0])]
        );
        let arm = &model.parts[0].bone.children[0];
        assert_eq!(arm.scale, [1.0, 2.0, 4.0]);
        assert_eq!(arm.attachments, [("fox_item".to_owned(), [0.0, -6.0, 1.0])]);
    }

    #[test]
    fn unread_keys_are_named_once() {
        let model = parse(
            r#"{"models": [{"part": "head", "texture": "a"}, {"part": "body", "sprites": []}]}"#,
        );
        assert_eq!(model.warnings.len(), 1);
        assert!(model.warnings[0].contains("textures and sprites"));
        let model = parse(r#"{"models": [{"part": "head", "submodels": [{"texture": "a"}]}]}"#);
        assert_eq!(model.warnings.len(), 1);
        assert!(model.warnings[0].contains("textures and sprites"));
    }
}
