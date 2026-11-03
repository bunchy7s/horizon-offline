//! The item file: JSON with the model, its attachment and its texture (a
//! base64 PNG), checked against the limits below when read.

use crate::pose::{Pose, pose_matrix};
use serde::Deserialize;

/// The item file's version.
pub const FORMAT: u32 = 1;
/// Vertices one item may draw (4 per face): 21 boxes. A player's own
/// model and its outer layer are 288.
pub const MAX_VERTICES: usize = 512;
/// The largest texture side: an item's texture fits one skin slot of the
/// game's atlas.
pub const MAX_TEXTURE: u32 = 64;
/// Parts per item, the attachment's root not counted.
pub const MAX_PARTS: usize = 16;
/// How far (pixels, 1/16 block) any vertex may reach from the bone's pivot
/// at rest, animations included: an item stays near its wearer.
pub const MAX_REACH: f32 = 24.0;
/// The largest side of a name tag's mark (square, drawn before the name).
pub const MAX_MARK: u32 = 16;

/// Where an item is worn: one item per slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Slot {
    Head,
    Back,
    Shoulder,
    /// A name tag style: no model, a square mark drawn before the player's
    /// name and a line under it (`Item::tag`).
    NameTag,
}

/// The part of the player model an item follows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bone {
    #[default]
    Head,
    Body,
    LeftArm,
    RightArm,
    LeftLeg,
    RightLeg,
}

impl Bone {
    /// The part's name in the player model.
    pub fn part(self) -> &'static str {
        match self {
            Bone::Head => "head",
            Bone::Body => "body",
            Bone::LeftArm => "left_arm",
            Bone::RightArm => "right_arm",
            Bone::LeftLeg => "left_leg",
            Bone::RightLeg => "right_leg",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    X,
    Y,
    Z,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Motion {
    /// Turns back and forth: `amplitude` degrees about the axis.
    Swing,
    /// Moves back and forth: `amplitude` pixels along the axis.
    Bob,
    /// Turns all the way round once per period.
    Spin,
}

/// A looping animation of one part, on top of its rest pose: a sine of
/// `period` ticks starting `phase` (a fraction of the period) in.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Animation {
    pub part: String,
    pub motion: Motion,
    pub axis: Axis,
    #[serde(default)]
    pub amplitude: f32,
    pub period: f32,
    #[serde(default)]
    pub phase: f32,
}

/// One quad: the game's `Polygon` (model pixels, then UV in 0..1 of the
/// item's texture).
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub normal: [f32; 3],
    pub vertices: [[f32; 5]; 4],
}

/// A part: its pose (as the game's: x, y, z in pixels, rotations in
/// radians, scales), its faces and its children.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub name: String,
    pub pose: Pose,
    pub faces: Vec<Face>,
    pub children: Vec<Part>,
}

/// RGBA pixels, row by row.
#[derive(Clone, Debug, PartialEq)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A name tag style: the line drawn under the name, RGB. The mark is the
/// item's texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tag {
    pub line: [u8; 3],
}

/// A checked item. `root` is named `cosmetic` and holds the attachment as
/// its pose; the item's own parts are its children.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: String,
    pub slot: Slot,
    pub bone: Bone,
    pub root: Part,
    pub texture: Texture,
    pub animations: Vec<Animation>,
    pub vertices: usize,
    /// A name tag's line (`Slot::NameTag` only; it has no parts).
    pub tag: Option<Tag>,
}

#[derive(Debug, PartialEq)]
pub enum Error {
    Json(String),
    Format(u32),
    Texture(String),
    TooManyVertices(usize),
    TooManyParts(usize),
    TooFar(f32),
    Invalid(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Json(e) => write!(f, "not an item file: {e}"),
            Error::Format(v) => write!(f, "item format {v}, this game reads {FORMAT}"),
            Error::Texture(e) => write!(f, "texture: {e}"),
            Error::TooManyVertices(n) => write!(f, "{n} vertices, at most {MAX_VERTICES}"),
            Error::TooManyParts(n) => write!(f, "{n} parts, at most {MAX_PARTS}"),
            Error::TooFar(d) => write!(
                f,
                "reaches {d:.1} pixels from its bone, at most {MAX_REACH}"
            ),
            Error::Invalid(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemFile {
    format: u32,
    id: String,
    slot: Slot,
    #[serde(default)]
    bone: Bone,
    #[serde(default)]
    attach: AttachFile,
    texture: TextureFile,
    parts: Vec<PartFile>,
    #[serde(default)]
    animations: Vec<Animation>,
    #[serde(default)]
    tag: Option<TagFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TagFile {
    line: String,
}

/// `#rrggbb`.
fn colour(text: &str) -> Option<[u8; 3]> {
    let hex = text.strip_prefix('#').filter(|h| h.len() == 6)?;
    let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AttachFile {
    #[serde(default)]
    offset: [f32; 3],
    #[serde(default)]
    rotation: [f32; 3],
    #[serde(default = "one")]
    scale: f32,
}

impl Default for AttachFile {
    fn default() -> Self {
        Self {
            offset: [0.0; 3],
            rotation: [0.0; 3],
            scale: 1.0,
        }
    }
}

fn one() -> f32 {
    1.0
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextureFile {
    png: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PartFile {
    name: String,
    #[serde(default)]
    pivot: [f32; 3],
    /// Degrees.
    #[serde(default)]
    rotation: [f32; 3],
    #[serde(default)]
    cubes: Vec<CubeFile>,
    #[serde(default)]
    children: Vec<PartFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CubeFile {
    origin: [f32; 3],
    size: [f32; 3],
    uv: [f32; 2],
    #[serde(default)]
    inflate: f32,
    #[serde(default)]
    mirror: bool,
}

fn invalid(text: impl Into<String>) -> Error {
    Error::Invalid(text.into())
}

fn finite(values: &[f32], what: &str) -> Result<(), Error> {
    if values.iter().all(|v| v.is_finite()) {
        Ok(())
    } else {
        Err(invalid(format!("{what} is not a number")))
    }
}

impl Item {
    /// Reads and checks an item file.
    pub fn parse(bytes: &[u8]) -> Result<Item, Error> {
        let file: ItemFile =
            serde_json::from_slice(bytes).map_err(|e| Error::Json(e.to_string()))?;
        if file.format != FORMAT {
            return Err(Error::Format(file.format));
        }
        if file.id.is_empty()
            || file.id.len() > 64
            || !file
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(invalid("the id is 1 to 64 of a-z, 0-9 and _"));
        }
        let texture = decode_png(&file.texture.png)?;
        let tag = match (file.slot, &file.tag) {
            (Slot::NameTag, Some(t)) => {
                if !file.parts.is_empty() || !file.animations.is_empty() {
                    return Err(invalid("a name tag has no parts or animations"));
                }
                if texture.width != texture.height || texture.width > MAX_MARK {
                    return Err(Error::Texture(format!(
                        "a name tag's mark is square, at most {MAX_MARK}x{MAX_MARK}"
                    )));
                }
                let line = colour(&t.line).ok_or_else(|| invalid("tag.line is #rrggbb"))?;
                Some(Tag { line })
            }
            (Slot::NameTag, None) => return Err(invalid("a name tag needs its tag")),
            (_, Some(_)) => return Err(invalid("only a name tag has a tag")),
            (_, None) => None,
        };
        let a = &file.attach;
        finite(&a.offset, "attach.offset")?;
        finite(&a.rotation, "attach.rotation")?;
        if !(a.scale > 0.0 && a.scale <= 2.0) {
            return Err(invalid("attach.scale is above 0 and at most 2"));
        }
        let [rx, ry, rz] = a.rotation.map(f32::to_radians);
        let pose = [
            a.offset[0],
            a.offset[1],
            a.offset[2],
            rx,
            ry,
            rz,
            a.scale,
            a.scale,
            a.scale,
        ];
        let size = [texture.width as f32, texture.height as f32];
        let mut names = Vec::new();
        let children = file
            .parts
            .iter()
            .map(|p| bake(p, size, &mut names))
            .collect::<Result<Vec<_>, _>>()?;
        if names.len() > MAX_PARTS {
            return Err(Error::TooManyParts(names.len()));
        }
        let root = Part {
            name: "cosmetic".into(),
            pose,
            faces: Vec::new(),
            children,
        };
        let vertices = count(&root);
        if vertices > MAX_VERTICES {
            return Err(Error::TooManyVertices(vertices));
        }
        for anim in &file.animations {
            finite(&[anim.amplitude, anim.period, anim.phase], "an animation")?;
            if !names.contains(&anim.part) {
                return Err(invalid(format!(
                    "an animation moves no part {:?}",
                    anim.part
                )));
            }
            let limit = match anim.motion {
                Motion::Swing => 45.0,
                Motion::Bob => 4.0,
                Motion::Spin => 0.0,
            };
            if anim.amplitude.abs() > limit || anim.period < 4.0 || anim.period > 1200.0 {
                return Err(invalid(format!(
                    "the animation of {:?} is out of bounds",
                    anim.part
                )));
            }
        }
        let item = Item {
            id: file.id,
            slot: file.slot,
            bone: file.bone,
            root,
            texture,
            animations: file.animations,
            vertices,
            tag,
        };
        let reach = item.reach();
        if reach > MAX_REACH {
            return Err(Error::TooFar(reach));
        }
        Ok(item)
    }

    /// The farthest any vertex gets from the bone's pivot (pixels): at rest,
    /// and at 32 points of each animation's period.
    fn reach(&self) -> f32 {
        let mut times = vec![0.0];
        for anim in &self.animations {
            times.extend((0..32).map(|k| anim.period * k as f32 / 32.0));
        }
        let mut far = 0.0_f32;
        let mut poses = std::collections::HashMap::new();
        for ticks in times {
            poses.clear();
            self.animate(ticks, |name, pose| {
                poses.insert(name.to_owned(), pose);
            });
            walk(&self.root, glam::Mat4::IDENTITY, &poses, &mut |p| {
                far = far.max(p.length() * 16.0);
            });
        }
        far
    }

    /// The animated parts' poses at `ticks` (game ticks, any start): each
    /// is the part's rest pose with its animations added.
    pub fn animate(&self, ticks: f32, mut set: impl FnMut(&str, Pose)) {
        let mut done: Vec<&str> = Vec::new();
        for anim in &self.animations {
            if done.contains(&anim.part.as_str()) {
                continue;
            }
            done.push(&anim.part);
            let Some(part) = find(&self.root, &anim.part) else {
                continue;
            };
            let mut pose = part.pose;
            for a in self.animations.iter().filter(|a| a.part == anim.part) {
                let turns = ticks / a.period + a.phase;
                let axis = a.axis as usize;
                match a.motion {
                    Motion::Swing => {
                        let wave = (turns * std::f32::consts::TAU).sin();
                        pose[3 + axis] += (a.amplitude * wave).to_radians();
                    }
                    Motion::Bob => {
                        pose[axis] += a.amplitude * (turns * std::f32::consts::TAU).sin();
                    }
                    Motion::Spin => {
                        pose[3 + axis] += turns.rem_euclid(1.0) * std::f32::consts::TAU;
                    }
                }
            }
            set(&anim.part, pose);
        }
    }
}

/// Every vertex of `part`'s subtree under `parent`, as posed by `poses`
/// (else the parts' rest poses), in blocks.
fn walk(
    part: &Part,
    parent: glam::Mat4,
    poses: &std::collections::HashMap<String, Pose>,
    each: &mut impl FnMut(glam::Vec3),
) {
    let m = parent * pose_matrix(*poses.get(&part.name).unwrap_or(&part.pose));
    for face in &part.faces {
        for v in face.vertices {
            each(m.transform_point3(glam::Vec3::new(v[0], v[1], v[2]) / 16.0));
        }
    }
    for child in &part.children {
        walk(child, m, poses, each);
    }
}

fn find<'a>(part: &'a Part, name: &str) -> Option<&'a Part> {
    if part.name == name {
        return Some(part);
    }
    part.children.iter().find_map(|c| find(c, name))
}

fn count(part: &Part) -> usize {
    part.faces.len() * 4 + part.children.iter().map(count).sum::<usize>()
}

fn bake(file: &PartFile, size: [f32; 2], names: &mut Vec<String>) -> Result<Part, Error> {
    if file.name.is_empty() || file.name == "cosmetic" || names.contains(&file.name) {
        return Err(invalid(format!("part names are unique: {:?}", file.name)));
    }
    names.push(file.name.clone());
    finite(&file.pivot, "a pivot")?;
    finite(&file.rotation, "a rotation")?;
    let mut faces = Vec::new();
    for cube in &file.cubes {
        finite(&cube.origin, "a cube")?;
        finite(&cube.size, "a cube")?;
        finite(&cube.uv, "a cube")?;
        if cube.size.iter().any(|s| *s < 0.0) || !(0.0..=4.0).contains(&cube.inflate) {
            return Err(invalid("a cube's size is negative"));
        }
        let [w, h, d] = cube.size;
        // Box UV: the unfolded box spans 2(d + w) by d + h texels.
        if cube.uv[0] < 0.0
            || cube.uv[1] < 0.0
            || cube.uv[0] + 2.0 * (d + w) > size[0]
            || cube.uv[1] + d + h > size[1]
        {
            return Err(invalid("a cube's UV leaves the texture"));
        }
        faces.extend(box_faces(cube, size));
    }
    let [rx, ry, rz] = file.rotation.map(f32::to_radians);
    let [x, y, z] = file.pivot;
    let children = file
        .children
        .iter()
        .map(|c| bake(c, size, names))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Part {
        name: file.name.clone(),
        pose: [x, y, z, rx, ry, rz, 1.0, 1.0, 1.0],
        faces,
        children,
    })
}

/// `box_faces` of a box given field by field.
pub(crate) fn cube_faces(
    origin: [f32; 3],
    size: [f32; 3],
    uv: [f32; 2],
    inflate: f32,
    mirror: bool,
    texture: [f32; 2],
) -> Vec<Face> {
    let cube = CubeFile {
        origin,
        size,
        uv,
        inflate,
        mirror,
    };
    box_faces(&cube, texture)
}

/// `ModelPart.Cube`: six faces with box UV, in vanilla's order (down, up,
/// west, north, east, south), grown by `inflate`, mirrored in x on request.
fn box_faces(cube: &CubeFile, [tw, th]: [f32; 2]) -> Vec<Face> {
    let [w, h, d] = cube.size;
    let g = cube.inflate;
    let (mut x0, y0, z0) = (cube.origin[0] - g, cube.origin[1] - g, cube.origin[2] - g);
    let (mut x1, y1, z1) = (
        cube.origin[0] + w + g,
        cube.origin[1] + h + g,
        cube.origin[2] + d + g,
    );
    if cube.mirror {
        std::mem::swap(&mut x0, &mut x1);
    }
    let t0 = [x0, y0, z0];
    let t1 = [x1, y0, z0];
    let t2 = [x1, y1, z0];
    let t3 = [x0, y1, z0];
    let l0 = [x0, y0, z1];
    let l1 = [x1, y0, z1];
    let l2 = [x1, y1, z1];
    let l3 = [x0, y1, z1];
    let [u, v] = cube.uv;
    let (u1, u2, u22) = (u + d, u + d + w, u + d + w + w);
    let (u3, u4) = (u + d + w + d, u + d + w + d + w);
    let (v1, v2) = (v + d, v + d + h);
    let faces: [([[f32; 3]; 4], [f32; 4], [f32; 3]); 6] = [
        ([l1, l0, t0, t1], [u1, v, u2, v1], [0.0, -1.0, 0.0]),
        ([t2, t3, l3, l2], [u2, v1, u22, v], [0.0, 1.0, 0.0]),
        ([t0, l0, l3, t3], [u, v1, u1, v2], [-1.0, 0.0, 0.0]),
        ([t1, t0, t3, t2], [u1, v1, u2, v2], [0.0, 0.0, -1.0]),
        ([l1, t1, t2, l2], [u2, v1, u3, v2], [1.0, 0.0, 0.0]),
        ([l0, l1, l2, l3], [u3, v1, u4, v2], [0.0, 0.0, 1.0]),
    ];
    faces
        .into_iter()
        .map(|(corners, [a, b, c, e], mut normal)| {
            // Polygon's remap: (u1, v0), (u0, v0), (u0, v1), (u1, v1).
            let uvs = [[c, b], [a, b], [a, e], [c, e]];
            let mut vertices: [[f32; 5]; 4] = std::array::from_fn(|i| {
                let [x, y, z] = corners[i];
                [x, y, z, uvs[i][0] / tw, uvs[i][1] / th]
            });
            if cube.mirror {
                vertices.reverse();
                normal[0] = -normal[0];
            }
            Face { normal, vertices }
        })
        .collect()
}

fn decode_png(text: &str) -> Result<Texture, Error> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(text)
        .map_err(|e| Error::Texture(e.to_string()))?;
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|e| Error::Texture(e.to_string()))?;
    let (width, height) = (reader.info().width, reader.info().height);
    if width == 0 || height == 0 || width > MAX_TEXTURE || height > MAX_TEXTURE {
        return Err(Error::Texture(format!(
            "{width}x{height}, at most {MAX_TEXTURE}x{MAX_TEXTURE}"
        )));
    }
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let frame = reader
        .next_frame(&mut buf)
        .map_err(|e| Error::Texture(e.to_string()))?;
    let pixels = &buf[..frame.buffer_size()];
    let rgba = match frame.color_type {
        png::ColorType::Rgba => pixels.to_vec(),
        png::ColorType::Rgb => pixels
            .chunks(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => pixels
            .chunks(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => pixels.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err(Error::Texture("unexpanded palette".into())),
    };
    Ok(Texture {
        width,
        height,
        rgba,
    })
}
