//! Block-entity renderers (`BlockEntityRenderers`): chests (single, double,
//! trapped, ender, copper), shulker boxes, banners, skulls and heads, bells,
//! decorated pots, conduits, enchanting-table and lectern books and copper
//! golem statues, End portals and End gateways. Models are vanilla's baked `LayerDefinitions` layers
//! (`entity-models.json`); transforms, animation and textures follow each
//! vanilla renderer. Faces are lit like the `entity` shader: the lightmap at
//! the block plus `minecraft_mix_light` with the level's two diffuse lights.
use crate::{
    hud::Rect,
    mesher::{CULL_BACK, Vertex},
    world::World,
};
use glam::{DVec3, Mat4, Quat, Vec3};
use horizon_core::geometry::{
    Direction,
    trig::{self, Mth},
};
use horizon_world::{
    block_render::FaceVisibility,
    registry::{BlockRegistry, BlockStateId},
};
use serde_json::Value;
use std::{collections::HashMap, f32::consts::PI, path::Path, sync::Mutex};

/// One baked `ModelPart`: initial pose, textured polygons (pixel units,
/// normalized UVs) and named children.
#[derive(Clone, Debug, Default)]
pub struct ModelPart {
    /// x, y, z, xRot, yRot, zRot, xScale, yScale, zScale.
    pub pose: [f32; 9],
    pub polygons: Vec<[[f32; 5]; 4]>,
    pub children: Vec<(String, ModelPart)>,
}
impl ModelPart {
    pub fn parse(v: &Value) -> Self {
        let mut pose = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        if let Some(a) = v["pose"].as_array() {
            for (p, x) in pose.iter_mut().zip(a) {
                *p = x.as_f64().unwrap_or(0.0) as f32;
            }
        }
        let polygons = v["polygons"]
            .as_array()
            .map(|polys| {
                polys
                    .iter()
                    .filter_map(|poly| {
                        let verts = poly.as_array()?;
                        if verts.len() != 4 {
                            return None;
                        }
                        let mut out = [[0f32; 5]; 4];
                        for (o, vert) in out.iter_mut().zip(verts) {
                            for (c, x) in o.iter_mut().zip(vert.as_array()?) {
                                *c = x.as_f64().unwrap_or(0.0) as f32;
                            }
                        }
                        Some(out)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let children = v["children"]
            .as_object()
            .map(|m| m.iter().map(|(k, c)| (k.clone(), Self::parse(c))).collect())
            .unwrap_or_default();
        Self {
            pose,
            polygons,
            children,
        }
    }
    pub fn child(&self, name: &str) -> Option<&ModelPart> {
        self.children
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, c)| c)
    }
}

/// `Lighting` level light directions (overworld/end and nether).
pub fn diffuse_lights(nether: bool) -> (Vec3, Vec3) {
    let l0 = Vec3::new(0.2, 1.0, -0.7).normalize();
    let l1 = if nether {
        Vec3::new(-0.2, -1.0, 0.7)
    } else {
        Vec3::new(-0.2, 1.0, 0.7)
    }
    .normalize();
    (l0, l1)
}
/// `minecraft_mix_light`: 0.6 power, 0.4 ambient.
pub fn diffuse(normal: Vec3, lights: (Vec3, Vec3)) -> f32 {
    ((lights.0.dot(normal).max(0.0) + lights.1.dot(normal).max(0.0)) * 0.6 + 0.4).min(1.0)
}
/// Packed vertex light from the level's packed `sky << 4 | block` byte.
pub fn vertex_light(packed: u8) -> u32 {
    (u32::from(packed >> 4) * 16) | ((u32::from(packed & 15) * 16) << 8)
}
/// `ChestRenderer.xmasTextures`: `SpecialDates.isExtendedChristmas` (24 to 26
/// December in the local time zone), read once when the renderer is created.
fn extended_christmas() -> bool {
    use chrono::Datelike;
    static XMAS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *XMAS.get_or_init(|| {
        let now = chrono::Local::now();
        now.month() == 12 && (24..=26).contains(&now.day())
    })
}
fn pack_color(rgb: u32, alpha: u32, brightness: f32) -> u32 {
    let c = |shift: u32| (((rgb >> shift) & 255) as f32 * brightness) as u32;
    alpha << 24 | c(0) << 16 | c(8) << 8 | c(16)
}

/// Everything a draw call needs besides the geometry.
#[derive(Clone, Copy)]
pub struct Paint {
    pub sprite: Rect,
    /// 0xRRGGBB multiplied with the texture (`-1` = white in vanilla).
    pub tint: u32,
    pub light: u32,
    pub lights: (Vec3, Vec3),
}
/// `ModelPart.render`: translate by pose/16, rotate ZYX, scale; children inherit.
/// `adjust` may change a part's pose by name (`setupAnim`); `skip` hides parts.
pub fn draw_part(
    part: &ModelPart,
    name: &str,
    parent: Mat4,
    adjust: &dyn Fn(&str, &mut [f32; 9]),
    paint: Paint,
    out: &mut Vec<Vertex>,
) {
    let mut p = part.pose;
    adjust(name, &mut p);
    let m = parent
        * Mat4::from_translation(Vec3::new(p[0], p[1], p[2]) / 16.0)
        * Mat4::from_quat(Quat::from_euler(glam::EulerRot::ZYX, p[5], p[4], p[3]))
        * Mat4::from_scale(Vec3::new(p[6], p[7], p[8]));
    let Paint {
        sprite: r,
        tint,
        light,
        lights,
    } = paint;
    for poly in &part.polygons {
        let pos = poly.map(|v| m.transform_point3(Vec3::new(v[0], v[1], v[2]) / 16.0));
        let normal = (pos[1] - pos[0]).cross(pos[2] - pos[0]).normalize_or_zero();
        let normal = if normal == Vec3::ZERO {
            // Degenerate polygon: use the other diagonal.
            (pos[2] - pos[1])
                .cross(pos[3] - pos[1])
                .normalize_or(Vec3::Y)
        } else {
            normal
        };
        let color = pack_color(tint, 255, diffuse(normal, lights));
        for (v, position) in poly.iter().zip(pos) {
            out.push(Vertex {
                normal: [0; 4],
                pos: position.to_array(),
                uv: [r.u0 + (r.u1 - r.u0) * v[3], r.v0 + (r.v1 - r.v0) * v[4]],
                layer: 0,
                color,
                light,

                glint_uv: [0.0, 0.0],
            });
        }
    }
    for (child_name, child) in &part.children {
        draw_part(child, child_name, m, adjust, paint, out);
    }
}

/// `Direction.toYRot` for horizontal facings (south 0, west 90, north 180, east 270).
fn y_rot(facing: &str) -> f32 {
    match facing {
        "west" => 90.0,
        "north" => 180.0,
        "east" => 270.0,
        _ => 0.0,
    }
}
fn opposite(facing: &str) -> &'static str {
    match facing {
        "north" => "south",
        "south" => "north",
        "west" => "east",
        "east" => "west",
        "up" => "down",
        _ => "up",
    }
}
fn clockwise(facing: &str) -> &'static str {
    match facing {
        "north" => "east",
        "east" => "south",
        "south" => "west",
        _ => "north",
    }
}
fn step(facing: &str) -> [i32; 3] {
    match facing {
        "north" => [0, 0, -1],
        "south" => [0, 0, 1],
        "west" => [-1, 0, 0],
        "east" => [1, 0, 0],
        "up" => [0, 1, 0],
        _ => [0, -1, 0],
    }
}
/// `Direction.getRotation`.
fn direction_rotation(facing: &str) -> Quat {
    let xyz = |x: f32, y: f32, z: f32| Quat::from_euler(glam::EulerRot::XYZ, x, y, z);
    match facing {
        "down" => Quat::from_rotation_x(PI),
        "north" => xyz(PI / 2.0, 0.0, PI),
        "south" => Quat::from_rotation_x(PI / 2.0),
        "west" => xyz(PI / 2.0, 0.0, PI / 2.0),
        "east" => xyz(PI / 2.0, 0.0, -PI / 2.0),
        _ => Quat::IDENTITY,
    }
}
fn around(rotation: Mat4, pivot: Vec3) -> Mat4 {
    Mat4::from_translation(pivot) * rotation * Mat4::from_translation(-pivot)
}

/// `DyeColor` names and texture diffuse colours, by id.
pub const DYES: [(&str, u32); 16] = [
    ("white", 0xF9FFFE),
    ("orange", 0xF9801D),
    ("magenta", 0xC74EBD),
    ("light_blue", 0x3AB3DA),
    ("yellow", 0xFED83D),
    ("lime", 0x80C71F),
    ("pink", 0xF38BAA),
    ("gray", 0x474F52),
    ("light_gray", 0x9D9D97),
    ("cyan", 0x169C9C),
    ("purple", 0x8932B8),
    ("blue", 0x3C44AA),
    ("brown", 0x835432),
    ("green", 0x5E7C16),
    ("red", 0xB02E26),
    ("black", 0x1D1D21),
];
fn dye_prefix(name: &str) -> Option<(usize, &str)> {
    // Longest names first so "light_gray" wins over "gray".
    let mut order: Vec<usize> = (0..16).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(DYES[i].0.len()));
    order.into_iter().find_map(|i| {
        name.strip_prefix(DYES[i].0)
            .and_then(|rest| rest.strip_prefix('_'))
            .map(|rest| (i, rest))
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChestPart {
    Single,
    Left,
    Right,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SkullType {
    Skeleton,
    WitherSkeleton,
    Zombie,
    Creeper,
    Dragon,
    Piglin,
    Player,
}
#[derive(Clone, Debug)]
enum Kind {
    None,
    Chest {
        /// The block (`neighbourState.is(state.getBlock())`).
        block: String,
        texture: &'static str,
        part: ChestPart,
        y_rot: f32,
        double_with: Option<[i32; 3]>,
    },
    Shulker {
        texture: String,
        facing: &'static str,
    },
    Banner {
        color: u32,
        wall: bool,
        angle: f32,
    },
    Skull {
        kind: SkullType,
        wall: Option<&'static str>,
        segment: u8,
    },
    Bell,
    DecoratedPot {
        y_rot: f32,
    },
    Conduit,
    EnchantingTable,
    Lectern {
        y_rot: f32,
    },
    CopperGolem {
        texture: &'static str,
        pose: &'static str,
        facing: &'static str,
    },
    /// `TheEndPortalRenderer`: the top and bottom faces of a 6/16-tall slab.
    EndPortal,
    /// `TheEndGatewayRenderer`: the visible faces of the cube and the beam.
    EndGateway {
        state: u32,
    },
    /// Sign text (`StandingSignRenderer` / `HangingSignRenderer`); the boards are block models.
    Sign {
        hanging: bool,
        wall: bool,
        angle: f32,
    },
}
fn intern(facing: &str) -> &'static str {
    match facing {
        "north" => "north",
        "south" => "south",
        "west" => "west",
        "east" => "east",
        "up" => "up",
        _ => "down",
    }
}
fn classify(name: &str, prop: &dyn Fn(&str) -> Option<String>) -> Kind {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    let facing = || intern(&prop("facing").unwrap_or_default());
    let chest = |texture: &'static str, left: &'static str, right: &'static str| {
        let facing = facing();
        let part = match prop("type").as_deref() {
            Some("left") => ChestPart::Left,
            Some("right") => ChestPart::Right,
            _ => ChestPart::Single,
        };
        // ChestBlock.getConnectedDirection: left connects clockwise, right counter-clockwise.
        let other = match part {
            ChestPart::Single => None,
            ChestPart::Left => Some(step(clockwise(facing))),
            ChestPart::Right => Some(step(opposite(clockwise(facing)))),
        };
        Kind::Chest {
            block: name.to_owned(),
            texture: match part {
                ChestPart::Single => texture,
                ChestPart::Left => left,
                ChestPart::Right => right,
            },
            part,
            y_rot: y_rot(facing),
            double_with: other,
        }
    };
    if let Some(hanging) = crate::signs::sign_kind(name) {
        let wall = name.contains("_wall_");
        let angle = if wall {
            y_rot(facing())
        } else {
            prop("rotation")
                .and_then(|r| r.parse::<f32>().ok())
                .unwrap_or(0.0)
                * 22.5
        };
        return Kind::Sign {
            hanging,
            wall,
            angle,
        };
    }
    let unwaxed = name.strip_prefix("waxed_").unwrap_or(name);
    match unwaxed {
        "chest" => {
            return chest(
                "entity/chest/normal",
                "entity/chest/normal_left",
                "entity/chest/normal_right",
            );
        }
        "trapped_chest" => {
            return chest(
                "entity/chest/trapped",
                "entity/chest/trapped_left",
                "entity/chest/trapped_right",
            );
        }
        "ender_chest" => {
            return chest(
                "entity/chest/ender",
                "entity/chest/ender",
                "entity/chest/ender",
            );
        }
        "copper_chest" => {
            return chest(
                "entity/chest/copper",
                "entity/chest/copper_left",
                "entity/chest/copper_right",
            );
        }
        "exposed_copper_chest" => {
            return chest(
                "entity/chest/copper_exposed",
                "entity/chest/copper_exposed_left",
                "entity/chest/copper_exposed_right",
            );
        }
        "weathered_copper_chest" => {
            return chest(
                "entity/chest/copper_weathered",
                "entity/chest/copper_weathered_left",
                "entity/chest/copper_weathered_right",
            );
        }
        "oxidized_copper_chest" => {
            return chest(
                "entity/chest/copper_oxidized",
                "entity/chest/copper_oxidized_left",
                "entity/chest/copper_oxidized_right",
            );
        }
        "shulker_box" => {
            return Kind::Shulker {
                texture: "entity/shulker/shulker".into(),
                facing: facing(),
            };
        }
        "bell" => return Kind::Bell,
        "decorated_pot" => {
            return Kind::DecoratedPot {
                y_rot: y_rot(facing()),
            };
        }
        "conduit" => return Kind::Conduit,
        "end_portal" => return Kind::EndPortal,
        "end_gateway" => return Kind::EndGateway { state: 0 },
        "enchanting_table" => return Kind::EnchantingTable,
        "lectern" => {
            return if prop("has_book").as_deref() == Some("true") {
                Kind::Lectern {
                    y_rot: y_rot(clockwise(facing())),
                }
            } else {
                Kind::None
            };
        }
        _ => {}
    }
    for (prefix, texture) in [
        ("copper_golem_statue", "entity/copper_golem/copper_golem"),
        (
            "exposed_copper_golem_statue",
            "entity/copper_golem/copper_golem_exposed",
        ),
        (
            "weathered_copper_golem_statue",
            "entity/copper_golem/copper_golem_weathered",
        ),
        (
            "oxidized_copper_golem_statue",
            "entity/copper_golem/copper_golem_oxidized",
        ),
    ] {
        if unwaxed == prefix {
            let pose = match prop("copper_golem_pose").as_deref() {
                Some("running") => "running",
                Some("sitting") => "sitting",
                Some("star") => "star",
                _ => "standing",
            };
            return Kind::CopperGolem {
                texture,
                pose,
                facing: facing(),
            };
        }
    }
    let skull = |base: &str| -> Option<SkullType> {
        Some(match base {
            "skeleton_skull" => SkullType::Skeleton,
            "wither_skeleton_skull" => SkullType::WitherSkeleton,
            "zombie_head" => SkullType::Zombie,
            "creeper_head" => SkullType::Creeper,
            "dragon_head" => SkullType::Dragon,
            "piglin_head" => SkullType::Piglin,
            "player_head" => SkullType::Player,
            _ => return None,
        })
    };
    if let Some(kind) = skull(name) {
        let segment = prop("rotation").and_then(|r| r.parse().ok()).unwrap_or(0);
        return Kind::Skull {
            kind,
            wall: None,
            segment,
        };
    }
    if name.contains("_wall_")
        && let Some(kind) = skull(&name.replace("_wall_", "_"))
    {
        return Kind::Skull {
            kind,
            wall: Some(facing()),
            segment: 0,
        };
    }
    if let Some((dye, rest)) = dye_prefix(name) {
        match rest {
            "shulker_box" => {
                return Kind::Shulker {
                    texture: format!("entity/shulker/shulker_{}", DYES[dye].0),
                    facing: facing(),
                };
            }
            "banner" => {
                let segment: f32 = prop("rotation").and_then(|r| r.parse().ok()).unwrap_or(0.0);
                return Kind::Banner {
                    color: DYES[dye].1,
                    wall: false,
                    angle: segment * 22.5,
                };
            }
            "wall_banner" => {
                return Kind::Banner {
                    color: DYES[dye].1,
                    wall: true,
                    angle: y_rot(facing()),
                };
            }
            _ => {}
        }
    }
    Kind::None
}

/// `BookModel.setupAnim` for `BookModel.State(openness, pageFlip1, pageFlip2)`.
fn book_pose(openness: f32, flip1: f32, flip2: f32) -> impl Fn(&str, &mut [f32; 9]) {
    move |name, p| match name {
        "left_lid" => p[4] = PI + openness,
        "right_lid" => p[4] = -openness,
        "left_pages" => {
            p[4] = openness;
            p[0] = openness.msin();
        }
        "right_pages" => {
            p[4] = -openness;
            p[0] = openness.msin();
        }
        "flip_page1" => {
            p[4] = openness - openness * 2.0 * flip1;
            p[0] = openness.msin();
        }
        "flip_page2" => {
            p[4] = openness - openness * 2.0 * flip2;
            p[0] = openness.msin();
        }
        _ => {}
    }
}
fn frac(x: f32) -> f32 {
    x - x.floor()
}

/// `StringSplitter.splitLines(...).get(0)` for plain text: the first line,
/// wrapped at the last space that fits or mid-word when none does.
fn first_line(line: &str, max_width: f32, advance: &dyn Fn(char) -> f32) -> String {
    let mut width = 0.0;
    let mut last_space = None;
    for (i, c) in line.char_indices() {
        if c == '\n' {
            return line[..i].to_owned();
        }
        if c == ' ' {
            last_space = Some(i);
        }
        width += advance(c);
        if width > max_width {
            let end = match last_space {
                Some(s) if s > 0 => s,
                _ if i > 0 => i,
                // The first char alone is too wide: take that one char.
                _ => c.len_utf8(),
            };
            return line[..end].to_owned();
        }
    }
    line.to_owned()
}

/// `AbstractSignRenderer.isOutlineVisible`: the camera within 16 blocks of
/// the sign's centre.
fn sign_text_near(pos: [i32; 3], camera: DVec3) -> bool {
    DVec3::new(
        f64::from(pos[0]) + 0.5,
        f64::from(pos[1]) + 0.5,
        f64::from(pos[2]) + 0.5,
    )
    .distance_squared(camera)
        < 256.0
}

/// `AbstractSignRenderer.submitSignText`: four centred lines, glowing text
/// full-bright with an 8x outline in the dark colour.
fn sign_text_calls(
    scene: &Scene,
    data: &crate::signs::SignData,
    text: &crate::signs::SignText,
    m: Mat4,
    packed: u8,
    near: bool,
) -> Vec<TextCall> {
    let line_height = data.line_height();
    let midpoint = (4.0 * line_height / 2.0).floor();
    let dark = 0xFF00_0000 | text.dark_color();
    let (color, outline, light) = if text.glowing {
        let c = text.text_color();
        (
            0xFF00_0000 | c,
            if c == 0 || near { dark } else { 0 },
            240 | 240 << 8,
        )
    } else {
        (dark, 0, vertex_light(packed))
    };
    let advance = |c: char| (scene.glyph)(c).1;
    let width = |s: &str| s.chars().map(advance).sum::<f32>().ceil();
    text.lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            // The line that fits, in the order it is drawn (getVisualOrder).
            let shown = first_line(line, data.max_line_width(), &advance);
            let shown = crate::bidi::visual_str(&shown).into_owned();
            TextCall {
                matrix: m,
                x: -((width(&shown) as i32) / 2) as f32,
                y: i as f32 * line_height - midpoint,
                text: shown,
                color,
                outline,
                light,
            }
        })
        .collect()
}

/// Draws a text call with the font atlas: 8x8 glyph cells, the outline as
/// eight one-pixel offsets under the text.
fn draw_text(scene: &Scene, call: &TextCall, out: &mut Vec<Vertex>) {
    let mut emit = |dx: f32, dy: f32, argb: u32| {
        let mut x = call.x + dx;
        let y0 = call.y + dy;
        for ch in call.text.chars() {
            let (cell, advance) = (scene.glyph)(ch);
            if let Some(r) = cell {
                let corners = [(x, y0 + 8.0), (x + 8.0, y0 + 8.0), (x + 8.0, y0), (x, y0)];
                let uvs = [[r.u0, r.v1], [r.u1, r.v1], [r.u1, r.v0], [r.u0, r.v0]];
                let c =
                    0xFF00_0000 | (argb & 255) << 16 | (argb >> 8 & 255) << 8 | (argb >> 16 & 255);
                for ((cx, cy), uv) in corners.into_iter().zip(uvs) {
                    out.push(Vertex {
                        normal: [0; 4],
                        pos: call
                            .matrix
                            .transform_point3(Vec3::new(cx, cy, 0.0))
                            .to_array(),
                        uv,
                        layer: 0,
                        color: c,
                        light: call.light,

                        glint_uv: [0.0, 0.0],
                    });
                }
            }
            x += advance;
        }
    };
    if call.outline != 0 {
        for (dx, dy) in [
            (-1.0, -1.0),
            (0.0, -1.0),
            (1.0, -1.0),
            (-1.0, 0.0),
            (1.0, 0.0),
            (-1.0, 1.0),
            (0.0, 1.0),
            (1.0, 1.0),
        ] {
            emit(dx, dy, call.outline);
        }
    }
    emit(0.0, 0.0, call.color);
}

/// Block-entity fields the level ticks, at one position (vanilla's own field
/// values; extraction to render state happens here as in each renderer).
#[derive(Clone, Debug, Default)]
pub struct BlockAnim {
    /// `ShulkerBoxBlockEntity` progress and progressOld.
    pub shulker: (f32, f32),
    /// `BellBlockEntity.ticks` and the click direction while `shaking`.
    pub bell_ticks: i32,
    pub bell_shake: Option<&'static str>,
    /// `ConduitBlockEntity` tickCount, isActive, isHunting and activeRotation.
    pub conduit: (i32, bool, bool, f32),
    /// `SkullBlockEntity` isAnimating and animationTickCount.
    pub skull: (bool, i32),
    /// `DecoratedPotBlockEntity` decorations as item ids: back, left, right, front.
    pub sherds: [Option<String>; 4],
    /// `DecoratedPotRenderState` wobble style (positive) and progress.
    pub wobble: Option<(bool, f32)>,
    /// `EnchantingTableBlockEntity` book fields.
    pub book: crate::block_entity_anim::EnchantingBook,
    /// `TheEndGatewayBlockEntity` age and teleportCooldown.
    pub gateway: (i64, i32),
}

/// One `submitText` call (`AbstractSignRenderer.submitSignText`).
#[derive(Clone, Debug, PartialEq)]
pub struct TextCall {
    pub matrix: Mat4,
    pub x: f32,
    pub y: f32,
    pub text: String,
    /// ARGB.
    pub color: u32,
    /// ARGB outline colour, 0 for none.
    pub outline: u32,
    /// Horizon packed light (sky*16 | block*16 << 8).
    pub light: u32,
}

/// Inputs from the game for one frame of block-entity rendering.
pub struct Scene<'a> {
    pub camera: DVec3,
    /// `Camera.rotation` (conduit eyes face the camera).
    pub orientation: Quat,
    /// Ticked block-entity state at a position.
    pub anim: &'a dyn Fn([i32; 3]) -> BlockAnim,
    /// When set, sign text is recorded here as text calls instead of drawn.
    pub texts: Option<&'a std::cell::RefCell<Vec<TextCall>>>,
    pub partial: f32,
    pub game_time: i64,
    /// `Level.getMaxY` (End gateway beams while spawning).
    pub max_y: i32,
    pub world: &'a World,
    pub sprites: &'a HashMap<String, Rect>,
    pub nether: bool,
    /// Chest lid openness (`ChestLidController.getOpenNess`) at a position.
    pub lid: &'a dyn Fn([i32; 3]) -> f32,
    /// Sign block-entity data at a position.
    pub sign: &'a dyn Fn([i32; 3]) -> Option<&'a crate::signs::SignData>,
    /// Banner layers at a position: pattern texture and RGB tint, bottom first.
    pub banner: &'a dyn Fn([i32; 3]) -> Vec<(String, u32)>,
    /// The skin sprite a `player_head` shows at a position (`SkullBlockRenderer`
    /// profile → skin): a downloaded `skins/<sha1>` slot or the profile's
    /// `DefaultPlayerSkin` fallback; None keeps `SKIN_BY_TYPE` (`DefaultPlayerSkin.getDefaultTexture`, slim steve).
    pub skull: &'a dyn Fn([i32; 3]) -> Option<String>,
    /// Font glyph cell (none for blank glyphs such as the space) and advance.
    pub glyph: &'a dyn Fn(char) -> (Option<Rect>, f32),
    /// Changes whenever `glyph` may answer differently (`Font::epoch`): sign
    /// text laid out under another value is laid out again.
    pub glyphs: u64,
}

pub struct BlockEntityRenderer {
    models: HashMap<&'static str, ModelPart>,
    kinds: Vec<Kind>,
    // Closed chest geometry: dimension lighting, chest part, horizontal facing.
    closed_chests: [[[Vec<Vertex>; 4]; 3]; 2],
    /// Item id to its `PROVIDES_POTTERY_PATTERN` side texture (`Sheets.DECORATED_POT_MAPPER`).
    pot_patterns: HashMap<String, String>,
    /// `Block.shouldRenderFace` for End gateway faces (its occlusion pair cache needs a lock).
    visibility: Option<Mutex<FaceVisibility>>,
    sign_texts: Mutex<SignTexts>,
}
/// Laid-out sign text by position. `AbstractSignRenderer` lays the text out
/// again each frame, glyph by glyph; its vertices only change with what is
/// in the key, so they are kept relative to the block and moved to the camera.
#[derive(Default)]
struct SignTexts {
    signs: crate::fast_hash::FxHashMap<[i32; 3], SignTextEntry>,
    /// Game time of the last sweep of signs out of view.
    swept: i64,
}
struct SignTextEntry {
    state: u32,
    data: crate::signs::SignData,
    packed: u8,
    /// Within the glowing text outline's distance (only kept for glowing text).
    near: bool,
    glyphs: u64,
    /// Game time it was last drawn.
    used: i64,
    vertices: Vec<Vertex>,
}
/// Game ticks a sign's text is kept after it was last drawn.
const SIGN_TEXT_KEEP: i64 = 100;
const MODELS: [(&str, &str); 27] = [
    ("chest", "minecraft:chest#main"),
    ("double_chest_left", "minecraft:double_chest_left#main"),
    ("double_chest_right", "minecraft:double_chest_right#main"),
    ("shulker_box", "minecraft:shulker_box#main"),
    ("standing_banner", "minecraft:standing_banner#main"),
    ("standing_banner_flag", "minecraft:standing_banner#flag"),
    ("wall_banner", "minecraft:wall_banner#main"),
    ("wall_banner_flag", "minecraft:wall_banner#flag"),
    ("skeleton_skull", "minecraft:skeleton_skull#main"),
    (
        "wither_skeleton_skull",
        "minecraft:wither_skeleton_skull#main",
    ),
    ("zombie_head", "minecraft:zombie_head#main"),
    ("creeper_head", "minecraft:creeper_head#main"),
    ("dragon_skull", "minecraft:dragon_skull#main"),
    ("piglin_head", "minecraft:piglin_head#main"),
    ("player_head", "minecraft:player_head#main"),
    ("bell", "minecraft:bell#main"),
    ("book", "minecraft:book#main"),
    ("conduit_shell", "minecraft:conduit#shell"),
    ("conduit_cage", "minecraft:conduit#cage"),
    ("conduit_wind", "minecraft:conduit#wind"),
    ("conduit_eye", "minecraft:conduit#eye"),
    ("decorated_pot_base", "minecraft:decorated_pot_base#main"),
    ("decorated_pot_sides", "minecraft:decorated_pot_sides#main"),
    ("copper_golem", "minecraft:copper_golem#main"),
    (
        "copper_golem_running",
        "minecraft:copper_golem_running#main",
    ),
    (
        "copper_golem_sitting",
        "minecraft:copper_golem_sitting#main",
    ),
    ("copper_golem_star", "minecraft:copper_golem_star#main"),
];
/// `BlockEntityRenderer.getViewDistance` default.
const VIEW_DISTANCE: f64 = 64.0;

impl BlockEntityRenderer {
    pub fn load(root: &Path, registry: &BlockRegistry) -> crate::Result<Self> {
        let all: Value = serde_json::from_slice(&std::fs::read(root.join("entity-models.json"))?)?;
        let models = MODELS
            .iter()
            .filter_map(|(key, layer)| all.get(*layer).map(|v| (*key, ModelPart::parse(v))))
            .collect();
        let mut renderer = Self::new(models, registry);
        renderer.visibility = Some(Mutex::new(crate::models::load_visibility(root, registry)?));
        // Sherd items: provides_pottery_pattern -> decorated_pot_pattern asset id.
        if let Ok(bytes) = std::fs::read(root.join("items.json")) {
            let items: Value = serde_json::from_slice(&bytes)?;
            for (item, components) in items.as_object().into_iter().flatten() {
                let Some(pattern) = components["minecraft:provides_pottery_pattern"].as_str()
                else {
                    continue;
                };
                let name = pattern.strip_prefix("minecraft:").unwrap_or(pattern);
                let file = root.join(format!("data/minecraft/decorated_pot_pattern/{name}.json"));
                let Ok(def) = std::fs::read(file) else {
                    continue;
                };
                let def: Value = serde_json::from_slice(&def)?;
                if let Some(asset) = def["asset_id"].as_str() {
                    let asset = asset.strip_prefix("minecraft:").unwrap_or(asset);
                    renderer
                        .pot_patterns
                        .insert(item.clone(), format!("entity/decorated_pot/{asset}"));
                }
            }
        }
        Ok(renderer)
    }
    fn new(models: HashMap<&'static str, ModelPart>, registry: &BlockRegistry) -> Self {
        let closed_chests = [false, true].map(|nether| {
            ["chest", "double_chest_left", "double_chest_right"].map(|name| {
                [0.0f32, 90.0, 180.0, 270.0].map(|y_rot| {
                    let mut vertices = Vec::new();
                    if let Some(model) = models.get(name) {
                        draw_part(
                            model,
                            "root",
                            around(
                                Mat4::from_rotation_y((-y_rot).to_radians()),
                                Vec3::new(0.5, 0.0, 0.5),
                            ),
                            &|name, p| {
                                if name == "lid" || name == "lock" {
                                    p[3] = crate::block_entity_anim::chest_lid_x_rot(0.0);
                                }
                            },
                            Paint {
                                sprite: Rect {
                                    u0: 0.0,
                                    v0: 0.0,
                                    u1: 1.0,
                                    v1: 1.0,
                                    w: 16,
                                    h: 16,
                                },
                                tint: 0xFFFFFF,
                                light: 0,
                                lights: diffuse_lights(nether),
                            },
                            &mut vertices,
                        );
                    }
                    vertices
                })
            })
        });
        let kinds = (0..registry.len() as u32)
            .map(|id| {
                registry.state(BlockStateId(id)).map_or(Kind::None, |s| {
                    let prop = |k: &str| s.properties.get(k).cloned();
                    match classify(&s.name, &prop) {
                        Kind::EndGateway { .. } => Kind::EndGateway { state: id },
                        kind => kind,
                    }
                })
            })
            .collect();
        Self {
            models,
            kinds,
            closed_chests,
            pot_patterns: HashMap::new(),
            visibility: None,
            sign_texts: Mutex::default(),
        }
    }
    fn kind(&self, state: u32) -> &Kind {
        self.kinds.get(state as usize).unwrap_or(&Kind::None)
    }
    pub fn render<'a>(
        &self,
        scene: &Scene,
        blocks: impl Iterator<Item = &'a ([i32; 3], u32)>,
        out: &mut Vec<Vertex>,
    ) {
        let lights = diffuse_lights(scene.nether);
        for (pos, state) in blocks {
            let kind = self.kind(*state);
            if matches!(kind, Kind::None) {
                continue;
            }
            let center = DVec3::new(
                f64::from(pos[0]) + 0.5,
                f64::from(pos[1]) + 0.5,
                f64::from(pos[2]) + 0.5,
            );
            // BlockEntityRenderer.getViewDistance: 256 for End gateways.
            let distance = if matches!(kind, Kind::EndGateway { .. }) {
                256.0
            } else {
                VIEW_DISTANCE
            };
            if center.distance_squared(scene.camera) > distance * distance {
                continue;
            }
            let origin = (DVec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]))
                - scene.camera)
                .as_vec3();
            let packed = scene.world.light(pos[0], pos[1], pos[2]);
            if matches!(kind, Kind::Sign { .. }) && scene.texts.is_none() {
                self.sign_text(scene, *pos, *state, origin, packed, out);
                continue;
            }
            let base = Mat4::from_translation(origin);
            self.render_one(scene, *pos, kind, base, packed, lights, false, true, out);
        }
    }
    /// A sign's text from the cache, laid out again when its data, light,
    /// state, outline or font changed.
    fn sign_text(
        &self,
        scene: &Scene,
        pos: [i32; 3],
        state: u32,
        origin: Vec3,
        packed: u8,
        out: &mut Vec<Vertex>,
    ) {
        let Some(data) = (scene.sign)(pos) else {
            return;
        };
        let near = (data.front.glowing || data.back.glowing) && sign_text_near(pos, scene.camera);
        let Ok(mut cache) = self.sign_texts.lock() else {
            return;
        };
        let now = scene.game_time;
        if (now - cache.swept).abs() >= SIGN_TEXT_KEEP {
            cache.swept = now;
            cache
                .signs
                .retain(|_, e| (now - SIGN_TEXT_KEEP..=now).contains(&e.used));
        }
        let glyphs = scene.glyphs;
        let signs = &mut cache.signs;
        if !signs.get(&pos).is_some_and(|e| {
            (e.state, e.packed, e.near, e.glyphs) == (state, packed, near, glyphs)
                && e.data == *data
        }) {
            let mut vertices = signs.remove(&pos).map_or_else(Vec::new, |e| e.vertices);
            vertices.clear();
            self.render_one(
                scene,
                pos,
                self.kind(state),
                Mat4::IDENTITY,
                packed,
                diffuse_lights(scene.nether),
                false,
                true,
                &mut vertices,
            );
            signs.insert(
                pos,
                SignTextEntry {
                    state,
                    data: data.clone(),
                    packed,
                    near,
                    glyphs,
                    used: now,
                    vertices,
                },
            );
        }
        let Some(entry) = signs.get_mut(&pos) else {
            return;
        };
        entry.used = now;
        out.extend(entry.vertices.iter().map(|v| Vertex {
            pos: (Vec3::from_array(v.pos) + origin).to_array(),
            ..*v
        }));
    }
    /// One block entity at `pos` drawn under `base` with the given packed light
    /// (the parity fixture's entry point).
    pub fn render_block(
        &self,
        scene: &Scene,
        pos: [i32; 3],
        state: u32,
        base: Mat4,
        packed: u8,
        out: &mut Vec<Vertex>,
    ) {
        let kind = self.kind(state);
        let lights = diffuse_lights(scene.nether);
        self.render_one(scene, pos, kind, base, packed, lights, false, false, out);
    }
    /// `TheEndGatewayBlockEntity.shouldRenderFace` for each face of the gateway
    /// at `pos`: the faces its portal is drawn on.
    pub fn gateway_faces(
        &self,
        world: &World,
        state: BlockStateId,
        pos: [i32; 3],
    ) -> Vec<Direction> {
        Direction::ALL
            .into_iter()
            .filter(|&d| {
                let [dx, dy, dz] = d.normal();
                let neighbor = world.block(pos[0] + dx, pos[1] + dy, pos[2] + dz);
                self.visibility.as_ref().is_none_or(|v| {
                    v.lock()
                        .map(|mut v| {
                            v.should_render_face(state, neighbor, d, true)
                                .unwrap_or(true)
                        })
                        .unwrap_or(true)
                })
            })
            .collect()
    }
    /// Whether this block state has a block-entity renderer here.
    pub fn renders(&self, state: u32) -> bool {
        !matches!(self.kind(state), Kind::None)
    }
    /// ChestRenderer and BannerRenderer submit their animated models for crumbling.
    pub fn render_breaking(
        &self,
        scene: &Scene,
        pos: [i32; 3],
        state: u32,
        stage: [f32; 2],
        out: &mut Vec<Vertex>,
    ) {
        if !matches!(self.kind(state), Kind::Banner { .. } | Kind::Chest { .. }) {
            return;
        }
        let origin = (DVec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]))
            - scene.camera)
            .as_vec3();
        let start = out.len();
        self.render_one(
            scene,
            pos,
            self.kind(state),
            Mat4::from_translation(origin),
            0xff,
            diffuse_lights(scene.nether),
            true,
            false,
            out,
        );
        crate::breaking::entity_geometry(&mut out[start..], origin, stage);
    }
    /// `ChestRenderer.extractRenderState`'s openness for the chest of `state`
    /// at `pos`: `ChestBlock.combine` (`DoubleBlockCombiner.combineWithNeigbour`)
    /// pairs a double half with its neighbour only when that is the same
    /// block, the other half and faces the same way; `opennessCombiner` then
    /// takes the more open of both. Also returns the paired half's position.
    /// `lid` is `getOpenNess(partial)` at a position; non-chests give 0.
    pub fn chest_openness_of(
        &self,
        pos: [i32; 3],
        state: u32,
        block_at: &dyn Fn([i32; 3]) -> u32,
        lid: &dyn Fn([i32; 3]) -> f32,
    ) -> (f32, Option<[i32; 3]>) {
        self.chest_openness(pos, self.kind(state), block_at, lid)
    }
    fn chest_openness(
        &self,
        pos: [i32; 3],
        kind: &Kind,
        block_at: &dyn Fn([i32; 3]) -> u32,
        lid: &dyn Fn([i32; 3]) -> f32,
    ) -> (f32, Option<[i32; 3]>) {
        let Kind::Chest {
            block,
            part,
            y_rot,
            double_with,
            ..
        } = kind
        else {
            return (0.0, None);
        };
        let open = lid(pos);
        let Some(d) = double_with else {
            return (open, None);
        };
        let other = [pos[0] + d[0], pos[1] + d[1], pos[2] + d[2]];
        match self.kind(block_at(other)) {
            Kind::Chest {
                block: other_block,
                part: other_part,
                y_rot: other_y_rot,
                ..
            } if other_block == block
                && *other_part != ChestPart::Single
                && other_part != part
                && other_y_rot == y_rot =>
            {
                let other_open = lid(other);
                // The right half is the first.
                let open = if *part == ChestPart::Right {
                    crate::block_entity_anim::combined_openness(open, Some(other_open))
                } else {
                    crate::block_entity_anim::combined_openness(other_open, Some(open))
                };
                (open, Some(other))
            }
            _ => (open, None),
        }
    }
    /// One block entity of `state` drawn under `base` (minecart contents).
    pub fn render_state(
        &self,
        scene: &Scene,
        state: u32,
        base: Mat4,
        packed: u8,
        out: &mut Vec<Vertex>,
    ) {
        let kind = self.kind(state);
        if !matches!(kind, Kind::None) {
            let lights = diffuse_lights(scene.nether);
            self.render_one(
                scene,
                [i32::MAX; 3],
                kind,
                base,
                packed,
                lights,
                false,
                false,
                out,
            );
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn render_one(
        &self,
        scene: &Scene,
        pos: [i32; 3],
        kind: &Kind,
        base: Mat4,
        packed: u8,
        lights: (Vec3, Vec3),
        crumbling: bool,
        world_chests: bool,
        out: &mut Vec<Vertex>,
    ) {
        let sprite = |name: &str| scene.sprites.get(name).copied();
        let paint = |name: &str, tint: u32, packed: u8| {
            let rect = if crumbling {
                Some(Rect {
                    u0: 0.0,
                    v0: 0.0,
                    u1: 1.0,
                    v1: 1.0,
                    w: 16,
                    h: 16,
                })
            } else {
                sprite(name)
            };
            rect.map(|s| Paint {
                sprite: s,
                tint,
                light: vertex_light(packed),
                lights,
            })
        };
        let none = |_: &str, _: &mut [f32; 9]| {};
        match kind {
            Kind::None => {}
            Kind::Sign {
                hanging,
                wall,
                angle,
            } => {
                let Some(data) = (scene.sign)(pos) else {
                    return;
                };
                let s = Vec3::new(1.0, -1.0, 1.0);
                for (front, text) in [(true, &data.front), (false, &data.back)] {
                    if text.is_empty() {
                        continue;
                    }
                    let mut m = base;
                    if *hanging {
                        // HangingSignRenderer.textTransformation.
                        m *= Mat4::from_translation(Vec3::new(0.5, 0.9375, 0.5))
                            * Mat4::from_rotation_y((-angle).to_radians())
                            * Mat4::from_translation(Vec3::new(0.0, -0.3125, 0.0));
                        if !front {
                            m *= Mat4::from_rotation_y(PI);
                        }
                        m *= Mat4::from_translation(Vec3::new(0.0, -0.32, 0.073))
                            * Mat4::from_scale(s * 0.014_062_5);
                    } else {
                        // StandingSignRenderer.textTransformation.
                        m *= Mat4::from_translation(Vec3::splat(0.5))
                            * Mat4::from_rotation_y((-angle).to_radians());
                        if *wall {
                            m *= Mat4::from_translation(Vec3::new(0.0, -0.3125, -0.4375));
                        }
                        if !front {
                            m *= Mat4::from_rotation_y(PI);
                        }
                        m *= Mat4::from_translation(Vec3::new(0.0, 1.0 / 3.0, 0.046_666_667))
                            * Mat4::from_scale(s * 0.010_416_667);
                    }
                    let near = sign_text_near(pos, scene.camera);
                    let calls = sign_text_calls(scene, data, text, m, packed, near);
                    match scene.texts {
                        Some(sink) => sink.borrow_mut().extend(calls),
                        None => calls.iter().for_each(|c| draw_text(scene, c, out)),
                    }
                }
            }
            Kind::Chest {
                block,
                texture,
                part,
                y_rot,
                ..
            } => {
                // ChestRenderer.getChestMaterial: normal and trapped chests use the christmas
                // sprites on 24-26 December. Copper and ender chests keep theirs.
                let texture = if matches!(block.as_str(), "chest" | "trapped_chest")
                    && extended_christmas()
                {
                    match part {
                        ChestPart::Single => "entity/chest/christmas",
                        ChestPart::Left => "entity/chest/christmas_left",
                        ChestPart::Right => "entity/chest/christmas_right",
                    }
                } else {
                    *texture
                };
                let model = match part {
                    ChestPart::Single => "chest",
                    ChestPart::Left => "double_chest_left",
                    ChestPart::Right => "double_chest_right",
                };
                let Some(model) = self.models.get(model) else {
                    return;
                };
                let (open, other) = self.chest_openness(
                    pos,
                    kind,
                    &|p| scene.world.block(p[0], p[1], p[2]).0,
                    scene.lid,
                );
                // BrightnessCombiner: the brighter of both halves, per channel.
                let mut packed = packed;
                if let Some(other) = other {
                    let l = scene.world.light(other[0], other[1], other[2]);
                    packed = (packed >> 4).max(l >> 4) << 4 | (packed & 15).max(l & 15);
                }
                if world_chests && open == 0.0 {
                    if let Some(paint) = paint(texture, 0xFFFFFF, packed) {
                        let vertices = &self.closed_chests[usize::from(scene.nether)]
                            [*part as usize][*y_rot as usize / 90];
                        let origin = base.w_axis.truncate();
                        out.extend(vertices.iter().map(|v| Vertex {
                            pos: (Vec3::from_array(v.pos) + origin).to_array(),
                            uv: [
                                paint.sprite.u0 + (paint.sprite.u1 - paint.sprite.u0) * v.uv[0],
                                paint.sprite.v0 + (paint.sprite.v1 - paint.sprite.v0) * v.uv[1],
                            ],
                            light: paint.light,
                            ..*v
                        }));
                    }
                    return;
                }
                let m = base
                    * around(
                        Mat4::from_rotation_y((-y_rot).to_radians()),
                        Vec3::new(0.5, 0.0, 0.5),
                    );
                let lid = crate::block_entity_anim::chest_lid_x_rot(open);
                let adjust = move |name: &str, p: &mut [f32; 9]| {
                    if name == "lid" || name == "lock" {
                        p[3] = lid;
                    }
                };
                if let Some(paint) = paint(texture, 0xFFFFFF, packed) {
                    draw_part(model, "root", m, &adjust, paint, out);
                }
            }
            Kind::Shulker { texture, facing } => {
                let (Some(model), Some(paint)) = (
                    self.models.get("shulker_box"),
                    paint(texture, 0xFFFFFF, packed),
                ) else {
                    return;
                };
                let m = base
                    * Mat4::from_translation(Vec3::splat(0.5))
                    * Mat4::from_scale(Vec3::splat(0.9995))
                    * Mat4::from_quat(direction_rotation(facing))
                    * Mat4::from_scale(Vec3::new(1.0, -1.0, -1.0))
                    * Mat4::from_translation(Vec3::new(0.0, -1.0, 0.0));
                // ShulkerBoxModel.setupAnim(getProgress(partial)): the lid rises and turns.
                let (progress, old) = (scene.anim)(pos).shulker;
                let progress = old + scene.partial * (progress - old);
                let lid = move |name: &str, p: &mut [f32; 9]| {
                    if name == "lid" {
                        p[0] = 0.0;
                        p[1] = 24.0 - progress * 0.5 * 16.0;
                        p[2] = 0.0;
                        p[4] = 270.0 * progress * (PI / 180.0);
                    }
                };
                draw_part(model, "root", m, &lid, paint, out);
            }
            Kind::Banner { color, wall, angle } => {
                let (main, flag) = if *wall {
                    ("wall_banner", "wall_banner_flag")
                } else {
                    ("standing_banner", "standing_banner_flag")
                };
                let (Some(main), Some(flag)) = (self.models.get(main), self.models.get(flag))
                else {
                    return;
                };
                let m = base
                    * Mat4::from_translation(Vec3::new(0.5, 0.0, 0.5))
                    * Mat4::from_rotation_y((-angle).to_radians())
                    * Mat4::from_scale(Vec3::new(2.0 / 3.0, -2.0 / 3.0, -2.0 / 3.0));
                let time = scene.game_time;
                let phase = ((i64::from(pos[0]) * 7
                    + i64::from(pos[1]) * 9
                    + i64::from(pos[2]) * 13
                    + time)
                    .rem_euclid(100) as f32
                    + scene.partial)
                    / 100.0;
                let wave = move |name: &str, p: &mut [f32; 9]| {
                    if name == "flag" {
                        p[3] = (-0.0125 + 0.01 * (2.0 * PI * phase).mcos()) * PI;
                    }
                };
                if let Some(paint) = paint("entity/banner/banner_base", 0xFFFFFF, packed) {
                    draw_part(main, "root", m, &none, paint, out);
                    draw_part(flag, "root", m, &wave, paint, out);
                }
                // Pattern passes must not multiply the crumbling decal.
                if crumbling {
                    return;
                }
                // BannerRenderer.submitPatterns: the base pattern layer in the dye colour,
                // then up to 16 pattern layers in order.
                if let Some(paint) = paint("entity/banner/base", *color, packed) {
                    draw_part(flag, "root", m, &wave, paint, out);
                }
                for (texture, tint) in (scene.banner)(pos).iter().take(16) {
                    if let Some(paint) = paint(texture, *tint, packed) {
                        draw_part(flag, "root", m, &wave, paint, out);
                    }
                }
            }
            Kind::Skull {
                kind,
                wall,
                segment,
            } => {
                let (model, texture) = match kind {
                    SkullType::Skeleton => ("skeleton_skull", "entity/skeleton/skeleton"),
                    SkullType::WitherSkeleton => {
                        ("wither_skeleton_skull", "entity/skeleton/wither_skeleton")
                    }
                    SkullType::Zombie => ("zombie_head", "entity/zombie/zombie"),
                    SkullType::Creeper => ("creeper_head", "entity/creeper/creeper"),
                    SkullType::Dragon => ("dragon_skull", "entity/enderdragon/dragon"),
                    SkullType::Piglin => ("piglin_head", "entity/piglin/piglin"),
                    SkullType::Player => ("player_head", "entity/player/slim/steve"),
                };
                // SkullBlockRenderer.getRenderType: the profile's skin when it
                // resolves, `SKIN_BY_TYPE` (steve) otherwise.
                let skin = if *kind == SkullType::Player {
                    (scene.skull)(pos)
                } else {
                    None
                };
                let texture = skin.as_deref().unwrap_or(texture);
                let (Some(model), Some(paint)) =
                    (self.models.get(model), paint(texture, 0xFFFFFF, packed))
                else {
                    return;
                };
                let m = match wall {
                    Some(facing) => {
                        let s = step(facing);
                        base * Mat4::from_translation(Vec3::new(
                            0.5 - s[0] as f32 * 0.25,
                            0.25,
                            0.5 - s[2] as f32 * 0.25,
                        )) * Mat4::from_rotation_y((-y_rot(opposite(facing))).to_radians())
                            * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0))
                    }
                    None => {
                        base * Mat4::from_translation(Vec3::new(0.5, 0.0, 0.5))
                            * Mat4::from_rotation_y((-(f32::from(*segment) * 22.5)).to_radians())
                            * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0))
                    }
                };
                // SkullBlockEntity.getAnimation: the dragon jaw and piglin ears
                // (DragonHeadModel / PiglinHeadModel use double-precision Math).
                let (animating, count) = (scene.anim)(pos).skull;
                let t = count as f32 + if animating { scene.partial } else { 0.0 };
                let anim = move |name: &str, p: &mut [f32; 9]| match name {
                    "jaw" => p[3] = (f64::from(t * PI * 0.2).sin() + 1.0) as f32 * 0.2,
                    "left_ear" => {
                        p[5] = -((f64::from(t * PI * 0.2 * 1.2).cos() + 2.5) as f32) * 0.2;
                    }
                    "right_ear" => p[5] = (f64::from(t * PI * 0.2).cos() + 2.5) as f32 * 0.2,
                    _ => {}
                };
                draw_part(model, "root", m, &anim, paint, out);
            }
            Kind::Bell => {
                let (Some(model), Some(paint)) = (
                    self.models.get("bell"),
                    paint("entity/bell/bell_body", 0xFFFFFF, packed),
                ) else {
                    return;
                };
                // BellModel.setupAnim(State(ticks + partial, shakeDirection)).
                let a = (scene.anim)(pos);
                let ticks = a.bell_ticks as f32 + scene.partial;
                let (mut x_rot, mut z_rot) = (0.0, 0.0);
                if let Some(dir) = a.bell_shake {
                    let base_rot = (ticks / PI).msin() / (4.0 + ticks / 3.0);
                    match dir {
                        "north" => x_rot = -base_rot,
                        "south" => x_rot = base_rot,
                        "east" => z_rot = -base_rot,
                        "west" => z_rot = base_rot,
                        _ => {}
                    }
                }
                let shake = move |name: &str, p: &mut [f32; 9]| {
                    if name == "bell_body" {
                        p[3] = x_rot;
                        p[5] = z_rot;
                    }
                };
                draw_part(model, "root", base, &shake, paint, out);
            }
            Kind::DecoratedPot { y_rot } => {
                let (Some(base_model), Some(sides)) = (
                    self.models.get("decorated_pot_base"),
                    self.models.get("decorated_pot_sides"),
                ) else {
                    return;
                };
                let a = (scene.anim)(pos);
                let mut m = base
                    * around(
                        Mat4::from_rotation_y((180.0 - y_rot).to_radians()),
                        Vec3::splat(0.5),
                    );
                // DecoratedPotRenderer.submit: the wobble tilts around the base centre.
                let pivot = Vec3::new(0.5, 0.0, 0.5);
                if let Some((positive, progress)) = a.wobble
                    && (0.0..=1.0).contains(&progress)
                {
                    if positive {
                        let delta = progress * (PI * 2.0);
                        let tilt_x = -1.5 * (delta.mcos() + 0.5) * (delta / 2.0).msin();
                        m *= around(Mat4::from_rotation_x(tilt_x * 0.015_625), pivot);
                        let tilt_z = delta.msin();
                        m *= around(Mat4::from_rotation_z(tilt_z * 0.015_625), pivot);
                    } else {
                        let turn = (-progress * 3.0 * PI).msin() * 0.125;
                        let decay = 1.0 - progress;
                        m *= around(Mat4::from_rotation_y(turn * decay), pivot);
                    }
                }
                let first = out.len();
                if let Some(paint) =
                    paint("entity/decorated_pot/decorated_pot_base", 0xFFFFFF, packed)
                {
                    draw_part(base_model, "root", m, &none, paint, out);
                }
                // PotDecorations (back, left, right, front); items without a pattern use the blank side.
                for (side, item) in ["back", "left", "right", "front"].iter().zip(&a.sherds) {
                    let texture = item
                        .as_ref()
                        .and_then(|i| self.pot_patterns.get(i))
                        .map_or("entity/decorated_pot/decorated_pot_side", String::as_str);
                    if let (Some(part), Some(paint)) =
                        (sides.child(side), paint(texture, 0xFFFFFF, packed))
                    {
                        draw_part(part, side, m, &none, paint, out);
                    }
                }
                // `DecoratedPotRenderer` submits every part with `entitySolid`, which culls
                // back faces: the flat `top` and `bottom` planes show one side each.
                for vertex in &mut out[first..] {
                    vertex.normal = CULL_BACK;
                }
            }
            Kind::EndPortal => {
                // TheEndPortalRenderer.TRANSFORMATION: y from 6/16 to 12/16; only the
                // vertical faces (TheEndPortalBlockEntity.shouldRenderFace).
                let m = base
                    * Mat4::from_translation(Vec3::new(0.0, 0.375, 0.0))
                    * Mat4::from_scale(Vec3::new(1.0, 0.375, 1.0));
                portal_cube(m, &[Direction::Down, Direction::Up], END_PORTAL_LAYER, out);
            }
            Kind::EndGateway { state } => {
                let anim = (scene.anim)(pos);
                let b = gateway_beam(anim.gateway, scene.partial, scene.max_y, scene.game_time);
                if b.height > 0
                    && let Some(s) = sprite("entity/end_portal/end_gateway_beam")
                {
                    beam(
                        base,
                        BeamShape {
                            scale: b.scale,
                            time: b.animation_time,
                            start: -b.height,
                            height: b.height * 2,
                            color: b.color,
                            solid_radius: 0.15,
                            glow_radius: 0.175,
                        },
                        s,
                        out,
                    );
                }
                // AbstractEndPortalRenderer.extractRenderState.
                let faces = self.gateway_faces(scene.world, BlockStateId(*state), pos);
                portal_cube(base, &faces, END_GATEWAY_LAYER, out);
            }
            Kind::Conduit => {
                let (ticks, active, hunting, raw_rotation) = (scene.anim)(pos).conduit;
                // ConduitRenderer.extractRenderState.
                let rotation = (raw_rotation + if active { scene.partial } else { 0.0 }) * -0.0375;
                let tex = |name: &str| paint(name, 0xFFFFFF, packed);
                if !active {
                    let (Some(model), Some(paint)) =
                        (self.models.get("conduit_shell"), tex("entity/conduit/base"))
                    else {
                        return;
                    };
                    let m = base
                        * Mat4::from_translation(Vec3::splat(0.5))
                        * Mat4::from_rotation_y(rotation.to_radians());
                    draw_part(model, "root", m, &none, paint, out);
                    return;
                }
                let anim_time = ticks as f32 + scene.partial;
                let phase = ticks / 66 % 3;
                let degrees = rotation * (180.0 / PI);
                let hh = (anim_time * 0.1).msin() / 2.0 + 0.5;
                let hh = hh * hh + hh;
                let lift = Vec3::new(0.5, 0.3 + hh * 0.2, 0.5);
                if let (Some(model), Some(paint)) =
                    (self.models.get("conduit_cage"), tex("entity/conduit/cage"))
                {
                    let axis = Vec3::new(0.5, 1.0, 0.5).normalize();
                    let m = base
                        * Mat4::from_translation(lift)
                        * Mat4::from_quat(Quat::from_axis_angle(axis, degrees * (PI / 180.0)));
                    draw_part(model, "root", m, &none, paint, out);
                }
                let wind = if phase == 1 {
                    "entity/conduit/wind_vertical"
                } else {
                    "entity/conduit/wind"
                };
                if let (Some(model), Some(paint)) = (self.models.get("conduit_wind"), tex(wind)) {
                    let turn = match phase {
                        1 => Mat4::from_rotation_x(PI / 2.0),
                        2 => Mat4::from_rotation_z(PI / 2.0),
                        _ => Mat4::IDENTITY,
                    };
                    let center = base * Mat4::from_translation(Vec3::splat(0.5));
                    draw_part(model, "root", center * turn, &none, paint, out);
                    let inner = center
                        * Mat4::from_scale(Vec3::splat(0.875))
                        * Mat4::from_quat(Quat::from_euler(glam::EulerRot::XYZ, PI, 0.0, PI));
                    draw_part(model, "root", inner, &none, paint, out);
                }
                let eye = if hunting {
                    "entity/conduit/open_eye"
                } else {
                    "entity/conduit/closed_eye"
                };
                if let (Some(model), Some(paint)) = (self.models.get("conduit_eye"), tex(eye)) {
                    let m = base
                        * Mat4::from_translation(lift)
                        * Mat4::from_scale(Vec3::splat(0.5))
                        * Mat4::from_quat(scene.orientation)
                        * Mat4::from_quat(Quat::from_rotation_z(PI) * Quat::from_rotation_y(PI))
                        * Mat4::from_scale(Vec3::splat(1.333_333_4));
                    draw_part(model, "root", m, &none, paint, out);
                }
            }
            Kind::EnchantingTable => {
                let (Some(model), Some(paint)) = (
                    self.models.get("book"),
                    paint("entity/enchantment/enchanting_table_book", 0xFFFFFF, packed),
                ) else {
                    return;
                };
                // EnchantTableRenderer.extractRenderState and submit.
                let b = (scene.anim)(pos).book.render(scene.partial);
                let m = base
                    * Mat4::from_translation(Vec3::new(0.5, 0.75, 0.5))
                    * Mat4::from_translation(Vec3::new(
                        0.0,
                        0.1 + (b.time * 0.1).msin() * 0.01,
                        0.0,
                    ))
                    * Mat4::from_rotation_y(-b.y_rot)
                    * Mat4::from_rotation_z(80f32.to_radians());
                let (openness, ff1, ff2) = (b.openness, b.page_flip1, b.page_flip2);
                draw_part(model, "root", m, &book_pose(openness, ff1, ff2), paint, out);
            }
            Kind::Lectern { y_rot } => {
                let (Some(model), Some(paint)) = (
                    self.models.get("book"),
                    paint("entity/enchantment/enchanting_table_book", 0xFFFFFF, packed),
                ) else {
                    return;
                };
                let m = base
                    * Mat4::from_translation(Vec3::new(0.5, 1.0625, 0.5))
                    * Mat4::from_rotation_y((-y_rot).to_radians())
                    * Mat4::from_rotation_z(67.5f32.to_radians())
                    * Mat4::from_translation(Vec3::new(0.0, -0.125, 0.0));
                // LecternRenderer.BOOK_STATE = forAnimation(0, 0.1, 0.9, 1.2).
                draw_part(
                    model,
                    "root",
                    m,
                    &book_pose(1.25 * 1.2, 0.1, 0.9),
                    paint,
                    out,
                );
            }
            Kind::CopperGolem {
                texture,
                pose,
                facing,
            } => {
                let key = match *pose {
                    "running" => "copper_golem_running",
                    "sitting" => "copper_golem_sitting",
                    "star" => "copper_golem_star",
                    _ => "copper_golem",
                };
                let (Some(model), Some(paint)) =
                    (self.models.get(key), paint(texture, 0xFFFFFF, packed))
                else {
                    return;
                };
                let m = base
                    * Mat4::from_translation(Vec3::new(0.5, 0.0, 0.5))
                    * Mat4::from_rotation_y((-y_rot(opposite(facing))).to_radians());
                // CopperGolemStatueModel.setupAnim: root.y = 0, root.zRot = PI.
                let statue = |name: &str, p: &mut [f32; 9]| {
                    if name == "root" {
                        p[1] = 0.0;
                        p[5] = PI;
                    }
                };
                draw_part(model, "root", m, &statue, paint, out);
            }
        }
    }
}

/// `BeaconRenderer.submit`: for each beam section (ARGB colour, height) the
/// rotating solid beam and its translucent glow, the last section 2048
/// blocks tall. `origin` is the beacon block's corner relative to the camera,
/// `time` the animation time (game time mod 40 plus partial tick) and `scale`
/// the radius scale.
pub fn beacon_beam(
    origin: Vec3,
    sections: &[(u32, i32)],
    time: f32,
    scale: f32,
    s: Rect,
    out: &mut Vec<Vertex>,
) {
    let base = Mat4::from_translation(origin);
    let mut start = 0i32;
    for (i, &(color, height)) in sections.iter().enumerate() {
        let height = if i == sections.len() - 1 {
            2048
        } else {
            height
        };
        let shape = BeamShape {
            scale: 1.0,
            time,
            start,
            height,
            color,
            solid_radius: 0.2 * scale,
            glow_radius: 0.25 * scale,
        };
        beam(base, shape, s, out);
        start += height;
    }
}
/// The arguments of `BeaconRenderer.submitBeaconBeam`.
pub struct BeamShape {
    /// Scales the v range (End gateways shrink and grow their beam with it).
    pub scale: f32,
    /// Animation time: game time mod 40 plus the partial tick.
    pub time: f32,
    pub start: i32,
    pub height: i32,
    /// RGB.
    pub color: u32,
    pub solid_radius: f32,
    pub glow_radius: f32,
}
/// `BeaconRenderer.submitBeaconBeam` under `base` (the block's corner): the
/// rotating solid beam and its translucent glow. The sprite's v range rides
/// in the light bytes (the entity shader wraps v for layer 6).
pub fn beam(base: Mat4, b: BeamShape, s: Rect, out: &mut Vec<Vertex>) {
    let q = |v: f32| (v * 65535.0).round().clamp(0.0, 65535.0) as u32;
    let wrap = q(s.v0) | q(s.v1 - s.v0) << 16;
    let color = b.color;
    let end = b.start + b.height;
    let scroll = if b.height < 0 { b.time } else { -b.time };
    let v_off = frac(scroll * 0.2 - (scroll * 0.1).floor());
    let abgr =
        |a: u32| a << 24 | (color & 255) << 16 | (color >> 8 & 255) << 8 | (color >> 16 & 255);
    // Solid beam, rotated with time.
    let r = b.solid_radius;
    let rot = glam::Mat3::from_rotation_y((b.time * 2.25 - 45.0).to_radians());
    let corners = [
        rot * Vec3::new(0.0, 0.0, r),
        rot * Vec3::new(r, 0.0, 0.0),
        rot * Vec3::new(-r, 0.0, 0.0),
        rot * Vec3::new(0.0, 0.0, -r),
    ];
    let v2 = -1.0 + v_off;
    let v1 = b.height as f32 * b.scale * (0.5 / r) + v2;
    let part = BeamPart {
        base,
        start: b.start,
        end,
        s,
        wrap,
    };
    part.emit(out, corners, [v1, v2], abgr(255));
    // Glow, translucent.
    let g = b.glow_radius;
    let corners = [
        Vec3::new(-g, 0.0, -g),
        Vec3::new(g, 0.0, -g),
        Vec3::new(-g, 0.0, g),
        Vec3::new(g, 0.0, g),
    ];
    let v1 = b.height as f32 * b.scale + v2;
    part.emit(out, corners, [v1, v2], abgr(32));
}
struct BeamPart {
    base: Mat4,
    start: i32,
    end: i32,
    s: Rect,
    wrap: u32,
}
impl BeamPart {
    /// `BeaconRenderer.renderPart`: four quads between the corners (wn, en, ws, es).
    fn emit(
        &self,
        out: &mut Vec<Vertex>,
        [wn, en, ws, es]: [Vec3; 4],
        [v1, v2]: [f32; 2],
        color: u32,
    ) {
        let s = self.s;
        for (a, b) in [(wn, en), (es, ws), (en, es), (ws, wn)] {
            let at = |c: Vec3, y: i32| {
                self.base
                    .transform_point3(Vec3::new(0.5, 0.0, 0.5) + c + Vec3::new(0.0, y as f32, 0.0))
            };
            for (p, uv) in [
                (at(a, self.end), [s.u1, v1]),
                (at(a, self.start), [s.u1, v2]),
                (at(b, self.start), [s.u0, v2]),
                (at(b, self.end), [s.u0, v1]),
            ] {
                out.push(Vertex {
                    normal: [0; 4],
                    pos: p.to_array(),
                    uv,
                    layer: 6,
                    color,
                    light: self.wrap,

                    glint_uv: [0.0, 0.0],
                });
            }
        }
    }
}

/// `EndGatewayRenderState`'s beam fields (zero when there is no beam).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GatewayBeam {
    pub height: i32,
    pub scale: f32,
    pub color: u32,
    pub animation_time: f32,
}

/// `TheEndGatewayRenderer.extractRenderState` from the block entity's age and
/// teleport cooldown, at `partial`, for a level whose top is `max_y`.
pub fn gateway_beam(
    (age, cooldown): (i64, i32),
    partial: f32,
    max_y: i32,
    game_time: i64,
) -> GatewayBeam {
    let spawning = age < 200;
    if !spawning && cooldown <= 0 {
        return GatewayBeam::default();
    }
    let percent = if spawning {
        ((age as f32 + partial) / 200.0).clamp(0.0, 1.0)
    } else {
        1.0 - ((cooldown as f32 - partial) / 40.0).clamp(0.0, 1.0)
    };
    let distance = if spawning { f64::from(max_y) } else { 50.0 };
    let scale = trig::sin(f64::from(percent * PI));
    GatewayBeam {
        height: (f64::from(scale) * distance).floor() as i32,
        scale,
        // DyeColor.MAGENTA / PURPLE getTextureDiffuseColor.
        color: if spawning { 0xFFC7_4EBD } else { 0xFF89_32B8 },
        animation_time: game_time.rem_euclid(40) as f32 + partial,
    }
}

/// Entity-shader layers for `rendertype_end_portal`: end portals draw 15
/// layers of `end_portal.png` over `end_sky.png`, End gateways 16.
pub const END_PORTAL_LAYER: u32 = 9;
pub const END_GATEWAY_LAYER: u32 = 10;
/// `AbstractEndPortalRenderer.FACES`: the unit cube's corners for each face
/// (`Direction` order) in `FaceInfo` vertex order.
#[rustfmt::skip]
const PORTAL_FACES: [[[f32; 3]; 4]; 6] = [
    [[0.0, 0.0, 1.0], [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 1.0]],
    [[0.0, 1.0, 0.0], [0.0, 1.0, 1.0], [1.0, 1.0, 1.0], [1.0, 1.0, 0.0]],
    [[1.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    [[0.0, 1.0, 1.0], [0.0, 0.0, 1.0], [1.0, 0.0, 1.0], [1.0, 1.0, 1.0]],
    [[0.0, 1.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 1.0]],
    [[1.0, 1.0, 1.0], [1.0, 0.0, 1.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
];
/// `AbstractEndPortalRenderer.submitCube`: position-only quads (the portal
/// shader projects its textures from the screen; uv, colour and light are unused).
fn portal_cube(m: Mat4, faces: &[Direction], layer: u32, out: &mut Vec<Vertex>) {
    for &d in faces {
        for c in PORTAL_FACES[d as usize] {
            out.push(Vertex {
                normal: [0; 4],
                pos: m.transform_point3(Vec3::from(c)).to_array(),
                uv: [0.0; 2],
                layer,
                color: 0,
                light: 0,
                glint_uv: [0.0, 0.0],
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires prepared models/textures in HORIZON_TEST_RESOURCES"]
    fn entity_block_particles_and_breaking_use_exported_models() -> crate::Result<()> {
        use crate::{models::catalog::StateModels, sprites::Sprites};
        use horizon_world::{DATA_VERSION, chunk::BlockState, nbt::Tag};
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("HORIZON_TEST_RESOURCES"),
        );
        let names = [
            "white_banner",
            "white_wall_banner",
            "chest",
            "trapped_chest",
            "ender_chest",
            "copper_chest",
        ];
        let mut facts: std::collections::BTreeMap<String, Value> =
            serde_json::from_slice(&std::fs::read(root.join("model-states.json"))?)?;
        let states = names.map(|name| {
            let name = format!("minecraft:{name}");
            let mut first = facts[&name]["states"][0].clone();
            if first.get("type").is_some() {
                first["type"] = serde_json::json!("single");
            }
            facts.get_mut(&name).unwrap()["states"] = serde_json::json!([first.clone()]);
            BlockState {
                name,
                properties: serde_json::from_value(first).unwrap(),
            }
            .to_nbt()
        });
        let registry = BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("states", Tag::List(states.into())),
            (
                "defaults",
                Tag::compound(names.iter().enumerate().map(|(id, name)| {
                    (
                        horizon_world::nbt::JavaString::from(format!("minecraft:{name}").as_str()),
                        Tag::Int(id as i32),
                    )
                })),
            ),
        ]))?;
        let mut sprites = Sprites::default();
        let catalog = StateModels::from_facts(&root, &registry, &names, &mut sprites, &facts)?;
        for (id, name) in names.iter().enumerate() {
            let models = catalog.collect(
                BlockStateId(id as u32),
                horizon_core::position::BlockPos::default(),
            )?;
            assert!(models.iter().all(|(_, model)| model.faces.is_empty()));
            let particle = match *name {
                "ender_chest" => "minecraft:block/obsidian",
                "copper_chest" => "minecraft:block/copper_block",
                _ => "minecraft:block/oak_planks",
            };
            assert_eq!(models[0].1.particle, Some(sprites.layers[particle]));
        }
        let all: Value = serde_json::from_slice(&std::fs::read(root.join("entity-models.json"))?)?;
        let renderer = BlockEntityRenderer::new(
            MODELS
                .iter()
                .filter_map(|(key, layer)| all.get(*layer).map(|v| (*key, ModelPart::parse(v))))
                .collect(),
            &registry,
        );
        let world = World::new(BlockStateId(0), 1);
        let base_sprites = [
            "entity/banner/banner_base",
            "entity/chest/normal",
            "entity/chest/trapped",
            "entity/chest/ender",
            "entity/chest/copper",
        ]
        .into_iter()
        .map(|name| {
            (
                name.to_owned(),
                Rect {
                    u0: 0.25,
                    v0: 0.125,
                    u1: 0.375,
                    v1: 0.25,
                    w: 16,
                    h: 16,
                },
            )
        })
        .collect();
        let cache_open = std::cell::Cell::new(0.0);
        let cache_lid = |_| cache_open.get();
        let mut scene = Scene {
            camera: DVec3::ZERO,
            orientation: Quat::IDENTITY,
            anim: &|_| BlockAnim::default(),
            texts: None,
            partial: 0.5,
            game_time: 0,
            max_y: 320,
            world: &world,
            sprites: &base_sprites,
            nether: false,
            lid: &|_| 0.0,
            sign: &|_| None,
            banner: &|_| Vec::new(),
            skull: &|_| None,
            glyph: &|_| (None, 0.0),
            glyphs: 0,
        };
        // Compare the cached world path with the original model renderer,
        // including atlas replacement, current light and animated fallback.
        scene.lid = &cache_lid;
        for nether in [false, true] {
            scene.nether = nether;
            for part in [ChestPart::Single, ChestPart::Left, ChestPart::Right] {
                for y_rot in [0.0, 90.0, 180.0, 270.0] {
                    let kind = Kind::Chest {
                        block: "chest".into(),
                        texture: "entity/chest/normal",
                        part,
                        y_rot,
                        double_with: None,
                    };
                    for (open, packed) in [(0.0, 0xF0), (0.0, 0x7F), (0.5, 0x7F)] {
                        cache_open.set(open);
                        let base = Mat4::from_translation(Vec3::new(3.25, -8.5, 12.75));
                        let mut cached = Vec::new();
                        let mut original = Vec::new();
                        for (use_cache, out) in [(true, &mut cached), (false, &mut original)] {
                            renderer.render_one(
                                &scene,
                                [0; 3],
                                &kind,
                                base,
                                packed,
                                diffuse_lights(nether),
                                false,
                                use_cache,
                                out,
                            );
                        }
                        assert!(!cached.is_empty());
                        assert_eq!(cached.len(), original.len());
                        for (a, b) in cached.iter().zip(&original) {
                            assert!(
                                (Vec3::from_array(a.pos) - Vec3::from_array(b.pos)).length() < 3e-6
                            );
                            assert_eq!(a.uv, b.uv);
                            assert_eq!(a.color, b.color);
                            assert_eq!(a.light, b.light);
                            assert_eq!(a.normal, b.normal);
                            assert_eq!(a.layer, b.layer);
                        }
                    }
                }
            }
        }
        scene.nether = false;
        scene.lid = &|_| 0.0;
        for state in 0..names.len() as u32 {
            let mut base = Vec::new();
            renderer.render_block(&scene, [0; 3], state, Mat4::IDENTITY, 0xff, &mut base);
            let mut cracks = Vec::new();
            renderer.render_breaking(&scene, [0; 3], state, [0.0; 2], &mut cracks);
            assert!(!cracks.is_empty());
            assert_eq!(
                base.iter().map(|v| v.pos).collect::<Vec<_>>(),
                cracks.iter().map(|v| v.pos).collect::<Vec<_>>()
            );
            assert!(cracks.iter().all(|v| v.layer == 11
                && v.color == u32::MAX
                && v.uv.iter().all(|u| u.is_finite())));
            scene.game_time = 20;
            if state >= 2 {
                scene.lid = &|_| 1.0;
            }
            let mut animated = Vec::new();
            renderer.render_breaking(&scene, [0; 3], state, [0.0; 2], &mut animated);
            assert!(cracks.iter().zip(&animated).any(|(a, b)| a.pos != b.pos));
            scene.game_time = 0;
            scene.lid = &|_| 0.0;
        }
        Ok(())
    }

    #[test]
    fn classifies_vanilla_block_names() {
        let props = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(a, _)| *a == k)
                    .map(|(_, v)| (*v).to_owned())
            }
        };
        let k = classify(
            "minecraft:light_gray_shulker_box",
            &props(&[("facing", "up")]),
        );
        assert!(
            matches!(k, Kind::Shulker { ref texture, .. } if texture == "entity/shulker/shulker_light_gray")
        );
        let k = classify("minecraft:gray_wall_banner", &props(&[("facing", "west")]));
        assert!(matches!(k, Kind::Banner { color: 0x474F52, wall: true, angle } if angle == 90.0));
        let k = classify(
            "minecraft:wither_skeleton_wall_skull",
            &props(&[("facing", "north")]),
        );
        assert!(matches!(
            k,
            Kind::Skull {
                kind: SkullType::WitherSkeleton,
                wall: Some("north"),
                ..
            }
        ));
        let k = classify("minecraft:piglin_head", &props(&[("rotation", "4")]));
        assert!(matches!(
            k,
            Kind::Skull {
                kind: SkullType::Piglin,
                wall: None,
                segment: 4
            }
        ));
        let k = classify(
            "minecraft:chest",
            &props(&[("facing", "north"), ("type", "left")]),
        );
        // A north-facing left half connects to the east (clockwise).
        assert!(matches!(
            k,
            Kind::Chest {
                part: ChestPart::Left,
                double_with: Some([1, 0, 0]),
                texture: "entity/chest/normal_left",
                ..
            }
        ));
        let k = classify(
            "minecraft:waxed_oxidized_copper_chest",
            &props(&[("facing", "south"), ("type", "single")]),
        );
        assert!(matches!(
            k,
            Kind::Chest {
                texture: "entity/chest/copper_oxidized",
                ..
            }
        ));
        assert!(matches!(
            classify("minecraft:lectern", &props(&[("has_book", "false")])),
            Kind::None
        ));
        assert!(matches!(
            classify("minecraft:furnace", &props(&[])),
            Kind::None
        ));
    }
    #[test]
    fn mix_light_matches_shader() {
        let lights = diffuse_lights(false);
        // Straight up: both lights contribute.
        let up = diffuse(Vec3::Y, lights);
        let expected = ((lights.0.y + lights.1.y) * 0.6 + 0.4).min(1.0);
        assert!((up - expected).abs() < 1e-6);
        // Straight down: ambient only.
        assert!((diffuse(-Vec3::Y, lights) - 0.4).abs() < 1e-6);
    }
    #[test]
    fn part_poses_compose_like_model_part() {
        let part = ModelPart {
            pose: [16.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            polygons: vec![[
                [0.0, 0.0, 0.0, 0.0, 0.0],
                [16.0, 0.0, 0.0, 1.0, 0.0],
                [16.0, 0.0, 16.0, 1.0, 1.0],
                [0.0, 0.0, 16.0, 0.0, 1.0],
            ]],
            children: vec![],
        };
        let mut out = Vec::new();
        let rect = Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 0.5,
            v1: 0.5,
            w: 1,
            h: 1,
        };
        draw_part(
            &part,
            "root",
            Mat4::IDENTITY,
            &|_, _| {},
            Paint {
                sprite: rect,
                tint: 0xFFFFFF,
                light: 0,
                lights: diffuse_lights(false),
            },
            &mut out,
        );
        assert_eq!(out.len(), 4);
        assert_eq!(out[1].pos, [2.0, 0.0, 0.0]);
        assert_eq!(out[2].uv, [0.5, 0.5]);
    }

    #[test]
    fn cached_sign_text_matches_and_follows_changes() {
        let renderer = BlockEntityRenderer {
            models: HashMap::new(),
            kinds: vec![
                Kind::Sign {
                    hanging: false,
                    wall: false,
                    angle: 180.0,
                },
                Kind::Sign {
                    hanging: true,
                    wall: true,
                    angle: 90.0,
                },
            ],
            closed_chests: Default::default(),
            pot_patterns: HashMap::new(),
            visibility: None,
            sign_texts: Mutex::default(),
        };
        let world = World::new(BlockStateId(0), 1);
        let sprites = HashMap::new();
        let text = |line: &str, glowing| crate::signs::SignText {
            lines: [line.into(), "two".into(), String::new(), "four".into()],
            color: 3,
            glowing,
        };
        let first_text = crate::signs::SignData {
            front: text("one", false),
            ..Default::default()
        };
        let changed = crate::signs::SignData {
            front: text("changed", false),
            ..Default::default()
        };
        let glowing = crate::signs::SignData {
            back: text("glow", true),
            ..changed.clone()
        };
        let wide = std::cell::Cell::new(6.0);
        let glyph = |c: char| {
            let u = f32::from(c as u8 % 16) / 16.0;
            let rect = Rect {
                u0: u,
                v0: 0.0,
                u1: u + 0.0625,
                v1: 0.0625,
                w: 8,
                h: 8,
            };
            ((c != ' ').then_some(rect), wide.get())
        };
        let (first_sign, changed_sign, glowing_sign) = (
            |_| Some(&first_text),
            |_| Some(&changed),
            |_| Some(&glowing),
        );
        let mut scene = Scene {
            camera: DVec3::new(3.25, 70.5, -4.75),
            orientation: Quat::IDENTITY,
            anim: &|_| BlockAnim::default(),
            texts: None,
            partial: 0.0,
            game_time: 0,
            max_y: 320,
            world: &world,
            sprites: &sprites,
            nether: false,
            lid: &|_| 0.0,
            sign: &first_sign,
            banner: &|_| Vec::new(),
            skull: &|_| None,
            glyph: &glyph,
            glyphs: 0,
        };
        let check = |scene: &Scene, pos: [i32; 3], state: u32| {
            let mut cached = Vec::new();
            renderer.render(scene, [(pos, state)].iter(), &mut cached);
            let corner = DVec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]));
            let mut original = Vec::new();
            renderer.render_one(
                scene,
                pos,
                renderer.kind(state),
                Mat4::from_translation((corner - scene.camera).as_vec3()),
                world.light(pos[0], pos[1], pos[2]),
                diffuse_lights(false),
                false,
                true,
                &mut original,
            );
            assert!(!cached.is_empty());
            assert_eq!(cached.len(), original.len());
            for (a, b) in cached.iter().zip(&original) {
                assert!((Vec3::from_array(a.pos) - Vec3::from_array(b.pos)).length() < 1e-5);
                assert_eq!((a.uv, a.color, a.light), (b.uv, b.color, b.light));
            }
            cached.len()
        };
        let (pos, other) = ([1, 70, 2], [-3, 64, 9]);
        let first = check(&scene, pos, 0);
        // The camera moves: the same text, moved.
        scene.camera.x += 0.5;
        check(&scene, pos, 0);
        // New text, glowing text near and far, another state, other glyphs.
        scene.sign = &changed_sign;
        assert_ne!(check(&scene, pos, 0), first);
        scene.sign = &glowing_sign;
        check(&scene, pos, 0);
        scene.camera.z -= 20.0;
        check(&scene, pos, 0);
        check(&scene, pos, 1);
        check(&scene, other, 1);
        wide.set(5.0);
        scene.glyphs = 1;
        check(&scene, pos, 1);
        // Signs out of view for a while are dropped.
        scene.game_time = 2 * SIGN_TEXT_KEEP;
        check(&scene, pos, 0);
        assert_eq!(renderer.sign_texts.lock().unwrap().signs.len(), 1);
    }
    #[test]
    fn first_line_wraps_on_bytes_without_splitting_a_char() {
        let advance = |c: char| if c == ' ' { 4.0 } else { 6.0 };
        assert_eq!(first_line("ab cd ef", 13.0, &advance), "ab");
        assert_eq!(first_line("abcdef", 13.0, &advance), "ab");
        assert_eq!(first_line("\u{6771}", 3.0, &advance), "\u{6771}");
        assert_eq!(first_line("ab\ncd", 100.0, &advance), "ab");
    }
    /// `DecoratedPotRenderer` submits every part with `RenderTypes.entitySolid`,
    /// whose pipeline culls back faces. The pot's `top` and `bottom` are flat
    /// boxes, so without that cull both sides of each plane are drawn and they
    /// z-fight: the pot renders wrong.
    #[test]
    fn the_decorated_pot_culls_its_back_faces() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("entity-models.json").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let registry = assets.registry;
        let renderer = BlockEntityRenderer::load(&root, registry).unwrap();
        let state = registry.default_state("minecraft:decorated_pot").unwrap();
        let sprites: HashMap<String, Rect> = [
            "entity/decorated_pot/decorated_pot_base",
            "entity/decorated_pot/decorated_pot_side",
        ]
        .into_iter()
        .map(|name| {
            (
                name.to_owned(),
                Rect {
                    u0: 0.25,
                    v0: 0.125,
                    u1: 0.375,
                    v1: 0.25,
                    w: 16,
                    h: 16,
                },
            )
        })
        .collect();
        let world = World::new(BlockStateId(0), 1);
        let scene = Scene {
            camera: DVec3::ZERO,
            orientation: Quat::IDENTITY,
            anim: &|_| BlockAnim::default(),
            texts: None,
            partial: 0.5,
            game_time: 0,
            max_y: 320,
            world: &world,
            sprites: &sprites,
            nether: false,
            lid: &|_| 0.0,
            sign: &|_| None,
            banner: &|_| Vec::new(),
            skull: &|_| None,
            glyph: &|_| (None, 0.0),
            glyphs: 0,
        };
        let mut verts = Vec::new();
        renderer.render_block(&scene, [0; 3], state.0, Mat4::IDENTITY, 0xff, &mut verts);
        // Vanilla submits 28 quads for a plain pot, and the cull keeps them all.
        assert_eq!(
            verts.len() / 4,
            28,
            "the pot submits all of vanilla's quads"
        );
        assert!(
            verts.iter().all(|v| v.normal == CULL_BACK),
            "entitySolid culls the pot's back faces"
        );
    }
}
