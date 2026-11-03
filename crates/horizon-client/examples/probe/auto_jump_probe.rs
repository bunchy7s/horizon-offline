#![forbid(unsafe_code)]
//! Runs tools/test_auto_jump_parity.py's cases through the local player's
//! `MotionWorld::update_auto_jump` over the client's resident test scene
//! (actual resources, collision registry and block-collision scan).
use horizon_client::{
    assets::Assets,
    collisions,
    motion::{MotionResources, MotionWorld, PlayerMotion},
    world::World,
};
use horizon_core::geometry::Vec3;
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader, BufWriter, Write},
    path::PathBuf,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn fl(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn db(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}

pub fn main() -> Result<()> {
    let args: Vec<_> = crate::args().skip(1).collect();
    let [root, input, output, scene_json] = &args[..] else {
        return Err("usage: auto_jump_probe RESOURCES INPUT OUTPUT SCENE_JSON".into());
    };
    let root = PathBuf::from(root);
    let assets = Assets::load(&root)?;
    let render = assets.reload(&root)?;
    let mut scene = World::test_scene(&assets, &render, 1)?;
    let binding = collisions::load(&root, assets.registry)?;
    let resources = MotionResources::load(&root, &assets)?;
    fs::write(
        scene_json,
        serde_json::to_vec(&serde_json::json!({
            "radius": 1, "resident_chunks": scene.chunks.len(), "model_gallery": render.model_gallery,
        }))?,
    )?;
    let mut out = BufWriter::new(fs::File::create(output)?);
    for (n, line) in BufReader::new(fs::File::open(input)?).lines().enumerate() {
        let c: Value = serde_json::from_str(&line?)?;
        writeln!(out, "== {n}")?;
        let mut restore = Vec::new();
        for b in c["blocks"].as_array().unwrap() {
            let (x, y, z) = (
                b[0].as_i64().unwrap() as i32,
                b[1].as_i64().unwrap() as i32,
                b[2].as_i64().unwrap() as i32,
            );
            let mut state = assets
                .registry
                .default_state(b[3].as_str().unwrap())
                .ok_or("missing fixture block")?;
            if let Some(properties) = b[4].as_object()
                && !properties.is_empty()
            {
                let mut definition = assets.registry.state(state).unwrap().clone();
                for (name, value) in properties {
                    definition
                        .properties
                        .insert(name.clone(), value.as_str().unwrap().into());
                }
                state = assets
                    .registry
                    .id(&definition)
                    .ok_or("missing fixture state")?;
            }
            restore.push((x, y, z, scene.block(x, y, z)));
            scene.set(x, y, z, state)?;
        }
        let p = &c["player"];
        let position = Vec3::new(db(&p["pos"][0]), db(&p["pos"][1]), db(&p["pos"][2]));
        let mut player = PlayerMotion::new(&resources, position)?;
        player.ground.on_ground = p["ground"].as_bool().unwrap();
        player.shift_key_down = p["shift"].as_bool().unwrap();
        player.yaw = fl(&p["yr"]);
        player.pitch = fl(&p["xr"]);
        let key = player
            .attributes
            .key("minecraft:movement_speed")
            .ok_or("speed")?;
        let handle = player
            .attributes
            .get_instance(Some(key))
            .map_err(|e| format!("{e:?}"))?
            .ok_or("speed instance")?;
        let speed = f64::from(fl(&p["speed"]));
        player
            .attributes
            .edit(&handle, |a, o| a.set_base_value(speed, o))
            .map_err(|e| format!("{e:?}"))?;
        player.move_vector = [fl(&p["move"][0]), fl(&p["move"][1])];
        player.dimensions.width = fl(&p["width"]);
        player.dimensions.height = fl(&p["height"]);
        player.status.jump_boost = p["boost"].as_i64().map(|a| a as i32);
        player.auto_jump_enabled = p["enabled"].as_bool().unwrap();
        player.auto_jump_time = p["time"].as_i64().unwrap() as i32;
        // canAutoJump, which is where the rider state is read: the movement world drives an
        // unmounted player, so the case's flag goes to the predicate itself.
        let ready = horizon_client::auto_jump::can_auto_jump(
            player.auto_jump_enabled,
            player.auto_jump_time,
            player.ground.on_ground,
            player.shift_key_down,
            p["passenger"].as_bool().unwrap_or(false),
            player.move_vector,
            1.0,
        );
        if ready {
            MotionWorld {
                entities: &[],
                player: &mut player,
                scene: &mut scene,
                assets: &assets,
                collision_binding: &binding,
                resources: &resources,
            }
            .update_auto_jump(fl(&c["xa"]), fl(&c["za"]))?;
        }
        writeln!(out, "jump {}", player.auto_jump_time)?;
        for (x, y, z, state) in restore.into_iter().rev() {
            scene.set(x, y, z, state)?;
        }
    }
    out.flush()?;
    Ok(())
}
