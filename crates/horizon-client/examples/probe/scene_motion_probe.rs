//! Repeated movement over the same owned actor and resident scene as the client.
#![forbid(unsafe_code)]
use horizon_client::{
    Result,
    assets::Assets,
    collisions,
    motion::{LAVA, MotionEvent, MotionResources, MotionWorld, PlayerMotion, WATER},
    world::World,
};
use horizon_core::geometry::Vec3;
use horizon_core::{position::BlockPos, random::RandomSource};
use horizon_world::attributes::{Modifier, Operation};
use horizon_world::movement_statistics::{self, MovementStat, MovementStatisticsWorld};
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, Write},
    path::PathBuf,
};
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn single(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn vector(v: &Value) -> Vec3 {
    Vec3::new(number(&v[0]), number(&v[1]), number(&v[2]))
}
fn position(p: Option<horizon_core::position::BlockPos>) -> String {
    p.map_or_else(|| "none".into(), |p| format!("{},{},{}", p.x, p.y, p.z))
}
struct StatisticsContext<'a> {
    flags: u16,
    invulnerable: bool,
    player: &'a mut PlayerMotion,
}
impl StatisticsContext<'_> {
    fn flag(&self, bit: u32) -> Result<bool> {
        Ok(self.flags & (1 << bit) != 0)
    }
}
impl MovementStatisticsWorld for StatisticsContext<'_> {
    type Error = Box<dyn std::error::Error>;
    fn passenger(&mut self) -> Result<bool> {
        self.flag(0)
    }
    fn swimming(&mut self) -> Result<bool> {
        self.flag(1)
    }
    fn eyes_in_water(&mut self) -> Result<bool> {
        self.flag(2)
    }
    fn in_water(&mut self) -> Result<bool> {
        self.flag(3)
    }
    fn on_climbable(&mut self) -> Result<bool> {
        self.flag(4)
    }
    fn on_ground(&mut self) -> Result<bool> {
        self.flag(5)
    }
    fn sprinting(&mut self) -> Result<bool> {
        self.flag(6)
    }
    fn crouching(&mut self) -> Result<bool> {
        self.flag(7)
    }
    fn fall_flying(&mut self) -> Result<bool> {
        self.flag(8)
    }
    fn award(&mut self, stat: MovementStat, amount: i32) -> Result<()> {
        self.player.events.push(MotionEvent::Health(
            horizon_client::health::HealthEvent::Statistic {
                name: stat.name(),
                amount,
            },
        ));
        Ok(())
    }
    fn exhaust(&mut self, amount: f32) -> Result<()> {
        if !self.invulnerable {
            self.player.health.food.add_exhaustion(amount);
        }
        Ok(())
    }
}
fn snapshot(out: &mut impl Write, player: &PlayerMotion) -> Result<()> {
    // Compare membership and stored values without allocating lazy instances or
    // evaluating computed values. Runtime identity hashes differ between VMs.
    let mut attributes = Vec::new();
    for (handle, key, value) in player.attributes.instances() {
        let mut modifiers: Vec<_> = value
            .modifiers()
            .iter()
            .map(|m| {
                format!(
                    "{}:{:016x}:{}:{}",
                    m.id(),
                    m.amount().to_bits(),
                    m.operation() as u8,
                    value.permanent_modifiers().iter().any(|p| p.id() == m.id())
                )
            })
            .collect();
        modifiers.sort();
        attributes.push(format!(
            "{}:{:016x}:{}:{:016x}:{}:{}:[{}]",
            key.name(),
            value.base_value().to_bits(),
            value.is_dirty(),
            value.cached_value().to_bits(),
            player.attributes.updates().contains(Some(handle)),
            player.attributes.sync().contains(Some(handle)),
            modifiers.join(",")
        ));
    }
    attributes.sort();
    for v in [
        player.position,
        player.velocity,
        player.bounds().min_position(),
        player.bounds().max_position(),
    ] {
        write!(
            out,
            "{:016x} {:016x} {:016x} ",
            v.x.to_bits(),
            v.y.to_bits(),
            v.z.to_bits()
        )?;
    }
    writeln!(
        out,
        "{} {} {} {} {} {} {} {:016x} {} {} {} {:08x} {} {:08x} {} {} {} | {}",
        player.collision.horizontal,
        player.collision.vertical,
        player.collision.below,
        player.collision.minor,
        player.ground.on_ground,
        player.ground.on_ground_no_blocks,
        position(player.ground.supporting_block),
        player.fall_distance.to_bits(),
        position(player.last_climbable),
        player.needs_sync,
        player.sync_position,
        player.speed.to_bits(),
        player.no_jump_delay,
        player.head_yaw.to_bits(),
        player.jump_trigger_time,
        player.impulse.reset_grace_time,
        player.impulse.impact_position.map_or_else(
            || "none".into(),
            |p| format!(
                "{:016x},{:016x},{:016x}",
                p.x.to_bits(),
                p.y.to_bits(),
                p.z.to_bits()
            )
        ),
        attributes.join(";")
    )?;
    let events: Vec<_> = player
        .events
        .iter()
        .filter_map(|e| match e {
            MotionEvent::Fall(e) => Some(e.trace()),
            // The oracle's player wears no armor; the game applies this one.
            MotionEvent::Health(horizon_client::health::HealthEvent::ArmorHurt { .. }) => None,
            MotionEvent::Health(e) => Some(e.trace()),
            MotionEvent::Snowflake { position, velocity } => Some(format!(
                "snowflake {:016x},{:016x},{:016x} {:016x},{:016x},{:016x}",
                position.x.to_bits(),
                position.y.to_bits(),
                position.z.to_bits(),
                velocity.x.to_bits(),
                velocity.y.to_bits(),
                velocity.z.to_bits()
            )),
            MotionEvent::Splash {
                high_speed,
                volume,
                pitch,
            } => Some(format!(
                "sound {} entity {:08x} {:08x}",
                if *high_speed { "splash-high" } else { "splash" },
                volume.to_bits(),
                pitch.to_bits()
            )),
            MotionEvent::SplashParticle {
                bubble,
                position,
                velocity,
            } => Some(format!(
                "splash-particle {} {:016x},{:016x},{:016x} {:016x},{:016x},{:016x}",
                if *bubble { "bubble" } else { "splash" },
                position.x.to_bits(),
                position.y.to_bits(),
                position.z.to_bits(),
                velocity.x.to_bits(),
                velocity.y.to_bits(),
                velocity.z.to_bits()
            )),
            MotionEvent::SplashEvent => Some("splash-event".into()),
            MotionEvent::Swim { volume, pitch } => Some(format!(
                "sound swim entity {:08x} {:08x}",
                volume.to_bits(),
                pitch.to_bits()
            )),
            MotionEvent::Emission {
                state,
                muffled: true,
                ..
            } => Some(format!("muffled-step {}", state.0)),
            MotionEvent::BubbleColumnParticle { bubble, position } => Some(format!(
                "column-particle {} {:016x},{:016x},{:016x} 1 {:016x},{:016x},{:016x} {:016x}",
                if *bubble { "bubble" } else { "splash" },
                position.x.to_bits(),
                position.y.to_bits(),
                position.z.to_bits(),
                0f64.to_bits(),
                if *bubble { 0.01f64 } else { 0.0 }.to_bits(),
                0f64.to_bits(),
                if *bubble { 0.2f64 } else { 1.0 }.to_bits()
            )),
            MotionEvent::Drown => Some("broadcast 67".into()),
            // The oracle marks every getAdvancements call (ENTER_BLOCK.trigger) so.
            MotionEvent::EnterBlock { .. } => Some("hurt-criterion".into()),
            MotionEvent::Waypoint { receiver, enabled } => Some(format!(
                "waypoint {} {enabled}",
                if *receiver { "receive" } else { "transmit" }
            )),
            _ => None,
        })
        .collect();
    writeln!(out, "events {}", events.join(" | "))?;
    Ok(())
}
pub fn main() -> Result<()> {
    let args: Vec<_> = crate::args().skip(1).collect();
    if args.len() != 3 {
        return Err(
            "usage: scene_motion_probe RESOURCES FIXTURE_DIRECTORY --describe|REFERENCE".into(),
        );
    }
    let root = PathBuf::from(&args[0]);
    let dir = PathBuf::from(&args[1]);
    let assets = Assets::load(&root)?;
    let render = assets.reload(&root)?;
    let mut scene = World::test_scene(&assets, &render, 1)?;
    let binding = collisions::load(&root, assets.registry)?;
    let resources = MotionResources::load(&root, &assets)?;
    fs::write(
        dir.join("scene.json"),
        serde_json::to_vec(
            &serde_json::json!({"radius":1,"resident_chunks":scene.chunks.len(),"model_gallery":render.model_gallery}),
        )?,
    )?;
    if args[2] == "--describe" {
        return Ok(());
    }
    let side = &args[2];
    let mut out = std::io::BufWriter::new(fs::File::create(dir.join(format!("rust-{side}.txt")))?);
    for line in
        std::io::BufReader::new(fs::File::open(dir.join(format!("inputs-{side}.txt")))?).lines()
    {
        let f: Value = serde_json::from_str(&line?)?;
        scene.health_rules = horizon_client::health::HealthRules::default();
        let hashes: Vec<_> = resources
            .attributes
            .attributes()
            .map(|(_, a)| {
                f["holder_hashes"][&a.name]
                    .as_i64()
                    .expect("missing holder identity") as i32
            })
            .collect();
        let mut instances = f["instance_hashes"]
            .as_array()
            .ok_or("missing instance identities")?
            .iter()
            .map(|v| v.as_i64().unwrap() as i32)
            .collect::<Vec<_>>()
            .into_iter();
        let count = instances.len();
        let used = std::rc::Rc::new(std::cell::Cell::new(0));
        let counter = used.clone();
        let mut player = PlayerMotion::with_attribute_identities(
            &resources,
            vector(&f["position"]),
            &hashes,
            Box::new(move || {
                counter.set(counter.get() + 1);
                instances.next().expect("unexpected instance allocation")
            }),
        )?;
        // The oracle runs block-inside effects only in "snow_tick".
        player.block_effects = false;
        // Entity.portalCooldown and portalProcess, and the entry of the last
        // portal_tick that went through.
        let mut portal = horizon_client::nether_portal::PortalState::default();
        let mut travel: Option<horizon_core::position::BlockPos> = None;
        for op in f["ops"].as_array().unwrap() {
            let mut world = MotionWorld {
                entities: &[],
                player: &mut player,
                scene: &mut scene,
                assets: &assets,
                collision_binding: &binding,
                resources: &resources,
            };
            match op["mode"].as_str().unwrap() {
                "shift" => world.player.shift_key_down = op["value"].as_bool().unwrap(),
                "pose_tick" => world.update_player_pose()?,
                "refresh_dimensions" => world.player.refresh_dimensions()?,
                "refresh_attributes" => world.player.refresh_attributes()?,
                "pose" => world.player.set_pose(match op["value"].as_str().unwrap() {
                    "STANDING" => horizon_client::pose::Pose::Standing,
                    "CROUCHING" => horizon_client::pose::Pose::Crouching,
                    "SWIMMING" => horizon_client::pose::Pose::Swimming,
                    _ => return Err("unknown fixture pose".into()),
                })?,
                "controlled_tick" => {
                    let mask = op["value"].as_u64().unwrap();
                    world.controlled_simulation_tick(horizon_client::sprint::Keys {
                        forward: mask & 1 != 0,
                        backward: mask & 2 != 0,
                        left: mask & 4 != 0,
                        right: mask & 8 != 0,
                        jump: mask & 16 != 0,
                        shift: mask & 32 != 0,
                        sprint: mask & 64 != 0,
                        analog: None,
                    })?;
                }
                "movement_statistics" => world.check_movement_statistics(vector(&op["value"]))?,
                "statistics_context" => movement_statistics::check(
                    &mut StatisticsContext {
                        player: world.player,
                        flags: op["flags"].as_u64().unwrap() as u16,
                        invulnerable: op["invulnerable"].as_bool().unwrap(),
                    },
                    vector(&op["value"]),
                )?,
                "tick_count" => {
                    world.player.health.tick_count = op["value"].as_i64().unwrap() as i32
                }
                "rules" => {
                    world.scene.health_rules = horizon_client::health::HealthRules {
                        difficulty: [
                            horizon_world::food::Difficulty::Peaceful,
                            horizon_world::food::Difficulty::Easy,
                            horizon_world::food::Difficulty::Normal,
                            horizon_world::food::Difficulty::Hard,
                        ][op["difficulty"].as_u64().unwrap() as usize],
                        natural_regeneration: op["regenerate"].as_bool().unwrap(),
                        ..Default::default()
                    };
                }
                "food" => {
                    world.player.health.food = horizon_world::food::FoodData {
                        food: op["food"].as_i64().unwrap() as i32,
                        saturation: single(&op["saturation"]),
                        exhaustion: single(&op["exhaustion"]),
                        timer: op["timer"].as_i64().unwrap() as i32,
                    };
                }
                "exhaust" => world
                    .player
                    .health
                    .food
                    .add_exhaustion(single(&op["value"])),
                "eat" => world.player.health.food.eat(
                    op["nutrition"].as_i64().unwrap() as i32,
                    single(&op["value"]),
                ),
                "food_tick" => world
                    .player
                    .food_tick(&resources.health, world.scene.health_rules)?,
                "peaceful" => world
                    .player
                    .peaceful_regeneration(world.scene.health_rules)?,
                "simulation_tick" => {
                    let v = vector(&op["value"]);
                    world.simulation_tick(
                        [v.x as f32, v.z as f32],
                        op["jumping"].as_bool().unwrap(),
                    )?;
                }
                "clock" => world.player.health.advance_clock(),
                "health" | "absorption" => world
                    .player
                    .set_health(single(&op["value"]), op["mode"] == "absorption")?,
                "hurt" => {
                    let source = resources
                        .health
                        .damage
                        .id(op["name"].as_str().unwrap())
                        .ok_or("missing fixture damage source")?;
                    let kind = match op["name"].as_str().unwrap() {
                        "minecraft:fall" => horizon_world::block_landing::LandingDamage::Fall,
                        "minecraft:stalagmite" => {
                            horizon_world::block_landing::LandingDamage::Stalagmite
                        }
                        _ => return Err("fixture source is not a landing source".into()),
                    };
                    let amount = single(&op["value"]);
                    world.player.events.push(MotionEvent::Fall(
                        horizon_client::landing::FallEvent::Damage {
                            source: kind,
                            amount,
                        },
                    ));
                    world.player.hurt(
                        &resources.health,
                        world.scene.health_rules,
                        source,
                        amount,
                    )?;
                }
                "seed" => world.scene.random.set_seed(op["value"].as_i64().unwrap()),
                "impulse" => world.player.impulse.set_ignore_fall_damage(
                    op["ignore"].as_bool().unwrap(),
                    if op["value"].is_null() {
                        None
                    } else {
                        Some(vector(&op["value"]))
                    },
                ),
                // A block, or every block of the box from position to "to".
                "block" | "fill" => {
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
                    let at = |v: &Value, i: usize| v[i].as_i64().unwrap() as i32;
                    let to = if op["to"].is_array() { &op["to"] } else { p };
                    for x in at(p, 0)..=at(to, 0) {
                        for y in at(p, 1)..=at(to, 1) {
                            for z in at(p, 2)..=at(to, 2) {
                                world.scene.set(x, y, z, state)?;
                            }
                        }
                    }
                }
                "landing" => {
                    let p = &op["position"];
                    let p = BlockPos::new(
                        p[0].as_i64().unwrap() as i32,
                        p[1].as_i64().unwrap() as i32,
                        p[2].as_i64().unwrap() as i32,
                    );
                    let state = world.scene.block(p.x, p.y, p.z);
                    horizon_world::entity_move::EntityMoveWorld::check_fall_damage(
                        &mut world,
                        number(&op["value"]),
                        op["ground"].as_bool().unwrap(),
                        state,
                        p,
                    )?;
                }
                // Entity.baseTick's fluid phase alone (wasEyeInWater,
                // updateFluidInteraction, lava fall halving, firstTick).
                "fluid_tick" => world.fluid_base_tick()?,
                // LivingEntity.baseTick's air supply alone.
                "air_tick" => world.tick_environment_damage()?,
                "air" => world.player.air = op["value"].as_i64().unwrap() as i32,
                "pitch" => world.player.pitch = single(&op["value"]),
                "swimming" => world.player.swimming = op["value"].as_bool().unwrap(),
                "effect" => {
                    let on = op["value"].as_bool().unwrap();
                    let status = &mut world.player.status;
                    match op["name"].as_str().unwrap() {
                        "minecraft:dolphins_grace" => status.dolphins_grace = on,
                        "minecraft:water_breathing" | "minecraft:conduit_power" => {
                            status.water_breathing = on
                        }
                        "minecraft:breath_of_the_nautilus" => status.breath_of_the_nautilus = on,
                        other => return Err(format!("unknown fixture effect {other}").into()),
                    }
                }
                "travel" => world.travel_in_air(vector(&op["value"]))?,
                "tick" => {
                    let v = vector(&op["value"]);
                    world.movement_tick([v.x as f32, v.y as f32, v.z as f32])?;
                }
                "player_tick" => {
                    let v = vector(&op["value"]);
                    world.player_movement_tick(
                        [v.x as f32, v.z as f32],
                        op["jumping"].as_bool().unwrap(),
                    )?;
                }
                "snow_tick" => {
                    // The clock, then the player tick: baseTick's powder snow
                    // flags lead PlayerMotion::player_movement_tick.
                    world.player.health.advance_clock();
                    let v = vector(&op["value"]);
                    world.player.block_effects = true;
                    let ticked = world.player_movement_tick(
                        [v.x as f32, v.z as f32],
                        op["jumping"].as_bool().unwrap(),
                    );
                    world.player.block_effects = false;
                    ticked?;
                }
                "boots" => {
                    // Leather boots are also freeze-immune wearables.
                    let boots = op["value"].as_bool().unwrap();
                    world.player.leather_boots = boots;
                    world.player.freeze_immune = boots;
                }
                "frozen" => world.player.ticks_frozen = op["value"].as_i64().unwrap() as i32,
                "portal_tick" => {
                    // Entity.handlePortal without the destination: players wait
                    // the creative delay (0) when invulnerable, else 80 ticks.
                    let creative = op["creative"].as_bool().unwrap();
                    let transition = if creative { 0 } else { 80 };
                    let alive = !world.player.health.dead_or_dying();
                    travel = portal.handle(alive, transition, 10);
                }
                "weaving" => world.player.status.weaving = op["value"].as_bool().unwrap(),
                "flying" => {
                    let flying = op["value"].as_bool().unwrap();
                    world.player.may_fly = op["may_fly"].as_bool().unwrap_or(flying);
                    world.player.flying = flying;
                    if let Some(speed) = op.get("speed") {
                        world.player.flying_speed = single(speed);
                    }
                }
                "trigger" => world.player.jump_trigger_time = op["value"].as_i64().unwrap() as i32,
                "delay" => world.player.no_jump_delay = op["value"].as_i64().unwrap() as i32,
                "move" => world.move_self(vector(&op["value"]))?,
                "jump" => world.jump_from_ground()?,
                "sprint" => world.player.set_sprinting(op["value"].as_bool().unwrap())?,
                "speed" => world.player.speed = single(&op["value"]),
                "yaw" => world.player.yaw = single(&op["value"]),
                "velocity" => world.player.velocity = vector(&op["value"]),
                "stuck" => world.player.stuck_multiplier = vector(&op["value"]),
                "fall" => world.player.fall_distance = number(&op["value"]),
                "jumping" => world.player.jumping = op["value"].as_bool().unwrap(),
                "ack_attributes" => {
                    world.player.attributes.updates_mut().clear();
                    world.player.attributes.sync_mut().clear();
                }
                "modifier" => {
                    let key = world
                        .player
                        .attributes
                        .key(op["name"].as_str().unwrap())
                        .unwrap();
                    let handle = world
                        .player
                        .attributes
                        .get_instance(Some(key))
                        .map_err(|e| format!("{e:?}"))?
                        .unwrap();
                    let operation = match op["operation"].as_u64().unwrap() {
                        0 => Operation::AddValue,
                        1 => Operation::AddMultipliedBase,
                        2 => Operation::AddMultipliedTotal,
                        _ => return Err("unknown modifier operation".into()),
                    };
                    let modifier =
                        Modifier::new(op["id"].as_str().unwrap(), number(&op["value"]), operation)?;
                    world
                        .player
                        .attributes
                        .edit(&handle, |a, o| {
                            if op["permanent"].as_bool().unwrap() {
                                a.add_permanent(modifier, o)
                            } else {
                                a.add_transient(modifier, o)
                            }
                        })
                        .map_err(|e| format!("{e:?}"))?;
                }
                "attribute" => {
                    let key = world
                        .player
                        .attributes
                        .key(op["name"].as_str().unwrap())
                        .unwrap();
                    let handle = world
                        .player
                        .attributes
                        .get_instance(Some(key))
                        .map_err(|e| format!("{e:?}"))?
                        .unwrap();
                    world
                        .player
                        .attributes
                        .edit(&handle, |a, o| a.set_base_value(number(&op["value"]), o))
                        .map_err(|e| format!("{e:?}"))?;
                }
                _ => return Err("unknown motion operation".into()),
            }
            // NetherPortalBlock.entityInside: canUsePortal, setAsInsidePortal.
            for e in &player.events {
                if let MotionEvent::InsidePortal { pos } = e
                    && !player.health.dead_or_dying()
                {
                    portal.set_inside(*pos, 10);
                }
            }
            let check_block = scene.block(25, 3, 25);
            snapshot(&mut out, &player)?;
            let stuck = player.stuck_multiplier;
            writeln!(
                out,
                "block {} frozen {} {} {} {:016x},{:016x},{:016x} fluid {} {} {} {} {:016x} {:016x} swim {} {} air {}",
                check_block.0,
                player.ticks_frozen,
                player.in_powder_snow,
                player.was_in_powder_snow,
                stuck.x.to_bits(),
                stuck.y.to_bits(),
                stuck.z.to_bits(),
                player.fluids.was_touching_water,
                player.fluids.was_eye_in_water,
                player.is_eye_in_fluid(WATER),
                player.fluids.first_tick,
                player.fluid_height(WATER).to_bits(),
                player.fluid_height(LAVA).to_bits(),
                player.swimming,
                player.fluids.was_eye_in_water,
                player.air
            )?;
            writeln!(
                out,
                "portal {} {} {}",
                portal.cooldown,
                portal.process.map_or("none".into(), |p| format!(
                    "{},{},{} {} {}",
                    p.entry.x, p.entry.y, p.entry.z, p.time, p.inside
                )),
                travel
                    .take()
                    .map_or("-".into(), |t| format!("{},{},{}", t.x, t.y, t.z))
            )?;
            let h = &player.health;
            let canonical = |v: f32| {
                if v.is_nan() {
                    f32::NAN.to_bits()
                } else {
                    v.to_bits()
                }
            };
            writeln!(
                out,
                "health {:08x} {:08x} {:08x} {} {} {:08x} {} {} {} {} {} {} {} {} {} {}",
                canonical(h.health()),
                canonical(h.absorption()),
                canonical(h.food.exhaustion),
                h.hurt.no_action_time,
                h.hurt.cooldown,
                canonical(h.hurt.last_hurt),
                h.hurt.hurt_duration,
                h.hurt.hurt_time,
                h.hurt.last_source.map_or("none", |id| resources
                    .health
                    .damage
                    .definition(id)
                    .unwrap()
                    .name
                    .as_str()),
                h.hurt.last_stamp,
                h.dead,
                h.sync_velocity,
                h.data.is_dirty(),
                h.data.item_dirty(h.ids.health).unwrap(),
                h.data.item_dirty(h.ids.absorption).unwrap(),
                h.game_time
            )?;
            writeln!(
                out,
                "food {} {:08x} {} {} {} {}",
                h.food.food,
                canonical(h.food.saturation),
                h.food.timer,
                h.tick_count,
                scene.health_rules.difficulty as u8,
                scene.health_rules.natural_regeneration
            )?;
            player.events.clear();
            writeln!(
                out,
                "input {} {} {} {}",
                player.sprinting,
                player.sprint_input.trigger_time,
                player.sprint_input.previous_forward,
                player.sprint_input.previous_shift
            )?;
            writeln!(
                out,
                "pose {} {} {} {:08x} {:08x} {:08x}",
                player.pose.name(),
                player.shift_key_down,
                player.local_crouching,
                player.dimensions.width.to_bits(),
                player.dimensions.height.to_bits(),
                player.dimensions.eye_height.to_bits()
            )?;
        }
        writeln!(out, "end")?;
        if used.get() != count {
            return Err("reference instance allocation count differs".into());
        }
    }
    Ok(())
}
