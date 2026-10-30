//! The ender dragon fight (`EnderDragonFight`, `EnderDragon`, its phases,
//! `EndCrystal`, `DragonFireball`): the fight state saved in the End's
//! `ender_dragon_fight.dat`, the dragon spawned over the island, the
//! holding pattern over the 24-node path graph, strafing with fireballs,
//! landing on the podium to breathe and charge, healing from the crystals on
//! the spikes, its parts (head and neck take full damage, the rest a quarter
//! plus one), wing knockback, block breaking, the death flight and rays, the
//! exit portal, the egg and the next gateway; and its rendering and boss bar.
use super::{Dim, Game};
use horizon_client::{
    hud::HudBatch,
    hud_layout, hud_overlays,
    mesher::Vertex,
    particles::{ParticleOptions, ParticleType},
    text::Text,
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::{LegacyRandom, RandomSource},
};
use horizon_protocol::entity_data::Value;
use horizon_world::nbt::Tag;

/// Mob ids from here on are dragon parts (`SPECIAL + part`) and crystals
/// (`SPECIAL + 16 + crystal`), so projectiles can target them.
pub(super) const SPECIAL: u32 = u32::MAX - 4096;
const CRYSTAL_IDS: u32 = SPECIAL + 16;
const FIGHT_FILE: &str = "data/minecraft/ender_dragon_fight.dat";
const MAX_HEALTH: f32 = 200.0;
/// `Mob.getAmbientSoundInterval`.
const AMBIENT_INTERVAL: i32 = 80;

#[derive(Default)]
pub(super) struct Fight {
    loaded: bool,
    needs_scan: bool,
    pub killed: bool,
    pub previously_killed: bool,
    gateways: Vec<i32>,
    exit_portal: Option<[i32; 3]>,
    alive_crystals: i32,
    crystal_scan: i32,
    /// `ticksSinceLastPlayerScan`, and whether the last scan found a player
    /// for the fight (`dragonEvent.getPlayers()` is not empty).
    player_scan: i32,
    players: bool,
    /// The spikes' crystals have been placed (worldgen does not place entities here).
    crystals_placed: bool,
    /// The respawn sequence (`DragonRespawnStage`) and its tick count.
    respawn: Option<(Respawn, i32)>,
    /// The four crystals on the exit portal driving the respawn.
    respawn_crystals: Vec<Vec3>,
    /// The player behind the crystal blast under way, whose `explosion`
    /// damage source names them (`EndCrystal.hurtServer`).
    blast_by: Option<FightWho>,
    dirty: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Respawn {
    Start,
    Preparing,
    Pillars,
    Summoning,
}
impl Respawn {
    fn name(self) -> &'static str {
        match self {
            Respawn::Start => "start",
            Respawn::Preparing => "preparing_to_summon_pillars",
            Respawn::Pillars => "summoning_pillars",
            Respawn::Summoning => "summoning_dragon",
        }
    }
    fn from_name(name: &str) -> Option<Self> {
        [
            Respawn::Start,
            Respawn::Preparing,
            Respawn::Pillars,
            Respawn::Summoning,
        ]
        .into_iter()
        .find(|r| r.name() == name)
    }
}
#[derive(Clone, Debug)]
pub(super) struct Crystal {
    /// The entity id a server tracks it by (0 until it is tracked).
    pub id: u32,
    pub position: Vec3,
    pub show_bottom: bool,
    /// EndCrystal.time (the bob and spin), starting random.
    pub time: i32,
    pub beam: Option<[i32; 3]>,
    /// `setPermanentlyInvulnerable` (respawned spike crystals).
    pub invulnerable: bool,
}
#[derive(Clone, Debug, Default)]
struct Path {
    nodes: Vec<[i32; 3]>,
    next: usize,
}
impl Path {
    fn done(&self) -> bool {
        self.next >= self.nodes.len()
    }
    fn advance(&mut self) {
        self.next += 1;
    }
    fn next_pos(&self) -> [i32; 3] {
        self.nodes[self.next]
    }
}
#[derive(Clone, Debug, PartialEq)]
enum Phase {
    Holding {
        path: Option<PathBox>,
        target: Option<Vec3>,
        clockwise: bool,
    },
    Strafe {
        charge: i32,
        path: Option<PathBox>,
        target: Option<Vec3>,
        clockwise: bool,
        /// `attackTarget`: the player it strafes.
        player: Option<FightWho>,
    },
    LandingApproach {
        path: Option<PathBox>,
        target: Option<Vec3>,
    },
    Landing {
        target: Option<Vec3>,
    },
    Takeoff {
        first: bool,
        path: Option<PathBox>,
        target: Option<Vec3>,
    },
    Scanning {
        time: i32,
    },
    Attacking {
        ticks: i32,
    },
    Flaming {
        ticks: i32,
    },
    Charging {
        target: Option<Vec3>,
        since: i32,
    },
    Dying {
        target: Option<Vec3>,
        time: i32,
    },
    Hover {
        target: Option<Vec3>,
    },
}
/// A path compared by identity only (phases compare by kind).
#[derive(Clone, Debug)]
struct PathBox(Path);
impl PartialEq for PathBox {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Phase {
    /// `EnderDragonPhase` ids.
    fn id(&self) -> i32 {
        match self {
            Self::Holding { .. } => 0,
            Self::Strafe { .. } => 1,
            Self::LandingApproach { .. } => 2,
            Self::Landing { .. } => 3,
            Self::Takeoff { .. } => 4,
            Self::Flaming { .. } => 5,
            Self::Scanning { .. } => 6,
            Self::Attacking { .. } => 7,
            Self::Charging { .. } => 8,
            Self::Dying { .. } => 9,
            Self::Hover { .. } => 10,
        }
    }
    fn from_id(id: i32) -> Self {
        match id {
            1 => Self::Strafe {
                charge: 0,
                path: None,
                target: None,
                clockwise: false,
                player: None,
            },
            2 => Self::LandingApproach {
                path: None,
                target: None,
            },
            3 => Self::Landing { target: None },
            4 => Self::Takeoff {
                first: true,
                path: None,
                target: None,
            },
            5 => Self::Flaming { ticks: 0 },
            6 => Self::Scanning { time: 0 },
            7 => Self::Attacking { ticks: 0 },
            8 => Self::Charging {
                target: None,
                since: 0,
            },
            9 => Self::Dying {
                target: None,
                time: 0,
            },
            10 => Self::Hover { target: None },
            _ => Self::Holding {
                path: None,
                target: None,
                clockwise: false,
            },
        }
    }
    fn sitting(&self) -> bool {
        matches!(
            self,
            Self::Scanning { .. }
                | Self::Attacking { .. }
                | Self::Flaming { .. }
                | Self::Hover { .. }
        )
    }
    fn target(&self) -> Option<Vec3> {
        match self {
            Self::Holding { target, .. }
            | Self::Strafe { target, .. }
            | Self::LandingApproach { target, .. }
            | Self::Landing { target }
            | Self::Takeoff { target, .. }
            | Self::Charging { target, .. }
            | Self::Dying { target, .. }
            | Self::Hover { target } => *target,
            _ => None,
        }
    }
    fn fly_speed(&self) -> f32 {
        match self {
            Self::Landing { .. } => 1.5,
            Self::Charging { .. } | Self::Dying { .. } => 3.0,
            _ => 0.6,
        }
    }
}
pub(super) struct Dragon {
    /// `Entity.id` once tracked to guests (0 before); its parts take the
    /// eight ids after it (`EnderDragon`'s constructor reserves them).
    pub id: u32,
    pub position: Vec3,
    pub previous: Vec3,
    velocity: Vec3,
    pub yaw: f32,
    pub previous_yaw: f32,
    yaw_accel: f32,
    pub health: f32,
    pub hurt_time: i32,
    /// `LivingEntity.damageCooldownTime` and `lastHurt`: within the window
    /// only damage above the last hit lands, and only the excess.
    invulnerable: i32,
    last_hurt: f32,
    /// `lastHurtByPlayer` with its 100 tick memory: the player credited with
    /// the kill, and the dragon tick of their last hit.
    hurt_by_player: Option<(FightWho, i32)>,
    pub death_time: i32,
    pub flap: f32,
    pub old_flap: f32,
    in_wall: bool,
    phase: Phase,
    flame_count: i32,
    sitting_damage: f32,
    growl: i32,
    /// `Mob.ambientSoundTime`.
    ambient_time: i32,
    ticks: i32,
    /// DragonFlightHistory: (y, yRot) samples, newest at `head`.
    history: [(f64, f32); 64],
    head: usize,
    /// head, neck, body, tail1-3, wing1-2.
    pub parts: [(Vec3, Vec3); 8],
    nodes: Option<[[i32; 3]; 24]>,
    nearest_crystal: Option<usize>,
    /// A crystal destroyed while it held its pattern (taken on its next tick).
    strafe_request: Option<StrafeRequest>,
    random: LegacyRandom,
}
/// A player in the fight: the window's own player, or a guest by
/// connection (a LAN or server player).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FightWho {
    Host,
    Guest(u32),
}
/// A player in the End as the dragon sees it.
#[derive(Clone, Copy, Debug)]
pub(super) struct FightPlayer {
    pub(super) who: FightWho,
    pub(super) position: Vec3,
    pub(super) bounds: Aabb,
    pub(super) alive: bool,
    creative: bool,
    spectator: bool,
}
impl FightPlayer {
    /// `Player.canBeSeenAsEnemy`: alive, neither creative nor spectator.
    fn enemy(&self) -> bool {
        self.alive && !self.creative && !self.spectator
    }
}
/// `TargetingConditions.forCombat` (visibility aside) in
/// `Level.getNearestPlayer`: the enemy nearest `at` among `players`, within
/// `range` of the dragon at `from` and passing `selector`; none on Peaceful.
fn nearest_enemy(
    players: &[FightPlayer],
    peaceful: bool,
    at: Vec3,
    range: Option<(Vec3, f64)>,
    selector: impl Fn(&FightPlayer) -> bool,
) -> Option<FightPlayer> {
    if peaceful {
        return None;
    }
    players
        .iter()
        .filter(|p| p.enemy() && selector(p))
        .filter(|p| range.is_none_or(|(from, r)| p.position.distance_squared(from) <= r * r))
        .min_by(|a, b| {
            a.position
                .distance_squared(at)
                .total_cmp(&b.position.distance_squared(at))
        })
        .copied()
}
/// Whom a destroyed crystal sends a holding dragon after
/// (`DragonHoldingPatternPhase.onCrystalDestroyed`).
#[derive(Clone, Copy, Debug)]
enum StrafeRequest {
    /// The player who destroyed it.
    Player(FightWho),
    /// Anything else: the player nearest the crystal.
    Nearest(Vec3),
}
/// Whether `EndCrystal.hurtServer` creates a blast before notifying the fight.
#[derive(Clone, Copy, PartialEq)]
enum CrystalDamage {
    Direct,
    Explosion,
}
/// What `EnderDragon.hurt` made of a hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Hurt {
    /// It returned true; `full` when it was a new hit, not just the excess
    /// over the last one (the hurt sound and animation).
    Landed { full: bool },
    /// It returned false (dying, too little damage).
    Refused,
    /// A perched dragon's `onHurt` took an arrow's damage away (and set the
    /// arrow alight for a second).
    Shrugged,
}
const PART_SIZES: [(f64, f64); 8] = [
    (1.0, 1.0),
    (3.0, 3.0),
    (5.0, 3.0),
    (2.0, 2.0),
    (2.0, 2.0),
    (2.0, 2.0),
    (4.0, 2.0),
    (4.0, 2.0),
];
const ADJACENCY: [u32; 24] = [
    6146, 8197, 8202, 16404, 32808, 32848, 65696, 131392, 131712, 263424, 526848, 525313, 1581057,
    3166214, 2138120, 6373424, 4358208, 12910976, 9044480, 9706496, 15216640, 13688832, 11763712,
    8257536,
];
impl Dragon {
    fn new(position: Vec3, yaw: f32, seed: i64) -> Self {
        Self {
            id: 0,
            position,
            previous: position,
            velocity: Vec3::ZERO,
            yaw,
            previous_yaw: yaw,
            yaw_accel: 0.0,
            health: MAX_HEALTH,
            hurt_time: 0,
            invulnerable: 0,
            last_hurt: 0.0,
            hurt_by_player: None,
            death_time: 0,
            flap: 0.0,
            old_flap: 0.0,
            in_wall: false,
            phase: Phase::Holding {
                path: None,
                target: None,
                clockwise: false,
            },
            flame_count: 0,
            sitting_damage: 0.0,
            growl: 100,
            ambient_time: 0,
            ticks: 0,
            history: [(position.y, yaw); 64],
            head: 0,
            parts: [(position, position); 8],
            nodes: None,
            nearest_crystal: None,
            strafe_request: None,
            random: LegacyRandom::new(seed),
        }
    }
    /// DragonFlightHistory.record.
    fn record(&mut self) {
        self.head = (self.head + 1) & 63;
        self.history[self.head] = (self.position.y, self.yaw);
    }
    /// DragonFlightHistory.get(delay).
    fn sample(&self, delay: usize) -> (f64, f32) {
        self.history[(self.head + 64 - (delay & 63)) & 63]
    }
    /// DragonFlightHistory.get(delay, partial).
    fn sample_lerp(&self, delay: usize, partial: f32) -> (f64, f32) {
        let a = self.sample(delay);
        let b = self.sample(delay + 1);
        let yaw = b.1 + wrap(a.1 - b.1) * partial;
        (b.0 + (a.0 - b.0) * f64::from(partial), yaw)
    }
    pub fn alive(&self) -> bool {
        self.health > 0.0
    }
    /// `isDeadOrDying` for the killing blow: the dying flight, or no health.
    fn down(&self) -> bool {
        matches!(self.phase, Phase::Dying { .. }) || !self.alive()
    }
    fn part_box(&self, i: usize) -> Aabb {
        let (w, h) = PART_SIZES[i];
        let p = self.parts[i].0;
        Aabb::new(
            Vec3::new(p.x - w / 2.0, p.y, p.z - w / 2.0),
            Vec3::new(p.x + w / 2.0, p.y + h, p.z + w / 2.0),
        )
    }
    /// `EnderDragon.hurt(part, source, damage)`: whether the hit landed
    /// (an arrow is spent, else it turns back). `counts` is
    /// `source.getEntity() instanceof Player` or a damage type that always
    /// hurts dragons (explosions); `arrow` a direct arrow or wind charge,
    /// which a perched dragon shrugs off (`AbstractDragonSittingPhase.onHurt`).
    fn hurt(&mut self, part: usize, damage: f32, counts: bool, arrow: bool) -> Hurt {
        if matches!(self.phase, Phase::Dying { .. }) || !self.alive() {
            return Hurt::Refused;
        }
        if arrow && self.phase.sitting() {
            return Hurt::Shrugged;
        }
        let mut damage = damage;
        if part != 0 && part != 1 {
            damage = damage / 4.0 + damage.min(1.0);
        }
        if damage < 0.01 {
            return Hurt::Refused;
        }
        if !counts {
            return Hurt::Landed { full: false };
        }
        let before = self.health;
        // LivingEntity.hurtServer: within the cooldown only what exceeds the last hit.
        let full = if self.invulnerable > 10 {
            if damage <= self.last_hurt {
                return Hurt::Landed { full: false };
            }
            self.health -= damage - self.last_hurt;
            self.last_hurt = damage;
            false
        } else {
            self.last_hurt = damage;
            self.invulnerable = 20;
            self.health -= damage;
            self.hurt_time = 10;
            true
        };
        if self.health <= 0.0 {
            if self.phase.sitting() {
                self.health = 0.0;
            } else {
                // handleKillingBlow: dying flight first.
                self.health = 1.0;
                self.set_phase(Phase::Dying {
                    target: None,
                    time: 0,
                });
            }
        }
        if self.phase.sitting() {
            self.sitting_damage += before - self.health.max(0.0);
            if self.sitting_damage > 0.25 * MAX_HEALTH {
                self.sitting_damage = 0.0;
                self.set_phase(Phase::Takeoff {
                    first: true,
                    path: None,
                    target: None,
                });
            }
        }
        Hurt::Landed { full }
    }
    /// `ServerExplosion.hurtEntities` on the dragon: the dragon itself (its
    /// body takes it), then each part, in `Level.getEntities` order, each by
    /// its distance and how much of its box the blast sees (`exposure`).
    fn explosion_hits(
        &self,
        center: [f64; 3],
        radius: f32,
        exposure: impl Fn(Aabb) -> f32,
    ) -> Vec<(usize, f32)> {
        let diameter = f64::from(radius * 2.0);
        let dist = |p: Vec3| {
            ((p.x - center[0]).powi(2) + (p.y - center[1]).powi(2) + (p.z - center[2]).powi(2))
                .sqrt()
                / diameter
        };
        let p = self.position;
        let whole = Aabb::new(
            Vec3::new(p.x - 8.0, p.y, p.z - 8.0),
            Vec3::new(p.x + 8.0, p.y + 8.0, p.z + 8.0),
        );
        std::iter::once((2, p, whole))
            .chain((0..8).map(|i| (i, self.parts[i].0, self.part_box(i))))
            .filter_map(|(part, at, b)| {
                let d = dist(at);
                (d <= 1.0).then(|| {
                    let pow = (1.0 - d) * f64::from(exposure(b));
                    (
                        part,
                        ((pow * pow + pow) / 2.0 * 7.0 * diameter + 1.0) as f32,
                    )
                })
            })
            .collect()
    }
    fn set_phase(&mut self, phase: Phase) {
        if self.phase.id() == phase.id() && phase.id() != 1 {
            return;
        }
        if let Phase::Flaming { .. } = phase {
            self.flame_count += 1;
        }
        self.phase = phase;
    }
}
fn wrap(d: f32) -> f32 {
    let r = d.rem_euclid(360.0);
    if r >= 180.0 { r - 360.0 } else { r }
}
fn wrap64(d: f64) -> f64 {
    let r = d.rem_euclid(360.0);
    if r >= 180.0 { r - 360.0 } else { r }
}
impl Game<'_> {
    fn fight_path(&self) -> std::path::PathBuf {
        horizon_client::save::dimension_dir(&self.world_root, "minecraft:the_end").join(FIGHT_FILE)
    }
    /// The saved fight, or `createDefault` with the gateways shuffled by the seed.
    fn load_fight(&mut self) {
        if self.fight.loaded {
            return;
        }
        let tag = horizon_client::save::read_dat(&self.fight_path());
        let data = tag.as_ref().and_then(|t| t.get("data"));
        let flag = |k: &str, default: bool| match data.and_then(|d| d.get(k)) {
            Some(Tag::Byte(v)) => *v != 0,
            _ => default,
        };
        let mut fight = Fight {
            loaded: true,
            needs_scan: flag("needs_state_scanning", true),
            killed: flag("dragon_killed", false),
            previously_killed: flag("previously_killed", false),
            crystals_placed: flag("horizon_crystals_placed", false),
            gateways: match data.and_then(|d| d.get("gateways")) {
                Some(Tag::IntArray(v)) => v.clone(),
                Some(Tag::List(v)) => v
                    .iter()
                    .filter_map(|t| match t {
                        Tag::Int(i) => Some(*i),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            },
            respawn: match data.and_then(|d| d.get("respawn_stage")) {
                Some(Tag::String(name)) => {
                    Respawn::from_name(&name.to_string().unwrap_or_default()).map(|r| {
                        let time = match data.and_then(|d| d.get("respawn_time")) {
                            Some(Tag::Int(t)) => *t,
                            _ => 0,
                        };
                        (r, time)
                    })
                }
                _ => None,
            },
            respawn_crystals: match data.and_then(|d| d.get("horizon_respawn_crystals")) {
                Some(Tag::IntArray(v)) => v
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|b| {
                        Vec3::new(
                            f64::from(b[0]) + 0.5,
                            f64::from(b[1]),
                            f64::from(b[2]) + 0.5,
                        )
                    })
                    .collect(),
                _ => Vec::new(),
            },
            exit_portal: match data.and_then(|d| d.get("exit_portal_location")) {
                Some(Tag::IntArray(v)) if v.len() == 3 => Some([v[0], v[1], v[2]]),
                _ => None,
            },
            // The first tick scans for players.
            player_scan: 21,
            ..Default::default()
        };
        if fight.gateways.is_empty() {
            // Util.shuffle(0..20, RandomSource.create(seed)).
            let mut gateways: Vec<i32> = (0..20).collect();
            let mut random = LegacyRandom::new(self.level.seed);
            for i in (2..=gateways.len()).rev() {
                let j = random.next_int_bound(i as i32).unwrap_or(0) as usize;
                gateways.swap(i - 1, j);
            }
            fight.gateways = gateways;
            fight.dirty = true;
        }
        // A world from before this fight existed: one with an active portal
        // already won it.
        if tag.is_none() && self.end_portal_active() {
            fight.killed = true;
            fight.previously_killed = true;
            fight.crystals_placed = true;
            fight.needs_scan = false;
        }
        self.fight = fight;
    }
    fn end_portal_active(&self) -> bool {
        (0..256).any(|y| {
            &*self.res.facts().get(self.world.block(0, y, 0)).name == "minecraft:bedrock"
                && (-3..=3).any(|d| {
                    &*self.res.facts().get(self.world.block(d, y, 0)).name == "minecraft:end_portal"
                })
        })
    }
    pub(super) fn save_fight(&mut self) {
        if !self.fight.loaded || !self.fight.dirty {
            return;
        }

        let f = &self.fight;
        let b = |v: bool| Tag::Byte(i8::from(v));
        let mut data = vec![
            ("needs_state_scanning", b(f.needs_scan)),
            ("dragon_killed", b(f.killed)),
            ("previously_killed", b(f.previously_killed)),
            ("respawn_time", Tag::Int(f.respawn.map_or(0, |r| r.1))),
            ("gateways", Tag::IntArray(f.gateways.clone())),
            ("horizon_crystals_placed", b(f.crystals_placed)),
        ];
        if let Some(p) = f.exit_portal {
            data.push(("exit_portal_location", Tag::IntArray(p.to_vec())));
        }
        if let Some((stage, _)) = f.respawn {
            data.push(("respawn_stage", Tag::String(stage.name().into())));
            data.push((
                "horizon_respawn_crystals",
                Tag::IntArray(
                    f.respawn_crystals
                        .iter()
                        .flat_map(|p| [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32])
                        .collect(),
                ),
            ));
        }
        let tag = Tag::compound([
            ("data", Tag::compound(data)),
            ("DataVersion", Tag::Int(horizon_world::DATA_VERSION)),
        ]);
        if horizon_client::save::write_dat(&self.fight_path(), tag).is_ok() {
            self.fight.dirty = false;
        }
    }
    /// The island's centre chunks are lit (the arena is "loaded").
    fn arena_loaded(&self) -> bool {
        (-2..=1).all(|cz| (-2..=1).all(|cx| self.streaming.is_lit(cx, cz)))
    }
    /// The player acting now: the guest whose turn it is, else the host.
    fn acting_player(&self) -> FightWho {
        self.lan_acting.map_or(FightWho::Host, FightWho::Guest)
    }
    /// The players in the level the dragon sees (`level.players()`): the
    /// window's own, then every guest in the level.
    pub(super) fn fight_players(&self) -> Vec<FightPlayer> {
        let mut out = Vec::new();
        if let Some(p) = self.player.as_ref() {
            out.push(FightPlayer {
                who: FightWho::Host,
                position: p.position,
                bounds: p.bounds(),
                alive: !p.health.dead_or_dying(),
                creative: self.creative,
                spectator: self.spectator(),
            });
        }
        if let Some(h) = self.lan.as_ref() {
            for g in h.hub.guests.iter().filter(|g| h.hub.here(g) && g.loaded) {
                let [x, y, z] = g.position;
                out.push(FightPlayer {
                    who: FightWho::Guest(g.conn),
                    position: Vec3::new(x, y, z),
                    bounds: super::lan_mobs::guest_bounds(g),
                    alive: !g.dead,
                    creative: g.creative(),
                    spectator: g.spectator(),
                });
            }
        }
        out
    }
    /// Run `f` as the fight's player `who` (a guest's turn, its knockback
    /// sent to its client).
    pub(super) fn as_fight_player(&mut self, who: FightWho, f: impl FnOnce(&mut Self)) {
        match who {
            FightWho::Host => f(self),
            FightWho::Guest(conn) => {
                let Some(mut host) = self.lan.take() else {
                    return;
                };
                self.guest_hit_by(&mut host, conn, f);
                self.lan = Some(host);
            }
        }
    }
    /// `EnderDragonFight.validPlayer`: the players alive within 192 blocks of
    /// (0, 128, 0), the host and the guests in the level alike.
    fn fight_has_players(&self) -> bool {
        let near =
            |p: [f64; 3]| p[0].powi(2) + (p[1] - 128.0).powi(2) + p[2].powi(2) <= 192.0 * 192.0;
        let host = self.player.as_ref().is_some_and(|p| {
            !p.health.dead_or_dying() && near([p.position.x, p.position.y, p.position.z])
        });
        host || self.lan.as_ref().is_some_and(|h| {
            h.hub
                .guests
                .iter()
                .any(|g| h.hub.here(g) && g.loaded && !g.dead && near(g.position))
        })
    }
    /// `EnderDragonFight.tick` and the fight's entities, while the End is
    /// ticked (in place for its player, or on a server with players in it).
    pub(super) fn tick_fight(&mut self) {
        if self.dim != Dim::End || self.travelling.is_some() {
            return;
        }
        self.load_fight();
        // updatePlayers, every 20 ticks.
        self.fight.player_scan += 1;
        if self.fight.player_scan >= 20 {
            self.fight.player_scan = 0;
            self.fight.players = self.fight_has_players();
        }
        if !self.arena_loaded() {
            return;
        }
        if !self.fight.crystals_placed {
            self.place_spike_crystals();
            self.fight.crystals_placed = true;
            self.fight.dirty = true;
        }
        if self.fight.players {
            if self.fight.needs_scan {
                // scanState: a missing exit portal means a fight in progress.
                self.fight.previously_killed = self.end_portal_active();
                if !self.fight.previously_killed {
                    self.fight.killed = false;
                }
                self.fight.needs_scan = false;
                self.fight.dirty = true;
            }
            if self.fight.respawn.is_some() {
                self.tick_respawn();
            }
            if !self.fight.killed {
                if self.dragon.is_none() {
                    self.create_new_dragon();
                }
                self.fight.crystal_scan += 1;
                if self.fight.crystal_scan >= 100 {
                    self.count_crystals();
                }
            }
        }
        // EnderDragon's constructor takes its id and its parts' from the
        // entity counter (a dragon loaded from the save gets them here too).
        if let Some(d) = self.dragon.as_mut()
            && d.id == 0
        {
            d.id = self.next_entity + 1;
            self.next_entity += 9;
        }
        // The spikes' crystals too (EndCrystal's constructor): an id whether
        // or not a client has tracked them yet, to be aimed at by it.
        for c in self.crystals.iter_mut().filter(|c| c.id == 0) {
            self.next_entity += 1;
            c.id = self.next_entity;
        }
        self.tick_dragon();
    }
    /// EndSpikeFeature's crystals (with fire under them on the bedrock).
    fn place_spike_crystals(&mut self) {
        for spike in horizon_world::worldgen::end_spike::spikes_for_seed(self.level.seed) {
            let at = Vec3::new(
                f64::from(spike.center_x) + 0.5,
                f64::from(spike.height + 1),
                f64::from(spike.center_z) + 0.5,
            );
            if self
                .crystals
                .iter()
                .any(|c| (c.position.x - at.x).abs() < 1.0 && (c.position.z - at.z).abs() < 1.0)
            {
                continue;
            }
            let time = self.world.random.next_int_bound(100_000).unwrap_or(0);
            // EndCrystal's constructor takes an id from the entity counter.
            self.next_entity += 1;
            self.crystals.push(Crystal {
                id: self.next_entity,
                position: at,
                show_bottom: true,
                time,
                beam: None,
                invulnerable: false,
            });
            self.world
                .modified
                .insert((at.x.floor() as i32 >> 4, at.z.floor() as i32 >> 4));
        }
    }
    /// `updateCrystalCount`: crystals on the spike tops.
    fn count_crystals(&mut self) {
        self.fight.crystal_scan = 0;
        let spikes = horizon_world::worldgen::end_spike::spikes_for_seed(self.level.seed);
        self.fight.alive_crystals = self
            .crystals
            .iter()
            .filter(|c| {
                spikes.iter().any(|s| {
                    (c.position.x - f64::from(s.center_x) - 0.5).abs() <= f64::from(s.radius)
                        && (c.position.z - f64::from(s.center_z) - 0.5).abs() <= f64::from(s.radius)
                        && c.position.y >= f64::from(s.height)
                })
            })
            .count() as i32;
    }
    /// `EndCrystal.tick` in every dimension: the time (the spin and bob),
    /// and fire at its block where there is a dragon fight (the End).
    pub(super) fn tick_crystals(&mut self) {
        let fire = (self.dim == Dim::End)
            .then(|| self.pack.assets.registry.default_state("minecraft:fire"))
            .flatten();
        for i in 0..self.crystals.len() {
            self.crystals[i].time += 1;
            let p = self.crystals[i].position;
            let b = [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32];
            if let Some(fire) = fire
                && self.res.facts().is_air(self.world.block(b[0], b[1], b[2]))
                && self.world.loaded(b[0], b[2])
            {
                let _ = self.level_set(b, fire);
            }
        }
    }
    /// `EndCrystal.hurtServer`: remove it, blast only for non-explosion damage,
    /// then notify the dragon fight even when no blast was created.
    fn destroy_crystal(&mut self, index: usize, by: Option<FightWho>, damage: CrystalDamage) {
        if index >= self.crystals.len() || self.crystals[index].invulnerable {
            return;
        }
        let c = self.crystals.remove(index);
        // Read before the blast, which can take other respawn crystals.
        let respawn_crystal = self.fight.respawn.is_some()
            && self
                .fight
                .respawn_crystals
                .iter()
                .any(|p| p.distance_squared(c.position) < 0.01);
        if damage == CrystalDamage::Direct {
            let at = [c.position.x, c.position.y, c.position.z];
            // The blast's damage source carries the player, so the dragon
            // remembers them before the blast can kill it.
            self.fight.blast_by = by;
            // EndCrystal.hurtServer: ExplosionInteraction.BLOCK.
            let interaction = self.destroy_type("block_explosion_drop_decay");
            let _ = self.server_explode(at, 6.0, interaction, false);
            self.fight.blast_by = None;
        }
        if respawn_crystal {
            self.abort_respawn();
            return;
        }
        // EnderDragonFight.onCrystalDestroyed.
        self.count_crystals();
        if let Some(d) = self.dragon.as_mut() {
            if d.nearest_crystal == Some(index) {
                d.nearest_crystal = None;
                self.hurt_dragon(0, 10.0, true, by, false);
            } else if let Some(n) = d.nearest_crystal.as_mut()
                && *n > index
            {
                *n -= 1;
            }
            // EnderDragon.onCrystalDestroyed: the player who did it, else the
            // nearest; DragonHoldingPatternPhase strafes them.
            if let Some(d) = self.dragon.as_mut()
                && matches!(d.phase, Phase::Holding { .. })
            {
                d.strafe_request = Some(match by {
                    Some(who) => StrafeRequest::Player(who),
                    None => StrafeRequest::Nearest(c.position),
                });
            }
        }
    }
    /// Targets for projectiles: dragon parts and crystals.
    pub(super) fn special_targets(&self) -> Vec<(u32, Aabb)> {
        let mut out = Vec::new();
        if let Some(d) = self.dragon.as_ref().filter(|d| d.alive()) {
            for i in 0..8 {
                out.push((SPECIAL + i as u32, d.part_box(i)));
            }
        }
        for (i, c) in self.crystals.iter().enumerate() {
            let p = c.position;
            out.push((
                CRYSTAL_IDS + i as u32,
                Aabb::new(
                    Vec3::new(p.x - 1.0, p.y, p.z - 1.0),
                    Vec3::new(p.x + 1.0, p.y + 2.0, p.z + 1.0),
                ),
            ));
        }
        out
    }
    /// The box of the dragon part or end crystal guests know as entity `id`
    /// (`EnderDragonPart`s follow the dragon's id).
    pub(super) fn tracked_special_box(&self, id: i32) -> Option<Aabb> {
        let id = u32::try_from(id).ok().filter(|&id| id != 0)?;
        if let Some(d) = self.dragon.as_ref().filter(|d| d.alive() && d.id != 0)
            && (d.id + 1..=d.id + 8).contains(&id)
        {
            return Some(d.part_box((id - d.id - 1) as usize));
        }
        let i = self.crystals.iter().position(|c| c.id == id)?;
        self.special_targets()
            .into_iter()
            .find(|(s, _)| *s == CRYSTAL_IDS + i as u32)
            .map(|(_, b)| b)
    }
    /// Damage to a special id (from projectiles and melee): what the
    /// dragon part or crystal made of it. `arrow` for a direct arrow.
    pub(super) fn hit_special(
        &mut self,
        id: u32,
        amount: f32,
        by_player: bool,
        arrow: bool,
    ) -> Hurt {
        if id >= CRYSTAL_IDS {
            // EndCrystal.hurtServer: false while invulnerable, else it blows up.
            let index = (id - CRYSTAL_IDS) as usize;
            if self.crystals.get(index).is_none_or(|c| c.invulnerable) {
                return Hurt::Refused;
            }
            let by = by_player.then(|| self.acting_player());
            self.destroy_crystal(index, by, CrystalDamage::Direct);
            Hurt::Landed { full: true }
        } else if id >= SPECIAL {
            let by = by_player.then(|| self.acting_player());
            self.hurt_dragon((id - SPECIAL) as usize, amount, by_player, by, arrow)
        } else {
            Hurt::Refused
        }
    }
    /// `EnderDragon.hurt(part, source, damage)` on the dragon in the level.
    /// `by` is the player behind the hit, remembered for the kill credit.
    fn hurt_dragon(
        &mut self,
        part: usize,
        damage: f32,
        counts: bool,
        by: Option<FightWho>,
        arrow: bool,
    ) -> Hurt {
        let Some(d) = self.dragon.as_mut() else {
            return Hurt::Refused;
        };
        let was_down = d.down();
        let (cooldown, last_hurt) = (d.invulnerable, d.last_hurt);
        let hurt = d.hurt(part, damage, counts, arrow);
        // LivingEntity.hurtServer: damage that is applied sets lastHurtByPlayer
        // (100 ticks); a hit the damage cooldown absorbs returns before that.
        if counts
            && matches!(hurt, Hurt::Landed { .. })
            && (d.invulnerable != cooldown || d.last_hurt != last_hurt)
            && let Some(who) = by
        {
            d.hurt_by_player = Some((who, d.ticks));
        }
        if !was_down && d.down() {
            self.credit_dragon_kill();
        }
        let Some(d) = self.dragon.as_mut() else {
            return hurt;
        };
        if hurt == (Hurt::Landed { full: true }) {
            // Mob.playHurtSound: the ambient sound clock starts again.
            d.ambient_time = -AMBIENT_INTERVAL;
            let at = [d.position.x, d.position.y, d.position.z];
            let pitch = 0.8 + self.world.random.next_float() * 0.3;
            self.audio
                .play("minecraft:entity.ender_dragon.hurt", Some(at), 5.0, pitch);
        }
        hurt
    }
    /// `LivingEntity.die`: `getKillCredit().awardKillScore`, for the player
    /// who hurt the dragon within the last 100 ticks. A guest's kill is
    /// theirs: their statistics, scores and advancements.
    fn credit_dragon_kill(&mut self) {
        let Some(d) = self.dragon.as_ref() else {
            return;
        };
        let Some((who, _)) = d.hurt_by_player.filter(|(_, hit)| d.ticks - hit <= 100) else {
            return;
        };
        let view = horizon_client::advancements::EntityView {
            kind: "minecraft:ender_dragon".to_owned(),
            position: [d.position.x, d.position.y, d.position.z],
            ..Default::default()
        };
        let id = d.id;
        let guest = match who {
            FightWho::Guest(conn) if self.lan_acting.is_none() => Some(conn),
            _ => None,
        };
        let credited = guest.is_some_and(|conn| {
            let Some(mut host) = self.lan.take() else {
                return false;
            };
            let done = self
                .as_guest(&mut host, conn, |g| g.award_dragon_kill(id, &view))
                .is_some();
            self.lan = Some(host);
            done
        });
        if !credited {
            self.award_dragon_kill(id, &view);
        }
    }
    /// `ServerPlayer.awardKillScore` for the dragon, on the killer's state.
    fn award_dragon_kill(&mut self, id: u32, view: &horizon_client::advancements::EntityView) {
        self.stats.add("killed", "minecraft:ender_dragon", 1);
        self.stats.custom("mob_kills", 1);
        self.score_player_killed(super::commands::Target::Mob(id));
        self.trigger_killed(view, None);
    }
    /// Player melee on the dragon's parts or a crystal under the crosshair.
    /// The dragon part under the crosshair within reach: its id and distance.
    pub(super) fn dragon_hit(&self) -> Option<(u32, f64)> {
        let (eye, dir) = self.aim()?;
        let reach: f64 = if self.creative { 5.0 } else { 3.0 };
        let mut best: Option<(u32, f64)> = None;
        for (id, b) in self.special_targets() {
            let (lo, hi) = (b.min_position(), b.max_position());
            let (lo, hi) = ([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]);
            let (mut t0, mut t1) = (0.0f64, reach);
            let mut hit = true;
            for a in 0..3 {
                if dir[a].abs() < 1e-9 {
                    hit &= eye[a] >= lo[a] && eye[a] <= hi[a];
                    continue;
                }
                let (mut ta, mut tb) = ((lo[a] - eye[a]) / dir[a], (hi[a] - eye[a]) / dir[a]);
                if ta > tb {
                    std::mem::swap(&mut ta, &mut tb);
                }
                t0 = t0.max(ta);
                t1 = t1.min(tb);
            }
            if hit && t0 <= t1 && best.is_none_or(|b| t0 < b.1) {
                best = Some((id, t0));
            }
        }
        best
    }
    pub(super) fn attack_dragon(&mut self) -> bool {
        let Some((id, d)) = self.dragon_hit() else {
            return false;
        };
        if !self.before_block(d) || self.mob_hit().is_some_and(|m| m.1 < d) {
            return false;
        }
        // A dragon part is no LivingEntity: `canCriticalAttack` is false.
        let (amount, _, _, _) = self.melee_damage("minecraft:ender_dragon", false);
        self.swing_arm();
        // The part's box.
        let area = self
            .special_targets()
            .into_iter()
            .find(|(part, _)| *part == id)
            .map(|(_, b)| b);
        let hurt = self.hit_special(id, amount, true, false);
        // The sweep and the sounds read the charge, which the hit then spends.
        if let (Some(area), Hurt::Landed { .. }) = (area, hurt) {
            self.attack_effects(area, None, super::mace::Spared::Nothing, None);
        }
        self.attack_ticks = 0;
        true
    }
    /// The 24 nodes (`findClosestNode` builds them on first use).
    fn dragon_nodes(&mut self) -> [[i32; 3]; 24] {
        if let Some(n) = self.dragon.as_ref().and_then(|d| d.nodes) {
            return n;
        }
        let mut nodes = [[0; 3]; 24];
        for (i, node) in nodes.iter_mut().enumerate() {
            let (r, step, base, add) = if i < 12 {
                (60.0f32, std::f32::consts::PI / 12.0, i, 5)
            } else if i < 20 {
                (40.0, std::f32::consts::PI / 8.0, i - 12, 15)
            } else {
                (20.0, std::f32::consts::PI / 4.0, i - 20, 5)
            };
            let a = 2.0 * (-std::f32::consts::PI + step * base as f32);
            let (x, z) = ((r * a.cos()).floor() as i32, (r * a.sin()).floor() as i32);
            let y = (self.motion_blocking_no_leaves(x, z) + add).max(73);
            *node = [x, y, z];
        }
        if let Some(d) = self.dragon.as_mut() {
            d.nodes = Some(nodes);
        }
        nodes
    }
    /// `getHeightmapPos(MOTION_BLOCKING_NO_LEAVES)`.
    fn motion_blocking_no_leaves(&self, x: i32, z: i32) -> i32 {
        (0..256)
            .rev()
            .find(|&y| {
                let f = self.res.facts().get(self.world.block(x, y, z));
                (f.full_collision
                    || f.legacy_solid
                    || f.fluid != horizon_client::blocks::Fluid::None)
                    && !f.name.ends_with("_leaves")
            })
            .map_or(0, |y| y + 1)
    }
    fn podium_top(&self) -> Vec3 {
        Vec3::new(0.5, f64::from(self.motion_blocking_no_leaves(0, 0)), 0.5)
    }
    fn closest_node(&mut self, x: f64, y: f64, z: f64) -> usize {
        let nodes = self.dragon_nodes();
        let start = if self.fight.alive_crystals == 0 {
            12
        } else {
            0
        };
        let p = [x.floor() as i32, y.floor() as i32, z.floor() as i32];
        let mut best = (10_000.0f32, 0);
        for (i, n) in nodes.iter().enumerate().skip(start) {
            let d = ((n[0] - p[0]).pow(2) + (n[1] - p[1]).pow(2) + (n[2] - p[2]).pow(2)) as f32;
            if d < best.0 {
                best = (d, i);
            }
        }
        best.1
    }
    /// `findPath` (A* over the adjacency masks), optionally ending at `last`.
    fn dragon_path(&mut self, from: usize, to: usize, last: Option<[i32; 3]>) -> Option<Path> {
        let nodes = self.dragon_nodes();
        let dist = |a: [i32; 3], b: [i32; 3]| {
            (((a[0] - b[0]).pow(2) + (a[1] - b[1]).pow(2) + (a[2] - b[2]).pow(2)) as f32).sqrt()
        };
        let min = if self.fight.alive_crystals == 0 {
            12
        } else {
            0
        };
        let mut g = [f32::MAX; 24];
        let mut came = [usize::MAX; 24];
        let mut closed = [false; 24];
        let mut open = vec![from];
        g[from] = 0.0;
        let mut closest = from;
        while !open.is_empty() {
            let (k, &cur) = open
                .iter()
                .enumerate()
                .min_by(|a, b| {
                    (g[*a.1] + dist(nodes[*a.1], nodes[to]))
                        .total_cmp(&(g[*b.1] + dist(nodes[*b.1], nodes[to])))
                })
                .expect("open set");
            open.swap_remove(k);
            if cur == to {
                closest = to;
                break;
            }
            if dist(nodes[cur], nodes[to]) < dist(nodes[closest], nodes[to]) {
                closest = cur;
            }
            closed[cur] = true;
            for i in min..24 {
                if ADJACENCY[cur] & (1 << i) != 0 && !closed[i] {
                    let tentative = g[cur] + dist(nodes[cur], nodes[i]);
                    if tentative < g[i] {
                        g[i] = tentative;
                        came[i] = cur;
                        if !open.contains(&i) {
                            open.push(i);
                        }
                    }
                }
            }
        }
        if closest == from && from != to {
            return None;
        }
        let mut list = vec![nodes[closest]];
        let mut n = closest;
        while came[n] != usize::MAX {
            n = came[n];
            list.insert(0, nodes[n]);
        }
        if let Some(l) = last {
            list.push(l);
        }
        Some(Path {
            nodes: list,
            next: 0,
        })
    }
    fn next_target(&mut self, path: &mut Option<PathBox>) -> Option<Vec3> {
        let p = path.as_mut()?;
        if p.0.done() {
            return None;
        }
        let c = p.0.next_pos();
        p.0.advance();
        let d = self.dragon.as_mut()?;
        let y = loop {
            let y = f64::from(c[1]) + f64::from(d.random.next_float() * 20.0);
            if y >= f64::from(c[1]) {
                break y;
            }
        };
        Some(Vec3::new(f64::from(c[0]), y, f64::from(c[2])))
    }
    /// The holding-pattern style path step: around the outer or inner ring.
    fn ring_path(&mut self, clockwise: &mut bool) -> Option<Path> {
        let d = self.dragon.as_mut()?;
        let flip = d.random.next_int_bound(8).unwrap_or(1) == 0;
        let current = {
            let p = d.position;
            self.closest_node(p.x, p.y, p.z)
        };
        let mut target = current as i32;
        if flip {
            *clockwise = !*clockwise;
            target += 6;
        }
        target += if *clockwise { 1 } else { -1 };
        if self.fight.alive_crystals > 0 {
            target = target.rem_euclid(12);
        } else {
            target = ((target - 12) & 7) + 12;
        }
        let mut path = self.dragon_path(current, target as usize, None)?;
        path.advance();
        Some(path)
    }
    /// `DragonStrafePlayerPhase.setTarget`: a path to the player at `p`.
    fn strafe_begin(&mut self, p: Vec3) {
        let Some(d) = self.dragon.as_ref() else {
            return;
        };
        let dp = d.position;
        let current = self.closest_node(dp.x, dp.y, dp.z);
        let target = self.closest_node(p.x, p.y, p.z);
        let (fx, fz) = (p.x.floor() as i32, p.z.floor() as i32);
        let sd = (f64::from(fx) - dp.x).hypot(f64::from(fz) - dp.z);
        let ho = (0.4 + sd / 80.0 - 1.0).min(10.0);
        let last = [fx, (p.y + ho).floor() as i32, fz];
        let mut path = self.dragon_path(current, target, Some(last));
        if let Some(path) = path.as_mut() {
            path.advance();
        }
        let mut boxed = path.map(PathBox);
        let target = self.next_target(&mut boxed);
        if let Some(Dragon {
            phase: Phase::Strafe {
                path, target: t, ..
            },
            ..
        }) = self.dragon.as_mut()
        {
            *path = boxed;
            *t = target;
        }
    }
    /// One phase server tick (`doServerTick`).
    fn tick_phase(&mut self) {
        let players = self.fight_players();
        let peaceful = self.ui.difficulty == 0;
        // A crystal destroyed since: strafe whoever it names, if the dragon
        // can attack them (`canAttack`).
        if let Some(d) = self.dragon.as_mut()
            && let Some(request) = d.strafe_request.take()
            && matches!(d.phase, Phase::Holding { .. })
        {
            let target = match request {
                StrafeRequest::Player(who) => players
                    .iter()
                    .find(|p| p.who == who && p.enemy() && !peaceful)
                    .copied(),
                StrafeRequest::Nearest(at) => nearest_enemy(&players, peaceful, at, None, |_| true),
            };
            if let Some(t) = target {
                d.set_phase(Phase::Strafe {
                    charge: 0,
                    path: None,
                    target: None,
                    clockwise: false,
                    player: Some(t.who),
                });
                self.strafe_begin(t.position);
            }
        }
        let Some(d) = self.dragon.as_ref() else {
            return;
        };
        let pos = d.position;
        let mut phase = d.phase.clone();
        let collided = false;
        let far = |t: Option<Vec3>| {
            t.map_or(0.0, |t| {
                (t.x - pos.x).powi(2) + (t.y - pos.y).powi(2) + (t.z - pos.z).powi(2)
            })
        };
        let podium = self.podium_top();
        // The egg's place (`getHeightmapPos` at the podium) and its centre.
        let egg = Vec3::new(0.0, podium.y, 0.0);
        let egg_centre = Vec3::new(0.5, podium.y + 0.5, 0.5);
        let mut next: Option<Phase> = None;
        // Where the player a new strafe goes after stands.
        let mut strafe_at: Option<Vec3> = None;
        match &mut phase {
            Phase::Holding {
                path,
                target,
                clockwise,
            } => {
                let dt = far(*target);
                if dt < 100.0 || dt > 22500.0 || collided {
                    let done = path.as_ref().is_some_and(|p| p.0.done());
                    let mut chosen = false;
                    if done {
                        let crystals = self.fight.alive_crystals;
                        let r = &mut self.dragon.as_mut().expect("dragon").random;
                        if r.next_int_bound(crystals + 3).unwrap_or(1) == 0 {
                            next = Some(Phase::LandingApproach {
                                path: None,
                                target: None,
                            });
                            chosen = true;
                        } else if let Some(nearest) =
                            nearest_enemy(&players, peaceful, egg, None, |_| true)
                        {
                            let dist = nearest.position.distance_squared(egg_centre) / 512.0;
                            let r = &mut self.dragon.as_mut().expect("dragon").random;
                            if r.next_int_bound((dist + 2.0) as i32).unwrap_or(1) == 0
                                || r.next_int_bound(crystals + 2).unwrap_or(1) == 0
                            {
                                next = Some(Phase::Strafe {
                                    charge: 0,
                                    path: None,
                                    target: None,
                                    clockwise: false,
                                    player: Some(nearest.who),
                                });
                                strafe_at = Some(nearest.position);
                                chosen = true;
                            }
                        }
                    }
                    if !chosen {
                        if path.as_ref().is_none_or(|p| p.0.done()) {
                            *path = self.ring_path(clockwise).map(PathBox);
                        }
                        if let Some(t) = self.next_target(path) {
                            *target = Some(t);
                        }
                    }
                }
            }
            Phase::Strafe {
                charge,
                path,
                target,
                clockwise,
                player,
            } => {
                // Its target as it is now, whatever became of it; gone from
                // the level, or never found: back to the holding pattern.
                let tracked = player.and_then(|who| players.iter().find(|p| p.who == who));
                let Some(pp) = tracked.map(|p| p.position) else {
                    next = Some(Phase::Holding {
                        path: None,
                        target: None,
                        clockwise: false,
                    });
                    self.dragon.as_mut().expect("dragon").phase = phase;
                    if let Some(n) = next {
                        self.dragon.as_mut().expect("dragon").set_phase(n);
                    }
                    return;
                };
                if path.as_ref().is_some_and(|p| p.0.done()) {
                    let dist = (pp.x - pos.x).hypot(pp.z - pos.z);
                    let ho = (0.4 + dist / 80.0 - 1.0).min(10.0);
                    *target = Some(Vec3::new(pp.x, pp.y + ho, pp.z));
                }
                let dt = far(*target);
                if dt < 100.0 || dt > 22500.0 {
                    if path.as_ref().is_none_or(|p| p.0.done()) {
                        *path = self.ring_path(clockwise).map(PathBox);
                    }
                    if let Some(t) = self.next_target(path) {
                        *target = Some(t);
                    }
                }
                let d2 = (pp.x - pos.x).powi(2) + (pp.y - pos.y).powi(2) + (pp.z - pos.z).powi(2);
                // Entity.hasLineOfSight: eye to eye (the dragon's eye is 0.85 of
                // its 8 block height), blocks only.
                let sees = d2 < 4096.0 && {
                    let eye = Vec3::new(pos.x, pos.y + 6.8, pos.z);
                    let target_eye =
                        tracked.map_or(pp.y + 1.62, |p| p.bounds.max_position().y - 0.18);
                    self.mob_world_base(&[])
                        .line_of_sight(eye, Vec3::new(pp.x, target_eye, pp.z))
                };
                if sees {
                    *charge += 1;
                    let d = self.dragon.as_ref().expect("dragon");
                    let r = d.yaw.to_radians();
                    let dir = glam::DVec2::new(f64::from(r.sin()), f64::from(-r.cos())).normalize();
                    let aim = glam::DVec2::new(pp.x - pos.x, pp.z - pos.z).normalize_or_zero();
                    let angle = dir.dot(aim).clamp(-1.0, 1.0).acos().to_degrees() + 0.5;
                    if *charge >= 5 && (0.0..10.0).contains(&angle) {
                        // From the head, one block back along the view vector.
                        let head = d.parts[0].0;
                        let view = super::view_vector(d.yaw, 0.0);
                        let start =
                            Vec3::new(head.x - view[0], head.y + 0.5 + 0.5, head.z - view[2]);
                        let to = Vec3::new(pp.x - start.x, pp.y + 0.9 - start.y, pp.z - start.z);
                        self.spawn_dragon_fireball(start, to);
                        *charge = 0;
                        next = Some(Phase::Holding {
                            path: None,
                            target: None,
                            clockwise: false,
                        });
                    }
                } else if *charge > 0 {
                    *charge -= 1;
                }
            }
            Phase::LandingApproach { path, target } => {
                let dt = far(*target);
                if dt < 100.0 || dt > 22500.0 || collided {
                    if path.as_ref().is_none_or(|p| p.0.done()) {
                        let current = self.closest_node(pos.x, pos.y, pos.z);
                        // NEAR_EGG_TARGETING: the enemy nearest the egg.
                        let near_egg = nearest_enemy(&players, peaceful, egg, None, |_| true);
                        let target_node = match near_egg.map(|p| p.position) {
                            Some(pp) => {
                                let aim = glam::DVec2::new(pp.x, pp.z).normalize_or_zero();
                                self.closest_node(-aim.x * 40.0, 105.0, -aim.y * 40.0)
                            }
                            None => self.closest_node(40.0, podium.y, 0.0),
                        };
                        let last = [
                            podium.x.floor() as i32,
                            podium.y as i32,
                            podium.z.floor() as i32,
                        ];
                        let mut p = self.dragon_path(current, target_node, Some(last));
                        if let Some(p) = p.as_mut() {
                            p.advance();
                        }
                        *path = p.map(PathBox);
                    }
                    if let Some(t) = self.next_target(path) {
                        *target = Some(t);
                    }
                    if path.as_ref().is_some_and(|p| p.0.done()) {
                        next = Some(Phase::Landing { target: None });
                    }
                }
            }
            Phase::Landing { target } => {
                let t = *target.get_or_insert(podium);
                if (t.x - pos.x).powi(2) + (t.y - pos.y).powi(2) + (t.z - pos.z).powi(2) < 1.0 {
                    self.dragon.as_mut().expect("dragon").flame_count = 0;
                    next = Some(Phase::Scanning { time: 0 });
                }
            }
            Phase::Takeoff {
                first,
                path,
                target,
            } => {
                if !*first && path.is_some() {
                    if (podium.x - pos.x).powi(2)
                        + (podium.y - pos.y).powi(2)
                        + (podium.z - pos.z).powi(2)
                        >= 100.0
                    {
                        next = Some(Phase::Holding {
                            path: None,
                            target: None,
                            clockwise: false,
                        });
                    }
                } else {
                    *first = false;
                    let current = self.closest_node(pos.x, pos.y, pos.z);
                    let look = self.dragon_look();
                    let mut t = self.closest_node(-look[0] * 40.0, 105.0, -look[2] * 40.0) as i32;
                    if self.fight.alive_crystals > 0 {
                        t = t.rem_euclid(12);
                    } else {
                        t = ((t - 12) & 7) + 12;
                    }
                    let mut p = self.dragon_path(current, t as usize, None).map(PathBox);
                    if let Some(pb) = p.as_mut() {
                        pb.0.advance();
                    }
                    if let Some(t) = self.next_target(&mut p) {
                        *target = Some(t);
                    }
                    *path = p.or(Some(PathBox(Path::default())));
                }
            }
            Phase::Scanning { time } => {
                *time += 1;
                // scanTargeting: within 20 blocks, at most 10 above or below.
                let near = nearest_enemy(&players, peaceful, pos, Some((pos, 20.0)), |p| {
                    (p.position.y - pos.y).abs() <= 10.0
                })
                .map(|p| p.position);
                if let Some(pp) = near {
                    if *time > 25 {
                        next = Some(Phase::Attacking { ticks: 0 });
                    } else {
                        let d = self.dragon.as_mut().expect("dragon");
                        let head = d.parts[0].0;
                        let (xd, zd) = (pp.x - head.x, pp.z - head.z);
                        let delta = wrap64(180.0 - xd.atan2(zd).to_degrees() - f64::from(d.yaw))
                            .clamp(-100.0, 100.0);
                        d.yaw_accel *= 0.8;
                        let dist = (xd.hypot(zd) as f32 + 1.0).min(40.0);
                        let rot = xd.hypot(zd) as f32 + 1.0;
                        d.yaw_accel += delta as f32 * (0.7 / dist / rot);
                        d.yaw += d.yaw_accel;
                    }
                } else if *time >= 100 {
                    // CHARGE_TARGETING: within 150 blocks.
                    let charge =
                        nearest_enemy(&players, peaceful, pos, Some((pos, 150.0)), |_| true);
                    next = Some(match charge {
                        Some(p) => Phase::Charging {
                            target: Some(p.position),
                            since: 0,
                        },
                        None => Phase::Takeoff {
                            first: true,
                            path: None,
                            target: None,
                        },
                    });
                }
            }
            Phase::Attacking { ticks } => {
                if *ticks == 0 {
                    let pitch = 0.8 + self.world.random.next_float() * 0.3;
                    self.audio.play(
                        "minecraft:entity.ender_dragon.growl",
                        Some([pos.x, pos.y, pos.z]),
                        2.5,
                        pitch,
                    );
                }
                *ticks += 1;
                if *ticks > 40 {
                    next = Some(Phase::Flaming { ticks: 0 });
                }
            }
            Phase::Flaming { ticks } => {
                *ticks += 1;
                if *ticks >= 200 {
                    let count = self.dragon.as_ref().map_or(0, |d| d.flame_count);
                    next = Some(if count >= 4 {
                        Phase::Takeoff {
                            first: true,
                            path: None,
                            target: None,
                        }
                    } else {
                        Phase::Scanning { time: 0 }
                    });
                } else if *ticks == 10 {
                    let d = self.dragon.as_ref().expect("dragon");
                    let head = d.parts[0].0;
                    let look = glam::DVec2::new(head.x - pos.x, head.z - pos.z).normalize_or_zero();
                    let (x, z) = (head.x + look.x * 2.5, head.z + look.y * 2.5);
                    let mut y = head.y + 0.5;
                    while y > 0.0
                        && self.res.facts().is_air(self.world.block(
                            x.floor() as i32,
                            y.floor() as i32,
                            z.floor() as i32,
                        ))
                    {
                        y -= 1.0;
                    }
                    self.breath_clouds.push(Breath {
                        id: 0,
                        at: [x, y.floor() + 1.0, z],
                        radius: 5.0,
                        age: 0,
                        duration: 200,
                        from_dragon: true,
                        potion: None,
                    });
                }
            }
            Phase::Charging { target, since } => {
                if target.is_none() {
                    next = Some(Phase::Holding {
                        path: None,
                        target: None,
                        clockwise: false,
                    });
                } else if *since > 0 {
                    *since += 1;
                    if *since >= 10 {
                        next = Some(Phase::Holding {
                            path: None,
                            target: None,
                            clockwise: false,
                        });
                    }
                } else {
                    let dt = far(*target);
                    if dt < 100.0 || dt > 22500.0 {
                        *since += 1;
                    }
                }
            }
            Phase::Dying { target, time } => {
                *time += 1;
                let t = *target.get_or_insert(Vec3::new(
                    0.5,
                    f64::from(self.motion_blocking_no_leaves(0, 0)),
                    0.5,
                ));
                let dt = (t.x - pos.x).powi(2) + (t.y - pos.y).powi(2) + (t.z - pos.z).powi(2);
                let d = self.dragon.as_mut().expect("dragon");
                d.health = if !(100.0..=22500.0).contains(&dt) {
                    0.0
                } else {
                    1.0
                };
            }
            Phase::Hover { target } => {
                target.get_or_insert(pos);
            }
        }
        let d = self.dragon.as_mut().expect("dragon");
        d.phase = phase;
        if let Some(n) = next {
            d.set_phase(n);
            if let Some(at) = strafe_at {
                self.strafe_begin(at);
            }
        }
    }
    /// `getHeadLookVector`.
    fn dragon_look(&self) -> [f64; 3] {
        let Some(d) = self.dragon.as_ref() else {
            return [0.0; 3];
        };
        let pitch = match d.phase {
            Phase::Landing { .. } | Phase::Takeoff { .. } => {
                let p = self.podium_top();
                let dist = ((p.x - d.position.x).powi(2)
                    + (p.y - d.position.y).powi(2)
                    + (p.z - d.position.z).powi(2))
                .sqrt() as f32
                    / 4.0;
                -6.0 / dist.max(1.0) * 1.5 * 5.0
            }
            _ if d.phase.sitting() => -45.0,
            _ => 0.0,
        };
        super::view_vector(d.yaw, pitch).map(|v| v)
    }
    /// `EnderDragon.aiStep` (and `tickDeath`) for the dragon.
    fn tick_dragon(&mut self) {
        let Some(d) = self.dragon.as_mut() else {
            return;
        };
        d.previous = d.position;
        d.previous_yaw = d.yaw;
        d.old_flap = d.flap;
        d.ticks += 1;
        d.hurt_time = (d.hurt_time - 1).max(0);
        d.invulnerable = (d.invulnerable - 1).max(0);
        for part in &mut d.parts {
            part.1 = part.0;
        }
        if !d.alive() {
            self.tick_dragon_death();
            return;
        }
        // Mob.baseTick: the ambient sound, rarer right after one.
        let roll = d.random.next_int_bound(1000).unwrap_or(999);
        let waited = d.ambient_time;
        d.ambient_time += 1;
        if roll < waited {
            d.ambient_time = -AMBIENT_INTERVAL;
            let at = [d.position.x, d.position.y, d.position.z];
            // LivingEntity.getVoicePitch, from the dragon's own random.
            let r = &mut d.random;
            let pitch = (r.next_float() - r.next_float()) * 0.2 + 1.0;
            self.audio.play(
                "minecraft:entity.ender_dragon.ambient",
                Some(at),
                5.0,
                pitch,
            );
        }
        let d = self.dragon.as_mut().expect("dragon");
        // Flapping, growls and the flap sound.
        let flap_was = (d.old_flap * std::f32::consts::TAU).cos();
        let v = d.velocity;
        let mut flap_speed = 0.2 / ((v.x.hypot(v.z) as f32) * 10.0 + 1.0);
        flap_speed *= 2f32.powf(v.y as f32);
        d.flap += if d.phase.sitting() {
            0.1
        } else if d.in_wall {
            flap_speed * 0.5
        } else {
            flap_speed
        };
        let flap_now = (d.flap * std::f32::consts::TAU).cos();
        let at = [d.position.x, d.position.y, d.position.z];
        if flap_was <= -0.3 && flap_now >= -0.3 {
            let pitch = 0.8 + self.world.random.next_float() * 0.3;
            self.audio
                .play("minecraft:entity.ender_dragon.flap", Some(at), 5.0, pitch);
        }
        let d = self.dragon.as_mut().expect("dragon");
        if !d.phase.sitting() {
            d.growl -= 1;
            if d.growl < 0 {
                d.growl = 200 + d.random.next_int_bound(200).unwrap_or(0);
                let pitch = 0.8 + self.world.random.next_float() * 0.3;
                self.audio
                    .play("minecraft:entity.ender_dragon.growl", Some(at), 2.5, pitch);
            }
        }
        // checkCrystals: heal 1 every 10 ticks from the nearest crystal, found
        // among those touching the dragon's box grown by 32.
        let d = self.dragon.as_mut().expect("dragon");
        if d.nearest_crystal.is_some() && d.ticks % 10 == 0 && d.health < MAX_HEALTH {
            // LivingEntity.setHealth clamps to the maximum.
            d.health = (d.health + 1.0).min(MAX_HEALTH);
        }
        if d.random.next_int_bound(10).unwrap_or(1) == 0 {
            let p = d.position;
            let nearest = self.nearest_crystal_to(p);
            self.dragon.as_mut().expect("dragon").nearest_crystal = nearest;
        }
        let d = self.dragon.as_mut().expect("dragon");
        d.yaw = wrap(d.yaw);
        d.record();
        self.tick_phase();
        // Steering towards the phase's fly target.
        let Some(d) = self.dragon.as_mut() else {
            return;
        };
        if let Some(t) = d.phase.target() {
            let (xd, mut yd, zd) = (t.x - d.position.x, t.y - d.position.y, t.z - d.position.z);
            let dist2 = xd * xd + yd * yd + zd * zd;
            let max = f64::from(d.phase.fly_speed());
            let horizontal = xd.hypot(zd);
            if horizontal > 0.0 {
                yd = (yd / horizontal).clamp(-max, max);
            }
            d.velocity.y += yd * 0.01;
            d.yaw = wrap(d.yaw);
            let aim = glam::DVec3::new(xd, t.y - d.position.y, zd).normalize_or_zero();
            let r = d.yaw.to_radians();
            let dir = glam::DVec3::new(f64::from(r.sin()), d.velocity.y, f64::from(-r.cos()))
                .normalize_or_zero();
            let dot = ((dir.dot(aim) as f32 + 0.5) / 1.5).max(0.0);
            if xd.abs() > 1.0e-5 || zd.abs() > 1.0e-5 {
                let turn = match d.phase {
                    Phase::Landing { .. } => {
                        let rot = d.velocity.x.hypot(d.velocity.z) as f32 + 1.0;
                        rot.min(40.0) / rot
                    }
                    _ => {
                        let rot = d.velocity.x.hypot(d.velocity.z) as f32 + 1.0;
                        0.7 / rot.min(40.0) / rot
                    }
                };
                let delta =
                    wrap(180.0 - xd.atan2(zd).to_degrees() as f32 - d.yaw).clamp(-50.0, 50.0);
                d.yaw_accel *= 0.8;
                d.yaw_accel += delta * turn;
                d.yaw += d.yaw_accel * 0.1;
            }
            let span = (2.0 / (dist2 + 1.0)) as f32;
            let speed = 0.06 * (dot * span + (1.0 - span));
            // moveRelative(speed, (0, 0, -1)).
            let r = d.yaw.to_radians();
            d.velocity.x += f64::from(speed * r.sin());
            d.velocity.z += f64::from(-speed * r.cos());
            let step = if d.in_wall { 0.8 } else { 1.0 };
            d.position = Vec3::new(
                d.position.x + d.velocity.x * step,
                d.position.y + d.velocity.y * step,
                d.position.z + d.velocity.z * step,
            );
            let actual =
                glam::DVec3::new(d.velocity.x, d.velocity.y, d.velocity.z).normalize_or_zero();
            let slide = 0.8 + 0.15 * (actual.dot(dir) + 1.0) / 2.0;
            d.velocity = Vec3::new(
                d.velocity.x * slide,
                d.velocity.y * 0.91,
                d.velocity.z * slide,
            );
        }
        self.place_dragon_parts();
        self.dragon_contacts();
        // The chunks the dragon passes through re-save their entity lists.
        if let Some(d) = &self.dragon {
            for p in [d.previous, d.position] {
                self.world
                    .modified
                    .insert((p.x.floor() as i32 >> 4, p.z.floor() as i32 >> 4));
            }
        }
    }
    /// The part positions from the yaw and the flight history.
    fn place_dragon_parts(&mut self) {
        let Some(d) = self.dragon.as_mut() else {
            return;
        };
        let tilt =
            ((d.sample(5).0 - d.sample(10).0) * 10.0) as f32 * (std::f32::consts::PI / 180.0);
        let (ct, st) = (tilt.cos(), tilt.sin());
        let r1 = d.yaw.to_radians();
        let (s1, c1) = (r1.sin(), r1.cos());
        let p = d.position;
        let set = |d: &mut Dragon, i: usize, x: f32, y: f64, z: f32| {
            d.parts[i].0 = Vec3::new(p.x + f64::from(x), p.y + y, p.z + f64::from(z));
        };
        set(d, 2, s1 * 0.5, 0.0, -c1 * 0.5);
        set(d, 6, c1 * 4.5, 2.0, s1 * 4.5);
        set(d, 7, c1 * -4.5, 2.0, s1 * -4.5);
        let r2 = d.yaw.to_radians() - d.yaw_accel * 0.01;
        let (s2, c2) = (r2.sin(), r2.cos());
        let y_off = if d.phase.sitting() {
            -1.0
        } else {
            d.sample(5).0 - d.sample(0).0
        } as f32;
        set(
            d,
            0,
            s2 * 6.5 * ct,
            f64::from(y_off + st * 6.5),
            -c2 * 6.5 * ct,
        );
        set(
            d,
            1,
            s2 * 5.5 * ct,
            f64::from(y_off + st * 5.5),
            -c2 * 5.5 * ct,
        );
        let p1 = d.sample(5);
        for i in 0..3 {
            let p0 = d.sample(12 + i * 2);
            let rot = d.yaw.to_radians() + wrap(p0.1 - p1.1).to_radians();
            let (ss, cc) = (rot.sin(), rot.cos());
            let dd = (i as f32 + 1.0) * 2.0;
            let x = -(s1 * 1.5 + ss * dd) * ct;
            let y = p0.0 - p1.0 - f64::from((dd + 1.5) * st) + 1.5;
            let z = (c1 * 1.5 + cc * dd) * ct;
            set(d, 3 + i, x, y, z);
        }
    }
    /// Wing knockback, head and neck damage, and breaking blocks in the way.
    fn dragon_contacts(&mut self) {
        let Some(d) = self.dragon.as_ref() else {
            return;
        };
        let sitting = d.phase.sitting();
        let recently_hurt = d.hurt_time > 0;
        let body = d.part_box(2);
        let (bc, bz) = (
            (body.min_position().x + body.max_position().x) / 2.0,
            (body.min_position().z + body.max_position().z) / 2.0,
        );
        let wings = [d.part_box(6), d.part_box(7)];
        let heads = [d.part_box(0), d.part_box(1)];
        if !recently_hurt {
            // EntitySelector.NO_CREATIVE_OR_SPECTATOR, every player alike.
            let players: Vec<FightPlayer> = self
                .fight_players()
                .into_iter()
                .filter(|p| p.alive && !p.creative && !p.spectator)
                .collect();
            for p in players {
                // knockBack, once for each wing whose zone holds them.
                for w in wings {
                    let zone = w
                        .inflate(Vec3::new(4.0, 2.0, 4.0))
                        .move_by(Vec3::new(0.0, -2.0, 0.0));
                    if zone.intersects(p.bounds) {
                        let (xd, zd) = (p.position.x - bc, p.position.z - bz);
                        let dd = (xd * xd + zd * zd).max(0.1);
                        let push = Vec3::new(xd / dd * 4.0, f64::from(0.2_f32), zd / dd * 4.0);
                        self.as_fight_player(p.who, |g| g.dragon_wing_knock_back(push, sitting));
                    }
                }
                // hurt: the head and the neck.
                for h in heads {
                    if h.inflate(Vec3::new(1.0, 1.0, 1.0)).intersects(p.bounds) {
                        self.as_fight_player(p.who, |g| {
                            g.dragon_hurts_player("minecraft:mob_attack", 10.0);
                        });
                    }
                }
            }
        }
        // checkWalls over head, neck and body.
        let griefing = self.ui.rules.bool("mob_griefing");
        let mut in_wall = false;
        let Some(d) = self.dragon.as_ref() else {
            return;
        };
        let boxes = [d.part_box(0), d.part_box(1), d.part_box(2)];
        let tags = &self.items().block_tags;
        let transparent = tags
            .get("minecraft:dragon_transparent")
            .cloned()
            .unwrap_or_default();
        let immune = tags
            .get("minecraft:dragon_immune")
            .cloned()
            .unwrap_or_default();
        let mut removed = Vec::new();
        for b in boxes {
            let (lo, hi) = (b.min_position(), b.max_position());
            for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
                for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
                    for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                        let state = self.world.block(x, y, z);
                        let name = &self.res.facts().get(state).name;
                        if self.res.facts().is_air(state) || transparent.contains(&**name) {
                            continue;
                        }
                        if griefing && !immune.contains(&**name) {
                            removed.push([x, y, z]);
                        } else {
                            in_wall = true;
                        }
                    }
                }
            }
        }
        let air = self.world.air;
        let broke = !removed.is_empty();
        for p in removed {
            let _ = self.level_set(p, air);
        }
        if broke && let Some(d) = self.dragon.as_ref() {
            let p = d.parts[0].0;
            // Level event 2008: an explosion puff where a block broke.
            self.particles.add(
                &ParticleOptions::simple(ParticleType::Explosion),
                [p.x, p.y, p.z],
                [0.0; 3],
            );
        }
        if let Some(d) = self.dragon.as_mut() {
            d.in_wall = in_wall;
        }
    }
    /// `tickDeath`: rising for 200 ticks, explosions, experience, then the
    /// exit portal, the egg and a gateway.
    fn tick_dragon_death(&mut self) {
        let Some(d) = self.dragon.as_mut() else {
            return;
        };
        d.death_time += 1;
        let t = d.death_time;
        let p = d.position;
        let r = &mut d.random;
        let mut off = || {
            [
                f64::from((r.next_float() - 0.5) * 8.0),
                f64::from((r.next_float() - 0.5) * 4.0),
                f64::from((r.next_float() - 0.5) * 8.0),
            ]
        };
        if (180..=200).contains(&t) {
            let o = off();
            self.particles.add(
                &ParticleOptions::simple(ParticleType::ExplosionEmitter),
                [p.x + o[0], p.y + 2.0 + o[1], p.z + o[2]],
                [0.0; 3],
            );
        }
        let first_kill = !self.fight.previously_killed;
        let xp = if first_kill { 12000 } else { 500 };
        let drops = self.ui.rules.bool("mob_drops");
        if t > 150 && t % 5 == 0 && drops {
            self.spawn_orbs([p.x, p.y, p.z], (xp as f32 * 0.08) as u32);
        }
        if t == 1 {
            // EnderDragon.tickDeath: global level event 1028, the death
            // roar, heard everywhere.
            let at = [p.x, p.y, p.z].map(|v| v.floor() as i32);
            self.global_level_event(1028, at, 0);
        }
        if let Some(d) = self.dragon.as_mut() {
            d.position.y += 0.1;
            for part in &mut d.parts {
                part.0.y += 0.1;
            }
        }
        if t >= 200 {
            if drops {
                self.spawn_orbs([p.x, p.y, p.z], (xp as f32 * 0.2) as u32);
            }
            self.dragon = None;
            self.dragon_killed();
        }
    }
    /// `EnderDragonFight.setDragonKilled`.
    fn dragon_killed(&mut self) {
        let _ = self.activate_exit_portal();
        if let Some(g) = self.fight.gateways.pop() {
            let a = 2.0 * (-std::f64::consts::PI + std::f64::consts::PI / 20.0 * f64::from(g));
            let pos = [
                (96.0 * a.cos()).floor() as i32,
                75,
                (96.0 * a.sin()).floor() as i32,
            ];
            self.level_event_at(3000, pos, 0);
            let _ = self.place_gateway_at(pos);
        }
        if !self.fight.previously_killed
            && let Some(egg) = self
                .pack
                .assets
                .registry
                .default_state("minecraft:dragon_egg")
        {
            let y = (0..256)
                .rev()
                .find(|&y| {
                    let f = self.res.facts().get(self.world.block(0, y, 0));
                    f.full_collision || f.legacy_solid
                })
                .map_or(64, |y| y + 1);
            let _ = self.level_set([0, y, 0], egg);
        }
        self.fight.previously_killed = true;
        self.fight.killed = true;
        self.fight.dirty = true;
        self.save_fight();
    }
    fn spawn_dragon_fireball(&mut self, at: Vec3, dir: Vec3) {
        let l = (dir.x * dir.x + dir.y * dir.y + dir.z * dir.z)
            .sqrt()
            .max(1.0e-6);
        let v = Vec3::new(dir.x / l, dir.y / l, dir.z / l);
        self.audio.play(
            "minecraft:entity.ender_dragon.shoot",
            Some([at.x, at.y, at.z]),
            10.0,
            1.0,
        );
        // assignDirectionalMovement: the direction times the acceleration power 0.1.
        let velocity = Vec3::new(v.x * 0.1, v.y * 0.1, v.z * 0.1);
        self.dragon_fireballs.push(DragonFireball {
            id: 0,
            position: at,
            previous: at,
            velocity,
            age: 0,
        });
    }
    /// `DragonFireball` (an AbstractHurtingProjectile): accelerating by its
    /// power, bursting into dragon's breath on impact.
    pub(super) fn tick_dragon_fireballs(&mut self) {
        let mut balls = std::mem::take(&mut self.dragon_fireballs);
        // Projectile.canHitEntity: any player but spectators.
        let players: Vec<Aabb> = self
            .fight_players()
            .iter()
            .filter(|p| p.alive && !p.spectator)
            .map(|p| p.bounds)
            .collect();
        balls.retain_mut(|b| {
            b.previous = b.position;
            b.age += 1;
            let v = b.velocity;
            let next = Vec3::new(b.position.x + v.x, b.position.y + v.y, b.position.z + v.z);
            // AbstractHurtingProjectile: accelerate along the motion, inertia 0.95.
            let l = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt().max(1.0e-9);
            b.velocity = Vec3::new(
                (v.x + v.x / l * 0.1) * 0.95,
                (v.y + v.y / l * 0.1) * 0.95,
                (v.z + v.z / l * 0.1) * 0.95,
            );
            let hit_block = {
                let p = [
                    next.x.floor() as i32,
                    next.y.floor() as i32,
                    next.z.floor() as i32,
                ];
                let f = self.res.facts().get(self.world.block(p[0], p[1], p[2]));
                f.full_collision || f.legacy_solid
            };
            let hit_player = players.iter().any(|pb| {
                pb.intersects(Aabb::new(
                    Vec3::new(next.x - 0.5, next.y - 0.5, next.z - 0.5),
                    Vec3::new(next.x + 0.5, next.y + 0.5, next.z + 0.5),
                ))
            });
            if hit_block || hit_player || b.age > 600 {
                self.breath_clouds.push(Breath {
                    id: 0,
                    at: [next.x, next.y, next.z],
                    radius: 3.0,
                    age: 0,
                    duration: 600,
                    from_dragon: false,
                    potion: None,
                });
                // DragonFireball.onHit: level event 2006 (the burst of dragon
                // breath and the explosion sound) at its block position.
                self.level_event_at(
                    2006,
                    [
                        next.x.floor() as i32,
                        next.y.floor() as i32,
                        next.z.floor() as i32,
                    ],
                    1,
                );
                return false;
            }
            b.position = next;
            self.particles.add(
                &ParticleOptions::simple(ParticleType::DragonBreath),
                [next.x, next.y + 0.5, next.z],
                [0.0; 3],
            );
            true
        });
        self.dragon_fireballs.append(&mut balls);
        self.tick_breath();
    }
    /// The dragon's breath clouds (`AreaEffectCloud` with instant damage at
    /// a quarter duration scale): shrinking, puffing particles, harming the player.
    fn tick_breath(&mut self) {
        // What instant damage reaches: neither creative nor spectator players.
        let players: Vec<FightPlayer> = self
            .fight_players()
            .into_iter()
            .filter(|p| p.alive && !p.creative && !p.spectator)
            .collect();
        let mut harmed: Vec<(FightWho, f32)> = Vec::new();
        // DragonSittingFlamingPhase.end discards the flame cloud.
        let flame_over = self
            .dragon
            .as_ref()
            .is_some_and(|d| !matches!(d.phase, Phase::Flaming { .. }));
        let mut clouds = std::mem::take(&mut self.breath_clouds);
        clouds.retain_mut(|c| {
            // Lingering potions' clouds tick with the potions.
            if c.potion.is_some() {
                return true;
            }
            if c.from_dragon && flame_over {
                return false;
            }
            c.age += 1;
            if c.age >= c.duration {
                return false;
            }
            if !c.from_dragon {
                // DragonFireball's cloud: radius per tick (7 - 3) / duration.
                c.radius += (7.0 - 3.0) / c.duration as f32;
            }
            let r = c.radius;
            for _ in 0..((std::f32::consts::PI * r * r) as i32).min(40) {
                let a = self.world.random.next_float() * std::f32::consts::TAU;
                let d = self.world.random.next_float().sqrt() * r;
                let at = [
                    c.at[0] + f64::from(a.cos() * d),
                    c.at[1],
                    c.at[2] + f64::from(a.sin() * d),
                ];
                self.particles.add_with(
                    &ParticleOptions::simple(ParticleType::DragonBreath),
                    at,
                    [0.0, 0.0, 0.0],
                    false,
                    true,
                );
            }
            // AreaEffectCloud: after its 20-tick wait, every 20 ticks per entity.
            if c.age >= 20 && c.age % 20 == 0 {
                // Instant damage: I (6) from the breath, II (12) from a fireball.
                let amount = if c.from_dragon { 6.0 } else { 12.0 };
                for p in &players {
                    let pp = p.position;
                    if (pp.x - c.at[0]).hypot(pp.z - c.at[2]) <= f64::from(r)
                        && (pp.y - c.at[1]).abs() < 1.0
                    {
                        harmed.push((p.who, amount));
                    }
                }
            }
            true
        });
        self.breath_clouds.append(&mut clouds);
        for (who, amount) in harmed {
            self.as_fight_player(who, |g| {
                g.dragon_hurts_player("minecraft:indirect_magic", amount);
            });
        }
    }
    /// `EnderDragon.knockBack` on the player in a wing's zone: pushed away
    /// from the body, and hurt for 5 unless the dragon is perched
    /// (`isSitting`) or a mob hurt the player within the last 2 ticks
    /// (`getLastHurtByMobTimestamp() < tickCount - 2`).
    fn dragon_wing_knock_back(&mut self, push: Vec3, sitting: bool) {
        let now = self.world.game_time;
        let hurt_lately = self.player_hurt_by().is_some_and(|(_, at)| at >= now - 2);
        let hurts = !sitting && !hurt_lately && !self.creative;
        if hurts {
            self.note_dragon_attacker("minecraft:mob_attack");
        }
        let (health, rules) = (&self.motion.health, self.world.health_rules);
        let id = health.damage.id("minecraft:mob_attack");
        if let Some(p) = self.player.as_mut() {
            push_seen_with_hurt(p, push, |p| {
                if let Some(id) = id.filter(|_| hurts) {
                    let _ = p.hurt(health, rules, id, 5.0);
                }
            });
        }
    }
    /// Damage from the dragon, which is its causing entity
    /// (`DamageSources.mobAttack(dragon)`, `indirectMagic(cloud, dragon)`): the
    /// death message names it, or the cloud once the dragon is gone.
    fn dragon_hurts_player(&mut self, damage_type: &str, amount: f32) {
        self.note_dragon_attacker(damage_type);
        self.hurt_player_by(damage_type, amount);
    }
    /// The causing entity of the dragon's next `damage_type` damage, for the
    /// death message.
    fn note_dragon_attacker(&mut self, damage_type: &str) {
        if let Some(id) = self.dragon.as_ref().map(|d| d.id) {
            let name = Text::translate("entity.minecraft.ender_dragon", &[]);
            self.note_named_attacker(damage_type, id, name);
        } else if damage_type == "minecraft:indirect_magic" {
            let name = Text::translate("entity.minecraft.area_effect_cloud", &[]);
            self.note_direct_entity(damage_type, name);
        }
    }
    /// `BossHealthOverlay.extractRenderState`: whatever `publish_boss_bars`
    /// (or a server's packets) put on this client's overlay.
    pub(super) fn draw_boss_bars(&self, batch: &mut HudBatch) {
        let bars = self
            .boss_overlay
            .bars(horizon_client::boss_overlay::now_millis());
        let draws = hud_overlays::boss_overlay(
            batch.width as i32,
            batch.height as i32,
            &bars,
            &self.gui.atlas.font,
        );
        hud_layout::paint(batch, &self.gui.atlas, &draws);
    }
    /// The dragon fight's bar for a player at `at`
    /// (`EnderDragonFight.dragonEvent`, visible while the fight is not won
    /// and the player is alive within 192 blocks of (0, 128, 0)).
    pub(super) fn dragon_boss_progress(&self, at: [f64; 3], alive: bool) -> Option<f32> {
        if self.dim != Dim::End || !alive || !self.fight.loaded || self.fight.killed {
            return None;
        }
        let near = at[0].powi(2) + (at[1] - 128.0).powi(2) + at[2].powi(2) <= 192.0 * 192.0;
        if !near {
            return None;
        }
        Some(
            self.dragon
                .as_ref()
                .map_or(1.0, |d| (d.health / MAX_HEALTH).clamp(0.0, 1.0)),
        )
    }
    /// `BossHealthOverlay.shouldPlayMusic`.
    pub(super) fn boss_music(&self) -> bool {
        self.boss_overlay.should_play_music()
    }
}
pub(super) struct DragonFireball {
    /// Its entity id once a server's players track it (0 until then).
    pub(super) id: u32,
    pub(super) position: Vec3,
    pub(super) previous: Vec3,
    pub(super) velocity: Vec3,
    age: i32,
}
impl DragonFireball {
    /// One a server sent (`handleAddEntity`).
    pub(super) fn remote(id: u32, at: Vec3, velocity: Vec3) -> Self {
        Self {
            id,
            position: at,
            previous: at,
            velocity,
            age: 0,
        }
    }
    /// `AbstractHurtingProjectile.tick` on a server's client: `applyInertia`
    /// first (power 0.1, inertia 0.95 or 0.8 in water), then the move; only
    /// the server decides what it hits.
    pub(super) fn remote_step(&mut self, water: bool) {
        self.previous = self.position;
        self.age += 1;
        let v = self.velocity;
        // Vec3.normalize: a vector shorter than 1e-5 stays zero.
        let l = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
        let n = if l < 1.0e-5 {
            Vec3::new(0.0, 0.0, 0.0)
        } else {
            Vec3::new(v.x / l, v.y / l, v.z / l)
        };
        let inertia = f64::from(if water { 0.8f32 } else { 0.95f32 });
        let v = Vec3::new(
            (v.x + n.x * 0.1) * inertia,
            (v.y + n.y * 0.1) * inertia,
            (v.z + n.z * 0.1) * inertia,
        );
        self.velocity = v;
        self.position = Vec3::new(
            self.position.x + v.x,
            self.position.y + v.y,
            self.position.z + v.z,
        );
    }
}
pub(super) struct Breath {
    /// Its entity id once a server's players track it (0 until then).
    pub(super) id: u32,
    pub(super) at: [f64; 3],
    pub(super) radius: f32,
    pub(super) age: i32,
    pub(super) duration: i32,
    pub(super) from_dragon: bool,
    /// A lingering potion's cloud (`ThrownLingeringPotion`): what it applies.
    pub(super) potion: Option<super::potions::PotionCloud>,
}
impl Breath {
    /// `AreaEffectCloud.isWaiting`: before its wait time (20 ticks, 10 for a
    /// lingering potion's).
    pub(super) fn waiting(&self) -> bool {
        self.age
            < if self.potion.is_some() {
                super::potions::LINGERING_WAIT
            } else {
                20
            }
    }
}
impl Game<'_> {
    /// `EnderDragonRenderer` and `EnderDragonModel.setupAnim`.
    pub(super) fn dragon_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
        additive: &mut Vec<Vertex>,
    ) {
        let (Some(d), Some(model)) = (self.dragon.as_ref(), self.dragon_model.as_ref()) else {
            return;
        };
        let a = alpha as f32;
        let sprite = |n: &str| {
            self.gui
                .atlas
                .sprites
                .get(n)
                .map(|r| [r.u0, r.v0, r.u1, r.v1])
        };
        let (Some(skin), Some(eyes)) = (
            sprite("entity/enderdragon/dragon"),
            sprite("entity/enderdragon/dragon_eyes"),
        ) else {
            return;
        };
        let pos = glam::DVec3::new(
            d.previous.x + (d.position.x - d.previous.x) * alpha,
            d.previous.y + (d.position.y - d.previous.y) * alpha,
            d.previous.z + (d.position.z - d.previous.z) * alpha,
        );
        let h = |delay: usize| d.sample_lerp(delay, a);
        let yr = h(7).1;
        let rot2 = (h(5).0 - h(10).0) as f32;
        let mut base = glam::Mat4::from_translation((pos - camera).as_vec3())
            * glam::Mat4::from_rotation_y((-yr).to_radians())
            * glam::Mat4::from_rotation_x((rot2 * 10.0).to_radians())
            * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.0, 1.0))
            * glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0))
            * glam::Mat4::from_translation(glam::Vec3::new(0.0, -1.501, 0.0));
        let rays_base = base * glam::Mat4::from_translation(glam::Vec3::new(0.0, -1.0, -2.0));
        // setupAnim.
        let flap = (d.old_flap + (d.flap - d.old_flap) * a) * std::f32::consts::TAU;
        let mut bounce = (flap - 1.0).sin() + 1.0;
        bounce = (bounce * bounce + bounce * 2.0) * 0.05;
        base *=
            glam::Mat4::from_translation(glam::Vec3::new(0.0, (bounce - 2.0) * 16.0, -48.0) / 16.0)
                * glam::Mat4::from_rotation_x((bounce * 2.0).to_radians());
        let mut poses = horizon_client::mobs::anim::Poses::new(model);
        poses.get("jaw")[3] = (flap.sin() + 1.0) * 0.2;
        let sitting = d.phase.sitting();
        let landing = matches!(d.phase, Phase::Landing { .. } | Phase::Takeoff { .. });
        let egg_dist = {
            let p = self.podium_top();
            ((p.x - d.position.x).powi(2)
                + (p.y - d.position.y).powi(2)
                + (p.z - d.position.z).powi(2))
            .sqrt()
        };
        let head_offset = |part: usize, start: (f64, f32), point: (f64, f32)| -> f32 {
            if landing {
                part as f32 / (egg_dist / 4.0).max(1.0) as f32
            } else if sitting {
                part as f32
            } else if part == 6 {
                0.0
            } else {
                (point.0 - start.0) as f32
            }
        };
        let start = h(6);
        let rot2y = wrap(h(5).1 - h(10).1);
        let rot = wrap(h(5).1 + rot2y / 2.0);
        let deg = std::f32::consts::PI / 180.0;
        let (mut xx, mut yy, mut zz) = (0.0f32, 20.0f32, -12.0f32);
        for i in 0..5 {
            let point = h(5 - i);
            let pose = poses.get(&format!("neck{i}"));
            let neck_x = (i as f32 * 0.45 + flap).cos() * 0.15;
            pose[4] = wrap(point.1 - start.1) * deg * 1.5;
            pose[3] = neck_x + head_offset(i, start, point) * deg * 1.5 * 5.0;
            pose[5] = -wrap(point.1 - rot) * deg * 1.5;
            pose[0] = xx;
            pose[1] = yy;
            pose[2] = zz;
            let (xr, yrr) = (pose[3], pose[4]);
            xx -= yrr.sin() * xr.cos() * 10.0;
            yy += xr.sin() * 10.0;
            zz -= yrr.cos() * xr.cos() * 10.0;
        }
        let current = h(0);
        let head = poses.get("head");
        head[0] = xx;
        head[1] = yy;
        head[2] = zz;
        head[4] = wrap(current.1 - start.1) * deg;
        head[3] = wrap(head_offset(6, start, current)) * deg * 1.5 * 5.0;
        head[5] = -wrap(current.1 - rot) * deg;
        poses.get("body")[5] = -rot2y * 1.5 * deg;
        let lw = [0.125 - flap.cos() * 0.2, -0.25, -(flap.sin() + 0.125) * 0.8];
        let lt = ((flap + 2.0).sin() + 0.5) * 0.75;
        {
            let p = poses.get("left_wing");
            (p[3], p[4], p[5]) = (lw[0], lw[1], lw[2]);
        }
        poses.get("left_wing_tip")[5] = lt;
        {
            let p = poses.get("right_wing");
            (p[3], p[4], p[5]) = (lw[0], -lw[1], -lw[2]);
        }
        poses.get("right_wing_tip")[5] = -lt;
        for side in ["left", "right"] {
            poses.get(&format!("{side}_hind_leg"))[3] = 1.0 + bounce * 0.1;
            poses.get(&format!("{side}_hind_leg_tip"))[3] = 0.5 + bounce * 0.1;
            poses.get(&format!("{side}_hind_foot"))[3] = 0.75 + bounce * 0.1;
            poses.get(&format!("{side}_front_leg"))[3] = 1.3 + bounce * 0.1;
            poses.get(&format!("{side}_front_leg_tip"))[3] = -0.5 - bounce * 0.1;
            poses.get(&format!("{side}_front_foot"))[3] = 0.75 + bounce * 0.1;
        }
        let mut tail_x = 0.0f32;
        let (mut xx, mut yy, mut zz) = (0.0f32, 10.0f32, 60.0f32);
        let start = h(11);
        for i in 0..12 {
            let point = h(12 + i);
            tail_x += (i as f32 * 0.45 + flap).sin() * 0.05;
            let p = poses.get(&format!("tail{i}"));
            p[4] = (wrap(point.1 - start.1) * 1.5 + 180.0) * deg;
            p[3] = tail_x + (point.0 - start.0) as f32 * deg * 1.5 * 5.0;
            p[5] = wrap(point.1 - rot) * deg * 1.5;
            p[0] = xx;
            p[1] = yy;
            p[2] = zz;
            let (xr, yrr) = (p[3], p[4]);
            yy += xr.sin() * 10.0;
            zz -= yrr.cos() * xr.cos() * 10.0;
            xx -= yrr.sin() * xr.cos() * 10.0;
        }
        let l = self.world.light(
            d.position.x.floor() as i32,
            d.position.y.floor() as i32,
            d.position.z.floor() as i32,
        );
        let light = (u32::from(l >> 4) * 16) | ((u32::from(l & 15) * 16) << 8);
        let death = if d.death_time > 0 {
            d.death_time as f32 + a
        } else {
            0.0
        };
        // The dissolve is approximated by fading out.
        let color = if death > 0.0 {
            let alpha = ((1.0 - death / 200.0) * 255.0).clamp(0.0, 255.0) as u32;
            alpha << 24 | 0x00FF_FFFF
        } else {
            0xFFFF_FFFF
        };
        let overlay = u32::from(d.hurt_time > 0);
        horizon_client::mobs::render::draw(model, base, &poses, skin, color, light, overlay, out);
        // RenderTypes.eyes: the glowing eyes, added on top.
        let mut eye_quads = Vec::new();
        horizon_client::mobs::render::draw(
            model,
            base,
            &poses,
            eyes,
            0xFFFF_FFFF,
            240 | 240 << 8,
            0,
            &mut eye_quads,
        );
        for v in &mut eye_quads {
            v.layer = 2;
        }
        additive.extend(eye_quads);
        if death > 0.0 {
            self.dragon_rays(rays_base, death / 200.0, additive);
        }
        // The healing beam to the nearest crystal.
        if let Some(c) = d.nearest_crystal.and_then(|i| self.crystals.get(i)) {
            let bob = f64::from(horizon_client::end_crystal::y(c.time as f32 + a));
            let to = glam::DVec3::new(c.position.x, c.position.y + bob, c.position.z);
            self.crystal_beam(pos, to - pos, self.ticks as f32 + a, camera, out);
        }
    }
    /// `EnderDragonRenderer.submitRays`: untextured, additive death rays.
    fn dragon_rays(&self, base: glam::Mat4, t: f32, out: &mut Vec<Vertex>) {
        dragon_rays(base, t, self.gui.atlas.font.white(), out);
    }
    /// `submitCrystalBeams`: an eight-sided tube from `from` (+2 up) along
    /// `delta`, the beam texture scrolling along it.
    pub(super) fn crystal_beam(
        &self,
        from: glam::DVec3,
        delta: glam::DVec3,
        time: f32,
        camera: glam::DVec3,
        out: &mut Vec<Vertex>,
    ) {
        let Some(s) = self
            .gui
            .atlas
            .sprites
            .get("entity/end_crystal/end_crystal_beam")
            .copied()
        else {
            return;
        };
        let q = |v: f32| (v * 65535.0).round().clamp(0.0, 65535.0) as u32;
        let wrap_light = q(s.v0) | q(s.v1 - s.v0) << 16;
        let (dx, dy, dz) = (delta.x as f32, delta.y as f32, delta.z as f32);
        let horizontal = (dx * dx + dz * dz).sqrt();
        let length = (dx * dx + dy * dy + dz * dz).sqrt();
        let m = glam::Mat4::from_translation(
            (from - camera).as_vec3() + glam::Vec3::new(0.0, 2.0, 0.0),
        ) * glam::Mat4::from_rotation_y(-dz.atan2(dx) - std::f32::consts::FRAC_PI_2)
            * glam::Mat4::from_rotation_x(-horizontal.atan2(dy) - std::f32::consts::FRAC_PI_2);
        let (v0, v1) = (-time * 0.01, length / 32.0 - time * 0.01);
        let (mut last_sin, mut last_cos, mut last_u) = (0.0f32, 0.75f32, 0.0f32);
        let u = |x: f32| s.u0 + (s.u1 - s.u0) * x;
        for i in 1..=8 {
            let a = i as f32 * std::f32::consts::TAU / 8.0;
            let (sin, cos) = (a.sin() * 0.75, a.cos() * 0.75);
            let uu = i as f32 / 8.0;
            for (p, uv, c) in [
                (
                    glam::Vec3::new(last_sin * 0.2, last_cos * 0.2, 0.0),
                    [u(last_u), v0],
                    0xFF00_0000,
                ),
                (
                    glam::Vec3::new(last_sin, last_cos, length),
                    [u(last_u), v1],
                    0xFFFF_FFFF,
                ),
                (glam::Vec3::new(sin, cos, length), [u(uu), v1], 0xFFFF_FFFF),
                (
                    glam::Vec3::new(sin * 0.2, cos * 0.2, 0.0),
                    [u(uu), v0],
                    0xFF00_0000,
                ),
            ] {
                out.push(Vertex {
                    normal: [0; 4],
                    pos: m.transform_point3(p).to_array(),
                    uv,
                    layer: 6,
                    color: c,
                    light: wrap_light,

                    glint_uv: [0.0, 0.0],
                });
            }
            (last_sin, last_cos, last_u) = (sin, cos, uu);
        }
    }
    /// Dragon fireballs as the dragon_fireball sprite facing the camera.
    pub(super) fn dragon_fireball_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        let Some(s) = self
            .gui
            .atlas
            .sprites
            .get("entity/enderdragon/dragon_fireball")
            .copied()
        else {
            return;
        };
        let forward = glam::DVec3::from_array(super::view_vector(self.yaw, self.pitch));
        let right = forward.cross(glam::DVec3::Y).normalize_or_zero();
        let up = right.cross(forward).normalize_or_zero();
        for b in &self.dragon_fireballs {
            let p = glam::DVec3::new(
                b.previous.x + (b.position.x - b.previous.x) * alpha,
                b.previous.y + (b.position.y - b.previous.y) * alpha,
                b.previous.z + (b.position.z - b.previous.z) * alpha,
            ) - camera;
            // DragonFireballRenderer: scale 2, full bright.
            let h = 1.0;
            for (c, uv) in [
                (p - right * h - up * h, [s.u0, s.v1]),
                (p + right * h - up * h, [s.u1, s.v1]),
                (p + right * h + up * h, [s.u1, s.v0]),
                (p - right * h + up * h, [s.u0, s.v0]),
            ] {
                out.push(Vertex {
                    normal: [0; 4],
                    pos: c.as_vec3().to_array(),
                    uv,
                    layer: 0,
                    color: 0xFFFF_FFFF,
                    light: 240 | 240 << 8,

                    glint_uv: [0.0, 0.0],
                });
            }
        }
    }
    pub(super) fn fight_ready(&mut self) -> bool {
        self.load_fight();
        // spawnExitPortal runs from scanState and the dragon's death, both
        // only with players in the fight.
        self.fight.loaded && self.fight.players
    }
    pub(super) fn exit_portal(&self) -> Option<[i32; 3]> {
        self.fight.exit_portal
    }
    pub(super) fn set_exit_portal(&mut self, origin: [i32; 3]) {
        if self.fight.exit_portal.is_none() {
            self.fight.exit_portal = Some(origin);
            self.fight.dirty = true;
        }
    }
}
/// Saving and loading the dragon and the crystals as chunk entities.
impl Dragon {
    pub(super) fn to_nbt(&self, uuid: [i32; 4], raw: Option<&Tag>) -> Tag {
        let mut tag = horizon_client::entity_nbt::vehicle_to_nbt(
            "minecraft:ender_dragon",
            self.position,
            self.velocity,
            self.yaw,
            uuid,
            raw,
        );
        if let Tag::Compound(map) = &mut tag {
            map.insert("Health".into(), Tag::Float(self.health.to_bits()));
            map.insert("DragonPhase".into(), Tag::Int(self.phase.id()));
            map.insert("DragonDeathTime".into(), Tag::Int(self.death_time));
            map.insert(
                "sitting_damage_received".into(),
                Tag::Float(self.sitting_damage.to_bits()),
            );
        }
        tag
    }
    pub(super) fn from_nbt(tag: &Tag, seed: i64) -> Option<Self> {
        let (position, velocity, yaw) = horizon_client::entity_nbt::boat_from_nbt(tag)?;
        let mut d = Self::new(position, yaw, seed);
        d.velocity = velocity;
        if let Some(Tag::Float(h)) = tag.get("Health") {
            d.health = f32::from_bits(*h);
        }
        if let Some(Tag::Int(p)) = tag.get("DragonPhase") {
            d.phase = Phase::from_id(*p);
        }
        if let Some(Tag::Int(t)) = tag.get("DragonDeathTime") {
            d.death_time = *t;
        }
        Some(d)
    }
}
impl Crystal {
    pub(super) fn to_nbt(&self, uuid: [i32; 4], raw: Option<&Tag>) -> Tag {
        let mut tag = horizon_client::entity_nbt::vehicle_to_nbt(
            "minecraft:end_crystal",
            self.position,
            Vec3::ZERO,
            0.0,
            uuid,
            raw,
        );
        if let Tag::Compound(map) = &mut tag {
            map.insert("ShowBottom".into(), Tag::Byte(i8::from(self.show_bottom)));
            if let Some(b) = self.beam {
                map.insert("beam_target".into(), Tag::IntArray(b.to_vec()));
            }
            map.insert(
                "Invulnerable".into(),
                Tag::Byte(i8::from(self.invulnerable)),
            );
        }
        tag
    }
    pub(super) fn from_nbt(tag: &Tag, time: i32) -> Option<Self> {
        let (position, _, _) = horizon_client::entity_nbt::boat_from_nbt(tag)?;
        Some(Self {
            id: 0,
            position,
            show_bottom: !matches!(tag.get("ShowBottom"), Some(Tag::Byte(0))),
            time,
            beam: match tag.get("beam_target") {
                Some(Tag::IntArray(v)) if v.len() == 3 => Some([v[0], v[1], v[2]]),
                _ => None,
            },
            invulnerable: matches!(tag.get("Invulnerable"), Some(Tag::Byte(1))),
        })
    }
}
impl Game<'_> {
    /// `ServerExplosion.hurtEntities`: crystals in reach are removed without
    /// another blast; explosions always hurt dragons.
    pub(super) fn explosion_hits_special(&mut self, center: [f64; 3], radius: f32) {
        let diameter = f64::from(radius * 2.0);
        let in_reach = |p: Vec3| {
            ((p.x - center[0]).powi(2) + (p.y - center[1]).powi(2) + (p.z - center[2]).powi(2))
                .sqrt()
                / diameter
                <= 1.0
        };
        let crystals: Vec<Vec3> = self
            .crystals
            .iter()
            .filter(|c| !c.invulnerable && in_reach(c.position))
            .map(|c| c.position)
            .collect();
        for at in crystals {
            // Look up the current index after each removal.
            if let Some(i) = self.crystals.iter().position(|c| c.position == at) {
                self.destroy_crystal(i, None, CrystalDamage::Explosion);
            }
        }
        let Some(d) = self.dragon.as_ref() else {
            return;
        };
        let hits = d.explosion_hits(center, radius, |b| {
            let (lo, hi) = (b.min_position(), b.max_position());
            self.seen_percent(center, [lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z])
        });
        let by = self.fight.blast_by;
        for (part, amount) in hits {
            self.hurt_dragon(part, amount, true, by, false);
        }
    }
}
impl Game<'_> {
    /// The dragon as guests track it (`EnderDragon.getAddEntityPacket` and
    /// the synched data that differs from the defaults: its health, defined
    /// as 1, and `DATA_PHASE`, defined as hovering). Vanilla pairs it with no
    /// attribute instances.
    pub(super) fn lan_dragon_state(&self) -> Option<horizon_client::lan::entity::EntityState> {
        use horizon_protocol::entity_data::{Value, serializer};
        let d = self.dragon.as_ref().filter(|d| d.id != 0)?;
        let mut meta = Vec::new();
        let health = d.health.max(0.0);
        if health != 1.0 {
            meta.push((9, serializer::FLOAT, Value::Float(health)));
        }
        let phase = d.phase.id();
        if phase != 10 {
            meta.push((16, serializer::INT, Value::Int(phase)));
        }
        Some(horizon_client::lan::entity::EntityState {
            id: d.id as i32,
            uuid: super::lan::entity_uuid(d.id),
            kind: "minecraft:ender_dragon".to_owned(),
            position: [d.position.x, d.position.y, d.position.z],
            yaw: d.yaw,
            pitch: 0.0,
            head_yaw: d.yaw,
            on_ground: false,
            velocity: [d.velocity.x, d.velocity.y, d.velocity.z],
            data: 0,
            meta,
            defaults: Vec::new(),
            equipment: Vec::new(),
            attributes: Vec::new(),
            precise: false,
            needs_sync: false,
            leash: None,
        })
    }
}
impl Game<'_> {
    /// The dragon and fight for `horizon-ctl status`.
    pub(super) fn dragon_status(&self) -> serde_json::Value {
        serde_json::json!({
            "dragon": self.dragon.as_ref().map(|d| serde_json::json!({
                "id": d.id,
                "position": [d.position.x, d.position.y, d.position.z],
                "health": d.health,
                "phase": d.phase.id(),
                "death_time": d.death_time,
                // The player it strafes: "host", or the guest's entity id.
                "strafing": match &d.phase {
                    Phase::Strafe { player: Some(FightWho::Host), .. } => serde_json::json!("host"),
                    Phase::Strafe { player: Some(FightWho::Guest(conn)), .. } => serde_json::json!(
                        self.lan.as_ref().and_then(|h| h.hub.guest(*conn)).map(|g| g.entity_id)
                    ),
                    _ => serde_json::Value::Null,
                },
            })),
            "players": self.fight.players,
            "crystals": self.crystals.len(),
            // The tracked id (0 until tracked) and position of each crystal.
            "crystal_list": self.crystals.iter().map(|c| serde_json::json!({
                "id": c.id,
                "position": [c.position.x, c.position.y, c.position.z],
            })).collect::<Vec<_>>(),
            "alive_crystals": self.fight.alive_crystals,
            "killed": self.fight.killed,
            "exit_portal": self.fight.exit_portal,
            "respawn": self.fight.respawn.map(|(s, t)| format!("{} {t}", s.name())),
        })
    }
}
/// Respawning the dragon with end crystals on the exit portal
/// (`EndCrystalItem.useOn`, `EnderDragonFight.tryRespawn`, `DragonRespawnStage`).
impl Game<'_> {
    /// `EndCrystalItem.useOn`: on obsidian or bedrock with room above.
    pub(super) fn place_end_crystal(
        &mut self,
        hit: &super::Hit,
        stack: &horizon_client::items::ItemStack,
    ) -> bool {
        if &*stack.item != "minecraft:end_crystal" {
            return false;
        }
        let name = self
            .res
            .facts()
            .get(self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]))
            .name
            .clone();
        if !matches!(&*name, "minecraft:obsidian" | "minecraft:bedrock") {
            return false;
        }
        let above = [hit.pos[0], hit.pos[1] + 1, hit.pos[2]];
        if !self
            .res
            .facts()
            .is_air(self.world.block(above[0], above[1], above[2]))
        {
            return false;
        }
        let (x, y, z) = (
            f64::from(above[0]),
            f64::from(above[1]),
            f64::from(above[2]),
        );
        let room = Aabb::new(Vec3::new(x, y, z), Vec3::new(x + 1.0, y + 2.0, z + 1.0));
        if self.entity_in(room) {
            return false;
        }
        let time = self.world.random.next_int_bound(100_000).unwrap_or(0);
        // EndCrystal's constructor takes an id from the entity counter, so
        // the crystal is an entity a command can name (`@e[type=end_crystal]`).
        self.next_entity += 1;
        self.crystals.push(Crystal {
            id: self.next_entity,
            position: Vec3::new(x + 0.5, y, z + 0.5),
            show_bottom: false,
            time,
            beam: None,
            invulnerable: false,
        });
        self.world.modified.insert((above[0] >> 4, above[2] >> 4));
        if self.dim == Dim::End {
            self.try_respawn();
        }
        if !self.creative {
            self.consume_held();
        }
        self.swing_arm();
        true
    }
    /// `EndCrystal.kill` (`/kill`): the crystal is removed, without a blast
    /// (`Entity.kill` does not hurt it).
    pub(super) fn kill_crystal(&mut self, id: u32) {
        if let Some(i) = self.crystals.iter().position(|c| c.id == id) {
            self.destroy_crystal(i, None, CrystalDamage::Explosion);
        }
    }
    /// `EnderDragon.kill` (`/kill`): `Entity.kill` removes the entity, it
    /// does not hurt it (the dragon flies its death flight when its health
    /// runs out). The fight's dragon ends the fight there and then
    /// (`dragonFight.setDragonKilled`): the exit portal, a gateway and the
    /// first dragon's egg, and no new dragon. No experience: `tickDeath`
    /// drops it, and a killed dragon never gets there.
    pub(super) fn kill_dragon(&mut self, id: u32) {
        if self.dragon.as_ref().is_some_and(|d| d.id == id) {
            self.dragon = None;
            // ServerLevel.getDragonFight: only the End has a fight.
            if self.dim == Dim::End {
                self.dragon_killed();
            }
        }
    }
    /// Any entity touching `room` (`Level.getEntities(null, box)`).
    fn entity_in(&self, room: Aabb) -> bool {
        let touches = |lo: Vec3, hi: Vec3| {
            let (a, b) = (room.min_position(), room.max_position());
            lo.x < b.x && hi.x > a.x && lo.y < b.y && hi.y > a.y && lo.z < b.z && hi.z > a.z
        };
        // Level.getEntities: the host's player and every guest in the level.
        let player = self
            .fight_players()
            .iter()
            .any(|p| !p.spectator && touches(p.bounds.min_position(), p.bounds.max_position()));
        player
            || self.crystals.iter().any(|c| {
                touches(
                    Vec3::new(c.position.x - 1.0, c.position.y, c.position.z - 1.0),
                    Vec3::new(c.position.x + 1.0, c.position.y + 2.0, c.position.z + 1.0),
                )
            })
            || self.mobs.iter().any(|m| {
                let w = f64::from(m.width()) / 2.0;
                let h = f64::from(m.height());
                touches(
                    Vec3::new(m.position.x - w, m.position.y, m.position.z - w),
                    Vec3::new(m.position.x + w, m.position.y + h, m.position.z + w),
                )
            })
    }
    /// `EnderDragonFight.tryRespawn`: a crystal on each side of the portal.
    fn try_respawn(&mut self) {
        self.load_fight();
        if !self.fight.killed || self.fight.respawn.is_some() {
            return;
        }
        let origin = match self.fight.exit_portal {
            Some(o) => o,
            None => {
                let _ = self.activate_exit_portal();
                match self.fight.exit_portal {
                    Some(o) => o,
                    None => return,
                }
            }
        };
        let mut found = Vec::new();
        for d in [[0, -1], [0, 1], [-1, 0], [1, 0]] {
            // new AABB(center.relative(direction, 3)): the block cube.
            let cell = [origin[0] + d[0] * 3, origin[1] + 1, origin[2] + d[1] * 3];
            let lo = Vec3::new(f64::from(cell[0]), f64::from(cell[1]), f64::from(cell[2]));
            let hits: Vec<Vec3> = self
                .crystals
                .iter()
                .filter(|c| {
                    let p = c.position;
                    p.x + 1.0 > lo.x
                        && p.x - 1.0 < lo.x + 1.0
                        && p.y + 2.0 > lo.y
                        && p.y < lo.y + 1.0
                        && p.z + 1.0 > lo.z
                        && p.z - 1.0 < lo.z + 1.0
                })
                .map(|c| c.position)
                .collect();
            if hits.is_empty() {
                return;
            }
            found.extend(hits);
        }
        // respawnDragon: the podium goes back to end stone, then inactive.
        for y in origin[1] - 1..=origin[1] + 3 {
            for z in origin[2] - 3..=origin[2] + 3 {
                for x in origin[0] - 3..=origin[0] + 3 {
                    let name = self.block_name_at([x, y, z]);
                    if matches!(&*name, "minecraft:bedrock" | "minecraft:end_portal") {
                        let _ = self.set_state_named([x, y, z], "minecraft:end_stone", &[]);
                    }
                }
            }
        }
        let _ = self.place_podium(origin, false);
        self.fight.respawn = Some((Respawn::Start, 0));
        self.fight.respawn_crystals = found;
        self.fight.dirty = true;
    }
    fn block_name_at(&self, p: [i32; 3]) -> std::sync::Arc<str> {
        self.res
            .facts()
            .get(self.world.block(p[0], p[1], p[2]))
            .name
            .clone()
    }
    fn respawn_beams(&mut self, target: Option<[i32; 3]>) {
        let list = self.fight.respawn_crystals.clone();
        for c in &mut self.crystals {
            if list.iter().any(|p| p.distance_squared(c.position) < 0.01) {
                c.beam = target;
            }
        }
    }
    /// `LevelEvent 3001`: the dragon's growl over the island.
    fn respawn_growl(&mut self) {
        let pitch = 0.8 + self.world.random.next_float() * 0.3;
        self.audio.play(
            "minecraft:entity.ender_dragon.growl",
            Some([0.5, 128.5, 0.5]),
            64.0,
            pitch,
        );
    }
    fn set_respawn(&mut self, stage: Option<Respawn>) {
        self.fight.respawn = stage.map(|s| (s, 0));
        self.fight.dirty = true;
    }
    /// `EnderDragonFight.tick` with a respawn in progress.
    fn tick_respawn(&mut self) {
        let Some((stage, time)) = self.fight.respawn else {
            return;
        };
        let alive = self.fight.respawn_crystals.iter().any(|p| {
            self.crystals
                .iter()
                .any(|c| p.distance_squared(c.position) < 0.01)
        });
        if !alive {
            self.abort_respawn();
            return;
        }
        if let Some(r) = self.fight.respawn.as_mut() {
            r.1 += 1;
        }
        const CENTER: [i32; 3] = [0, 128, 0];
        match stage {
            Respawn::Start => {
                self.respawn_beams(Some(CENTER));
                self.set_respawn(Some(Respawn::Preparing));
            }
            Respawn::Preparing => {
                if time < 100 {
                    if matches!(time, 0 | 50..=52) || time >= 95 {
                        self.respawn_growl();
                    }
                } else {
                    self.set_respawn(Some(Respawn::Pillars));
                }
            }
            Respawn::Pillars => {
                let (start, end) = (time % 40 == 0, time % 40 == 39);
                if !(start || end) {
                    return;
                }
                let spikes = horizon_world::worldgen::end_spike::spikes_for_seed(self.level.seed);
                let index = (time / 40) as usize;
                if let Some(spike) = spikes.get(index) {
                    if start {
                        self.respawn_beams(Some([
                            spike.center_x,
                            spike.height + 1,
                            spike.center_z,
                        ]));
                    } else {
                        self.rebuild_spike(spike);
                    }
                } else if start {
                    self.set_respawn(Some(Respawn::Summoning));
                }
            }
            Respawn::Summoning => {
                if time >= 100 {
                    self.finish_respawn();
                } else if time >= 80 || (1..5).contains(&time) {
                    self.respawn_growl();
                } else if time == 0 {
                    self.respawn_beams(Some(CENTER));
                }
            }
        }
    }
    /// SUMMONING_PILLARS at the end of a beam: the spike is cleared, blown up
    /// and grown again with a guarded crystal.
    fn rebuild_spike(&mut self, spike: &horizon_world::worldgen::end_spike::EndSpike) {
        let (cx, cz, h) = (spike.center_x, spike.center_z, spike.height);
        let air = self.pack.assets.registry.default_state("minecraft:air");
        let Some(air) = air else {
            return;
        };
        for y in h - 10..=h + 10 {
            for z in cz - 10..=cz + 10 {
                for x in cx - 10..=cx + 10 {
                    if !self.res.facts().is_air(self.world.block(x, y, z)) {
                        let _ = self.level_set([x, y, z], air);
                    }
                }
            }
        }
        // DragonRespawnStage.SUMMONING_PILLARS: ExplosionInteraction.BLOCK.
        let interaction = self.destroy_type("block_explosion_drop_decay");
        let _ = self.server_explode(
            [f64::from(cx) + 0.5, f64::from(h), f64::from(cz) + 0.5],
            5.0,
            interaction,
            false,
        );
        // EndSpikeFeature.placeSpike with crystal_invulnerable and the beam.
        let r = spike.radius;
        for z in cz - r..=cz + r {
            for y in 0..=h + 10 {
                for x in cx - r..=cx + r {
                    let (dx, dz) = (f64::from(x - cx), f64::from(z - cz));
                    if dx * dx + dz * dz <= f64::from(r * r + 1) && y < h {
                        if &*self.block_name_at([x, y, z]) != "minecraft:obsidian" {
                            let _ = self.set_state_named([x, y, z], "minecraft:obsidian", &[]);
                        }
                    } else if y > 65 && !self.res.facts().is_air(self.world.block(x, y, z)) {
                        let _ = self.level_set([x, y, z], air);
                    }
                }
            }
        }
        if spike.guarded {
            for dx in -2i32..=2 {
                for dz in -2i32..=2 {
                    for dy in 0..=3 {
                        let top = dy == 3;
                        if !(dx.abs() == 2 || dz.abs() == 2 || top) {
                            continue;
                        }
                        let x_edge = dx.abs() == 2 || top;
                        let z_edge = dz.abs() == 2 || top;
                        self.set_bars(
                            [cx + dx, h + dy, cz + dz],
                            [
                                x_edge && dz != -2,
                                x_edge && dz != 2,
                                z_edge && dx != -2,
                                z_edge && dx != 2,
                            ],
                        );
                    }
                }
            }
        }
        let at = Vec3::new(f64::from(cx) + 0.5, f64::from(h + 1), f64::from(cz) + 0.5);
        self.crystals
            .retain(|c| c.position.distance_squared(at) > 0.01);
        let time = self.world.random.next_int_bound(100_000).unwrap_or(0);
        self.crystals.push(Crystal {
            id: 0,
            position: at,
            show_bottom: true,
            time,
            beam: Some([0, 128, 0]),
            invulnerable: true,
        });
        let _ = self.set_state_named([cx, h, cz], "minecraft:bedrock", &[]);
        let _ = self.set_state_named([cx, h + 1, cz], "minecraft:fire", &[]);
        self.world.modified.insert((cx >> 4, cz >> 4));
    }
    /// Iron bars connected north, south, west and east.
    fn set_bars(&mut self, pos: [i32; 3], sides: [bool; 4]) {
        let registry = self.pack.assets.registry;
        let Some(mut state) = registry
            .default_state("minecraft:iron_bars")
            .and_then(|d| registry.state(d).cloned())
        else {
            return;
        };
        for (name, on) in ["north", "south", "west", "east"].into_iter().zip(sides) {
            state
                .properties
                .insert(name.into(), if on { "true" } else { "false" }.into());
        }
        if let Ok(id) = registry.resolve(&state) {
            let _ = self.level_set(pos, id);
        }
    }
    /// `resetSpikeCrystals`: spike crystals lose their beams and protection.
    fn reset_spike_crystals(&mut self) {
        let spikes = horizon_world::worldgen::end_spike::spikes_for_seed(self.level.seed);
        for c in &mut self.crystals {
            let on_spike = spikes.iter().any(|s| {
                (c.position.x - f64::from(s.center_x) - 0.5).abs() <= f64::from(s.radius)
                    && (c.position.z - f64::from(s.center_z) - 0.5).abs() <= f64::from(s.radius)
                    && c.position.y >= f64::from(s.height)
            });
            if on_spike {
                c.invulnerable = false;
                c.beam = None;
            }
        }
    }
    /// `EnderDragonFight.createNewDragon`: over the podium, holding pattern, a random yaw.
    fn create_new_dragon(&mut self) {
        let yaw = self.world.random.next_float() * 360.0;
        let seed = self.world.random.next_long();
        self.dragon = Some(Dragon::new(Vec3::new(0.0, 128.0, 0.0), yaw, seed));
    }
    /// The END stage: the portal crystals blow up (no block damage) and a
    /// new dragon takes over.
    fn finish_respawn(&mut self) {
        self.reset_spike_crystals();
        self.fight.respawn = None;
        let list = std::mem::take(&mut self.fight.respawn_crystals);
        for p in list {
            if let Some(i) = self
                .crystals
                .iter()
                .position(|c| c.position.distance_squared(p) < 0.01)
            {
                let c = self.crystals.remove(i);
                self.world.modified.insert((
                    c.position.x.floor() as i32 >> 4,
                    c.position.z.floor() as i32 >> 4,
                ));
                let _ = self.server_explode(
                    [c.position.x, c.position.y, c.position.z],
                    6.0,
                    super::explosion::BlockInteraction::Keep,
                    false,
                );
            }
        }
        self.fight.killed = false;
        // setRespawnStage(END): the new dragon is summoned for every player
        // of the fight (the boss bar's players: alive within 192 blocks).
        self.create_new_dragon();
        let view = self
            .dragon
            .as_ref()
            .map(|d| horizon_client::advancements::EntityView {
                kind: "minecraft:ender_dragon".to_owned(),
                position: [d.position.x, d.position.y, d.position.z],
                ..Default::default()
            });
        if let Some(view) = view {
            self.for_each_player(|g| {
                let near = g.player.as_ref().is_some_and(|p| {
                    !p.health.dead_or_dying()
                        && p.position.x.powi(2)
                            + (p.position.y - 128.0).powi(2)
                            + p.position.z.powi(2)
                            <= 192.0 * 192.0
                });
                if near {
                    g.trigger_entity("minecraft:summoned_entity", "entity", &view);
                }
            });
        }
        self.fight.dirty = true;
        self.count_crystals();
    }
    /// `abortRespawnSequence`.
    fn abort_respawn(&mut self) {
        self.fight.respawn = None;
        self.fight.respawn_crystals.clear();
        self.reset_spike_crystals();
        let _ = self.activate_exit_portal();
        self.fight.dirty = true;
    }
    /// `EnderDragon.checkCrystals`' search: the nearest crystal whose box
    /// meets the dragon's box grown by 32.
    fn nearest_crystal_to(&self, p: Vec3) -> Option<usize> {
        // The dragon's 16 by 8 box.
        let reach = Aabb::new(
            Vec3::new(p.x - 8.0, p.y, p.z - 8.0),
            Vec3::new(p.x + 8.0, p.y + 8.0, p.z + 8.0),
        )
        .inflate(Vec3::new(32.0, 32.0, 32.0));
        self.crystals
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                // An end crystal's 2 by 2 box.
                reach.intersects(Aabb::new(
                    Vec3::new(c.position.x - 1.0, c.position.y, c.position.z - 1.0),
                    Vec3::new(c.position.x + 1.0, c.position.y + 2.0, c.position.z + 1.0),
                ))
            })
            .min_by(|a, b| {
                let da = (a.1.position.x - p.x).powi(2)
                    + (a.1.position.y - p.y).powi(2)
                    + (a.1.position.z - p.z).powi(2);
                let db = (b.1.position.x - p.x).powi(2)
                    + (b.1.position.y - p.y).powi(2)
                    + (b.1.position.z - p.z).powi(2);
                da.total_cmp(&db)
            })
            .map(|(i, _)| i)
    }
}

/// The dragon on a server's client (`ClientPacketListener.handleAddEntity`
/// and the client side of `EnderDragon.aiStep`): the server flies it and
/// sends its moves, health and phase; the client flaps its wings, keeps
/// its flight history and places its parts, so it can be drawn and hit.
impl Game<'_> {
    /// `handleAddEntity` for `minecraft:ender_dragon`. Its constructor
    /// sets the full health and the hovering phase until the server's
    /// synched data says otherwise.
    pub(super) fn spawn_remote_dragon(&mut self, id: i32, position: [f64; 3], yaw: f32) {
        let at = Vec3::new(position[0], position[1], position[2]);
        let mut d = Dragon::new(at, yaw, i64::from(id));
        d.id = id as u32;
        d.phase = Phase::from_id(10);
        self.dragon = Some(d);
        self.place_dragon_parts();
        if let Some(d) = self.dragon.as_mut() {
            for part in &mut d.parts {
                part.1 = part.0;
            }
        }
    }
    /// The remote dragon, if `id` is it.
    fn remote_dragon_mut(&mut self, id: i32) -> Option<&mut Dragon> {
        self.dragon.as_mut().filter(|d| d.id as i32 == id)
    }
    /// `handleSetEntityData`: LivingEntity.DATA_HEALTH_ID and
    /// EnderDragon.DATA_PHASE (`onSyncedDataUpdated` sets the phase).
    pub(super) fn remote_dragon_data(&mut self, id: i32, values: &[(u8, i32, Value)]) {
        let Some(d) = self.remote_dragon_mut(id) else {
            return;
        };
        for (index, _, value) in values {
            match (index, value) {
                (9, Value::Float(health)) => d.health = *health,
                (16, Value::Int(phase)) => d.set_phase(Phase::from_id(*phase)),
                _ => {}
            }
        }
    }
    /// `handleSetEntityMotion`: the velocity its wings flap by.
    pub(super) fn remote_dragon_motion(&mut self, id: i32, velocity: [f64; 3]) {
        if let Some(d) = self.remote_dragon_mut(id) {
            d.velocity = Vec3::new(velocity[0], velocity[1], velocity[2]);
        }
    }
    /// `handleHurtAnimation` and `handleDamageEvent`: the red flash.
    pub(super) fn remote_dragon_hurt(&mut self, id: i32) {
        if let Some(d) = self.remote_dragon_mut(id) {
            d.hurt_time = 10;
        }
    }
    /// `handleRemoveEntities`, or a crystal removed: the healing beam
    /// keeps pointing at the same crystal.
    pub(super) fn remove_remote_dragon(&mut self, id: i32) {
        if self.remote_dragon_mut(id).is_some() {
            self.dragon = None;
            return;
        }
        let Some(index) = self.crystals.iter().position(|c| c.id as i32 == id) else {
            return;
        };
        if let Some(d) = self.dragon.as_mut() {
            match d.nearest_crystal {
                Some(n) if n == index => d.nearest_crystal = None,
                Some(n) if n > index => d.nearest_crystal = Some(n - 1),
                _ => {}
            }
        }
    }
    /// One client tick of the dragon. `lerp`: the step toward its last
    /// sent position and yaw (LivingEntity's interpolation), and the
    /// steps left including this one.
    pub(super) fn tick_remote_dragon(&mut self, lerp: Option<([f64; 3], Option<f32>, u32)>) {
        let Some(d) = self.dragon.as_mut() else {
            return;
        };
        d.previous = d.position;
        d.previous_yaw = d.yaw;
        d.old_flap = d.flap;
        d.ticks += 1;
        d.hurt_time = (d.hurt_time - 1).max(0);
        for part in &mut d.parts {
            part.1 = part.0;
        }
        if let Some((target, yaw, steps)) = lerp {
            let n = f64::from(steps.max(1));
            d.position = Vec3::new(
                d.position.x + (target[0] - d.position.x) / n,
                d.position.y + (target[1] - d.position.y) / n,
                d.position.z + (target[2] - d.position.z) / n,
            );
            if let Some(yaw) = yaw {
                d.yaw += wrap(yaw - d.yaw) / steps.max(1) as f32;
            }
        }
        if !d.alive() {
            // EnderDragon.tickDeath on a client: it rises, and bursts
            // from tick 180 to 200; the server removes it.
            d.death_time += 1;
            let (t, p) = (d.death_time, d.position);
            if (180..=200).contains(&t) {
                let r = &mut d.random;
                let o = [
                    f64::from((r.next_float() - 0.5) * 8.0),
                    f64::from((r.next_float() - 0.5) * 4.0),
                    f64::from((r.next_float() - 0.5) * 8.0),
                ];
                self.particles.add(
                    &ParticleOptions::simple(ParticleType::ExplosionEmitter),
                    [p.x + o[0], p.y + 2.0 + o[1], p.z + o[2]],
                    [0.0; 3],
                );
            }
            if let Some(d) = self.dragon.as_mut() {
                d.position.y += 0.1;
                for part in &mut d.parts {
                    part.0.y += 0.1;
                }
            }
            return;
        }
        // processFlappingMovement and onFlap: its wing beats, and their sound.
        let flap_was = (d.old_flap * std::f32::consts::TAU).cos();
        let v = d.velocity;
        let mut flap_speed = 0.2 / ((v.x.hypot(v.z) as f32) * 10.0 + 1.0);
        flap_speed *= 2f32.powf(v.y as f32);
        d.flap += if d.phase.sitting() { 0.1 } else { flap_speed };
        let flap_now = (d.flap * std::f32::consts::TAU).cos();
        let at = [d.position.x, d.position.y, d.position.z];
        if flap_was <= -0.3 && flap_now >= -0.3 {
            let pitch = 0.8 + self.world.random.next_float() * 0.3;
            self.audio
                .play("minecraft:entity.ender_dragon.flap", Some(at), 5.0, pitch);
        }
        let d = self.dragon.as_mut().expect("dragon");
        if !d.phase.sitting() {
            d.growl -= 1;
            if d.growl < 0 {
                d.growl = 200 + d.random.next_int_bound(200).unwrap_or(0);
                let pitch = 0.8 + self.world.random.next_float() * 0.3;
                self.audio
                    .play("minecraft:entity.ender_dragon.growl", Some(at), 2.5, pitch);
            }
        }
        // checkCrystals' search runs on a client too (the healing beam).
        let d = self.dragon.as_mut().expect("dragon");
        if d.random.next_int_bound(10).unwrap_or(1) == 0 {
            let p = d.position;
            let nearest = self.nearest_crystal_to(p);
            self.dragon.as_mut().expect("dragon").nearest_crystal = nearest;
        }
        let d = self.dragon.as_mut().expect("dragon");
        d.yaw = wrap(d.yaw);
        d.record();
        self.place_dragon_parts();
    }
    /// The dragon's parts as a server's client attacks them: the server's
    /// ids for `EnderDragonPart`s follow the dragon's.
    pub(super) fn remote_dragon_part_hit(
        &self,
        eye: [f64; 3],
        dir: [f64; 3],
        reach: f64,
    ) -> Option<(i32, f64, [f64; 3])> {
        let d = self.dragon.as_ref().filter(|d| d.alive() && d.id != 0)?;
        (0..8)
            .filter_map(|i| {
                let b = d.part_box(i);
                let p = d.parts[i].0;
                super::ray_box(eye, dir, reach, b)
                    .map(|t| ((d.id + 1 + i as u32) as i32, t, [p.x, p.y, p.z]))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }
}

/// `submitRays`: the death rays from a fixed seed, fading in white to magenta.
fn dragon_rays(base: glam::Mat4, t: f32, white: horizon_client::hud::Rect, out: &mut Vec<Vertex>) {
    // Vanilla's dragonRays has no texture. Sample inside the white sprite:
    // its border can round into transparent neighbours during interpolation.
    let uv = [(white.u0 + white.u1) / 2.0, (white.v0 + white.v1) / 2.0];
    let over = if t > 0.8 {
        ((t - 0.8) / 0.2).min(1.0)
    } else {
        0.0
    };
    let inner = (((1.0 - over) * 255.0) as u32) << 24 | 0x00FF_FFFF;
    let outer = 0x00FF_00FF;
    let mut random = LegacyRandom::new(432);
    let mut m = base;
    let count = ((t + t * t) / 2.0 * 60.0).floor() as i32;
    let half_sqrt3 = 3f32.sqrt() / 2.0;
    let tau = std::f32::consts::TAU;
    for _ in 0..count {
        let mut f = || random.next_float() * tau;
        let q1 = glam::Quat::from_rotation_x(f())
            * glam::Quat::from_rotation_y(f())
            * glam::Quat::from_rotation_z(f());
        let (a, b, c) = (f(), f(), f());
        let q2 = glam::Quat::from_rotation_x(a)
            * glam::Quat::from_rotation_y(b)
            * glam::Quat::from_rotation_z(c + t * std::f32::consts::FRAC_PI_2);
        m *= glam::Mat4::from_quat(q1 * q2);
        let length = random.next_float() * 20.0 + 5.0 + over * 10.0;
        let width = random.next_float() * 2.0 + 1.0 + over * 2.0;
        let o = glam::Vec3::ZERO;
        let left = glam::Vec3::new(-half_sqrt3 * width, length, -0.5 * width);
        let right = glam::Vec3::new(half_sqrt3 * width, length, -0.5 * width);
        let bottom = glam::Vec3::new(0.0, length, width);
        for (p1, p2) in [(left, right), (right, bottom), (bottom, left)] {
            // A triangle as a quad with its last corner repeated.
            for (p, c) in [(o, inner), (p1, outer), (p2, outer), (p2, outer)] {
                out.push(Vertex {
                    normal: [0; 4],
                    pos: m.transform_point3(p).to_array(),
                    uv,
                    layer: 2,
                    color: c,
                    light: 0,

                    glint_uv: [0.0, 0.0],
                });
            }
        }
    }
}

/// `Entity.push` on the server's copy of a player, then `hurt`. A player's
/// own client hears of its velocity only when a hurt marks it
/// (`LivingEntity.hurtServer`'s `markHurt`, sent by
/// `ServerEntity.sendChanges`); `push` alone only sets `needsSync`, which
/// reaches the players tracking them, not them. So the push moves the player
/// only together with a hurt that lands outside the hurt cooldown, and is
/// dropped otherwise.
fn push_seen_with_hurt(
    p: &mut horizon_client::motion::PlayerMotion,
    push: Vec3,
    hurt: impl FnOnce(&mut horizon_client::motion::PlayerMotion),
) {
    let before = p.velocity;
    let marked_before = std::mem::take(&mut p.health.sync_velocity);
    p.velocity = before.add_vector(push);
    hurt(p);
    if !p.health.sync_velocity {
        p.velocity = before;
    }
    p.health.sync_velocity |= marked_before;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn death_rays_sample_inside_the_white_sprite() {
        // A four-texel white sprite surrounded by transparent atlas pixels.
        // A border UV can round outside it during perspective interpolation.
        let size = 1024.0;
        let white = horizon_client::hud::Rect {
            u0: 20.0 / size,
            v0: 60.0 / size,
            u1: 24.0 / size,
            v1: 64.0 / size,
            w: 4,
            h: 4,
        };
        for t in [0.25, 0.8, 0.95, 1.0] {
            let mut vertices = Vec::new();
            dragon_rays(glam::Mat4::IDENTITY, t, white, &mut vertices);
            assert!(!vertices.is_empty());
            for v in &vertices {
                assert!(
                    v.uv[0] > white.u0 + 0.5 / size && v.uv[0] < white.u1 - 0.5 / size,
                    "death-ray U touches the atlas border: {}",
                    v.uv[0]
                );
                assert!(
                    v.uv[1] > white.v0 + 0.5 / size && v.uv[1] < white.v1 - 0.5 / size,
                    "death-ray V touches the atlas border: {}",
                    v.uv[1]
                );
            }
        }
    }

    fn dragon(phase: Phase) -> Dragon {
        let mut d = Dragon::new(Vec3::new(0.0, 70.0, 0.0), 0.0, 1);
        d.phase = phase;
        d
    }
    fn flying() -> Phase {
        Phase::Holding {
            path: None,
            target: None,
            clockwise: false,
        }
    }

    #[test]
    fn a_perched_dragon_shrugs_arrows_off() {
        // AbstractDragonSittingPhase.onHurt: an arrow does nothing, melee does.
        let mut d = dragon(Phase::Scanning { time: 0 });
        assert_eq!(d.hurt(0, 9.0, true, true), Hurt::Shrugged);
        assert_eq!(d.health, MAX_HEALTH);
        assert_eq!(d.hurt(0, 9.0, true, false), Hurt::Landed { full: true });
        assert_eq!(d.health, MAX_HEALTH - 9.0);
        // In flight an arrow lands.
        let mut d = dragon(flying());
        assert_eq!(d.hurt(0, 9.0, true, true), Hurt::Landed { full: true });
        assert_eq!(d.health, MAX_HEALTH - 9.0);
    }

    #[test]
    fn the_cooldown_lets_only_the_excess_through() {
        // LivingEntity.hurtServer: damageCooldownTime > 10 keeps lastHurt.
        let mut d = dragon(flying());
        assert_eq!(d.hurt(0, 6.0, true, false), Hurt::Landed { full: true });
        assert_eq!(d.hurt(0, 4.0, true, false), Hurt::Landed { full: false });
        assert_eq!(d.health, MAX_HEALTH - 6.0);
        assert_eq!(d.hurt(0, 10.0, true, false), Hurt::Landed { full: false });
        assert_eq!(d.health, MAX_HEALTH - 10.0);
        // Ten ticks on, the window is over: a full hit again.
        d.invulnerable = 10;
        assert_eq!(d.hurt(0, 3.0, true, false), Hurt::Landed { full: true });
        assert_eq!(d.health, MAX_HEALTH - 13.0);
    }

    #[test]
    fn parts_and_sources() {
        // Neither head nor neck: a quarter plus one.
        let mut d = dragon(flying());
        assert_eq!(d.hurt(2, 20.0, true, false), Hurt::Landed { full: true });
        assert_eq!(d.health, MAX_HEALTH - 6.0);
        // Not a player's and not an explosion: true, but no damage.
        let mut d = dragon(flying());
        assert_eq!(d.hurt(0, 20.0, false, false), Hurt::Landed { full: false });
        assert_eq!(d.health, MAX_HEALTH);
        // Too little after the part's cut, and nothing while dying.
        assert_eq!(d.hurt(3, 0.001, true, false), Hurt::Refused);
        d.phase = Phase::Dying {
            target: None,
            time: 0,
        };
        assert_eq!(d.hurt(0, 20.0, true, false), Hurt::Refused);
    }

    #[test]
    fn a_quarter_of_its_health_while_perched_makes_it_take_off() {
        let mut d = dragon(Phase::Flaming { ticks: 0 });
        d.hurt(0, 30.0, true, false);
        assert!(d.phase.sitting());
        d.invulnerable = 0;
        d.hurt(0, 21.0, true, false);
        assert!(matches!(d.phase, Phase::Takeoff { .. }));
        assert_eq!(d.sitting_damage, 0.0);
    }

    #[test]
    fn explosions_hit_each_part_by_its_exposure() {
        let mut d = dragon(flying());
        for (i, part) in d.parts.iter_mut().enumerate() {
            part.0 = Vec3::new(i as f64 * 4.0, 70.0, 0.0);
        }
        // A power-6 blast at the dragon's feet: diameter 12.
        let center = [0.0, 70.0, 0.0];
        let open = d.explosion_hits(center, 6.0, |_| 1.0);
        // The dragon itself (its body), then the parts within 12 blocks.
        let parts: Vec<usize> = open.iter().map(|h| h.0).collect();
        assert_eq!(parts, vec![2, 0, 1, 2, 3]);
        // At the centre: (1 + 1) / 2 * 7 * 12 + 1.
        assert_eq!(open[0].1, 85.0);
        // Behind cover (no ray reaches the box) only the 1 is left.
        let covered = d.explosion_hits(center, 6.0, |_| 0.0);
        assert!(covered.iter().all(|h| h.1 == 1.0));
        // Through the cooldown the strongest hit after the part's cut wins.
        for (part, amount) in open {
            d.hurt(part, amount, true, false);
        }
        assert_eq!(d.health, MAX_HEALTH - 85.0);
    }

    fn player(who: FightWho, x: f64, creative: bool, spectator: bool) -> FightPlayer {
        let position = Vec3::new(x, 64.0, 0.0);
        FightPlayer {
            who,
            position,
            bounds: Aabb::new(position, Vec3::new(x + 0.6, 65.8, 0.6)),
            alive: true,
            creative,
            spectator,
        }
    }

    /// #2893: a player under a wing is pushed only when the wing's hurt
    /// lands, not on every tick they stay there, and never under a perched
    /// dragon's wing.
    #[test]
    fn a_wing_pushes_the_player_only_with_a_landed_hurt() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = horizon_client::assets::Assets::load(&root).unwrap();
        let resources = horizon_client::motion::MotionResources::load(&root, &assets).unwrap();
        let (health, rules) = (&resources.health, Default::default());
        let id = health.damage.id("minecraft:mob_attack").unwrap();
        let push = Vec3::new(0.5, f64::from(0.2_f32), -0.25);
        let wing = |p: &mut horizon_client::motion::PlayerMotion, sitting: bool| {
            push_seen_with_hurt(p, push, |p| {
                if !sitting {
                    let _ = p.hurt(health, rules, id, 5.0);
                }
            });
        };
        // In flight: the first tick's hurt lands and carries the push.
        let mut p = horizon_client::motion::PlayerMotion::new(&resources, Vec3::ZERO).unwrap();
        wing(&mut p, false);
        assert_eq!(p.velocity, push);
        assert_eq!(p.health.health(), 15.0);
        // The next 9 ticks are inside the hurt cooldown: no hurt, no push.
        for _ in 0..9 {
            p.health.advance_clock();
            wing(&mut p, false);
        }
        assert_eq!(p.velocity, push, "pushed again with no hurt");
        assert_eq!(p.health.health(), 15.0);
        // Ten ticks on, the next hurt lands and pushes again.
        p.health.advance_clock();
        wing(&mut p, false);
        assert_eq!(p.velocity, push.add_vector(push));
        assert_eq!(p.health.health(), 10.0);
        // Under a perched dragon's wing nothing reaches the player.
        let mut p = horizon_client::motion::PlayerMotion::new(&resources, Vec3::ZERO).unwrap();
        for _ in 0..20 {
            wing(&mut p, true);
        }
        assert_eq!(p.velocity, Vec3::ZERO, "pushed by a perched dragon");
        assert!(!p.health.sync_velocity);
    }

    #[test]
    fn the_dragon_targets_any_player_it_can_attack() {
        let at = Vec3::new(0.0, 64.0, 0.0);
        // A spectating host and a creative guest are never targets: the
        // survival guest further away is.
        let players = [
            player(FightWho::Host, 1.0, false, true),
            player(FightWho::Guest(7), 2.0, true, false),
            player(FightWho::Guest(8), 30.0, false, false),
        ];
        let t = nearest_enemy(&players, false, at, None, |_| true);
        assert_eq!(t.map(|p| p.who), Some(FightWho::Guest(8)));
        // Out of a 20-block range, or filtered out, or on Peaceful: none.
        assert!(nearest_enemy(&players, false, at, Some((at, 20.0)), |_| true).is_none());
        assert!(nearest_enemy(&players, false, at, None, |p| p.position.x < 10.0).is_none());
        assert!(nearest_enemy(&players, true, at, None, |_| true).is_none());
        // The nearest of several, measured from where it looks (the egg).
        let players = [
            player(FightWho::Host, 5.0, false, false),
            player(FightWho::Guest(8), -3.0, false, false),
        ];
        let t = nearest_enemy(&players, false, at, None, |_| true);
        assert_eq!(t.map(|p| p.who), Some(FightWho::Guest(8)));
        let t = nearest_enemy(&players, false, Vec3::new(9.0, 64.0, 0.0), None, |_| true);
        assert_eq!(t.map(|p| p.who), Some(FightWho::Host));
    }
}
