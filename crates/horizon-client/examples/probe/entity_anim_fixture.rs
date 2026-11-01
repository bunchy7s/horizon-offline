//! Replays the render states of the vanilla entity-animation oracle through
//! Horizon's `mobs::anim::setup` and writes every part's pose, then draws the
//! posed model with `mobs::render::draw` under the case's living-entity root
//! transform and writes each drawn part's packed vertex normals.
//!
//! cargo run --profile fast -p horizon-client --example probe -- entity_anim_fixture \
//!     RESOURCES VANILLA.jsonl OUT.jsonl
use horizon_client::mesher::Vertex;
use horizon_client::mobs::{
    Mob, MobData, MobTypes, Part, Species, allay,
    anim::{self, Poses, State},
    aquatic, bat, body_anim as b_anim, equine, guardian,
    keyframes::AnimationState,
    render, shulker, sniffer, species, tame,
};
use horizon_core::geometry::Vec3;
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::PathBuf, sync::Arc};

fn walk(part: &Part, path: &str, poses: &Poses, out: &mut BTreeMap<String, Value>) {
    let p = poses.pose(part);
    let mut polys = 0;
    let mut sum = 0.0f64;
    for polygon in &part.polygons {
        polys += 1;
        for v in polygon.vertices {
            sum += f64::from(v[0])
                + 2.0 * f64::from(v[1])
                + 3.0 * f64::from(v[2])
                + 5.0 * f64::from(v[3])
                + 7.0 * f64::from(v[4]);
        }
    }
    let visible = !poses.is_hidden(part) && !poses.skips(part);
    let mut row: Vec<Value> = p.iter().map(|&x| json!(x)).collect();
    row.push(json!(u8::from(visible)));
    row.push(json!(polys));
    row.push(json!(sum));
    out.insert(path.to_owned(), Value::Array(row));
    for c in &part.children {
        let child = if path.is_empty() {
            c.name.clone()
        } else {
            format!("{path}/{}", c.name)
        };
        walk(c, &child, poses, out);
    }
}

/// Splits the vertices `render::draw` emitted for `part` into its drawn parts,
/// following draw's walk: a hidden or zero-scaled part ends its branch, a
/// skip-draw part keeps its children. Per part: n = packed normal bytes per
/// vertex, p = sum of x + 2y + 3z over the positions.
fn drawn(
    part: &Part,
    path: &str,
    poses: &Poses,
    vertices: &[Vertex],
    cursor: &mut usize,
    out: &mut BTreeMap<String, Value>,
) {
    let p = poses.pose(part);
    if poses.is_hidden(part) || p[6] == 0.0 && p[7] == 0.0 && p[8] == 0.0 {
        return;
    }
    let count = if poses.skips(part) {
        0
    } else {
        part.polygons.len() * 4
    };
    if count > 0 {
        let own = &vertices[*cursor..*cursor + count];
        let normals: Vec<i8> = own.iter().flat_map(|v| v.normal[..3].to_vec()).collect();
        let sum: f64 = own
            .iter()
            .map(|v| f64::from(v.pos[0]) + 2.0 * f64::from(v.pos[1]) + 3.0 * f64::from(v.pos[2]))
            .sum();
        out.insert(path.to_owned(), json!({"n": normals, "p": sum}));
    }
    *cursor += count;
    for c in &part.children {
        let child = if path.is_empty() {
            c.name.clone()
        } else {
            format!("{path}/{}", c.name)
        };
        drawn(c, &child, poses, vertices, cursor, out);
    }
}

/// LivingEntityRenderer's root transform without the entity's position:
/// rotate 180 - yaw about Y, scale(-1, -1, 1), the renderer's own scale, then
/// translate(0, -1.501, 0) (as `render::base_transform`).
fn root_transform(case: &Value) -> glam::Mat4 {
    let yaw = case["@rootYaw"].as_f64().unwrap_or(0.0) as f32;
    let scale = case["@rootScale"].as_array().map_or(glam::Vec3::ONE, |s| {
        glam::Vec3::from_array(std::array::from_fn(|i| s[i].as_f64().unwrap_or(1.0) as f32))
    });
    glam::Mat4::from_rotation_y((180.0 - yaw).to_radians())
        * glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0))
        * glam::Mat4::from_scale(scale)
        * glam::Mat4::from_translation(glam::Vec3::new(0.0, -1.501, 0.0))
}

/// The entity-side values behind the render-state fields the case sets, for
/// the species whose Horizon animation reads them from the `Mob`.
fn entity_state(mob: &mut Mob, species: Species, case: &Value) {
    let flag = |k: &str| case[k].as_bool().unwrap_or(false);
    let num = |k: &str| case[k].as_f64().unwrap_or(0.0) as f32;
    match species {
        Species::Bat => {
            mob.extra.flag = flag("isResting");
            mob.anim.states[0].started = case["flyAnimationState"].as_f64().map(|v| v as f32);
            mob.anim.states[1].started = case["restAnimationState"].as_f64().map(|v| v as f32);
            if let Some(ticks) = case["@batTicks"].as_array() {
                for (i, flags) in ticks.iter().enumerate() {
                    mob.age = i as u32 + 1;
                    bat::set_resting(mob, flags.as_i64().unwrap_or(0) & 1 != 0);
                    if flag("@batInFrame") {
                        bat::tick(mob);
                    } else {
                        bat::setup_animation_states(mob);
                    }
                }
            }
        }
        Species::Sheep => mob.extra.anim_ticks = num("eatTick") as i32,
        Species::Guardian | Species::ElderGuardian => {
            mob.anim.v[guardian::TAIL] = num("tailAnimation");
            mob.anim.vo[guardian::TAIL] = num("tailAnimation");
            mob.anim.v[guardian::SPIKES] = num("spikesAnimation");
            mob.anim.vo[guardian::SPIKES] = num("spikesAnimation");
        }
        Species::Shulker => {
            mob.anim.v[shulker::PEEK] = num("peekAmount");
            mob.anim.vo[shulker::PEEK] = num("peekAmount");
        }
        Species::Zombie
        | Species::Husk
        | Species::Drowned
        | Species::ZombieVillager
        | Species::ZombifiedPiglin
        | Species::Giant
        | Species::Skeleton
        | Species::Stray
        | Species::Bogged
        | Species::WitherSkeleton
        | Species::Parched => {
            mob.ai.aggressive = flag("isAggressive");
            mob.extra.crouching = flag("isCrouching");
            mob.extra.passenger = flag("isPassenger");
            if flag("isHoldingBow") {
                mob.extra.set_main_hand(Some("minecraft:bow".into()));
            }
        }
        Species::IronGolem => {
            mob.extra.anim_ticks = num("attackTicksRemaining") as i32;
            mob.extra.counter = num("offerFlowerTick") as i32;
        }
        Species::Hoglin | Species::Zoglin => {
            mob.extra.anim_ticks = num("attackAnimationRemainingTicks") as i32;
        }
        Species::Ravager => {
            mob.extra.counter2 = num("stunnedTicksRemaining") as i32;
            mob.extra.counter = num("attackTicksRemaining") as i32;
            let roar = num("roarAnimation");
            if roar > 0.0 {
                mob.extra.anim_ticks = (20.0 - roar * 20.0) as i32;
            }
        }
        Species::Vex => mob.extra.flag = flag("isCharging"),
        Species::Squid | Species::GlowSquid => {
            mob.anim.v[aquatic::TENTACLE_ANGLE] = num("tentacleAngle");
            mob.anim.vo[aquatic::TENTACLE_ANGLE] = num("tentacleAngle");
        }
        Species::Dolphin => {
            if flag("isMoving") {
                mob.velocity.x = 0.1;
            }
        }
        Species::Cod | Species::Salmon | Species::TropicalFish | Species::Tadpole => {
            mob.fluids.in_water = flag("isInWater");
        }
        Species::Allay => {
            mob.extra.flag = flag("isDancing");
            mob.extra.ints[4] = if flag("isSpinning") { 10 } else { 20 };
            mob.anim.v[allay::SPIN] = num("spinningProgress") * 15.0;
            mob.anim.vo[allay::SPIN] = num("spinningProgress") * 15.0;
            mob.anim.v[allay::HOLD] = num("holdingAnimationProgress") * 5.0;
            mob.anim.vo[allay::HOLD] = num("holdingAnimationProgress") * 5.0;
        }
        Species::Horse
        | Species::Donkey
        | Species::Mule
        | Species::ZombieHorse
        | Species::SkeletonHorse => {
            for (slot, key) in [
                (equine::EAT, "eatAnimation"),
                (equine::STAND, "standAnimation"),
                (equine::MOUTH, "feedingAnimation"),
            ] {
                mob.anim.v[slot] = num(key);
                mob.anim.vo[slot] = num(key);
            }
            mob.fluids.in_water = flag("isInWater");
            mob.extra.chested = flag("hasChest");
        }
        Species::Camel => {
            for (i, k) in [
                "sitAnimationState",
                "sitPoseAnimationState",
                "sitUpAnimationState",
                "idleAnimationState",
                "dashAnimationState",
            ]
            .into_iter()
            .enumerate()
            {
                mob.anim.states[i].started = case[k].as_f64().map(|v| v as f32);
            }
            mob.extra.anim_ticks = num("jumpCooldown") as i32;
        }
        Species::Chicken => {
            mob.anim.flap = num("flap");
            mob.anim.flap_o = num("flap");
            mob.anim.flap_speed = num("flapSpeed");
            mob.anim.flap_speed_o = num("flapSpeed");
        }
        Species::Wolf => {
            mob.extra.sitting = flag("isSitting");
            mob.extra.anger = if flag("isAngry") { 100 } else { 0 };
            mob.anim.v[tame::SHAKE] = num("shakeAnim");
            mob.anim.vo[tame::SHAKE] = num("shakeAnim");
            mob.anim.v[tame::INTEREST] = num("interest");
            mob.anim.vo[tame::INTEREST] = num("interest");
        }
        Species::Creaking => {
            mob.extra.flag2 = !flag("canMove");
            for (i, k) in [
                "attackAnimationState",
                "invulnerabilityAnimationState",
                "deathAnimationState",
            ]
            .into_iter()
            .enumerate()
            {
                mob.anim.states[i].started = case[k].as_f64().map(|v| v as f32);
            }
        }
        Species::Breeze => {
            // breeze::anim's state slots.
            for (i, k) in ["idle", "shoot", "slide", "slideBack", "inhale", "longJump"]
                .into_iter()
                .enumerate()
            {
                mob.anim.states[i].started = case[k].as_f64().map(|v| v as f32);
            }
        }
        Species::Sniffer => {
            if flag("isSearching") {
                mob.extra.variant = sniffer::SEARCHING;
            }
            // Sniffer's animation states, in `sniffer::anim`'s order.
            for (i, k) in [
                "feelingHappyAnimationState",
                "scentingAnimationState",
                "sniffingAnimationState",
                "diggingAnimationState",
                "risingAnimationState",
            ]
            .into_iter()
            .enumerate()
            {
                mob.anim.states[i].started = case[k].as_f64().map(|v| v as f32);
            }
        }
        Species::HappyGhast => {
            mob.extra.head = case["bodyItem"].as_str().map(Into::into);
            if flag("isRidden") {
                mob.rider = Some(Default::default());
            }
        }
        Species::Warden => {
            use horizon_client::mobs::warden;
            mob.anim.v[warden::TENDRIL] = num("tendrilAnimation") * 10.0;
            mob.anim.vo[warden::TENDRIL] = num("tendrilAnimation") * 10.0;
            mob.anim.v[warden::HEART] = num("heartAnimation") * 10.0;
            mob.anim.vo[warden::HEART] = num("heartAnimation") * 10.0;
            for (i, k) in [
                (warden::ROAR, "roarAnimationState"),
                (warden::SNIFF, "sniffAnimationState"),
                (warden::EMERGE, "emergeAnimationState"),
                (warden::DIGGING, "diggingAnimationState"),
                (warden::ATTACK, "attackAnimationState"),
                (warden::SONIC_BOOM, "sonicBoomAnimationState"),
            ] {
                mob.anim.states[i].started = case[k].as_f64().map(|v| v as f32);
            }
        }
        Species::Witch if flag("isHoldingItem") => {
            mob.extra.set_main_hand(Some("minecraft:potion".into()));
        }
        _ => {}
    }
}

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().skip(1).collect();
    let root = PathBuf::from(&args[0]);
    let types = MobTypes::load(&root)?;
    let animations = MobData::load_animations(&root);
    let mut out = String::new();
    let crystal = horizon_client::mobs::load_model(&root, "minecraft:end_crystal#main");
    for line in fs::read_to_string(&args[1])?.lines() {
        let v: Value = serde_json::from_str(line)?;
        let layer = v["layer"]
            .as_str()
            .unwrap_or_default()
            .trim_start_matches("minecraft:");
        // EndCrystalModel is not a mob's: horizon_client::end_crystal poses it.
        if layer == "end_crystal#main"
            && let Some(model) = &crystal
        {
            let case = &v["case"];
            let mut poses = horizon_client::end_crystal::poses(
                model,
                case["ageInTicks"].as_f64().unwrap_or(0.0) as f32,
                case["showsBottom"].as_bool().unwrap_or(false),
            );
            out.push_str(&format!("{}\n", emit(model, &mut poses, case, layer)));
            continue;
        }
        let baby = layer.contains("_baby#");
        let adult = layer.replace("_baby#", "#");
        // Variant layers (sizes, hatless villagers, climate cows...) animate
        // like their base model.
        let mut base = adult.clone();
        // Armour layers (horse, wolf, nautilus), the coral zombie nautilus,
        // the happy ghast's harness and ropes and hatless babies animate like
        // their base model too; the undead horses share one armour layer.
        let cuts = [
            "_no_hat", "_small", "_medium", "_large", "_big", "_armor", "_coral", "_harness",
            "_ropes", "_baby",
        ];
        let norm = |s: &str| {
            let mut s = s.to_owned();
            for cut in cuts {
                s = s.replace(cut, "");
            }
            s.replace("undead_horse#", "zombie_horse#")
        };
        for cut in cuts {
            base = base.replace(cut, "");
        }
        let base = base.replace("undead_horse#", "zombie_horse#");
        // The breeze's wind and eyes and the creaking's eyes are layers of
        // the species' own model class; LlamaDecorLayer's model animates like
        // the llama.
        let base = base
            .replace("#wind", "#main")
            .replace("#eyes", "#main")
            .replace("#decor", "#main");
        let base = base.trim_start_matches("cold_").trim_start_matches("warm_");
        let Some(def) = species::ALL.iter().find(|d| {
            d.model == layer
                || (baby && d.model == adult)
                || d.model == base
                || d.model.trim_end_matches("#main") == base.trim_end_matches("#main")
                || norm(d.model) == base
                // The warden's emissive layers are its model over other parts.
                || (["#tendrils", "#heart", "#bioluminescent", "#pulsating_spots"]
                    .iter()
                    .any(|s| layer.ends_with(s))
                    && d.model == format!("{}#main", layer.split('#').next().unwrap_or_default()))
        }) else {
            out.push_str(&format!("{}\n", json!({"skip": "no species"})));
            continue;
        };
        let Some(kind) = types.types.get(&format!("minecraft:{}", def.name)) else {
            out.push_str(&format!("{}\n", json!({"skip": "no type"})));
            continue;
        };
        let kind = Arc::new(kind.clone());
        let model = kind
            .layer(layer)
            .cloned()
            .unwrap_or_else(|| kind.model.clone());
        let mut mob = Mob::new(7, kind, Vec3::new(0.0, 64.0, 0.0), 0.0);
        mob.extra.set_main_hand(None);
        let case = &v["case"];
        let f = |k: &str| case[k].as_f64().unwrap_or(0.0) as f32;
        if baby {
            mob.extra.age = -24000;
        }
        mob.age = f("ageInTicks") as u32;
        let mut state = State {
            pos: f("walkAnimationPos"),
            speed: f("walkAnimationSpeed"),
            age: f("ageInTicks"),
            y_rot: f("yRot"),
            x_rot: f("xRot"),
            swing: f("swingAnimation"),
            baby,
            partial: 0.0,
        };
        let mut poses = Poses::new(&model);
        entity_state(&mut mob, def.species, case);
        if let Some(ticks) = case["@walkTicks"].as_array() {
            for speed in ticks {
                mob.previous_limb_speed = mob.limb_speed;
                mob.limb_speed += (speed.as_f64().unwrap() as f32 - mob.limb_speed) * 0.4;
                mob.limb_position += mob.limb_speed;
            }
            if !case["@alive"].as_bool().unwrap() {
                mob.health = 0.0;
            }
            let extracted = anim::state(&mob, f("@partial"));
            state.pos = extracted.pos;
            state.speed = extracted.speed;
        }

        let flag = |k: &str| case[k].as_bool().unwrap_or(false);
        let num = |k: &str| case[k].as_f64().unwrap_or(0.0) as f32;
        let started = |k: &str| AnimationState {
            started: case[k].as_f64().map(|v| v as f32),
        };
        let (p, s) = (&mut poses, &state);
        match def.species {
            Species::Goat => b_anim::goat(
                p,
                s,
                &b_anim::Goat {
                    left_horn: flag("hasLeftHorn"),
                    right_horn: flag("hasRightHorn"),
                    ramming_x_head_rot: num("rammingXHeadRot"),
                },
            ),
            Species::PolarBear => b_anim::polar_bear(p, s, num("standScale")),
            Species::Panda => b_anim::panda(
                p,
                s,
                &b_anim::Panda {
                    unhappy: flag("isUnhappy"),
                    sneezing: flag("isSneezing"),
                    sneeze_time: num("sneezeTime") as i32,
                    eating: flag("isEating"),
                    scared: flag("isScared"),
                    sit_amount: num("sitAmount"),
                    lie_on_back_amount: num("lieOnBackAmount"),
                    roll_amount: num("rollAmount"),
                },
            ),
            Species::Llama | Species::TraderLlama => b_anim::llama(p, s, flag("hasChest")),
            Species::Turtle => b_anim::turtle(
                p,
                s,
                &b_anim::Turtle {
                    on_land: flag("isOnLand"),
                    laying_egg: flag("isLayingEgg"),
                    has_egg: flag("hasEgg"),
                },
            ),
            Species::Strider => b_anim::strider(p, s, flag("isRidden")),
            Species::SnowGolem => b_anim::snow_golem(p, s),
            Species::Silverfish => b_anim::silverfish(p, s),
            Species::Endermite => b_anim::endermite(p, s),
            Species::Parrot => b_anim::parrot(
                p,
                s,
                match case["pose"].as_str() {
                    Some("FLYING") => b_anim::ParrotPose::Flying,
                    Some("ON_SHOULDER") => b_anim::ParrotPose::OnShoulder,
                    Some("SITTING") => b_anim::ParrotPose::Sitting,
                    Some("PARTY") => b_anim::ParrotPose::Party,
                    _ => b_anim::ParrotPose::Standing,
                },
                num("flapAngle"),
            ),
            Species::Bee => b_anim::bee(
                p,
                s,
                &b_anim::Bee {
                    has_stinger: flag("hasStinger"),
                    on_ground: flag("isOnGround"),
                    angry: flag("isAngry"),
                    roll_amount: num("rollAmount"),
                },
            ),
            Species::Armadillo => b_anim::armadillo(
                p,
                s,
                &b_anim::Armadillo {
                    hiding_in_shell: flag("isHidingInShell"),
                    roll_out: started("rollOutAnimationState"),
                    roll_up: started("rollUpAnimationState"),
                    peek: started("peekAnimationState"),
                },
                &animations,
            ),
            Species::Axolotl => b_anim::axolotl(
                p,
                s,
                &b_anim::Axolotl {
                    playing_dead: num("playingDeadFactor"),
                    in_water: num("inWaterFactor"),
                    on_ground: num("onGroundFactor"),
                    moving: num("movingFactor"),
                    swim: started("swimAnimation"),
                    walk: started("walkAnimationState"),
                    idle_on_ground: started("idleOnGroundAnimationState"),
                    idle_underwater_on_ground: started("idleUnderWaterOnGroundAnimationState"),
                    idle_underwater: started("idleUnderWaterAnimationState"),
                    play_dead: started("playDeadAnimationState"),
                },
                &animations,
            ),
            Species::Frog => b_anim::frog(
                p,
                s,
                &b_anim::Frog {
                    swimming: flag("isSwimming"),
                    jump: started("jumpAnimationState"),
                    croak: started("croakAnimationState"),
                    tongue: started("tongueAnimationState"),
                    swim_idle: started("swimIdleAnimationState"),
                },
                &animations,
            ),
            Species::CopperGolem => {
                // CopperGolemRenderState's animation states, in the golem's slots.
                for (slot, key) in [
                    "idleAnimationState",
                    "interactionGetItem",
                    "interactionGetNoItem",
                    "interactionDropItem",
                    "interactionDropNoItem",
                ]
                .into_iter()
                .enumerate()
                {
                    mob.anim.states[slot] = started(key);
                }
                horizon_client::mobs::copper_golem::anim(&mob, s, p, &animations);
            }
            Species::Rabbit => b_anim::rabbit(
                p,
                s,
                &started("hopAnimationState"),
                &started("idleHeadTiltAnimationState"),
                &animations,
            ),
            Species::Fox => b_anim::fox(
                p,
                s,
                &b_anim::Fox {
                    crouching: flag("isCrouching"),
                    sleeping: flag("isSleeping"),
                    sitting: flag("isSitting"),
                    pouncing: flag("isPouncing"),
                    faceplanted: flag("isFaceplanted"),
                    head_roll_angle: num("headRollAngle"),
                    crouch_amount: num("crouchAmount"),
                    leg_motion_pos: 0.67,
                },
                &animations,
            ),
            Species::Cat | Species::Ocelot => horizon_client::mobs::tame::feline(
                p,
                s,
                &horizon_client::mobs::tame::Feline {
                    crouching: flag("isCrouching"),
                    sprinting: flag("isSprinting"),
                    sitting: flag("isSitting"),
                    lie_down: num("lieDownAmount"),
                    lie_down_tail: num("lieDownAmountTail"),
                    relax_one: num("relaxStateOneAmount"),
                },
            ),
            Species::Piglin | Species::PiglinBrute => {
                use horizon_client::mobs::piglin::{PiglinPose, piglin_pose};
                let pose = match case["armPose"].as_str() {
                    Some("DANCING") => PiglinPose::Dancing,
                    Some("ATTACKING_WITH_MELEE_WEAPON") => PiglinPose::MeleeWeapon,
                    Some("CROSSBOW_HOLD") => PiglinPose::CrossbowHold,
                    Some("CROSSBOW_CHARGE") => PiglinPose::CrossbowCharge,
                    Some("ADMIRING_ITEM") => PiglinPose::AdmiringItem,
                    _ => PiglinPose::Default,
                };
                piglin_pose(
                    p,
                    s,
                    pose,
                    num("maxCrossbowChageDuration"),
                    num("ticksUsingItem"),
                    false,
                    flag("isRiding"),
                );
            }
            Species::Evoker | Species::Illusioner | Species::Pillager | Species::Vindicator => {
                use horizon_client::mobs::illager::{ArmPose, anim_pose};
                let pose = match case["armPose"].as_str() {
                    Some("CROSSED") => ArmPose::Crossed,
                    Some("ATTACKING") => ArmPose::Attacking,
                    Some("SPELLCASTING") => ArmPose::Spellcasting,
                    Some("BOW_AND_ARROW") => ArmPose::BowAndArrow,
                    Some("CROSSBOW_HOLD") => ArmPose::CrossbowHold,
                    Some("CROSSBOW_CHARGE") => ArmPose::CrossbowCharge,
                    Some("CELEBRATING") => ArmPose::Celebrating,
                    _ => ArmPose::Neutral,
                };
                mob.extra.passenger = flag("isRiding");
                mob.extra.using_item = num("ticksUsingItem") as i32;
                anim_pose(&mob, s, p, pose);
            }
            _ => anim::setup(&mob, s, p, &animations),
        }
        let mut result = emit(&model, &mut poses, case, layer);
        if case["@walkTicks"].is_array() {
            result["walk"] = json!([state.pos, state.speed]);
        }
        out.push_str(&format!("{result}\n"));
    }
    fs::write(&args[2], out)?;
    Ok(())
}

/// Every part's pose, and the vertices drawn under the harness's root transform.
fn emit(model: &Part, poses: &mut Poses, case: &Value, layer: &str) -> Value {
    // The harness's part-scale override, set after setupAnim as the oracle does.
    if let Some(scale) = case["@partScale"].as_array() {
        let pose = poses.get(scale[0].as_str().unwrap_or_default());
        for i in 0..3 {
            pose[6 + i] = scale[1 + i].as_f64().unwrap_or(1.0) as f32;
        }
    }
    let poses = &*poses;
    let mut parts = BTreeMap::new();
    walk(model, "", poses, &mut parts);
    let mut emitted = Vec::new();
    render::draw(
        model,
        root_transform(case),
        poses,
        [0.0, 0.0, 1.0, 1.0],
        0xFFFF_FFFF,
        240 | 240 << 8,
        0,
        &mut emitted,
    );
    let mut vertices = BTreeMap::new();
    let mut cursor = 0;
    drawn(model, "", poses, &emitted, &mut cursor, &mut vertices);
    assert_eq!(cursor, emitted.len(), "{layer}: draw's walk changed");
    json!({"parts": parts, "vertices": vertices})
}
