//! Headless transport for the block-entity render parity gate: replays each
//! case (block state, light, partial tick and ticked block-entity fields)
//! through `BlockEntityRenderer` at the block origin and writes the quads
//! (texture, sprite-local UVs, baked colour, light) and sign text calls.
#![forbid(unsafe_code)]
use glam::{DVec3, Mat4, Quat, Vec3};
use horizon_client::{
    assets::Assets,
    banner::{self, Banners},
    block_entity_anim::EnchantingBook,
    block_entity_render::{
        BlockAnim, BlockEntityRenderer, END_GATEWAY_LAYER, END_PORTAL_LAYER, Scene, TextCall,
    },
    chunk::{Chunk, Light},
    hud::{HudAtlas, Rect},
    item_render::{ItemRenderer, gui},
    signs::{DYE_TEXT, SignData, SignText},
    world::World,
};
use horizon_world::registry::{BlockRegistry, BlockStateId};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::HashMap,
    fs,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Every texture the vanilla oracle used gets its own unit square on a small
/// grid (offsets stay small so f32 UVs keep their precision), so a vertex's
/// texture and sprite-local UV can be recovered.
fn fake_sprites(list: &Path) -> Result<(HashMap<String, Rect>, Vec<String>, usize)> {
    let names: Vec<String> = fs::read_to_string(list)?
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    let grid = (names.len() as f64).sqrt().ceil().max(1.0) as usize;
    let map = names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let (u0, v0) = ((i % grid) as f32 * 2.0, (i / grid) as f32 * 2.0);
            (
                n.clone(),
                Rect {
                    u0,
                    v0,
                    u1: u0 + 1.0,
                    v1: v0 + 1.0,
                    w: 16,
                    h: 16,
                },
            )
        })
        .collect();
    Ok((map, names, grid))
}

fn find_state(registry: &BlockRegistry, spec: &str) -> Option<u32> {
    let (name, props) = match spec.split_once('[') {
        Some((n, rest)) => (n, rest.trim_end_matches(']')),
        None => (spec, ""),
    };
    let wanted: Vec<(&str, &str)> = props
        .split(',')
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.split_once('='))
        .collect();
    let default = registry.state(registry.default_state(name)?)?.clone();
    (0..registry.len() as u32).find(|&id| {
        registry.state(BlockStateId(id)).is_some_and(|s| {
            s.name == name
                && s.properties.iter().all(|(k, v)| {
                    wanted
                        .iter()
                        .find(|(wk, _)| wk == k)
                        .map_or(default.properties.get(k) == Some(v), |(_, wv)| wv == v)
                })
        })
    })
}

fn sign_side(v: &Value) -> SignText {
    let mut side = SignText::default();
    for (line, text) in side
        .lines
        .iter_mut()
        .zip(v["lines"].as_array().into_iter().flatten())
    {
        *line = text.as_str().unwrap_or_default().to_owned();
    }
    let color = v["color"].as_str().unwrap_or("black");
    side.color = DYE_TEXT.iter().position(|(n, _)| *n == color).unwrap_or(15) as u8;
    side.glowing = v["glowing"].as_bool().unwrap_or(false);
    side
}

fn intern(s: &str) -> &'static str {
    match s {
        "north" => "north",
        "south" => "south",
        "west" => "west",
        "east" => "east",
        "up" => "up",
        _ => "down",
    }
}

pub fn main() -> Result<()> {
    let args: Vec<PathBuf> = crate::args_os().skip(1).map(PathBuf::from).collect();
    let [root, inputs, textures, output] = args.as_slice() else {
        return Err("usage: block_entity_render_fixture RESOURCES INPUTS TEXTURES OUTPUT".into());
    };
    let assets = Assets::load(root)?;
    let registry = assets.registry;
    let renderer = BlockEntityRenderer::load(root, registry)?;
    let atlas = HudAtlas::build(root)?;
    let banners = Banners::load(root);
    let items = ItemRenderer::load(root)?;
    let (sprites, names, grid) = fake_sprites(textures)?;
    let air = registry.default_state("minecraft:air").ok_or("no air")?;
    let mut out = BufWriter::new(fs::File::create(output)?);
    for line in fs::read_to_string(inputs)?
        .lines()
        .filter(|l| !l.is_empty())
    {
        let case: Value = serde_json::from_str(line)?;
        let block = case["block"].as_str().unwrap_or_default();
        if let Some(loom) = case.get("loom") {
            // The loom's result preview, as LoomScreen draws it at (0, 0).
            let layers: Vec<(String, String)> = loom["patterns"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|p| {
                    (
                        banners.banner_texture(p[0].as_str().unwrap_or_default()),
                        p[1].as_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect();
            let base = loom["base"].as_str().unwrap_or_default();
            let flat = gui::items_flat_lights();
            let quads: Vec<Value> =
                gui::banner_result_quads(&items, &sprites, 0.0, 0.0, base, &layers)
                    .iter()
                    .map(|q| {
                        let (cu, cv) = ((q.uv[0][0] / 2.0).floor(), (q.uv[0][1] / 2.0).floor());
                        let texture = names
                            .get(cu as usize + cv as usize * grid)
                            .cloned()
                            .unwrap_or_default();
                        let v: Vec<Value> = (0..4)
                            .map(|k| {
                                let p = q.pos[k];
                                json!([p.x, p.y, p.z, q.uv[k][0] - cu * 2.0, q.uv[k][1] - cv * 2.0])
                            })
                            .collect();
                        // Full bright (15728880).
                        json!({"texture": texture, "v": v, "color": gui::lit_color(q, flat), "light": 0xF0F0})
                    })
                    .collect();
            writeln!(out, "{}", json!({"quads": quads, "texts": []}))?;
            continue;
        }
        if block == "minecraft:beacon" {
            // BeaconRenderer: the beam sections from the case, as Horizon's beacon keeps them.
            let int = |v: &Value| v.as_i64().unwrap_or(0);
            let pos = [
                int(&case["pos"][0]),
                int(&case["pos"][1]),
                int(&case["pos"][2]),
            ];
            let partial = case["partial"].as_f64().unwrap_or(0.0) as f32;
            let time = int(&case["gameTime"]).rem_euclid(40) as f32 + partial;
            let cam = &case["camera"];
            let dx = cam[0].as_f64().unwrap_or(0.0) - (pos[0] as f64 + 0.5);
            let dz = cam[2].as_f64().unwrap_or(0.0) - (pos[2] as f64 + 0.5);
            let scale = ((dx * dx + dz * dz).sqrt() as f32 / 96.0).max(1.0);
            let sections: Vec<(u32, i32)> = case["beam"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|s| (int(&s[0]) as u32 & 0xFF_FFFF, int(&s[1]) as i32))
                .collect();
            let texture = "entity/beacon/beacon_beam";
            let rect = sprites.get(texture).copied().ok_or("no beam sprite")?;
            let mut verts = Vec::new();
            horizon_client::block_entity_render::beacon_beam(
                Vec3::ZERO,
                &sections,
                time,
                scale,
                rect,
                &mut verts,
            );
            let quads: Vec<Value> = verts
                .chunks(4)
                .map(|q| {
                    let v: Vec<Value> = q
                        .iter()
                        .map(|v| json!([v.pos[0], v.pos[1], v.pos[2], v.uv[0] - rect.u0, v.uv[1]]))
                        .collect();
                    json!({"texture": texture, "v": v, "color": q[0].color, "light": q[0].light, "custom": true})
                })
                .collect();
            writeln!(out, "{}", json!({"quads": quads, "texts": []}))?;
            continue;
        }
        let Some(state) = find_state(registry, block) else {
            writeln!(out, "{}", json!({"skip": "unknown block state"}))?;
            continue;
        };
        if !renderer.renders(state) {
            writeln!(out, "{}", json!({"skip": "no renderer"}))?;
            continue;
        }
        let int = |v: &Value| v.as_i64().unwrap_or(0) as i32;
        let float = |v: &Value| v.as_f64().unwrap_or(0.0) as f32;
        let pos = [
            int(&case["pos"][0]),
            int(&case["pos"][1]),
            int(&case["pos"][2]),
        ];
        let packed = (int(&case["light"][0]) << 4 | int(&case["light"][1])) as u8;
        let mut world = World::new(air, 0);
        for dx in -1..=1 {
            for dz in -1..=1 {
                let (cx, cz) = ((pos[0] >> 4) + dx, (pos[2] >> 4) + dz);
                let mut chunk = Chunk::empty(cx, cz);
                for section in &mut chunk.sections {
                    Arc::make_mut(section).light = Light::Uniform(packed);
                }
                world.chunks.insert((cx, cz), Arc::new(chunk));
            }
        }
        // Neighbour blocks the case sets (End gateway faces).
        for (direction, spec) in case["neighbours"].as_object().into_iter().flatten() {
            let d = match direction.as_str() {
                "down" => [0, -1, 0],
                "up" => [0, 1, 0],
                "north" => [0, 0, -1],
                "south" => [0, 0, 1],
                "west" => [-1, 0, 0],
                _ => [1, 0, 0],
            };
            let neighbour = find_state(registry, spec.as_str().unwrap_or_default())
                .ok_or("unknown neighbour state")?;
            world.set(
                pos[0] + d[0],
                pos[1] + d[1],
                pos[2] + d[2],
                BlockStateId(neighbour),
            )?;
        }
        let book = case
            .get("book")
            .map_or_else(EnchantingBook::default, |book| {
                let [flip, o_flip, open, o_open, rot, o_rot] =
                    ["flip", "oFlip", "open", "oOpen", "rot", "oRot"].map(|k| float(&book[k]));
                EnchantingBook {
                    time: int(&book["time"]),
                    flip,
                    o_flip,
                    open,
                    o_open,
                    rot,
                    o_rot,
                    ..EnchantingBook::default()
                }
            });
        let sign = case.get("sign").map(|s| {
            let mut data = SignData {
                hanging: block.contains("hanging"),
                ..SignData::default()
            };
            data.front = sign_side(&s["front"]);
            data.back = sign_side(&s["back"]);
            data
        });
        let layers: Vec<(String, u32)> = case["patterns"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| {
                (
                    banners.banner_texture(p[0].as_str().unwrap_or_default()),
                    banner::dye_rgb(p[1].as_str().unwrap_or_default()),
                )
            })
            .collect();
        let anim = BlockAnim {
            shulker: (float(&case["progress"]), float(&case["progressOld"])),
            bell_ticks: int(&case["bellTicks"]),
            bell_shake: case["shake"].as_str().map(intern),
            conduit: (
                int(&case["conduit"]["tickCount"]),
                case["conduit"]["active"].as_bool().unwrap_or(false),
                case["conduit"]["hunting"].as_bool().unwrap_or(false),
                float(&case["conduit"]["activeRotation"]),
            ),
            skull: (
                case["skullAnimation"][0].as_bool().unwrap_or(false),
                int(&case["skullAnimation"][1]),
            ),
            sherds: [0, 1, 2, 3].map(|i| {
                case["sherds"][i]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            }),
            wobble: case["wobble"]
                .as_array()
                .map(|w| (w[0].as_str() == Some("POSITIVE"), float(&w[1]))),
            book,
            gateway: (
                case["gateway"]["age"].as_i64().unwrap_or(0),
                int(&case["gateway"]["cooldown"]),
            ),
        };
        // The case's level height (minimum y, height): the End's by default.
        let max_y = match case["level"].as_array() {
            Some(l) => int(&l[0]) + int(&l[1]) - 1,
            None => 255,
        };
        let open = float(&case["open"]);
        let near = case["sign"]["near"].as_bool().unwrap_or(false);
        let camera = DVec3::new(
            case["camera"][0].as_f64().unwrap_or(0.0),
            case["camera"][1].as_f64().unwrap_or(0.0),
            case["camera"][2].as_f64().unwrap_or(0.0),
        );
        // Signs pick the outline from the camera distance: put the camera where the case says.
        let camera = if case.get("sign").is_some() {
            let centre = DVec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])) + 0.5;
            centre
                + if near {
                    DVec3::ZERO
                } else {
                    DVec3::new(20.0, 0.0, 0.0)
                }
        } else {
            camera
        };
        let (yaw, pitch) = (float(&case["yaw"]), float(&case["pitch"]));
        let texts = RefCell::new(Vec::<TextCall>::new());
        let scene = Scene {
            camera,
            orientation: Quat::from_euler(
                glam::EulerRot::YXZ,
                std::f32::consts::PI - yaw * (std::f32::consts::PI / 180.0),
                -pitch * (std::f32::consts::PI / 180.0),
                0.0,
            ),
            anim: &|_| anim.clone(),
            texts: Some(&texts),
            partial: float(&case["partial"]),
            game_time: case["gameTime"].as_i64().unwrap_or(0),
            max_y,
            world: &world,
            sprites: &sprites,
            nether: false,
            lid: &|_| open,
            sign: &|_| sign.as_ref(),
            banner: &|_| layers.clone(),
            skull: &|_| None,
            glyph: &|c| atlas.glyph_cell(c),
            glyphs: atlas.font.epoch(),
        };
        let mut verts = Vec::new();
        renderer.render_block(&scene, pos, state, Mat4::IDENTITY, packed, &mut verts);
        let quads: Vec<Value> = verts
            .chunks(4)
            .map(|q| {
                // Custom geometry: the End portal shader's position-only cube (its
                // render type samples end_sky first) and End gateway beams.
                if q[0].layer == END_PORTAL_LAYER || q[0].layer == END_GATEWAY_LAYER {
                    let v: Vec<Value> = q
                        .iter()
                        .map(|v| json!([v.pos[0], v.pos[1], v.pos[2], v.uv[0], v.uv[1]]))
                        .collect();
                    return json!({"texture": "environment/end_sky", "v": v, "color": q[0].color,
                                  "light": q[0].light, "custom": true});
                }
                if q[0].layer == 6 {
                    let texture = "entity/end_portal/end_gateway_beam";
                    let u0 = sprites.get(texture).map_or(0.0, |r| r.u0);
                    let v: Vec<Value> = q
                        .iter()
                        .map(|v| json!([v.pos[0], v.pos[1], v.pos[2], v.uv[0] - u0, v.uv[1]]))
                        .collect();
                    return json!({"texture": texture, "v": v, "color": q[0].color,
                                  "light": q[0].light, "custom": true});
                }
                let (cu, cv) = ((q[0].uv[0] / 2.0).floor(), (q[0].uv[1] / 2.0).floor());
                let texture = names
                    .get(cu as usize + cv as usize * grid)
                    .cloned()
                    .unwrap_or_default();
                let v: Vec<Value> = q
                    .iter()
                    .map(|v| {
                        json!([
                            v.pos[0],
                            v.pos[1],
                            v.pos[2],
                            v.uv[0] - cu * 2.0,
                            v.uv[1] - cv * 2.0
                        ])
                    })
                    .collect();
                json!({"texture": texture, "v": v, "color": q[0].color, "light": q[0].light})
            })
            .collect();
        let texts: Vec<Value> = texts
            .borrow()
            .iter()
            .filter(|t| !t.text.is_empty())
            .map(|t| {
                json!({"matrix": t.matrix.to_cols_array(), "x": t.x, "y": t.y, "text": t.text,
                       "color": t.color, "outline": t.outline,
                       "light": ((t.light & 0xFF) << 16 | (t.light >> 8 & 0xFF))})
            })
            .collect();
        writeln!(out, "{}", json!({"quads": quads, "texts": texts}))?;
    }
    Ok(())
}
