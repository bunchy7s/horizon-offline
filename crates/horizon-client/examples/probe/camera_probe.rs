#![forbid(unsafe_code)]
//! Runs the JSON-lines scenarios of tools/test_camera_parity.py through the
//! Rust camera, field-of-view, avatar bob and view/hurt bobbing code.
use horizon_client::{
    Result,
    camera::{
        AvatarBob, BobState, Camera, CameraEntity, CameraFluid, CameraType, FovPlayer, HurtBob,
        LivingCamera, UseItem, bob_matrix, field_of_view_modifier, perspective,
    },
};
use horizon_core::{
    geometry::{Aabb, ShapeOp, Vec3, VoxelShape},
    position::BlockPos,
};
use horizon_world::world_ray::{RayBlockMode, RayFluidMode, RayWorld, clip_world};
use serde_json::Value;
use std::{
    collections::HashMap,
    convert::Infallible,
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
};

fn fl(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn db(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn fb(v: f32) -> String {
    format!("{:08x}", if v.is_nan() { 0x7fc0_0000 } else { v.to_bits() })
}
fn dbs(v: f64) -> String {
    format!(
        "{:016x}",
        if v.is_nan() {
            0x7ff8_0000_0000_0000
        } else {
            v.to_bits()
        }
    )
}
fn b(v: &Value) -> bool {
    v.as_bool().unwrap()
}
fn vec3(v: &Value) -> [f64; 3] {
    [db(&v[0]), db(&v[1]), db(&v[2])]
}

fn fov_player(c: &Value) -> FovPlayer {
    FovPlayer {
        flying: b(&c["flying"]),
        walking_speed: fl(&c["ws"]),
        movement_speed: db(&c["speed"]),
        using: c["use"].as_array().map(|u| {
            let item = match u[0].as_str().unwrap() {
                "bow" => UseItem::Bow,
                "spyglass" => UseItem::Spyglass,
                _ => UseItem::Other,
            };
            (item, u[1].as_i64().unwrap() as i32)
        }),
    }
}

/// A level of visual shapes keyed by cell, without fluids.
struct Cells {
    shapes: Vec<VoxelShape>,
    index: HashMap<BlockPos, usize>,
}
impl RayWorld for Cells {
    type Error = Infallible;
    type Block = usize;
    type Fluid = ();
    fn block_state(&mut self, p: BlockPos) -> std::result::Result<usize, Infallible> {
        Ok(self.index.get(&p).copied().unwrap_or(0))
    }
    fn fluid_state(&mut self, _: BlockPos) -> std::result::Result<(), Infallible> {
        Ok(())
    }
    fn block_shape(
        &mut self,
        s: usize,
        _: BlockPos,
        _: RayBlockMode,
    ) -> std::result::Result<VoxelShape, Infallible> {
        Ok(self.shapes[s].clone())
    }
    fn interaction_shape(
        &mut self,
        _: usize,
        _: BlockPos,
    ) -> std::result::Result<VoxelShape, Infallible> {
        Ok(VoxelShape::empty())
    }
    fn fluid_is_source(&mut self, _: ()) -> std::result::Result<bool, Infallible> {
        Ok(false)
    }
    fn fluid_is_empty(&mut self, _: ()) -> std::result::Result<bool, Infallible> {
        Ok(true)
    }
    fn fluid_is_water(&mut self, _: ()) -> std::result::Result<bool, Infallible> {
        Ok(false)
    }
    fn fluid_shape(&mut self, _: (), _: BlockPos) -> std::result::Result<VoxelShape, Infallible> {
        Ok(VoxelShape::empty())
    }
}

fn cells(c: &Value) -> Result<Cells> {
    let mut world = Cells {
        shapes: vec![VoxelShape::empty()],
        index: HashMap::new(),
    };
    for (key, boxes) in c["cells"].as_object().unwrap() {
        let at: Vec<i32> = key.split(' ').map(|v| v.parse().unwrap()).collect();
        let mut shape = VoxelShape::empty();
        for a in boxes.as_array().unwrap() {
            shape = shape.join(
                &VoxelShape::from_box(Aabb::new(
                    Vec3::new(db(&a[0]), db(&a[1]), db(&a[2])),
                    Vec3::new(db(&a[3]), db(&a[4]), db(&a[5])),
                ))?,
                ShapeOp::OR,
            )?;
        }
        world
            .index
            .insert(BlockPos::new(at[0], at[1], at[2]), world.shapes.len());
        world.shapes.push(shape);
    }
    Ok(world)
}

fn camera_type(v: &Value) -> CameraType {
    [
        CameraType::FirstPerson,
        CameraType::ThirdPersonBack,
        CameraType::ThirdPersonFront,
    ][v.as_u64().unwrap() as usize]
}

/// The camera entity and, for players, what the field of view reads.
fn entity(s: &Value) -> (CameraEntity, Option<FovPlayer>) {
    let player = s["kind"] == "player";
    let living = player.then(|| LivingCamera {
        scale: db(&s["scale_attr"]) as f32,
        camera_distance: db(&s["dist"]),
        sleeping: s["sleeping"].as_array().map(|a| {
            a[0].as_u64()
                .map(|d| [0.0f32, 90.0, 180.0, 270.0][d as usize])
        }),
        dead: s["dead"].as_i64().map(|t| t as i32),
    });
    let mount = if player {
        s["mount"].as_array().map(|a| (db(&a[0]) as f32, db(&a[1])))
    } else {
        None
    };
    (
        CameraEntity {
            position: vec3(&s["pos"]),
            previous: vec3(&s["prev"]),
            x_rot: fl(&s["xr"]),
            y_rot: fl(&s["yr"]),
            x_rot_o: fl(&s["xro"]),
            y_rot_o: fl(&s["yro"]),
            eye_height: fl(&s["eye"]),
            living,
            mount,
        },
        player.then(|| fov_player(s)),
    )
}

fn camera(c: &Value, out: &mut impl Write) -> Result<()> {
    let mut world = cells(c)?;
    let fov_option = c["fov"].as_i64().unwrap() as i32;
    let effect = db(&c["effect"]) as f32;
    let mut kind = camera_type(&c["type"]);
    let mut camera = Camera::default();
    let mut current: Option<(CameraEntity, Option<FovPlayer>)> = None;
    for op in c["ops"].as_array().unwrap() {
        match op[0].as_str().unwrap() {
            "entity" => {
                current = Some(entity(&op[1]));
                writeln!(out, "entity")?;
            }
            "type" => {
                kind = camera_type(&op[1]);
                writeln!(out, "type")?;
            }
            "tick" => {
                let target = current
                    .as_ref()
                    .and_then(|(_, p)| p.as_ref())
                    .map_or(1.0, |p| {
                        field_of_view_modifier(p, kind.is_first_person(), effect)
                    });
                camera.tick(current.as_ref().map(|(e, _)| e.eye_height), target);
                writeln!(
                    out,
                    "tick {} {} {} {}",
                    fb(camera.eye_height),
                    fb(camera.eye_height_old),
                    fb(camera.fov_modifier),
                    fb(camera.old_fov_modifier)
                )?;
            }
            "frame" => {
                let pt = fl(&op[1]);
                let fluid = [
                    CameraFluid::None,
                    CameraFluid::Water,
                    CameraFluid::Lava,
                    CameraFluid::PowderSnow,
                ][op[2].as_u64().unwrap() as usize];
                let panoramic = b(&op[3]);
                let (e, _) = current.as_ref().expect("entity");
                camera.align_with_entity(e, pt, kind, |from, to| {
                    let Ok(hit) = clip_world(
                        &mut world,
                        Vec3::new(from[0], from[1], from[2]),
                        Vec3::new(to[0], to[1], to[2]),
                        RayBlockMode::Visual,
                        RayFluidMode::None,
                        usize::MAX,
                    ) else {
                        panic!("ray")
                    };
                    (!hit.miss).then(|| {
                        let l = hit.hit.location;
                        [l.x, l.y, l.z]
                    })
                });
                let dead = e.living.and_then(|l| l.dead);
                let fov = camera.calculate_fov(pt, fov_option, effect, dead, fluid, panoramic);
                let hud = camera.calculate_hud_fov(pt, effect, dead, fluid);
                let p = camera.position;
                let q = camera.rotation;
                let v = |a: [f32; 3]| format!("{} {} {}", fb(a[0]), fb(a[1]), fb(a[2]));
                writeln!(
                    out,
                    "frame {} {} {} {} {} {} {} {} {} {} {} {} {} {} {}",
                    dbs(p[0]),
                    dbs(p[1]),
                    dbs(p[2]),
                    fb(camera.x_rot),
                    fb(camera.y_rot),
                    fb(q.x),
                    fb(q.y),
                    fb(q.z),
                    fb(q.w),
                    v(camera.forwards),
                    v(camera.up),
                    v(camera.left),
                    camera.detached,
                    fb(fov),
                    fb(hud)
                )?;
                let (w, h, far) = (fl(&op[4]), fl(&op[5]), fl(&op[6]));
                let matrix =
                    |m: [f32; 16]| m.iter().map(|&x| format!(" {}", fb(x))).collect::<String>();
                writeln!(
                    out,
                    "projection{}{}",
                    matrix(perspective(fov, w, h, far, 0.05, true)),
                    matrix(perspective(hud, w, h, far, 0.05, true))
                )?;
            }
            other => panic!("{other}"),
        }
    }
    Ok(())
}

fn bob(c: &Value, out: &mut impl Write) -> Result<()> {
    let state = BobState {
        living: (!c["living"].is_null()).then(|| {
            let l = &c["living"];
            HurtBob {
                hurt_time: fl(&l["hurt"]),
                hurt_duration: l["duration"].as_i64().unwrap() as i32,
                hurt_dir: fl(&l["dir"]),
                death_time: (!l["death"].is_null()).then(|| fl(&l["death"])),
            }
        }),
        player: c["player"].as_array().map(|p| (fl(&p[0]), fl(&p[1]))),
    };
    let m = bob_matrix(&state, db(&c["tilt"]), b(&c["view"]));
    let mut line = String::from("bob");
    for v in m.m {
        line.push(' ');
        line.push_str(&fb(v));
    }
    writeln!(out, "{line} {}", m.properties)?;
    Ok(())
}

fn avatar(c: &Value, out: &mut impl Write) -> Result<()> {
    let mut state = AvatarBob::default();
    for op in c["ops"].as_array().unwrap() {
        let name = op[0].as_str().unwrap();
        match name {
            "tick" => state.tick([0.0; 3]),
            "move" => state.add_walked(fl(&op[1]), fl(&op[2])),
            "bob" => state.update_bob(b(&op[1]) && !b(&op[2]) && !b(&op[3]), vec3(&op[4])),
            "reset" => state.reset_bob(),
            "frame" => {
                let pt = fl(&op[1]);
                writeln!(
                    out,
                    "frame {} {}",
                    fb(state.interpolated_bob(pt)),
                    fb(state.backwards_interpolated_walk_distance(pt))
                )?;
                continue;
            }
            other => panic!("{other}"),
        }
        writeln!(out, "{name}")?;
    }
    Ok(())
}

pub fn main() -> Result<()> {
    let args: Vec<String> = crate::args().collect();
    let input = BufReader::new(File::open(&args[1])?);
    let mut out = BufWriter::new(File::create(&args[2])?);
    for (n, line) in input.lines().enumerate() {
        let c: Value = serde_json::from_str(&line?)?;
        writeln!(out, "== {n}")?;
        match c["t"].as_str().unwrap() {
            "fov" => writeln!(
                out,
                "{}",
                fb(field_of_view_modifier(
                    &fov_player(&c),
                    b(&c["fp"]),
                    fl(&c["scale"])
                ))
            )?,
            "camera" => camera(&c, &mut out)?,
            "bob" => bob(&c, &mut out)?,
            "avatar" => avatar(&c, &mut out)?,
            other => panic!("{other}"),
        }
    }
    out.flush()?;
    Ok(())
}
