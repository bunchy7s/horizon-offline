//! Headless transport for the block-entity animation parity gate: replays each
//! scripted case (block, neighbourhood, entities, block events, game times)
//! through `horizon_client::block_entity_anim` and prints the same lines as
//! `VanillaBlockEntityAnimOracle`: the block entity's fields, the level calls
//! its tick made and the render state the renderer derives from `BlockAnim`.
#![forbid(unsafe_code)]
use horizon_client::{
    assets::Assets,
    block_entity_anim::{
        Bell, BellEntity, BlockEntityAnims, ChestLid, Conduit, ConduitTarget, DIRECTIONS,
        EndGateway, PotWobble, Shulker, ShulkerStatus, Skull, WobbleStyle, chest_lid_x_rot,
        direction_index,
    },
    block_entity_render::{BlockEntityRenderer, gateway_beam},
    blocks::{BlockFacts, Fluid},
    items::Items,
};
use horizon_core::random::LegacyRandom;
use horizon_world::loot::PackOverlay;
use horizon_world::registry::{BlockRegistry, BlockStateId};
use serde_json::Value;
use std::{
    collections::HashMap,
    fmt::Write as _,
    fs,
    io::{BufWriter, Write},
    path::PathBuf,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn f(v: f32) -> String {
    format!("{:08x}", v.to_bits())
}
fn d(v: f64) -> String {
    format!("{:016x}", v.to_bits())
}
fn bx(b: [f64; 6]) -> String {
    b.map(d).join(" ")
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

fn prop<'a>(spec: &'a str, key: &str) -> Option<&'a str> {
    let props = spec.split_once('[')?.1.trim_end_matches(']');
    props
        .split(',')
        .filter_map(|p| p.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v)
}

fn offset(key: &str) -> [i32; 3] {
    let mut it = key.split(',').map(|v| v.parse::<i32>().unwrap_or(0));
    [
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
    ]
}

fn mob(e: &Value) -> BellEntity {
    let p = |i: usize| e["pos"][i].as_f64().unwrap_or(0.0);
    BellEntity {
        position: [p(0), p(1), p(2)],
        alive: e["alive"].as_bool().unwrap_or(true) && !e["removed"].as_bool().unwrap_or(false),
        raider: e["raider"].as_bool().unwrap_or(false),
    }
}

fn lid_text(l: Option<&ChestLid>) -> String {
    l.map_or("-".to_owned(), |l| {
        format!("{} {} {}", l.should_be_open, f(l.openness), f(l.o_openness))
    })
}

/// The case's players: feet positions and whether each is a spectator.
fn players(v: &Value) -> Vec<([f64; 3], bool)> {
    v.as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            let c = |i: usize| p["pos"][i].as_f64().unwrap_or(0.0);
            (
                [c(0), c(1), c(2)],
                p["spectator"].as_bool().unwrap_or(false),
            )
        })
        .collect()
}

fn status(s: ShulkerStatus) -> &'static str {
    match s {
        ShulkerStatus::Closed => "CLOSED",
        ShulkerStatus::Opening => "OPENING",
        ShulkerStatus::Opened => "OPENED",
        ShulkerStatus::Closing => "CLOSING",
    }
}

pub fn main() -> Result<()> {
    let args: Vec<PathBuf> = crate::args_os().skip(1).map(PathBuf::from).collect();
    let [root, inputs, output] = args.as_slice() else {
        return Err("usage: block_entity_anim_fixture RESOURCES INPUTS OUTPUT".into());
    };
    let assets = Assets::load(root)?;
    let registry = assets.registry;
    let facts = BlockFacts::load(root, registry)?;
    let renderer = BlockEntityRenderer::load(root, registry)?;
    let tags = Items::tags_only(root, &PackOverlay::NONE)?;
    let frames = tags
        .block_tags
        .get("minecraft:conduit_effect_block")
        .cloned()
        .unwrap_or_default();
    let mut out = BufWriter::new(fs::File::create(output)?);
    for line in fs::read_to_string(inputs)?
        .lines()
        .filter(|l| !l.is_empty())
    {
        let case: Value = serde_json::from_str(line)?;
        writeln!(out, "case {}", case["name"].as_str().unwrap_or_default())?;
        let block = case["block"].as_str().unwrap_or_default().to_owned();
        let int = |v: &Value| v.as_i64().unwrap_or(0) as i32;
        let pos = [
            int(&case["pos"][0]),
            int(&case["pos"][1]),
            int(&case["pos"][2]),
        ];
        // The neighbourhood as block states (conduits).
        let mut blocks: HashMap<[i32; 3], String> = HashMap::new();
        blocks.insert(pos, block.clone());
        let place = |blocks: &mut HashMap<[i32; 3], String>, set: &Value| {
            for (k, v) in set.as_object().into_iter().flatten() {
                let o = offset(k);
                blocks.insert(
                    [pos[0] + o[0], pos[1] + o[1], pos[2] + o[2]],
                    v.as_str().unwrap_or_default().to_owned(),
                );
            }
        };
        place(&mut blocks, &case["world"]);
        let entities: Vec<BellEntity> = case["entities"]
            .as_array()
            .into_iter()
            .flatten()
            .map(mob)
            .collect();
        let target = case.get("target").map(|t| {
            let e = mob(t);
            ConduitTarget {
                position: e.position,
                eye_y: e.position[1] + f64::from(t["eye"].as_f64().unwrap_or(0.0) as f32),
                width: t["width"].as_f64().unwrap_or(0.0) as f32,
                height: t["height"].as_f64().unwrap_or(0.0) as f32,
            }
        });
        let mut random = LegacyRandom::new(case["seed"].as_i64().unwrap_or(0));
        let name = block.split('[').next().unwrap_or_default().to_owned();
        let mut anims = BlockEntityAnims::default();
        let partner = case["partner"].as_str().map(|k| {
            let o = offset(k);
            [pos[0] + o[0], pos[1] + o[1], pos[2] + o[2]]
        });
        let mut gamers = players(&case["players"]);
        if name == "minecraft:enchanting_table" {
            anims.book_random = LegacyRandom::new(case["seed"].as_i64().unwrap_or(0));
        }
        let facing = prop(&block, "facing")
            .and_then(direction_index)
            .unwrap_or(1);
        // End gateways: the saved age, the level's height range and the
        // integrated server's copy.
        let initial = EndGateway {
            age: case["age"].as_i64().unwrap_or(0),
            teleport_cooldown: 0,
        };
        let max_y = int(&case["level"][0]) + int(&case["level"][1]) - 1;
        let mut server = initial;
        let mut text = String::new();
        for step in case["steps"].as_array().into_iter().flatten() {
            let time = step["time"].as_i64().unwrap_or(0);
            let partial = step["partial"].as_f64().unwrap_or(0.0) as f32;
            let mut log = String::new();
            place(&mut blocks, &step["set"]);
            let event = step["event"].as_array().map(|e| (int(&e[0]), int(&e[1])));
            text.clear();
            if step.get("players").is_some() {
                gamers = players(&step["players"]);
            }
            if name.ends_with("chest") {
                let lid = anims.chests.entry(pos).or_default();
                if let Some((a, b)) = event {
                    let handled = lid.trigger_event(a, b);
                    let _ = write!(log, " event {handled}");
                }
                if let Some(other) = partner {
                    let lid = anims.chests.entry(other).or_default();
                    if let Some(e) = step["partnerEvent"].as_array() {
                        let handled = lid.trigger_event(int(&e[0]), int(&e[1]));
                        let _ = write!(log, " partner event {handled}");
                    }
                }
                // lidAnimateTick: this chest, then the other half.
                if let Some(l) = anims.chests.get_mut(&pos) {
                    l.tick();
                }
                if let Some(l) = partner.and_then(|p| anims.chests.get_mut(&p)) {
                    l.tick();
                }
                let state_at = |p: [i32; 3]| {
                    find_state(
                        registry,
                        blocks.get(&p).map_or("minecraft:air", String::as_str),
                    )
                    .unwrap_or(0)
                };
                let (open, _) = renderer
                    .chest_openness_of(pos, state_at(pos), &state_at, &|p| anims.lid(p, partial));
                let _ = write!(
                    text,
                    "{} partner {} render {} {}",
                    lid_text(anims.chests.get(&pos)),
                    partner.map_or("none".to_owned(), |p| lid_text(anims.chests.get(&p))),
                    f(open),
                    f(chest_lid_x_rot(open))
                );
            } else if name == "minecraft:enchanting_table" {
                let feet: Vec<[f64; 3]> = gamers
                    .iter()
                    .filter(|(_, spectator)| !spectator)
                    .map(|(p, _)| *p)
                    .collect();
                let BlockEntityAnims {
                    books, book_random, ..
                } = &mut anims;
                books.entry(pos).or_default().tick(pos, &feet, book_random);
                let b = anims.books[&pos];
                let r = anims
                    .block_anim(pos, time, partial, Default::default())
                    .book
                    .render(partial);
                let _ = write!(
                    text,
                    "{} {} {} {} {} {} {} {} {} {} render {} {} {} {} book {} {} {}",
                    b.time,
                    f(b.flip),
                    f(b.o_flip),
                    f(b.flip_t),
                    f(b.flip_a),
                    f(b.open),
                    f(b.o_open),
                    f(b.rot),
                    f(b.o_rot),
                    f(b.t_rot),
                    f(r.flip),
                    f(r.open),
                    f(r.time),
                    f(r.y_rot),
                    f(r.openness),
                    f(r.page_flip1),
                    f(r.page_flip2)
                );
            } else if name.ends_with("shulker_box") {
                let s = anims.shulkers.entry(pos).or_insert_with(Shulker::default);
                if let Some((a, b)) = event {
                    let handled = s.trigger_event(a, b);
                    let _ = write!(log, " event {handled}");
                }
                let tick = s.tick(pos, facing);
                for _ in 0..tick.neighbor_updates {
                    log.push_str(" neighbors");
                }
                if let Some(b) = tick.push {
                    let _ = write!(log, " push {}", bx(b));
                }
                let (count, st) = (s.open_count, s.status);
                let a = anims.block_anim(pos, time, partial, Default::default());
                // ShulkerBoxRenderer: getProgress(partial).
                let (progress, old) = a.shulker;
                let render = old + partial * (progress - old);
                let s = &anims.shulkers[&pos];
                let _ = write!(
                    text,
                    "{} {} {} {} render {}",
                    status(st),
                    count,
                    f(s.progress),
                    f(s.progress_old),
                    f(render)
                );
            } else if name == "minecraft:bell" {
                let b = anims.bells.entry(pos).or_insert_with(Bell::default);
                if let Some((e0, e1)) = event {
                    let handled = b.trigger_event(e0, e1, time, || {
                        let inflated = [
                            f64::from(pos[0]) - 48.0,
                            f64::from(pos[1]) - 48.0,
                            f64::from(pos[2]) - 48.0,
                            f64::from(pos[0]) + 1.0 + 48.0,
                            f64::from(pos[1]) + 1.0 + 48.0,
                            f64::from(pos[2]) + 1.0 + 48.0,
                        ];
                        let _ = write!(log, " gather {}", bx(inflated));
                        (0..entities.len() as u64)
                            .filter(|&i| {
                                let p = entities[i as usize].position;
                                (0..3).all(|a| p[a] >= inflated[a] && p[a] < inflated[a + 3])
                            })
                            .collect()
                    });
                    let _ = write!(log, " event {handled}");
                }
                if !step["noTick"].as_bool().unwrap_or(false) {
                    let tick = b.tick(pos, |h| entities.get(h as usize).copied());
                    if tick.resonate {
                        log.push_str(" sound minecraft:block.bell.resonate");
                    }
                    if let Some((particles, _)) = tick.ended {
                        for (p, color) in particles {
                            let _ = write!(
                                log,
                                " particle minecraft:entity_effect {color} {} {} {} {} {} {}",
                                d(p[0]),
                                d(p[1]),
                                d(p[2]),
                                d(0.0),
                                d(0.0),
                                d(0.0)
                            );
                        }
                    }
                }
                let b = &anims.bells[&pos];
                let a = anims.block_anim(pos, time, partial, Default::default());
                let dir = |d: Option<&str>| d.map_or("null".to_owned(), str::to_owned);
                let _ = write!(
                    text,
                    "{} {} {} {} {} {} render {} {}",
                    b.ticks,
                    b.shaking,
                    dir(b.click_direction.map(|d| DIRECTIONS[usize::from(d)])),
                    b.resonating,
                    b.resonation_ticks,
                    b.last_ring_timestamp,
                    f(a.bell_ticks as f32 + partial),
                    dir(a.bell_shake)
                );
            } else if name == "minecraft:conduit" {
                let c = anims.conduits.entry(pos).or_insert_with(Conduit::default);
                let state_of = |p: [i32; 3]| -> Option<u32> {
                    find_state(
                        registry,
                        blocks.get(&p).map_or("minecraft:air", String::as_str),
                    )
                };
                let is_water = |p: [i32; 3]| {
                    state_of(p).is_some_and(|s| facts.get(BlockStateId(s)).fluid == Fluid::Water)
                };
                let is_frame = |p: [i32; 3]| {
                    let spec = blocks.get(&p).map_or("minecraft:air", String::as_str);
                    frames.contains(spec.split('[').next().unwrap_or_default())
                };
                let particles = c.client_tick(pos, time, is_water, is_frame, target, &mut random);
                for (p, v) in particles {
                    let _ = write!(
                        log,
                        " particle minecraft:nautilus {} {} {} {} {} {}",
                        d(p[0]),
                        d(p[1]),
                        d(p[2]),
                        d(v[0]),
                        d(v[1]),
                        d(v[2])
                    );
                }
                let c = &anims.conduits[&pos];
                let a = anims.block_anim(pos, time, partial, Default::default());
                // ConduitRenderer.extractRenderState as block_entity_render derives it.
                let (ticks, active, hunting, raw) = a.conduit;
                let rotation = (raw + if active { partial } else { 0.0 }) * -0.0375;
                let _ = write!(
                    text,
                    "{} {} {} {} {} render {} {} {} {} {}",
                    c.tick_count,
                    c.is_active,
                    c.is_hunting,
                    f(c.active_rotation),
                    c.effect_blocks.len(),
                    active,
                    f(rotation),
                    f(ticks as f32 + partial),
                    ticks / 66 % 3,
                    hunting
                );
            } else if name.ends_with("_head") || name.ends_with("_skull") {
                let now = step["state"].as_str().unwrap_or(&block);
                let s = anims.skulls.entry(pos).or_insert_with(Skull::default);
                s.tick(prop(now, "powered") == Some("true"));
                let s = anims.skulls[&pos];
                let a = anims.block_anim(pos, time, partial, Default::default());
                // block_entity_render: count + partial while animating.
                let (animating, count) = a.skull;
                let render = count as f32 + if animating { partial } else { 0.0 };
                let _ = write!(
                    text,
                    "{} {} render {}",
                    s.animating,
                    s.tick_count,
                    f(render)
                );
            } else if name == "minecraft:decorated_pot" {
                let w = anims.pots.entry(pos).or_insert_with(PotWobble::default);
                if let Some((e0, e1)) = event {
                    let handled = w.trigger_event(e0, e1, time);
                    let _ = write!(log, " event {handled}");
                }
                let w = anims.pots[&pos];
                let a = anims.block_anim(pos, time, partial, Default::default());
                let (style, progress) = match a.wobble {
                    Some((true, p)) => ("POSITIVE", p),
                    Some((false, p)) => ("null", p),
                    None => ("null", 0.0),
                };
                let _ = write!(
                    text,
                    "{} {} render {} {}",
                    w.started_at,
                    match w.style {
                        Some(WobbleStyle::Positive) => "POSITIVE",
                        Some(WobbleStyle::Negative) => "NEGATIVE",
                        None => "null",
                    },
                    f(progress),
                    style
                );
            } else if name == "minecraft:end_gateway" {
                let client = anims.gateways.entry(pos).or_insert(initial);
                if step["enter"].as_bool() == Some(true) && server.enter() {
                    log.push_str(" blockEvent 1 0");
                    client.trigger_event(1, 0);
                }
                if server.portal_tick() {
                    log.push_str(" blockEvent 1 0");
                    client.trigger_event(1, 0);
                }
                if let Some((e0, e1)) = event {
                    let handled = client.trigger_event(e0, e1);
                    let _ = write!(log, " event {handled}");
                }
                client.tick();
                let g = *client;
                let a = anims.block_anim(pos, time, partial, Default::default());
                let b = gateway_beam(a.gateway, partial, max_y, time);
                let _ = write!(
                    text,
                    "{} {} server {} {} render {} {} {:x} {}",
                    g.age,
                    g.teleport_cooldown,
                    server.age,
                    server.teleport_cooldown,
                    b.height,
                    f(b.scale),
                    b.color,
                    f(b.animation_time)
                );
            } else {
                text = format!("unsupported {block}");
            }
            writeln!(out, "{text} |{log}")?;
        }
    }
    Ok(())
}
