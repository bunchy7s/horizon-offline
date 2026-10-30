//! Replays `tools/test_mob_ai_parity.py` scenarios on Horizon's mob AI and
//! prints each mob's AI-visible state after every tick in the format of
//! `VanillaMobAiOracle`.
//!
//! usage: mob_ai_probe RESOURCES SCENARIOS OUTPUT
use horizon_client::{
    assets::Assets,
    beehive::{BeehiveExt, HiveHost},
    biome::Biomes,
    blocks::BlockFacts,
    chunk::{Chunk, HEIGHT, MIN_Y},
    collisions,
    containers::{BlockEntity, CreakingHeart},
    items::{ItemStack, Items},
    light::{LightTables, apply_lighting, light_chunk},
    mobs::{
        Damage, MobType, creaking,
        creaking_heart::{self, HeartEvent, HeartHost},
    },
    mobs::{
        Mob, MobData, MobEvent, MobTypes, MobWorld, PlayerView, Seen, Species, Who,
        control::Operation, seen_grid::SeenGrid, spawning::LocalDifficulty,
    },
    world::World,
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::LegacyRandom,
};
use horizon_world::loot::PackOverlay;
use horizon_world::{chunk::BlockState, registry::BlockRegistry, registry::BlockStateId};
use std::{
    collections::{BTreeMap, HashMap},
    fmt::Write as _,
    fs,
    path::Path,
    sync::Arc,
};

struct MobSpec {
    kind: String,
    pos: [f64; 3],
    yaw: f32,
    seed: i64,
    baby: bool,
    flags: Vec<String>,
}
#[derive(Default)]
struct Scenario {
    name: String,
    ticks: i32,
    time: i64,
    difficulty: u8,
    rain: bool,
    thunder: bool,
    origin: [i32; 2],
    blocks: Vec<Vec<String>>,
    player: Option<[f64; 3]>,
    creative: bool,
    /// The tick from which the player faces another yaw.
    turn: Option<(i32, f32)>,
    /// Creaking hearts placed when the scenario starts.
    hearts: Vec<Vec<String>>,
    /// `ServerLevel.isSpawningMonsters`.
    spawning: bool,
    /// The day time moves on with every tick.
    daycycle: bool,
    /// The player hits a mob for 1 at the start of a tick: (tick, mob index).
    hurts: Vec<(i32, usize)>,
    /// The player feeds an animal at the start of a tick (Animal.mobInteract):
    /// (tick, mob index, None); the scenario only feeds each animal its food.
    /// `interact` uses another item on a horse: (tick, mob index, Some(item)).
    feeds: Vec<(i32, usize, Option<String>)>,
    hand: Option<String>,
    mobs: Vec<MobSpec>,
}

fn parse(text: &str) -> Vec<Scenario> {
    let mut out = Vec::new();
    let mut cur: Option<Scenario> = None;
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.is_empty() || f[0].starts_with('#') {
            continue;
        }
        let s = cur.get_or_insert_with(Scenario::default);
        match f[0] {
            "scenario" => {
                s.name = f[1].to_owned();
                s.time = 6000;
                s.difficulty = 2;
                for kv in &f[2..] {
                    let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
                    match k {
                        "ticks" => s.ticks = v.parse().unwrap(),
                        "time" => s.time = v.parse().unwrap(),
                        "difficulty" => s.difficulty = v.parse().unwrap(),
                        "rain" => s.rain = true,
                        "thunder" => s.thunder = true,
                        "levelseed" => {}
                        "spawning" => s.spawning = true,
                        "daycycle" => s.daycycle = true,
                        _ => panic!("unknown scenario option {kv}"),
                    }
                }
            }
            "origin" => s.origin = [f[1].parse().unwrap(), f[2].parse().unwrap()],
            "fill" | "block" | "chest" => {
                s.blocks.push(f.iter().map(|x| (*x).to_owned()).collect())
            }
            "heart" => s.hearts.push(f.iter().map(|x| (*x).to_owned()).collect()),
            "hurt" => s.hurts.push((f[1].parse().unwrap(), f[2].parse().unwrap())),
            "feed" => s
                .feeds
                .push((f[1].parse().unwrap(), f[2].parse().unwrap(), None)),
            "interact" => s.feeds.push((
                f[1].parse().unwrap(),
                f[2].parse().unwrap(),
                Some(f[3].to_owned()),
            )),
            "player" => {
                s.player = Some([
                    f[1].parse().unwrap(),
                    f[2].parse().unwrap(),
                    f[3].parse().unwrap(),
                ]);
                for opt in &f[4..] {
                    if *opt == "creative" {
                        s.creative = true;
                    } else if let Some(t) = opt.strip_prefix("turn=") {
                        let (tick, yaw) = t.split_once(',').unwrap();
                        s.turn = Some((tick.parse().unwrap(), yaw.parse().unwrap()));
                    } else if let Some(item) = opt.strip_prefix("hand=") {
                        s.hand = Some(item.to_owned());
                    } else {
                        panic!("unknown player option {opt}");
                    }
                }
            }
            "mob" => s.mobs.push(MobSpec {
                kind: f[1].to_owned(),
                pos: [
                    f[2].parse().unwrap(),
                    f[3].parse().unwrap(),
                    f[4].parse().unwrap(),
                ],
                yaw: f[5].parse().unwrap(),
                seed: f[6].parse().unwrap(),
                baby: f[7..].contains(&"baby"),
                flags: f[7..]
                    .iter()
                    .filter(|x| **x != "baby")
                    .map(|x| (*x).to_owned())
                    .collect(),
            }),
            "end" => out.push(cur.take().unwrap()),
            other => panic!("unknown scenario line {other}"),
        }
    }
    out
}

/// `minecraft:name[key=value,...]`
fn state(registry: &BlockRegistry, text: &str) -> BlockStateId {
    let (name, props) = match text.split_once('[') {
        Some((n, rest)) => (n, rest.trim_end_matches(']')),
        None => (text, ""),
    };
    let mut properties = BTreeMap::new();
    for kv in props.split(',').filter(|s| !s.is_empty()) {
        let (k, v) = kv.split_once('=').expect("property");
        properties.insert(k.to_owned(), v.to_owned());
    }
    registry
        .resolve(&BlockState {
            name: name.to_owned(),
            properties,
        })
        .unwrap_or_else(|e| panic!("{text}: {e}"))
}

fn d(v: f64) -> String {
    format!("{:x}", v.to_bits())
}
fn f(v: f32) -> String {
    format!("{:x}", v.to_bits())
}
fn who(w: Option<Who>) -> String {
    match w {
        None => "none".into(),
        Some(Who::Player) => "player".into(),
        Some(Who::Mob(id)) => format!("e{id}"),
    }
}

/// Vanilla's goal class for a Horizon goal (most share the name).
fn goal_name(m: &Mob, name: &str) -> String {
    let vanilla = match name {
        "Float" => "FloatGoal",
        "Stroll" => "WaterAvoidingRandomStrollGoal",
        "ParrotWander" => "ParrotWanderGoal",
        "RandomStroll" => "RandomStrollGoal",
        // AbstractFish's private subclass of RandomSwimmingGoal.
        "RandomSwimming"
            if matches!(
                m.species(),
                Species::Cod | Species::Salmon | Species::TropicalFish | Species::Pufferfish
            ) =>
        {
            "FishSwimGoal"
        }
        "RandomSwimming" => "RandomSwimmingGoal",
        "LookAtPlayer" => "LookAtPlayerGoal",
        "RandomLookAround" => "RandomLookAroundGoal",
        "HurtByTarget" => "HurtByTargetGoal",
        "NearestTarget" => "NearestAttackableTargetGoal",
        "Panic" => "PanicGoal",
        "Breed" => "BreedGoal",
        "Tempt" => "TemptGoal",
        "FollowParent" => "FollowParentGoal",
        "MeleeAttack" => match m.species() {
            Species::Zombie | Species::Husk | Species::ZombieVillager => "ZombieAttackGoal",
            _ => "MeleeAttackGoal",
        },
        "MoveThroughVillage" => "MoveThroughVillageGoal",
        "RestrictSun" => "RestrictSunGoal",
        "FleeSun" => "FleeSunGoal",
        // `Cat.CatAvoidEntityGoal`, `Fox.FoxAvoidEntityGoal`... name their subclass.
        "AvoidEntity" if m.species() == Species::Cat => "CatAvoidEntityGoal",
        "AvoidEntity" => "AvoidEntityGoal",
        "BowAttack" => "RangedBowAttackGoal",
        "Swell" => "SwellGoal",
        "LeapAtTarget" => "LeapAtTargetGoal",
        "EatBlock" => "EatBlockGoal",
        "MoveTowardsRestriction" => "MoveTowardsRestrictionGoal",
        "TradeWithPlayer" => "TradeWithPlayerGoal",
        "LookAtTradingPlayer" => "LookAtTradingPlayerGoal",
        "WanderToPosition" => "WanderToPositionGoal",
        "DrinkItem" => "UseItemGoal",
        "BreathAir" => "BreathAirGoal",
        "TryFindWater" => "TryFindWaterGoal",
        "RangedAttack" => "RangedAttackGoal",
        "MoveTowardsTarget" => "MoveTowardsTargetGoal",
        "DrownedGoToWater" => "DrownedGoToWaterGoal",
        "TakeBlock" => "EndermanTakeBlockGoal",
        "LeaveBlock" => "EndermanLeaveBlockGoal",
        "FreezeWhenLookedAt" => "EndermanFreezeWhenLookedAt",
        "LookForPlayer" => "EndermanLookForPlayerGoal",
        "HealRaiders" => "NearestHealableRaiderTargetGoal",
        "MountPanic" => "MountPanicGoal",
        "LlamaHurtBy" => "LlamaHurtByTargetGoal",
        other => return other.to_owned(),
    };
    vanilla.to_owned()
}

fn goals(m: &Mob, names: Vec<&'static str>) -> String {
    if names.is_empty() {
        return "-".into();
    }
    names
        .iter()
        .map(|n| goal_name(m, n))
        .collect::<Vec<_>>()
        .join("|")
}

/// The oracle's villager flags: profession, level, xp, food, inventory,
/// gossip about the player and the last sleep.
fn villager_flag(m: &mut Mob, flag: &str) {
    use horizon_client::mobs::villager;
    let (k, v) = flag
        .split_once('=')
        .unwrap_or_else(|| panic!("unknown mob flag {flag}"));
    match k {
        "profession" => {
            let p = horizon_client::mobs::render::PROFESSIONS
                .iter()
                .position(|p| *p == v)
                .expect("profession");
            villager::set_profession(m, p as u8);
        }
        "level" => m.extra.variant3 = v.parse().unwrap(),
        "xp" => {
            m.extra.trade.get_or_insert_with(Default::default).xp = v.parse().unwrap();
        }
        "food" => villager::ext_mut(m).food_level = v.parse().unwrap(),
        "inv" => {
            for item in v.split(',') {
                let (name, count) = item.split_once('*').unwrap_or((item, "1"));
                villager::ext_mut(m)
                    .inventory
                    .push(horizon_client::items::ItemStack::new(
                        &if name.contains(':') {
                            name.to_owned()
                        } else {
                            format!("minecraft:{name}")
                        },
                        count.parse().unwrap(),
                    ));
            }
        }
        "gossip" => {
            let g: Vec<&str> = v.split(':').collect();
            assert_eq!(g[0], "player", "gossip about the player only");
            let kind = horizon_client::gossip::GossipType::from_id(g[1]).expect("gossip type");
            villager::ext_mut(m)
                .gossips
                .add(PLAYER_UUID, kind, g[2].parse().unwrap());
        }
        "slept" => villager::ext_mut(m).last_slept = Some(v.parse().unwrap()),
        _ => panic!("unknown mob flag {flag}"),
    }
}

/// The random seed of the entity a scenario spawns at this index (the
/// oracle's `spawnSeed`).
fn spawn_seed(index: usize) -> i64 {
    1000 + 7919 * index as i64
}

/// `Villager.spawnGolemIfNeeded` for villager `a`, the game's way: the golem
/// draws from the level's random and joins the scenario's mobs next tick,
/// seeded as the oracle seeds what a tick spawned.
fn summon_golem(
    mobs: &mut [Mob],
    a: usize,
    needed: usize,
    w: &MobWorld,
    types: &MobTypes,
    born: &mut Vec<Mob>,
    level_random: &mut LegacyRandom,
) {
    use horizon_client::mobs::{villager, villager_brain};
    let Some(near) = villager::golem_summoners(mobs, a, w.game_time, needed) else {
        return;
    };
    let kind = Arc::new(
        types
            .types
            .get("minecraft:iron_golem")
            .expect("iron golem")
            .clone(),
    );
    let index = mobs.len() + born.len();
    let p = mobs[a].position;
    let start = [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32];
    let golem = villager::summon_golem(w, start, level_random, |random, at, yaw| {
        let mut m = Mob::new(100_000 + index as u32, kind.clone(), at, yaw);
        villager::finalize_summoned_golem(&mut m, random);
        m
    });
    let Some(mut golem) = golem else {
        return;
    };
    golem.random = LegacyRandom::new(spawn_seed(index));
    born.push(golem);
    for j in near {
        villager_brain::golem_detected(&mut mobs[j], w.game_time);
    }
}

/// Villager events that act at once in vanilla (`TradeWithVillager` and
/// `VillagerMakeLove` write the other villager directly).
fn villager_events(
    mobs: &mut [Mob],
    events: &mut Vec<MobEvent>,
    from: usize,
    w: &MobWorld,
    types: &MobTypes,
    items: &Items,
    born: &mut Vec<Mob>,
    day_time: i64,
    level_random: &mut LegacyRandom,
) {
    use horizon_client::mobs::villager;
    let index = |mobs: &[Mob], id: u32| {
        mobs.iter()
            .position(|m| m.id == id && m.species() == Species::Villager)
    };
    let mut i = from;
    while i < events.len() {
        match events[i] {
            MobEvent::VillagerMeet { mob, other } => {
                events.remove(i);
                if let (Some(a), Some(b)) = (index(mobs, mob), index(mobs, other)) {
                    let meeting = villager::meet(mobs, a, b, w.game_time, |item| {
                        items.get(item).map_or(64, |i| i.max_stack)
                    });
                    assert!(
                        meeting.thrown.is_empty(),
                        "villagers toss items: not modelled here"
                    );
                    if meeting.gossiped {
                        summon_golem(mobs, a, 5, w, types, born, level_random);
                    }
                }
            }
            MobEvent::GolemCheck { mob, needed } => {
                events.remove(i);
                if let Some(a) = index(mobs, mob) {
                    summon_golem(mobs, a, needed, w, types, born, level_random);
                }
            }
            MobEvent::VillagerBirth {
                parent,
                partner,
                child,
            } => {
                events.remove(i);
                let (Some(a), Some(b)) = (index(mobs, parent), index(mobs, partner)) else {
                    continue;
                };
                villager::ext_mut(&mut mobs[b]).eat_and_digest();
                let Some((bed, variant)) = child else {
                    continue;
                };
                mobs[b].extra.age = 6000;
                let index = mobs.len() + born.len();
                let kind = types.types.get("minecraft:villager").expect("villager");
                let p = mobs[a].position;
                let mut child = Mob::new(100_000 + index as u32, Arc::new(kind.clone()), p, 0.0);
                child.persistent = true;
                // The flat world is all plains (type 0).
                child.extra.variant = variant.unwrap_or(0);
                child.extra.variant2 = villager::NONE;
                child.extra.age = -24000;
                child.extra.bed = Some(bed);
                let seed = spawn_seed(index);
                child.random = LegacyRandom::new(seed);
                horizon_client::mobs::villager_brain::init(&mut child, w.game_time, day_time, seed);
                born.push(child);
            }
            _ => i += 1,
        }
    }
}

/// The player's UUID (the oracle's `PLAYER_UUID`).
const PLAYER_UUID: [i32; 4] = [0, 0x1234, 0, 0x5678];

fn seen(m: &Mob) -> Seen {
    m.seen(Some(PLAYER_UUID))
}

/// The probe's mobs as a creaking heart sees them: creakings it creates take
/// the n-th seed from 1000, as the oracle's do.
struct Host<'a> {
    mobs: &'a mut Vec<Mob>,
    added: Vec<Mob>,
    types: &'a MobTypes,
    created: &'a mut i64,
    spawning: bool,
}
impl HeartHost for Host<'_> {
    fn mob(&self, id: u32) -> Option<&Mob> {
        self.mobs
            .iter()
            .chain(self.added.iter())
            .find(|m| m.id == id)
    }
    fn mob_mut(&mut self, id: u32) -> Option<&mut Mob> {
        self.mobs
            .iter_mut()
            .chain(self.added.iter_mut())
            .find(|m| m.id == id)
    }
    fn mob_for_uuid(&self, uuid: [i32; 4]) -> Option<u32> {
        self.mob(uuid[3] as u32)
            .filter(|m| !m.removed)
            .map(|m| m.id)
    }
    fn uuid_for(&mut self, id: u32) -> [i32; 4] {
        [0, 0, 0, id as i32]
    }
    fn creaking_type(&self) -> Option<Arc<MobType>> {
        self.types
            .types
            .get("minecraft:creaking")
            .map(|t| Arc::new(t.clone()))
    }
    fn create_mob(&mut self, kind: Arc<MobType>, at: Vec3) -> Mob {
        let seed = 1000 + *self.created;
        *self.created += 1;
        let mut m = Mob::new(200_000 + seed as u32, kind, at, 0.0);
        m.random = LegacyRandom::new(seed);
        horizon_client::mobs::monster_brain::init(&mut m, seed);
        m
    }
    fn add_mob(&mut self, m: Mob) {
        self.added.push(m);
    }
    fn spawning_monsters(&self) -> bool {
        self.spawning
    }
}
/// A scenario's beehives as `BeehiveBlockEntity.serverTick` sees the level:
/// the bees let out are newcomers seeded as the oracle seeds them.
struct ProbeHive<'a, 'w> {
    w: &'a MobWorld<'w>,
    random: &'a mut LegacyRandom,
    types: &'a HashMap<String, Arc<MobType>>,
    /// The index of the first newcomer of this tick.
    first: usize,
    added: &'a mut Vec<Mob>,
    honey: &'a mut Vec<([i32; 3], i32)>,
    events: &'a mut Vec<MobEvent>,
}
impl HiveHost for ProbeHive<'_, '_> {
    fn random(&mut self) -> &mut LegacyRandom {
        self.random
    }
    fn bees_stay_in_hive(&self, _at: [i32; 3]) -> bool {
        horizon_client::mobs::bee::bees_stay_in_hive(self.w)
    }
    fn blocked(&self, at: [i32; 3]) -> bool {
        !self.w.block_collision_boxes(at[0], at[1], at[2]).is_empty()
    }
    fn set_honey_level(&mut self, at: [i32; 3], level: i32) {
        self.honey.push((at, level));
    }
    fn load(&mut self, tag: &horizon_world::nbt::Tag) -> Option<Mob> {
        let index = self.first + self.added.len();
        let mut m =
            horizon_client::entity_nbt::mob_from_nbt(tag, self.types, 100_000 + index as u32)?;
        if m.species() != Species::Bee {
            return None;
        }
        m.random = LegacyRandom::new(spawn_seed(index));
        horizon_client::mobs::bee::draw_cooldowns(&mut m);
        Some(m)
    }
    fn add(&mut self, m: Mob) -> bool {
        self.added.push(m);
        true
    }
    fn sound(&mut self, name: &'static str, at: [f64; 3]) {
        self.events.push(MobEvent::Sound(name, at, 1.0, 1.0));
    }
}
/// The hive's honey level and front from its block state.
fn hive_state(registry: &BlockRegistry, world: &World, at: [i32; 3]) -> (Option<i32>, [i32; 2]) {
    let props = registry
        .state(world.block(at[0], at[1], at[2]))
        .map(|s| s.properties.clone())
        .unwrap_or_default();
    let honey = props.get("honey_level").and_then(|v| v.parse().ok());
    let facing = horizon_client::beehive::facing_step(props.get("facing").map(String::as_str));
    (honey, facing)
}
fn is_hive(b: &[String]) -> bool {
    b[0] == "block"
        && (b[4].starts_with("minecraft:beehive") || b[4].starts_with("minecraft:bee_nest"))
}

/// The heart's state, block entity and the level random, as the oracle
/// prints them.
fn print_heart(
    out: &mut String,
    t: i32,
    registry: &BlockRegistry,
    world: &World,
    at: [i32; 3],
    heart: &CreakingHeart,
    lr: &LegacyRandom,
) {
    let state = registry
        .state(world.block(at[0], at[1], at[2]))
        .and_then(|s| s.properties.get("creaking_heart_state").cloned())
        .unwrap_or_default();
    let _ = writeln!(
        out,
        "{t} heart {},{},{}\t{state}\t{} {} {} {} {}\tlrng {:x}",
        at[0],
        at[1],
        at[2],
        heart.ticker,
        heart.emitter,
        heart.output_signal,
        heart.ticks_existed,
        if heart.creaking.is_some() {
            "linked"
        } else {
            "none"
        },
        lr.state()
    );
}

/// The damage of a mob's direct hit on the player (others count as none).
fn player_damage(e: &MobEvent) -> f32 {
    match e {
        MobEvent::AttackPlayer(amount, _)
        | MobEvent::AttackPlayerWithEffect { amount, .. }
        | MobEvent::DamagePlayer { amount, .. } => *amount,
        MobEvent::SonicBoom {
            target: Who::Player,
            ..
        } => 10.0,
        _ => 0.0,
    }
}
/// Whether an event hurts the scenario's player (survival, normal difficulty,
/// no armour, made hurtable again every tick by the oracle).
fn player_hurt(player: Option<&PlayerView>, e: &MobEvent) -> bool {
    let Some(p) = player.filter(|p| !p.creative) else {
        return false;
    };
    match e {
        MobEvent::AttackPlayer(amount, _)
        | MobEvent::AttackPlayerWithEffect { amount, .. }
        | MobEvent::DamagePlayer { amount, .. } => *amount > 0.0,
        MobEvent::SonicBoom {
            target: Who::Player,
            ..
        } => true,
        // ServerExplosion.hurtEntities: within twice the radius every hit does damage.
        MobEvent::Explode(at, radius, _) => {
            let d = Vec3::new(at[0], at[1], at[2])
                .distance_squared(p.position)
                .sqrt();
            let r = 2.0 * f64::from(*radius);
            d / r <= 1.0 && d != 0.0
        }
        _ => false,
    }
}

/// `player_down`: the scenario's player is no valid target (creative, or
/// killed this tick).
fn print(out: &mut String, t: i32, i: usize, m: &Mob, player_down: bool, now: i64) {
    let _ = write!(out, "{t} {i}");
    if m.removed {
        // Entity.RemovalReason: the end of the death animation, else a discard.
        let reason = if m.alive() { "DISCARDED" } else { "KILLED" };
        let _ = writeln!(out, " removed {reason}");
        return;
    }
    let _ = write!(out, "\tleashed {}", u8::from(m.extra.leash.is_some()));
    let p = m.position;
    let v = m.velocity;
    let _ = write!(
        out,
        "\tp {},{},{}\tv {},{},{}",
        d(p.x),
        d(p.y),
        d(p.z),
        d(v.x),
        d(v.y),
        d(v.z)
    );
    let _ = write!(
        out,
        "\tr {},{},{},{}",
        f(m.yaw),
        f(m.head_pitch),
        f(m.head_yaw),
        f(m.body_yaw)
    );
    let _ = write!(
        out,
        "\tg {}{}\thp {}",
        u8::from(m.on_ground),
        u8::from(m.horizontal_collision),
        f(m.health)
    );
    let _ = write!(
        out,
        "\tgoals {}\ttargets {}\ttarget {}",
        goals(m, m.ai.goals.running_names()),
        goals(m, m.ai.targets.running_names()),
        // Warden.getTarget: the brain's attack target, if still valid.
        who(match horizon_client::mobs::warden::brain_target(m) {
            Some(Who::Player) if player_down => None,
            Some(target) => Some(target),
            None if m.species() == Species::Warden => None,
            None => m.ai.target,
        })
    );
    let _ = write!(out, "\tnav ");
    match &m.ai.nav.path {
        None => out.push_str("none"),
        Some(path) => {
            let _ = write!(
                out,
                "{},{},{} {}/{}{}",
                path.target[0],
                path.target[1],
                path.target[2],
                path.next,
                path.nodes.len(),
                if path.next >= path.nodes.len() {
                    " done"
                } else {
                    ""
                }
            );
        }
    }
    let mv = &m.ai.mov;
    let op = match mv.operation {
        Operation::Wait => "WAIT",
        Operation::MoveTo => "MOVE_TO",
        Operation::Strafe => "STRAFE",
        Operation::Jumping => "JUMPING",
    };
    let _ = write!(
        out,
        "\tmove {op} {},{},{} {}",
        d(mv.wanted[0]),
        d(mv.wanted[1]),
        d(mv.wanted[2]),
        d(mv.speed_modifier)
    );
    let lk = &m.ai.look;
    let _ = write!(
        out,
        "\tlook {} {},{},{}",
        lk.cooldown,
        d(lk.wanted[0]),
        d(lk.wanted[1]),
        d(lk.wanted[2])
    );
    let _ = write!(
        out,
        "\tin {},{},{},{}",
        f(m.ai.xxa),
        f(m.ai.zza),
        f(m.ai.speed),
        u8::from(m.ai.jumping)
    );
    let _ = write!(out, "\trng {:x}", m.random.state());
    let brain = horizon_client::mobs::villager_brain::describe(m)
        .or_else(|| horizon_client::mobs::warden_brain::describe(m))
        .or_else(|| horizon_client::mobs::monster_brain::describe(m))
        .or_else(|| horizon_client::mobs::animal_brain::describe(m));
    if let Some((acts, mem)) = brain {
        let _ = write!(out, "\tact {acts}\tmem {mem}");
    }
    if m.species() == Species::Villager {
        let _ = write!(out, "\tvx {}", villager(m));
    }
    if m.species() == Species::CopperGolem {
        let held = horizon_client::mobs::transport_items::held(m).map_or("-".into(), stack);
        let opened = m
            .extra
            .opened_chest
            .map_or("-".into(), |p| format!("{},{},{}", p[0], p[1], p[2]));
        let _ = write!(out, "\tcg {} {held} {opened}", m.extra.variant3);
    }
    if m.species() == Species::Wolf {
        // NeutralMob anger left (the angry texture), -1 when calm.
        let anger = if m.extra.anger > 0 {
            i64::from(m.extra.anger)
        } else {
            -1
        };
        let _ = write!(out, "\twolf {anger}");
    }
    if let Some(b) = horizon_client::mobs::bee::get(m) {
        let anger = if b.anger_end > 0 {
            b.anger_end - now
        } else {
            -1
        };
        let flower = b
            .flower
            .map_or("-".into(), |p| format!("{},{},{}", p[0], p[1], p[2]));
        let _ = write!(
            out,
            "\tbee {} {anger} {} {} {flower}",
            b.flags, b.ticks_without_nectar, b.flower_cooldown
        );
    }
    out.push('\n');
}

fn stack(s: &horizon_client::items::ItemStack) -> String {
    if s.count == 0 {
        return "-".into();
    }
    format!("{}*{}", s.item.trim_start_matches("minecraft:"), s.count)
}

/// The oracle's `villager`: data, food, inventory, gossip, sleep and offers.
fn villager(m: &Mob) -> String {
    use horizon_client::mobs::villager;
    let e = villager::ext(m);
    let mut slots: Vec<String> = (0..8)
        .map(|k| {
            e.and_then(|e| e.inventory.get(k))
                .map_or_else(|| "-".to_owned(), stack)
        })
        .collect();
    while slots.last().is_some_and(|s| s == "-") {
        slots.pop();
    }
    // Villagers gossip about players only.
    let label = |t: [i32; 4]| {
        if t == PLAYER_UUID {
            "player".to_owned()
        } else {
            format!("{t:?}")
        }
    };
    let mut gossips: Vec<String> = e
        .map(|e| {
            e.gossips
                .entries()
                .map(|(t, k, v)| format!("{}:{}={v}", label(t), k.id()))
                .collect()
        })
        .unwrap_or_default();
    gossips.sort();
    let join = |v: &[String]| {
        if v.is_empty() {
            "-".to_owned()
        } else {
            v.join(",")
        }
    };
    let mut s = format!(
        "prof {} lvl {} xp {} food {} age {} inv {} gossip {} rep {} sleep {}",
        horizon_client::mobs::render::PROFESSIONS[m.extra.variant2 as usize % 15],
        m.extra.variant3.max(1),
        villager::xp(m),
        e.map_or(0, |e| e.food_level),
        m.extra.age,
        join(&slots),
        join(&gossips),
        e.map_or(0, |e| e.gossips.reputation(PLAYER_UUID)),
        match (villager::sleeping(m), m.extra.bed) {
            (true, Some(b)) => format!("{},{},{}", b[0], b[1], b[2]),
            _ => "-".into(),
        }
    );
    if let Some(t) = m.extra.trade.as_ref().filter(|t| !t.offers.is_empty()) {
        let offers: Vec<String> = t
            .offers
            .iter()
            .map(|o| {
                format!(
                    "{}+{}>{}/{}/{}/{}/{}",
                    stack(&o.base_cost_a.stack),
                    o.cost_b
                        .as_ref()
                        .map_or_else(|| "-".to_owned(), |c| stack(&c.stack)),
                    stack(&o.result),
                    o.uses,
                    o.max_uses,
                    o.demand,
                    o.special_price_diff
                )
            })
            .collect();
        s.push_str(&format!(" offers {}", join(&offers)));
    }
    s
}

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().skip(1).collect();
    if args.len() != 3 {
        return Err("usage: mob_ai_probe RESOURCES SCENARIOS OUTPUT".into());
    }
    let root = Path::new(&args[0]);
    let assets = Assets::load(root)?;
    let registry = assets.registry;
    let facts = BlockFacts::load(root, registry)?;
    let motion = horizon_client::motion::MotionResources::load(root, &assets)?;
    let data = MobData::load(root, &facts);
    let types = MobTypes::load(root)?;
    // The types a bee leaving its hive is loaded with (EntityType.loadEntityRecursive).
    let arc_types: HashMap<String, Arc<MobType>> = types
        .types
        .iter()
        .map(|(k, v)| (k.clone(), Arc::new(v.clone())))
        .collect();
    let biomes = Biomes::load(root)?;
    let items = Items::load(root, registry, &facts, &PackOverlay::NONE)?;
    let tables = LightTables::load(root, registry, &facts)?;
    let collision = collisions::load(root, registry)?;
    let air = registry.default_state("minecraft:air").ok_or("no air")?;
    let layers = [
        state(registry, "minecraft:bedrock"),
        state(registry, "minecraft:dirt"),
        state(registry, "minecraft:dirt"),
        state(registry, "minecraft:grass_block"),
    ];
    let plains = biomes.id("minecraft:plains");
    let mut out = String::new();
    for s in parse(&fs::read_to_string(&args[1])?) {
        let _ = writeln!(out, "== {}", s.name);
        let mut world = World::new(air, 0);
        let (ocx, ocz) = (s.origin[0] >> 4, s.origin[1] >> 4);
        const R: i32 = 4;
        for cz in ocz - R..=ocz + R {
            for cx in ocx - R..=ocx + R {
                let mut chunk = Chunk::empty(cx, cz);
                for qy in 0..HEIGHT / 4 {
                    for qz in 0..4 {
                        for qx in 0..4 {
                            chunk.set_biome_cell(qx, qy, qz, plains);
                        }
                    }
                }
                world.chunks.insert((cx, cz), Arc::new(chunk));
                for z in 0..16 {
                    for x in 0..16 {
                        for (y, layer) in layers.iter().enumerate() {
                            world.set(cx * 16 + x, MIN_Y + y as i32, cz * 16 + z, *layer)?;
                        }
                    }
                }
            }
        }
        for b in &s.blocks {
            let n = |i: usize| b[i].parse::<i32>().unwrap();
            if b[0] == "block" || b[0] == "chest" {
                world.set(n(1), n(2), n(3), state(registry, &b[4]))?;
                if is_hive(b) {
                    world
                        .block_entities
                        .insert((n(1), n(2), n(3)), BlockEntity::Beehive(Box::default()));
                }
                if b[0] == "chest" {
                    // chest X Y Z STATE SLOTS: item*count or - per slot.
                    let mut slots = vec![None; 27];
                    for (k, item) in b.get(5).map_or("", String::as_str).split(',').enumerate() {
                        if let Some((id, count)) = item.split_once('*') {
                            slots[k] = Some(ItemStack::new(
                                &format!("minecraft:{id}"),
                                count.parse().unwrap(),
                            ));
                        }
                    }
                    world
                        .block_entities
                        .insert((n(1), n(2), n(3)), BlockEntity::Chest(slots));
                }
            } else {
                let st = state(registry, &b[7]);
                for x in n(1).min(n(4))..=n(1).max(n(4)) {
                    for y in n(2).min(n(5))..=n(2).max(n(5)) {
                        for z in n(3).min(n(6))..=n(3).max(n(6)) {
                            world.set(x, y, z, st)?;
                        }
                    }
                }
            }
        }
        let mut hearts: Vec<([i32; 3], CreakingHeart)> = Vec::new();
        for h in &s.hearts {
            let at = [
                h[1].parse::<i32>().unwrap(),
                h[2].parse::<i32>().unwrap(),
                h[3].parse::<i32>().unwrap(),
            ];
            world.set(at[0], at[1], at[2], state(registry, &h[4]))?;
            hearts.push((at, CreakingHeart::default()));
        }
        let mut created = 0_i64;
        let keys: Vec<(i32, i32)> = world.chunks.keys().copied().collect();
        let lit: Vec<((i32, i32), Chunk)> = keys
            .iter()
            .map(|&(x, z)| {
                let region: [[Option<&Chunk>; 3]; 3] = std::array::from_fn(|dz| {
                    std::array::from_fn(|dx| {
                        world
                            .chunks
                            .get(&(x + dx as i32 - 1, z + dz as i32 - 1))
                            .map(|c| &**c)
                    })
                });
                let mut c = (*world.chunks[&(x, z)]).clone();
                apply_lighting(&mut c, light_chunk(&region, &tables, true));
                c.lit = true;
                ((x, z), c)
            })
            .collect();
        for (k, c) in lit {
            world.chunks.insert(k, Arc::new(c));
        }
        world.day_time = s.time;
        world.game_time = 1000;
        let mut mobs: Vec<Mob> = Vec::new();
        for (i, spec) in s.mobs.iter().enumerate() {
            let kind = types
                .types
                .get(&spec.kind)
                .ok_or_else(|| format!("unknown mob {}", spec.kind))?;
            let mut m = Mob::new(
                100_000 + i as u32,
                Arc::new(kind.clone()),
                Vec3::new(spec.pos[0], spec.pos[1], spec.pos[2]),
                spec.yaw,
            );
            if spec.baby {
                m.extra.age = -24000;
            }
            for flag in &spec.flags {
                match flag.as_str() {
                    "persistent" => m.persistent = true,
                    "named" => {
                        m.custom_name = Some("Peaceful regression".into());
                        m.persistent = true;
                    }
                    "ridden" => {
                        m.extra.head = Some("minecraft:red_harness".into());
                        m.rider = Some(horizon_client::mobs::equine::Rider::default());
                    }
                    "noai" => m.no_ai = true,
                    // Warden.finalizeSpawn's own part.
                    "digcooldown" | "emerge" => horizon_client::mobs::warden::finalize(
                        &mut m,
                        flag == "emerge",
                        &mut Vec::new(),
                    ),
                    "inlove" => m.extra.in_love = 600,
                    "nectar" => horizon_client::mobs::bee::set_has_nectar(&mut m, true),
                    "tame" => m.extra.tame = true,
                    x if x.starts_with("age=") => m.extra.age = x[4..].parse().unwrap(),
                    x if x.starts_with("strength=") => m.extra.variant2 = x[9..].parse().unwrap(),
                    x if x.starts_with("hand=") => {
                        let (id, count) = x[5..].split_once('*').unwrap();
                        let stack =
                            ItemStack::new(&format!("minecraft:{id}"), count.parse().unwrap());
                        horizon_client::mobs::transport_items::set_held(&mut m, Some(stack));
                    }
                    // Set once the brain exists (below).
                    x if x.starts_with("cooldown=") => {}
                    "dead" => m.health = 0.0,
                    "leashed" => m.extra.leash = Some(horizon_client::mobs::leash::Holder::Player),
                    x if x.starts_with("leashto=") => {
                        let n: u32 = x[8..].parse().unwrap();
                        m.extra.leash = Some(horizon_client::mobs::leash::Holder::Mob(100_000 + n));
                    }
                    other => villager_flag(&mut m, other),
                }
            }
            m.random = LegacyRandom::new(spec.seed);
            if m.species() == Species::Bee {
                // The oracle draws the constructor's cooldowns again once seeded.
                horizon_client::mobs::bee::draw_cooldowns(&mut m);
            }
            if horizon_client::mobs::monster_brain::has_brain(m.species()) {
                // The oracle delays the sensors again with the seeded random
                // and seeds RunOne's shuffles with the mob's seed.
                horizon_client::mobs::monster_brain::init(&mut m, spec.seed);
            }
            if m.species() == Species::Villager {
                // The oracle seeds the random after the brain is built, then
                // delays the sensors again and seeds the gates' shuffles.
                horizon_client::mobs::villager_brain::init(
                    &mut m,
                    world.game_time,
                    s.time,
                    spec.seed,
                );
            }
            if m.species() == Species::Warden {
                // As for villagers: the sensors delayed again, the RunOne seeded.
                horizon_client::mobs::warden::init_brain(&mut m, spec.seed);
            }
            // The same for the sniffer's and the happy ghast's brains.
            if horizon_client::mobs::animal_brain::kind_of(&m).is_some() {
                horizon_client::mobs::animal_brain::init(&mut m, spec.seed);
            }
            if let Some(c) = spec.flags.iter().find_map(|f| f.strip_prefix("cooldown=")) {
                horizon_client::mobs::animal_brain::set_transport_cooldown(
                    &mut m,
                    c.parse().unwrap(),
                );
            }
            mobs.push(m);
        }
        // Without a player of its own a scenario still has the oracle's,
        // 5000 blocks east (Mob.checkDespawn draws for it).
        let far = [f64::from(s.origin[0] + 5000), -60.0, f64::from(s.origin[1])];
        let mut player = Some(s.player.unwrap_or(far)).map(|p| {
            let position = Vec3::new(p[0], p[1], p[2]);
            PlayerView {
                position,
                bounds: Aabb::new(
                    Vec3::new(p[0] - 0.3, p[1], p[2] - 0.3),
                    Vec3::new(p[0] + 0.3, p[1] + 1.8, p[2] + 0.3),
                ),
                eye: [p[0], p[1] + f64::from(1.62_f32), p[2]],
                alive: true,
                creative: s.creative,
                sneaking: false,
                swimming: false,
                look: [0.0, 0.0, 1.0],
                main_hand: s.hand.as_deref().map(Into::into),
                off_hand: None,
                head: None,
                armor_cover: 0.0,
                invisible: false,
                spectator: false,
                gold_armor: false,
                hurt_by: None,
                last_hurt: None,
                velocity: Vec3::ZERO,
                yaw: 0.0,
                sprinting: false,
                riding: false,
                hero: false,
                effects: Vec::new(),
                uuid: PLAYER_UUID,
                allies: Default::default(),
                remote: false,
                sleeping: false,
                sleep_timer: 0,
                passenger: false,
            }
        });
        let mut scratch = LegacyRandom::new(0);
        // The level's random for eating sounds: only goats and sniffers draw
        // from it, and the scenarios feed neither.
        let mut level_random = LegacyRandom::new(0);
        for (i, m) in mobs.iter().enumerate() {
            print(&mut out, 0, i, m, s.creative, world.game_time);
        }
        // No player has a chest open in the scenarios.
        let lid_openers = std::collections::HashMap::new();
        // The MobWorld every step sees, over a snapshot of the mobs.
        macro_rules! mob_world {
            ($snapshot:expr) => {
                mob_world!($snapshot, None)
            };
            ($snapshot:expr, $grid:expr) => {
                MobWorld {
                    teams: Default::default(),
                    world: &world,
                    registry,
                    facts: &facts,
                    fluid_tables: &motion.fluids,
                    movement: &motion.blocks,
                    collision: &collision,
                    data: &data,
                    player: player.clone(),
                    nearest_player: player.as_ref().filter(|p| !p.spectator).map(|p| p.position),
                    others: $snapshot,
                    others_grid: $grid,
                    others_index: None,
                    daylight: (world.day_time.rem_euclid(24000)) < 12000,
                    monsters_burn: !(12542..23460).contains(&world.day_time.rem_euclid(24000)),
                    game_time: world.game_time,
                    day_time: world.day_time,
                    raining: s.rain,
                    thundering: s.thunder,
                    nether: false,
                    block_tags: &items.block_tags,
                    biomes: &biomes,
                    mob_griefing: true,
                    mob_drops: true,
                    max_entity_cramming: 24,
                    difficulty: LocalDifficulty::new(s.difficulty, world.day_time, 0, 0.0),
                    end: false,
                    world_seed: 0,
                    // FlatLevelSource.getSeaLevel.
                    sea_level: -63,
                    items: Some(&items),
                    lid_openers: &lid_openers,
                }
            };
        }
        let mut dropped_leads = 0;
        for t in 1..=s.ticks {
            world.game_time += 1;
            if s.daycycle {
                world.day_time = s.time + i64::from(t);
            }
            let mut heart_events_later: Vec<HeartEvent> = Vec::new();
            // Creaking.tick reads the heart's block entity.
            for (at, h) in &hearts {
                world.block_entities.insert(
                    (at[0], at[1], at[2]),
                    BlockEntity::CreakingHeart(Box::new(h.clone())),
                );
            }
            if let (Some(p), Some((tick, yaw))) = (player.as_mut(), s.turn)
                && t >= tick
            {
                // Entity.calculateViewVector(0, yaw).
                let r = -yaw * horizon_core::geometry::trig::DEG_TO_RAD;
                let (cos, sin) = (
                    horizon_core::geometry::trig::cos(f64::from(r)),
                    horizon_core::geometry::trig::sin(f64::from(r)),
                );
                p.look = [f64::from(sin), -0.0, f64::from(cos)];
                p.yaw = yaw;
            }
            let mut snapshot: Vec<Seen> = mobs.iter().filter(|m| !m.removed).map(seen).collect();
            // The snapshot by position, kept as the game keeps it.
            let mut grid = SeenGrid::new(snapshot.iter().map(|e| e.bounds));
            let w = mob_world!(&snapshot);
            let mut events = Vec::new();
            let mut heart_events = Vec::new();
            let mut added = Vec::new();
            for &(tick, i) in &s.hurts {
                if tick != t || i >= mobs.len() || mobs[i].removed {
                    continue;
                }
                let at = player
                    .as_ref()
                    .map_or([0.0; 3], |p| [p.position.x, p.position.y, p.position.z]);
                mobs[i].damage(1.0, Damage::player(at));
                // Creaking.hurtServer: the heart's reaction, then the sway.
                if let Some((home, by_player)) = creaking::take_hurt(&mut mobs[i], &w) {
                    if by_player && let Some((_, h)) = hearts.iter_mut().find(|(at, _)| *at == home)
                    {
                        let mut host = Host {
                            mobs: &mut mobs,
                            added: Vec::new(),
                            types: &types,
                            created: &mut created,
                            spawning: s.spawning,
                        };
                        creaking_heart::creaking_hurt(
                            h,
                            home,
                            &w,
                            &mut scratch,
                            &mut host,
                            &mut heart_events,
                        );
                        added.append(&mut host.added);
                    }
                    for e in heart_events.drain(..) {
                        match e {
                            HeartEvent::Sound(name, _, volume, pitch) => {
                                events.push(MobEvent::Sound(name, [0.0; 3], volume, pitch))
                            }
                            HeartEvent::SetBlock(..) => heart_events_later.push(e),
                            HeartEvent::Mob(ev) => events.push(ev),
                            _ => {}
                        }
                    }
                    creaking::sway(&mut mobs[i], &mut events);
                }
                events.append(&mut mobs[i].outbox);
            }
            for (tick, i, item) in &s.feeds {
                let (tick, i) = (*tick, *i);
                if tick != t || i >= mobs.len() || mobs[i].removed {
                    continue;
                }
                // Horse.mobInteract's held-item branch (makeMad).
                if let Some(item) = item {
                    horizon_client::mobs::equine::held_item(
                        &mut mobs[i],
                        item,
                        false,
                        item == "minecraft:golden_dandelion",
                        &mut events,
                    );
                    continue;
                }
                if let Some(Some((name, volume, pitch))) =
                    horizon_client::mobs::ai::feed_animal(&mut mobs[i], &mut level_random)
                {
                    events.push(MobEvent::Sound(name, [0.0; 3], volume, pitch));
                }
            }
            #[allow(unused_mut)] // filled only with the rng-trace feature
            let mut traces: Vec<String> = Vec::new();
            let mut pushes = horizon_client::mobs::PendingPushes::default();
            let mut born = Vec::new();
            for i in 0..mobs.len() {
                if mobs[i].removed {
                    continue;
                }
                let w = mob_world!(&snapshot, Some(&grid));
                let m = &mut mobs[i];
                horizon_client::mobs::apply_mob_pushes(m, &mut pushes);
                let from = events.len();
                #[cfg(feature = "rng-trace")]
                horizon_core::random::trace::set(true);
                let alive = m.tick(&w, &mut scratch, &mut events);
                #[cfg(feature = "rng-trace")]
                {
                    horizon_core::random::trace::set(false);
                    for line in horizon_core::random::trace::take() {
                        traces.push(format!("{t} trace {i} {line}"));
                    }
                }
                horizon_client::mobs::take_mob_pushes(&mut events, from, &mut pushes);
                if !alive {
                    m.removed = true;
                }
                // Apply the same detach/drop event as Game::break_leash.
                for event in &events[from..] {
                    if let MobEvent::LeashBreak { mob, drop, .. } = event
                        && let Some(m) = mobs.iter_mut().find(|m| m.id == *mob)
                        && horizon_client::mobs::leash::remove(m).is_some()
                        && *drop
                    {
                        dropped_leads += 1;
                    }
                }
                horizon_client::mobs::apply_brain_events(&mut mobs, &mut events, from);
                villager_events(
                    &mut mobs,
                    &mut events,
                    from,
                    &w,
                    &types,
                    &items,
                    &mut born,
                    s.time,
                    &mut scratch,
                );
                horizon_client::mobs::warden::hear_mob_events(&mut mobs, &events, from, &w);
                // Melee hits on other mobs land during the attacker's tick
                // (LivingEntity.hurtServer), with the sounds they raise.
                let mut k = from;
                while k < events.len() {
                    if let MobEvent::AttackMob {
                        target,
                        amount,
                        source,
                    } = events[k]
                        && let Some(t) = mobs.iter_mut().find(|m| m.id == target)
                        && t.damage(amount, source)
                    {
                        let raised = std::mem::take(&mut t.outbox);
                        let n = raised.len();
                        events.splice(k + 1..k + 1, raised);
                        k += n;
                    }
                    k += 1;
                }
                // What a copper golem did to chests, before the next mob ticks.
                for e in &events[from.min(events.len())..] {
                    match e {
                        MobEvent::ChestOpen { chest, open } => {
                            let o = world
                                .chest_openers
                                .entry((chest[0], chest[1], chest[2]))
                                .or_insert((0, 0.0));
                            if *open {
                                o.0 += 1;
                                o.1 = o.1.max(3.0);
                            } else {
                                o.0 -= 1;
                                if o.0 == 0 {
                                    o.1 = 0.0;
                                }
                            }
                        }
                        MobEvent::ChestSlot { chest, slot, stack } => {
                            if let Some(e) = world
                                .block_entities
                                .get_mut(&(chest[0], chest[1], chest[2]))
                                && let Some(sl) = e.slots_mut().get_mut(*slot)
                            {
                                *sl = stack.clone();
                            }
                        }
                        _ => {}
                    }
                }
                // BeeEnterHiveGoal.start: BeehiveBlockEntity.addOccupant.
                let entering: Vec<[i32; 3]> = events[from.min(events.len())..]
                    .iter()
                    .filter_map(|e| match e {
                        MobEvent::EnterHive { hive, .. } => Some(*hive),
                        _ => None,
                    })
                    .collect();
                for at in entering {
                    let key = (at[0], at[1], at[2]);
                    let Some(BlockEntity::Beehive(mut hive)) = world.block_entities.remove(&key)
                    else {
                        continue;
                    };
                    let tag = horizon_client::entity_nbt::mob_to_nbt(&mobs[i], [0; 4], None);
                    let flower = horizon_client::mobs::bee::get(&mobs[i]).and_then(|b| b.flower);
                    let mut honey = Vec::new();
                    let stored = {
                        let w = mob_world!(&snapshot);
                        let mut host = ProbeHive {
                            w: &w,
                            random: &mut scratch,
                            types: &arc_types,
                            first: 0,
                            added: &mut Vec::new(),
                            honey: &mut honey,
                            events: &mut events,
                        };
                        hive.add_occupant(at, tag, flower, &mut host)
                    };
                    world.block_entities.insert(key, BlockEntity::Beehive(hive));
                    if stored {
                        mobs[i].removed = true;
                    }
                }
                // ServerLevel ticks entities one after another: the ones
                // after this mob see it as it is now.
                if let Some(k) = snapshot.iter().position(|e| e.id == mobs[i].id) {
                    snapshot[k] = seen(&mobs[i]);
                    snapshot[k].alive &= !mobs[i].removed;
                    grid.update(k, snapshot[k].bounds);
                }
            }
            for m in &mut mobs {
                horizon_client::mobs::apply_mob_pushes(m, &mut pushes);
            }
            let w = mob_world!(&snapshot);
            // ServerLevel.tickBlockEntities: the hearts (none ticks once uprooted).
            heart_events.append(&mut heart_events_later);
            for (at, h) in &mut hearts {
                let uprooted = registry
                    .state(w.world.block(at[0], at[1], at[2]))
                    .and_then(|s| s.properties.get("creaking_heart_state").cloned())
                    .is_some_and(|v| v == "uprooted");
                if uprooted {
                    continue;
                }
                let mut host = Host {
                    mobs: &mut mobs,
                    added: Vec::new(),
                    types: &types,
                    created: &mut created,
                    spawning: s.spawning,
                };
                creaking_heart::server_tick(h, *at, &w, &mut scratch, &mut host, &mut heart_events);
                added.append(&mut host.added);
            }
            drop(w);
            // Then the hives (BeehiveBlockEntity.serverTick).
            let mut hive_keys: Vec<(i32, i32, i32)> = world
                .block_entities
                .iter()
                .filter(|(_, e)| matches!(e, BlockEntity::Beehive(_)))
                .map(|(k, _)| *k)
                .collect();
            hive_keys.sort_unstable();
            let mut honey = Vec::new();
            for key in hive_keys {
                let at = [key.0, key.1, key.2];
                let (level, facing) = hive_state(registry, &world, at);
                let Some(BlockEntity::Beehive(mut hive)) = world.block_entities.remove(&key) else {
                    continue;
                };
                {
                    let w = mob_world!(&snapshot);
                    let mut host = ProbeHive {
                        w: &w,
                        random: &mut scratch,
                        types: &arc_types,
                        first: mobs.len() + born.len(),
                        added: &mut added,
                        honey: &mut honey,
                        events: &mut events,
                    };
                    hive.server_tick(at, facing, level, &mut host);
                }
                world.block_entities.insert(key, BlockEntity::Beehive(hive));
            }
            for (at, level) in honey {
                if let Some(mut st) = registry.state(world.block(at[0], at[1], at[2])).cloned() {
                    st.properties
                        .insert("honey_level".to_owned(), level.to_string());
                    world.set(at[0], at[1], at[2], registry.resolve(&st)?)?;
                }
            }
            // Doors villagers opened or closed (DoorBlock.setOpen), both halves.
            let doors: Vec<([i32; 3], bool)> = events
                .iter()
                .filter_map(|e| match e {
                    MobEvent::SetDoorOpen { pos, open } => Some((*pos, *open)),
                    _ => None,
                })
                .collect();
            for (at, open) in doors {
                let Some(half) = registry
                    .state(world.block(at[0], at[1], at[2]))
                    .and_then(|s| s.properties.get("half").cloned())
                else {
                    continue;
                };
                let other = if half == "lower" { 1 } else { -1 };
                for y in [at[1], at[1] + other] {
                    if let Some(mut st) = registry.state(world.block(at[0], y, at[2])).cloned()
                        && st.properties.contains_key("open")
                    {
                        st.properties.insert("open".to_owned(), open.to_string());
                        world.set(at[0], y, at[2], registry.resolve(&st)?)?;
                    }
                }
            }
            mobs.append(&mut born);
            mobs.append(&mut added);
            for e in &heart_events {
                match e {
                    HeartEvent::SetBlock(at, st) => world.set(at[0], at[1], at[2], *st)?,
                    HeartEvent::Sound(name, _, volume, pitch) => {
                        events.push(MobEvent::Sound(name, [0.0; 3], *volume, *pitch))
                    }
                    HeartEvent::Mob(ev) => events.push(ev.clone()),
                    _ => {}
                }
            }
            // A hit for the player's whole health kills it until the oracle
            // heals it next tick.
            let player_killed = events
                .iter()
                .any(|e| player_hurt(player.as_ref(), e) && player_damage(e) >= 20.0);
            for (i, m) in mobs.iter().enumerate() {
                print(
                    &mut out,
                    t,
                    i,
                    m,
                    s.creative || player_killed,
                    world.game_time,
                );
            }
            if s.name.starts_with("leash_death_") {
                let _ = writeln!(out, "{t} leads {dropped_leads}");
            }
            for e in &events {
                if let MobEvent::Sound(name, _, volume, pitch) = e {
                    let _ = writeln!(
                        out,
                        "{t} sound minecraft:{name} {} {}",
                        f(*volume),
                        f(*pitch)
                    );
                } else if let MobEvent::SoundId(name, _, volume, pitch) = e {
                    let _ = writeln!(out, "{t} sound {name} {} {}", f(*volume), f(*pitch));
                } else if player_hurt(player.as_ref(), e) {
                    // The oracle's survival player, healed and made hurtable every tick,
                    // takes the damage (LivingEntity.playHurtSound).
                    let sound = if player_damage(e) >= 20.0 {
                        "death"
                    } else {
                        "hurt"
                    };
                    let _ = writeln!(
                        out,
                        "{t} sound minecraft:entity.player.{sound} 3f800000 3f800000"
                    );
                }
            }
            for (at, h) in &hearts {
                print_heart(&mut out, t, registry, &world, *at, h, &scratch);
            }
            for b in s.blocks.iter().filter(|b| b[0] == "chest") {
                let n = |i: usize| b[i].parse::<i32>().unwrap();
                let key = (n(1), n(2), n(3));
                let Some(e) = world.block_entities.get(&key) else {
                    continue;
                };
                let mut slots: Vec<String> = e
                    .slots()
                    .iter()
                    .map(|s| s.as_ref().map_or("-".into(), stack))
                    .collect();
                while slots.last().is_some_and(|s| s == "-") {
                    slots.pop();
                }
                let count = world.chest_openers.get(&key).map_or(0, |o| o.0);
                let _ = writeln!(
                    out,
                    "{t} chest {},{},{} {} {count}",
                    key.0 - s.origin[0],
                    key.1,
                    key.2 - s.origin[1],
                    if slots.is_empty() {
                        "-".into()
                    } else {
                        slots.join(",")
                    }
                );
            }
            for b in s.blocks.iter().filter(|b| is_hive(b)) {
                let n = |i: usize| b[i].parse::<i32>().unwrap();
                let Some(BlockEntity::Beehive(hive)) =
                    world.block_entities.get(&(n(1), n(2), n(3)))
                else {
                    continue;
                };
                let (level, _) = hive_state(registry, &world, [n(1), n(2), n(3)]);
                let bees: Vec<String> = hive
                    .bees
                    .iter()
                    .map(|b| format!("{}/{}", b.ticks_in_hive, b.min_ticks_in_hive))
                    .collect();
                let _ = writeln!(
                    out,
                    "{t} hive {},{},{} honey {} bees {} flower {}",
                    b[1],
                    b[2],
                    b[3],
                    level.unwrap_or(-1),
                    if bees.is_empty() {
                        "-".into()
                    } else {
                        bees.join(",")
                    },
                    hive.flower
                        .map_or("-".into(), |f| format!("{},{},{}", f[0], f[1], f[2]))
                );
            }
            for line in traces {
                let _ = writeln!(out, "{line}");
            }
        }
    }
    fs::write(&args[2], out)?;
    Ok(())
}
