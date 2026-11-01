//! Thrown tridents over the client's resident scene (`arrows::clip_block_stop`),
//! for tools/test_trident_parity.py: `AbstractArrow.tick`'s block step and
//! `AbstractArrow.onHitBlock`, one line of state per fixture.
#![forbid(unsafe_code)]
use horizon_client::{
    Result, arrows, assets::Assets, blocks::BlockFacts, collisions, world::World,
};
use horizon_core::geometry::Vec3;
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, Write},
    path::PathBuf,
};

fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn vector(v: &Value) -> Vec3 {
    Vec3::new(number(&v[0]), number(&v[1]), number(&v[2]))
}
fn bits(v: Vec3) -> String {
    format!(
        "{:016x},{:016x},{:016x}",
        v.x.to_bits(),
        v.y.to_bits(),
        v.z.to_bits()
    )
}

pub fn main() -> Result<()> {
    let args: Vec<_> = crate::args().skip(1).collect();
    if args.len() != 3 {
        return Err("usage: trident_probe RESOURCES FIXTURE_DIRECTORY --describe|REFERENCE".into());
    }
    let root = PathBuf::from(&args[0]);
    let dir = PathBuf::from(&args[1]);
    let assets = Assets::load(&root)?;
    let render = assets.reload(&root)?;
    let mut scene = World::test_scene(&assets, &render, 1)?;
    fs::write(
        dir.join("scene.json"),
        serde_json::to_vec(
            &serde_json::json!({"radius":1,"resident_chunks":scene.chunks.len(),"model_gallery":render.model_gallery}),
        )?,
    )?;
    if args[2] == "--describe" {
        return Ok(());
    }
    let binding = collisions::load(&root, assets.registry)?;
    let facts = BlockFacts::load(&root, assets.registry)?;
    let side = &args[2];
    let mut out = std::io::BufWriter::new(fs::File::create(dir.join(format!("rust-{side}.txt")))?);
    for line in std::io::BufReader::new(fs::File::open(dir.join("inputs.txt"))?).lines() {
        let f: Value = serde_json::from_str(&line?)?;
        let mut position = vector(&f["position"]);
        let mut velocity = vector(&f["velocity"]);
        let mut in_ground = false;
        let mut shake = 0u32;
        for op in f["ops"].as_array().unwrap() {
            match op["mode"].as_str().unwrap() {
                "block" => {
                    let p = &op["position"];
                    let mut state = assets
                        .registry
                        .default_state(op["name"].as_str().unwrap())
                        .ok_or("missing fixture block")?;
                    if let Some(properties) = op["properties"].as_object() {
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
                    scene.set(
                        p[0].as_i64().unwrap() as i32,
                        p[1].as_i64().unwrap() as i32,
                        p[2].as_i64().unwrap() as i32,
                        state,
                    )?;
                }
                // Ops only the vanilla oracle needs.
                "level_seed" | "owner" | "pickup" | "touch" | "target" | "velocity" | "ignite" => {}
                "tick" => {
                    // AbstractArrow.tick: shakeTime down, then the block step.
                    shake = shake.saturating_sub(1);
                    if in_ground {
                        continue;
                    }
                    if let Some((stop, _)) =
                        arrows::clip_block_stop(&scene, &facts, &binding, position, velocity)
                    {
                        // onHitBlock: pulled back, stopped, stuck, shaking 7 ticks.
                        position = stop;
                        velocity = Vec3::ZERO;
                        in_ground = true;
                        shake = 7;
                    } else {
                        // A MISS moves the whole movement, then air drag and gravity.
                        position = Vec3::new(
                            position.x + velocity.x,
                            position.y + velocity.y,
                            position.z + velocity.z,
                        );
                        velocity = Vec3::new(
                            velocity.x * f64::from(0.99f32),
                            velocity.y * f64::from(0.99f32) - 0.05,
                            velocity.z * f64::from(0.99f32),
                        );
                    }
                }
                other => return Err(format!("unknown operation {other}").into()),
            }
        }
        out.write_all(
            format!(
                "trident {} {} {in_ground} {shake}\n",
                bits(position),
                bits(velocity)
            )
            .as_bytes(),
        )?;
    }
    Ok(())
}
