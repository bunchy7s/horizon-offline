#![forbid(unsafe_code)]
//! Runs tools/test_particle_providers_parity.py's particles through
//! `ParticleEngine`: built with the seeds the vanilla oracle recovered, then
//! ticked over air or a stone floor at y 63, printing their state each tick.
use horizon_client::{
    Result,
    assets::Assets,
    blocks::BlockFacts,
    chunk::Chunk,
    collisions,
    particles::{ParticleEngine, ParticleLevel, ParticleOptions, ParticleSnapshot, ParticleType},
    world::World,
};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
    sync::Arc,
};

fn d(v: f64) -> String {
    format!("{:016x}", v.to_bits())
}
fn f(v: f32) -> String {
    format!("{:08x}", v.to_bits())
}
fn color(v: &Value, default: u32) -> u32 {
    v.as_u64().map_or(default, |c| c as u32)
}
fn rgb(c: u32) -> [f32; 3] {
    [
        ((c >> 16) & 255) as f32 / 255.0,
        ((c >> 8) & 255) as f32 / 255.0,
        (c & 255) as f32 / 255.0,
    ]
}
fn options(case: &Value) -> Option<ParticleOptions> {
    let ty = ParticleType::from_name(case["type"].as_str()?)?;
    let o = &case["options"];
    let scale = o["scale"].as_f64().unwrap_or(1.0) as f32;
    let c = color(&o["color"], 0xFFFF_FFFF);
    let argb = {
        let [r, g, b] = rgb(c);
        [r, g, b, ((c >> 24) & 255) as f32 / 255.0]
    };
    Some(match ty {
        ParticleType::Block
        | ParticleType::BlockCrumble
        | ParticleType::DustPillar
        | ParticleType::BlockMarker => ParticleOptions::block(ty, "terrain".into(), 0xFF_FFFF),
        // Stone's map colour: FallingDustParticle.Provider's tint without a tint source.
        ParticleType::FallingDust => ParticleOptions::block(ty, "terrain".into(), 0x70_7070),
        ParticleType::Dust => ParticleOptions::dust(rgb(c), scale),
        ParticleType::DustColorTransition => {
            ParticleOptions::dust_transition(rgb(c), rgb(color(&o["to"], 0)), scale)
        }
        ParticleType::Effect | ParticleType::InstantEffect => ParticleOptions {
            scale,
            ..ParticleOptions::colored(ty, argb)
        },
        // The oracle's item models: one layer whose sprite is named after the item.
        ParticleType::Item => {
            let item = o["item"].as_str()?;
            ParticleOptions::item(vec![format!(
                "item_{}",
                item.strip_prefix("minecraft:").unwrap_or(item)
            )])
        }
        ParticleType::EntityEffect | ParticleType::TintedLeaves | ParticleType::Flash => {
            ParticleOptions::colored(ty, argb)
        }
        ParticleType::DragonBreath => ParticleOptions {
            scale,
            ..ParticleOptions::simple(ty)
        },
        ParticleType::SculkCharge => ParticleOptions::sculk_charge(scale),
        ParticleType::Geyser | ParticleType::GeyserPlume => {
            ParticleOptions::geyser(ty, o["water_blocks"].as_i64()? as i32)
        }
        ParticleType::GeyserBase | ParticleType::GeyserPoof => ParticleOptions::geyser_base(
            ty,
            o["water_blocks"].as_i64()? as i32,
            o["burst_impulse_base"].as_f64()? as f32,
        ),
        ParticleType::Shriek => ParticleOptions::shriek(scale as i32),
        ParticleType::Trail => {
            let t = &o["target"];
            ParticleOptions::trail(
                [t[0].as_f64()?, t[1].as_f64()?, t[2].as_f64()?],
                c & 0xFF_FFFF,
                o["duration"].as_i64()? as i32,
            )
        }
        _ => ParticleOptions::simple(ty),
    })
}
fn state(s: &ParticleSnapshot) -> String {
    let mut out = format!(
        "{} {} {} {}",
        s.age,
        s.lifetime,
        u8::from(s.removed),
        u8::from(s.on_ground)
    );
    for v in s.position.iter().chain(&s.velocity).chain(&s.previous) {
        out += &format!(" {}", d(*v));
    }
    if let Some(q) = &s.quad {
        out += &format!(" | {} {}", f(q.size[0]), f(q.size[1]));
        for v in q.rgba.iter().chain(&q.roll) {
            out += &format!(" {}", f(*v));
        }
        out += &format!(" {}", q.sprite);
        for v in q.uv {
            out += &format!(" {}", f(v));
        }
        out += &format!(" {:x}", q.light);
    }
    out
}
fn world(assets: &Assets, facts: &BlockFacts, floor: bool, water: bool) -> Result<World> {
    let mut world = World::new(facts.air, 0);
    let state = |name: &str| assets.registry.default_state(name).ok_or("missing block");
    let (stone, still) = (state("minecraft:stone")?, state("minecraft:water")?);
    for cz in -3..=2 {
        for cx in -3..=2 {
            world
                .chunks
                .insert((cx, cz), Arc::new(Chunk::empty(cx, cz)));
            for z in 0..16 {
                for x in 0..16 {
                    let (x, z) = (cx * 16 + x, cz * 16 + z);
                    if floor {
                        world.set(x, 63, z, stone)?;
                    }
                    // The oracle's water: still sources from y 64 to 67.
                    if water {
                        for y in 64..=67 {
                            world.set(x, y, z, still)?;
                        }
                    }
                }
            }
        }
    }
    Ok(world)
}

pub fn main() -> Result<()> {
    let args: Vec<String> = crate::args().collect();
    let root = &Path::new(".local/horizon-client").join(horizon_core::VANILLA_VERSION);
    let assets = Assets::load(root)?;
    let facts = BlockFacts::load(root, assets.registry)?;
    let collision = collisions::load(root, assets.registry)?;
    let worlds = [
        world(&assets, &facts, false, false)?,
        world(&assets, &facts, true, false)?,
        world(&assets, &facts, false, true)?,
        world(&assets, &facts, true, true)?,
    ];
    let mut engine = ParticleEngine::new(root, &HashMap::new(), &Default::default());
    for (ty, item) in [
        (ParticleType::ItemSlime, "item_slime_ball"),
        (ParticleType::ItemSnowball, "item_snowball"),
        (ParticleType::ItemCobweb, "item_cobweb"),
    ] {
        engine.item_sprites.insert(ty, vec![item.to_owned()]);
    }
    // The oracle's output: the seed line of each case.
    let mut seeds = Vec::new();
    for line in BufReader::new(File::open(&args[2])?).lines() {
        let line = line?;
        if line.starts_with("seed ") || line.starts_with("null ") || line == "no provider" {
            seeds.push(line);
        }
    }
    let mut out = BufWriter::new(File::create(&args[3])?);
    for (n, line) in BufReader::new(File::open(&args[1])?).lines().enumerate() {
        let case: Value = serde_json::from_str(&line?)?;
        writeln!(out, "== {n}")?;
        let Some(options) = options(&case) else {
            writeln!(out, "no provider")?;
            continue;
        };
        let seed = case["seed"].as_i64().ok_or("seed")?;
        engine.seed_randoms(seed, seed ^ 0x6C_6576_656C, seed ^ 0x73_7065_6C6C);
        engine.requested = Some(Vec::new());
        engine.sounds.clear();
        let level = ParticleLevel {
            world: &worlds[usize::from(case["floor"].as_bool() == Some(true))
                + 2 * usize::from(case["water"].as_bool() == Some(true))],
            facts: &facts,
            collision: &collision,
            player: None,
        };
        let v = |key: &str| -> [f64; 3] {
            std::array::from_fn(|i| case[key][i].as_f64().unwrap_or(0.0))
        };
        let seed_line = seeds.get(n).cloned().unwrap_or_default();
        let local = seed_line
            .split(' ')
            .nth(1)
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        let Some(mut particle) = engine.probe_create(&options, v("pos"), v("vel"), local, &level)
        else {
            writeln!(out, "null {}", engine.next_randoms()[0])?;
            continue;
        };
        writeln!(out, "{seed_line}")?;
        let [a, b, c] = engine.next_randoms();
        writeln!(out, "next {a} {b} {c}")?;
        let events = |engine: &mut ParticleEngine, out: &mut BufWriter<File>| -> Result<()> {
            for (o, p, v) in engine.requested.replace(Vec::new()).unwrap_or_default() {
                write!(out, "+ {}", o.ty.name())?;
                for x in p.iter().chain(&v) {
                    write!(out, " {}", d(*x))?;
                }
                writeln!(out)?;
            }
            for (name, p, volume, pitch) in std::mem::take(&mut engine.sounds) {
                writeln!(
                    out,
                    "~ {name} {} {} {} {} {}",
                    d(p[0]),
                    d(p[1]),
                    d(p[2]),
                    f(volume),
                    f(pitch)
                )?;
            }
            Ok(())
        };
        writeln!(out, "0 {}", state(&engine.snapshot(&particle)))?;
        events(&mut engine, &mut out)?;
        let ticks = case["ticks"].as_i64().unwrap_or(0);
        let mut t = 1;
        while t <= ticks && particle.is_alive() {
            engine.probe_tick(&mut particle, &level);
            writeln!(out, "{t} {}", state(&engine.snapshot(&particle)))?;
            events(&mut engine, &mut out)?;
            t += 1;
        }
    }
    out.flush()?;
    Ok(())
}
