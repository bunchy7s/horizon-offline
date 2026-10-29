//! The per-tick mob update in vanilla order (`Entity.baseTick`,
//! `LivingEntity.baseTick/tick/aiStep`, `Mob.serverAiStep`) and the
//! dispatch to each species family's goals and special behaviour.
use super::{
    Damage, DamageKind, Mob, MobEvent, MobWorld, Species, Who, aquatic,
    control::{BodyRotation, LookControl, MoveControl},
    farm,
    goals::{Cx, GoalSelector},
    hostile,
    nav::Navigation,
    path::{PATH_TYPES, Path},
    undead,
};
use crate::blocks::Fluid;
use horizon_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource},
};

#[derive(Clone, Debug, Default)]
pub struct Ai {
    /// `Mob.homePosition` and `homeRadius`: where a restricted mob stays
    /// (a lead's holder, a happy ghast's float area).
    pub home: Option<([i32; 3], i32)>,
    pub goals: GoalSelector,
    pub targets: GoalSelector,
    pub nav: Navigation,
    pub mov: MoveControl,
    pub look: LookControl,
    pub body: BodyRotation,
    /// JumpControl.jump (requested this tick).
    pub jump: bool,
    /// LivingEntity.jumping
    pub jumping: bool,
    pub no_jump_delay: i32,
    pub xxa: f32,
    pub yya: f32,
    pub zza: f32,
    pub speed: f32,
    pub target: Option<Who>,
    pub last_hurt_by: Option<Who>,
    pub last_hurt_by_time: i32,
    /// Ticks left during which kills count as player kills.
    pub last_hurt_by_player: i32,
    pub no_action_time: i32,
    pub aggressive: bool,
    pub panicking: bool,
    pub malus: [Option<f32>; PATH_TYPES],
    /// A path computed in canUse and consumed by start.
    pub pending_path: Option<Path>,
    /// Sensing: line-of-sight results cached for one tick.
    pub seen_cache: Vec<(Who, bool)>,
    /// Swing animation progress (attackAnim) and ticks.
    pub swing_time: i32,
    pub swinging: bool,
    /// The current swing's `SwingDescription.durationTicks`; 0 is the default 6.
    pub swing_duration: i32,
    /// Walked distance and the next footstep.
    pub steps: super::living::Steps,
}
impl Mob {
    /// `LivingEntity.getCurrentSwingDuration`: 6 ticks unless a swing set its own.
    pub fn swing_duration(&self) -> i32 {
        if self.ai.swing_duration > 0 {
            self.ai.swing_duration
        } else {
            6
        }
    }
    /// `SwingState.startIfAble`: a swing past the first half of the current
    /// one restarts it.
    fn start_swing(&mut self, duration: i32) -> bool {
        let half = self.swing_duration() / 2;
        if self.ai.swinging && self.ai.swing_time >= 0 && self.ai.swing_time < half {
            return false;
        }
        self.ai.swing_time = -1;
        self.ai.swinging = true;
        self.ai.swing_duration = duration.max(1);
        true
    }
    /// LivingEntity.swing
    pub fn swing(&mut self) {
        if self.start_swing(6) {
            self.extra.swing_right_arm = None;
            self.extra.swing_stab = false;
        }
    }
    /// `LivingEntity.swing(hand, animation, false)` for a swing of `duration`
    /// ticks (`getModifiedSwingDuration`): the arm is `hand.asArm(mainArm)`.
    pub fn swing_hand(&mut self, off_hand: bool, duration: i32) {
        if self.start_swing(duration) {
            self.extra.swing_right_arm = Some(!off_hand != self.main_arm_left());
            self.extra.swing_stab = false;
        }
    }
    /// `swing_hand` for a swing of `animation`, lengthened or shortened by
    /// this mob's haste, conduit power or mining fatigue
    /// (`LivingEntity.getModifiedSwingDuration`).
    pub fn swing_hand_modified(
        &mut self,
        off_hand: bool,
        animation: crate::first_person::SwingAnimation,
    ) {
        let amplifier = |id: &str| self.extra.effects.iter().find(|e| e.0 == id).map(|e| e.2);
        let dig_speed = amplifier("minecraft:haste").max(amplifier("minecraft:conduit_power"));
        let fatigue = amplifier("minecraft:mining_fatigue");
        let duration = crate::first_person::swing_duration(animation, dig_speed, fatigue);
        if self.start_swing(duration) {
            self.extra.swing_right_arm = Some(!off_hand != self.main_arm_left());
            self.extra.swing_stab = animation.kind == 2;
        }
    }
}
/// The module implementing a species' behaviour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Farm,
    Undead,
    Hostile,
    Aquatic,
    Generic,
    /// No goals at all (Giant).
    Inert,
    Witch,
    Enderman,
    Tame,
    /// Blazes and ghasts.
    Nether,
    Golem,
    Illager,
    /// Piglins, brutes, hoglins and zoglins.
    Piglin,
    Equine,
    Phantom,
    /// Slimes and magma cubes.
    Cube,
    Guardian,
    Shulker,
    Vex,
    Ravager,
    Wither,
    Allay,
    Villager,
    Trader,
    Breeze,
    Sniffer,
    HappyGhast,
    Warden,
    Creaking,
    /// Nautili and zombie nautili.
    Nautilus,
    Strider,
    Bat,
    Bee,
    Panda,
    PolarBear,
}
pub fn family(s: Species) -> Family {
    use Species as S;
    match s {
        S::Pig | S::Cow | S::Mooshroom | S::Sheep | S::Chicken => Family::Farm,
        S::Zombie
        | S::Husk
        | S::Drowned
        | S::ZombieVillager
        | S::Skeleton
        | S::Stray
        | S::Bogged
        | S::Parched
        | S::WitherSkeleton
        | S::ZombifiedPiglin => Family::Undead,
        S::Creeper | S::Spider | S::CaveSpider | S::Silverfish => Family::Hostile,
        S::Squid
        | S::GlowSquid
        | S::Cod
        | S::Tadpole
        | S::Salmon
        | S::TropicalFish
        | S::Pufferfish
        | S::Dolphin
        | S::Axolotl => Family::Aquatic,
        S::Giant => Family::Inert,
        S::Witch => Family::Witch,
        S::Wolf | S::Cat => Family::Tame,
        S::Blaze | S::Ghast => Family::Nether,
        S::IronGolem | S::SnowGolem | S::CopperGolem => Family::Golem,
        S::Pillager | S::Vindicator | S::Evoker | S::Illusioner => Family::Illager,
        S::Vex => Family::Vex,
        S::Ravager => Family::Ravager,
        S::Wither => Family::Wither,
        S::Allay => Family::Allay,
        S::Creaking => Family::Creaking,
        S::Villager => Family::Villager,
        S::WanderingTrader => Family::Trader,
        S::Breeze => Family::Breeze,
        S::Sniffer => Family::Sniffer,
        S::HappyGhast => Family::HappyGhast,
        S::Piglin | S::PiglinBrute | S::Hoglin | S::Zoglin => Family::Piglin,
        S::Horse
        | S::Donkey
        | S::Mule
        | S::SkeletonHorse
        | S::ZombieHorse
        | S::Camel
        | S::CamelHusk
        | S::Llama
        | S::TraderLlama => Family::Equine,
        S::Nautilus | S::ZombieNautilus => Family::Nautilus,
        S::Phantom => Family::Phantom,
        S::Enderman => Family::Enderman,
        S::Slime | S::MagmaCube | S::SulfurCube => Family::Cube,
        S::Guardian | S::ElderGuardian => Family::Guardian,
        S::Warden => Family::Warden,
        S::Shulker => Family::Shulker,
        S::Strider => Family::Strider,
        S::Bat => Family::Bat,
        S::Bee => Family::Bee,
        S::Panda => Family::Panda,
        S::PolarBear => Family::PolarBear,
        _ => Family::Generic,
    }
}
pub fn register(m: &mut Mob) {
    // Raider subclasses call super.registerGoals() first.
    super::raider::register(m);
    match family(m.species()) {
        Family::Farm => farm::register(m),
        Family::Undead => undead::register(m),
        Family::Hostile => hostile::register(m),
        Family::Aquatic => aquatic::register(m),
        // Turtle.registerGoals has its own list, none of the generic goals.
        Family::Generic if m.species() == Species::Turtle => super::turtle::register(m),
        Family::Generic if m.species() == Species::Armadillo => super::armadillo::register(m),
        Family::Generic if m.species() == Species::Frog => super::frog::register(m),
        Family::Generic if m.species() == Species::Fox => super::fox::register(m),
        Family::Generic if m.species() == Species::Parrot => super::tame::register_parrot(m),
        Family::Generic if m.species() == Species::Rabbit => super::rabbit::register(m),
        Family::Generic if m.species() == Species::Ocelot => super::tame::register_ocelot(m),
        Family::Generic => {
            generic(m);
            if m.species() == Species::Frog {
                // `Frog()`: SmoothSwimmingMoveControl(85, 10, 0.02, 0.1, true).
                // The speed attribute is 1.0, so the controller's scaling is
                // what keeps a frog at a walking pace on land.
                m.ai.mov.kind = super::control::MoveKind::SmoothSwimming {
                    max_turn_x: 85.0,
                    max_turn_y: 10.0,
                    in_water: 0.02,
                    outside: 0.1,
                    gravity: true,
                };
            }
            if m.species() == Species::Goat {
                super::goat::register(m);
            }
        }
        Family::Inert => {}
        Family::Witch => super::witch::register(m),
        Family::Tame if m.species() == Species::Cat => super::tame::register_cat(m),
        Family::Tame => super::tame::register(m),
        Family::Nether if m.species() == Species::Ghast => super::nether::register_ghast(m),
        Family::Nether => super::nether::register_blaze(m),
        Family::Golem if m.species() == Species::IronGolem => super::golem::register_iron(m),
        Family::Golem if m.species() == Species::CopperGolem => super::copper_golem::register(m),
        Family::Golem => super::golem::register_snow(m),
        Family::Illager if m.species() == Species::Pillager => super::illager::register_pillager(m),
        Family::Illager if m.species() == Species::Evoker => super::illager::register_evoker(m),
        Family::Illager if m.species() == Species::Illusioner => super::illusioner::register(m),
        Family::Illager => super::illager::register_vindicator(m),
        Family::Equine => super::equine::register(m),
        Family::Phantom => super::phantom::register(m),
        Family::Piglin => match m.species() {
            Species::Piglin => super::piglin::register_piglin(m),
            Species::PiglinBrute => super::piglin::register_brute(m),
            Species::Hoglin => super::piglin::register_hoglin(m),
            _ => super::piglin::register_zoglin(m),
        },
        Family::Enderman => super::enderman::register(m),
        Family::Cube => super::cube::register(m),
        Family::Guardian => super::guardian::register(m),
        Family::Shulker => super::shulker::register(m),
        Family::Vex => super::vex::register(m),
        Family::Ravager => super::ravager::register(m),
        Family::Wither => super::wither::register(m),
        Family::Allay => super::allay::register(m),
        Family::Villager => super::villager::register(m),
        Family::Trader => super::trader::register(m),
        Family::Breeze => super::breeze::register(m),
        Family::Sniffer => super::sniffer::register(m),
        Family::HappyGhast => super::happy_ghast::register(m),
        Family::Warden => super::warden::register(m),
        Family::Creaking => super::creaking::register(m),
        Family::Nautilus => super::nautilus::register(m),
        Family::Strider => super::strider::register(m),
        Family::Bat => super::bat::register(m),
        Family::Bee => super::bee::register(m),
        Family::Panda => super::panda::register(m),
        Family::PolarBear => super::polar_bear::register(m),
    }
    // FloatGoal's constructor lets the navigation float, as do the
    // constructors of these brain-driven and flying mobs. A floater rises
    // 0.04 a tick in Mob.jumpInLiquid, the others 0.3 (the bounce of #542).
    if m.ai.goals.has("Float")
        || matches!(
            m.species(),
            Species::Allay
                | Species::Armadillo
                | Species::Camel
                | Species::Goat
                | Species::Parrot
                | Species::Wither
        )
    {
        m.ai.nav.can_float = true;
    }
}
/// Mob.tick additions that run before the entity tick (creeper fuse, puffing).
fn pre_tick(m: &mut Mob, events: &mut Vec<MobEvent>) {
    if m.species() == Species::Wither && m.alive() {
        super::wither::steer(m);
    }
    if m.species() == Species::Breeze {
        super::breeze::tick(m, events);
    }
    match family(m.species()) {
        Family::Hostile => {
            if m.species() == Species::Creeper {
                hostile::creeper_tick(m, events)
            }
        }
        Family::Aquatic => aquatic::tick(m, events),
        _ => {}
    }
}
/// Move controls implemented by the species (MoveKind::Custom).
pub fn custom_move_control(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if m.species() == Species::Drowned {
        return undead::drowned_move_control(m, w);
    }
    if m.species() == Species::Ghast {
        return super::nether::ghast_move_control(m, w);
    }
    if m.species() == Species::HappyGhast {
        return super::happy_ghast::move_control(m, w);
    }
    if m.species() == Species::Phantom {
        return super::phantom::move_control(m);
    }
    if family(m.species()) == Family::Guardian {
        return super::guardian::move_control(m);
    }
    if m.species() == Species::Vex {
        return super::vex::move_control(m);
    }
    match family(m.species()) {
        Family::Aquatic => aquatic::fish_move_control(m),
        Family::Cube => super::cube::move_control(m, events),
        _ => {}
    }
}
/// A fallback for species without bespoke goals yet: animals stroll and
/// panic, monsters hunt the player in melee.
fn generic(m: &mut Mob) {
    use super::goals::*;
    let hostile = m.hostile();
    let g = &mut m.ai.goals;
    if hostile {
        g.add(1, Float);
        g.add(2, MeleeAttack::new(1.0, false));
        g.add(5, Stroll::avoiding_water(0.8));
        g.add(6, LookAtPlayer::new(8.0));
        g.add(6, RandomLookAround::default());
        m.ai.targets.add(1, HurtByTarget::new());
        m.ai.targets.add(2, NearestTarget::player(true));
    } else {
        if m.kind.species == Species::Goat {
            // GoatAi's idle and core behaviours (the ram and long jump
            // activities are not here): panic 2.0, AnimalMakeLove, tempting
            // with goat_food and baby follow at 1.25, then stroll.
            g.add(0, Float);
            g.add(1, Panic::new(2.0));
            g.add(2, Breed::new(1.0));
            g.add(3, Tempt::new(1.25, m.kind.food.clone(), false));
            g.add(4, FollowParent::new(1.25));
            g.add(5, Stroll::avoiding_water(1.0));
            g.add(6, LookAtPlayer::new(6.0));
            g.add(7, RandomLookAround::default());
            return;
        }
        g.add(0, Float);
        g.add(1, Panic::new(1.25));
        g.add(5, Stroll::avoiding_water(1.0));
        g.add(6, LookAtPlayer::new(6.0));
        g.add(7, RandomLookAround::default());
        if m.kind.species == Species::Armadillo {
            // Armadillo's constructor: pickNextScuteDropTime.
            m.extra.egg_time = next_scute_time(m);
        }
    }
}
/// Subclasses of `net.minecraft.world.entity.monster.Monster`.
pub fn is_monster(s: Species) -> bool {
    use Species as S;
    matches!(
        s,
        S::Zombie
            | S::Husk
            | S::Drowned
            | S::ZombieVillager
            | S::ZombifiedPiglin
            | S::Giant
            | S::Skeleton
            | S::Stray
            | S::Bogged
            | S::Parched
            | S::WitherSkeleton
            | S::Creeper
            | S::Spider
            | S::CaveSpider
            | S::Enderman
            | S::Endermite
            | S::Silverfish
            | S::Witch
            | S::Pillager
            | S::Vindicator
            | S::Evoker
            | S::Illusioner
            | S::Ravager
            | S::Vex
            | S::Piglin
            | S::PiglinBrute
            | S::Zoglin
            | S::Blaze
            | S::Guardian
            | S::ElderGuardian
            | S::Breeze
            | S::Creaking
            | S::Warden
            | S::Wither
    )
}
/// One full entity tick. Returns false when the mob is gone.
pub fn tick(
    m: &mut Mob,
    w: &MobWorld,
    level_random: &mut LegacyRandom,
    events: &mut Vec<MobEvent>,
) -> bool {
    if m.removed {
        return false;
    }
    // ServerLevel.tick checks despawning before ticking the entity.
    if !check_despawn(m, w) {
        m.removed = true;
        return false;
    }
    m.previous = m.position;
    m.previous_yaw = m.yaw;
    m.previous_body_yaw = m.body_yaw;
    m.previous_head_yaw = m.head_yaw;
    m.previous_head_pitch = m.head_pitch;
    m.previous_swell = m.swell;
    m.anim.begin_tick();
    m.age += 1;
    m.ai.seen_cache.clear();
    if m.ai.last_hurt_by_player > 0 {
        m.ai.last_hurt_by_player -= 1;
    }
    // Mob.tick pre-update hooks (Creeper.tick swells before super.tick).
    pre_tick(m, events);
    if m.removed {
        return false;
    }
    if m.species() == Species::ZombieVillager {
        // ZombieVillager.tick counts the cure down before super.tick, with
        // or without AI (NoAI only skips serverAiStep).
        super::zombie_villager::step(
            m,
            &mut Cx {
                w,
                lr: level_random,
                events,
            },
        );
    }
    super::warden::pre_tick(m, w, events);
    if m.species() == Species::Strider {
        super::strider::pre_tick(m, w, events);
    }
    base_tick(m, w, events);
    if m.species() == Species::Rabbit {
        super::rabbit::base_tick(m);
    }
    if m.death_time >= super::creaking::death_limit(m) {
        // Creaking.tickDeath: a heart's creaking crumbles away.
        if super::creaking::is_creaking(m) && super::creaking::tearing_down(m) {
            super::creaking::tear_down(m, events);
        }
        return false;
    }
    if m.no_ai {
        // Mob.aiStep still runs without AI (only serverAiStep is skipped),
        // so undead still burn in daylight and the endermite's life keeps
        // counting.
        burn_undead(m, w);
        if m.species() == Species::Endermite {
            endermite_lifetime(m);
        }
    } else {
        ai_step(m, w, level_random, events);
    }
    effects_from_blocks(m, w);
    if m.species() == Species::Rabbit {
        super::rabbit::post_tick(m);
    }
    // Mob.tickHeadTurn -> BodyRotationControl.clientTick (a frozen
    // creaking's waits).
    if m.alive()
        && (!super::creaking::is_creaking(m) || super::creaking::can_move(m))
        && !super::armadillo::scared(m)
    {
        m.body_rotation_tick();
    }
    events.append(&mut m.outbox);
    m.update_walk_animation();
    super::axolotl::tick_animation(m);
    m.anim.end_tick(m.ai.swinging);
    tick_swing(m);
    flower_contact_effects(m, w);
    if m.species() == Species::CopperGolem {
        // CopperGolem.tick: updateWeathering after the living tick.
        super::copper_golem::update_weathering(m, w, level_random, events);
    }
    if m.species() == Species::Strider {
        super::strider::float(m, w);
    }
    if m.species() == Species::Bat {
        super::bat::tick(m);
    }
    if m.species() == Species::Bee {
        super::bee::tick(m, level_random);
    }
    if m.species() == Species::Panda {
        super::panda::tick(m, w, events);
    }
    true
}
/// `LivingEntity.aiStep` -> `Entity.applyEffectsFromBlocks` for the blocks
/// that hurt or hold a mob: the cactus (`CactusBlock.entityInside`, 1), the
/// cobweb (stuck at 0.25, 0.05, 0.25) and the sweet berry bush
/// (`SweetBerryBushBlock.entityInside`: stuck at 0.8, 0.75, 0.8, and a grown bush pricks for 1 when the mob moved at least 0.003 on X
/// or Z this tick). The blocks are those the mob's box touches anywhere along
/// its move this tick, from where it started to where it ended.
fn effects_from_blocks(m: &mut Mob, w: &MobWorld) {
    let mut visited: Vec<[i32; 3]> = Vec::new();
    let travel = m.position.subtract(m.previous);
    let longest = travel.x.abs().max(travel.y.abs()).max(travel.z.abs());
    let steps = ((longest / 0.2).ceil() as i32).clamp(1, 100);
    for step in 0..=steps {
        if !m.alive() {
            return;
        }
        let at = m
            .previous
            .add_vector(travel.scale(f64::from(step) / f64::from(steps)));
        let b = m.bounds_at(at).deflate(Vec3::new(1e-5, 1e-5, 1e-5));
        for x in b.min_position().x.floor() as i32..=b.max_position().x.floor() as i32 {
            for y in b.min_position().y.floor() as i32..=b.max_position().y.floor() as i32 {
                for z in b.min_position().z.floor() as i32..=b.max_position().z.floor() as i32 {
                    let p = [x, y, z];
                    if visited.contains(&p) {
                        continue;
                    }
                    match w.block_name(x, y, z) {
                        "minecraft:cactus" => {
                            m.damage(1.0, Damage::of(DamageKind::Cactus));
                        }
                        // WebBlock.entityInside -> makeStuckInBlock, which a
                        // spider ignores for cobwebs and a wither always.
                        "minecraft:cobweb"
                            if !matches!(
                                m.species(),
                                Species::Spider | Species::CaveSpider | Species::Wither
                            ) =>
                        {
                            m.fall_distance = 0.0;
                            m.stuck_multiplier = if m.has_effect("minecraft:weaving") {
                                Vec3::new(0.5, 0.25, 0.5)
                            } else {
                                Vec3::new(0.25, f64::from(0.05_f32), 0.25)
                            };
                        }
                        "minecraft:sweet_berry_bush"
                            if !matches!(m.species(), Species::Fox | Species::Bee) =>
                        {
                            m.fall_distance = 0.0;
                            m.stuck_multiplier =
                                Vec3::new(f64::from(0.8_f32), 0.75, f64::from(0.8_f32));
                            let moved = m.previous.subtract(m.position);
                            if w.prop(x, y, z, "age") != Some("0")
                                && moved.horizontal_distance_squared() > 0.0
                                && (moved.x.abs() >= f64::from(0.003_f32)
                                    || moved.z.abs() >= f64::from(0.003_f32))
                            {
                                m.damage(1.0, Damage::of(DamageKind::SweetBerryBush));
                            }
                        }
                        _ => continue,
                    }
                    visited.push(p);
                }
            }
        }
    }
}
/// LivingEntity.updateSwingTime.
pub fn tick_swing(m: &mut Mob) {
    let duration = m.swing_duration();
    if m.ai.swinging {
        m.ai.swing_time += 1;
        if m.ai.swing_time >= duration {
            m.ai.swing_time = 0;
            m.ai.swinging = false;
            m.ai.swing_duration = 0;
            m.extra.swing_right_arm = None;
            m.extra.swing_stab = false;
        }
    } else {
        m.ai.swing_time = 0;
    }
    m.anim.attack_anim = m.ai.swing_time as f32 / duration as f32;
}
/// `Entity.isInRain`: rain at the block position, or at the top of the box.
pub(super) fn in_rain(m: &Mob, w: &MobWorld) -> bool {
    let (x, y, z) = (
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    );
    let top = (m.position.y + f64::from(m.height())).floor() as i32;
    w.raining_at(x, y, z) || w.raining_at(x, top, z)
}
/// `LivingEntity.baseTick` refills a breather's air by 4, then
/// `Axolotl.handleAirSupply` (unless the mob has no AI): water or rain fills
/// it, anywhere else it runs down from the pre-tick value, and at -20 the
/// axolotl dries out.
fn axolotl_air_supply(m: &mut Mob, w: &MobWorld) {
    let before = m.air;
    m.air = (m.air + 4).min(super::axolotl::MAX_AIR);
    if m.no_ai {
        return;
    }
    if m.fluids.in_water || in_rain(m, w) {
        m.air = super::axolotl::MAX_AIR;
    } else {
        m.air = before - 1;
        if m.air <= -20 {
            m.air = 0;
            m.damage(2.0, Damage::of(DamageKind::DryOut));
        }
    }
}
/// Entity.baseTick + LivingEntity.baseTick + Mob.baseTick.
fn base_tick(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    // Entity.canSpawnSprintParticle / spawnSprintParticle (a dashing cat or
    // ocelot): a server level drops the particle, but its spot is drawn
    // from the mob's random first.
    if m.extra.sprinting && !m.fluids.in_water && !m.fluids.in_lava && m.alive() {
        let at = m.block_on_legacy(w);
        let state = w.world.block(at[0], at[1], at[2]);
        if w.facts.get(state).render_shape != crate::blocks::RenderShape::Invisible {
            let _ = (m.random.next_double(), m.random.next_double());
        }
    }
    m.was_eye_in_water = m.fluids.eye_in_water;
    m.update_fluids(w, events);
    if m.species() == Species::Drowned {
        super::undead::drowned_update_swimming(m, w);
    }
    let fire = crate::motion::environment::fire_contact(w.world, w.registry, m.bounds());
    if fire > 0.0 {
        m.ticks_frozen = 0;
    }
    if fire > 0.0 && !m.kind.flags.fire_immune {
        let multiplier = m.burning_time();
        m.fire_ticks = crate::motion::environment::ignite_contact(
            m.fire_ticks,
            false,
            &mut m.random,
            multiplier,
        );
        m.damage(fire, Damage::of(DamageKind::InFire));
    }
    // Fire: water or rain (Entity.isInRain) extinguishes, lava ignites,
    // burning hurts every second.
    if m.fire_ticks > 0 && (m.fluids.in_water || in_rain(m, w)) {
        m.fire_ticks = 0;
        events.push(MobEvent::Sound(
            "entity.generic.extinguish_fire",
            m.center(),
            0.7,
            1.6 + (m.random.next_float() - m.random.next_float()) * 0.4,
        ));
    }
    if m.fluids.in_lava && !m.kind.flags.fire_immune {
        m.ignite_for_ticks(300);
        m.damage(4.0, Damage::of(DamageKind::Lava));
    }
    if m.fire_ticks > 0 {
        if m.kind.flags.fire_immune {
            m.fire_ticks = 0;
        } else {
            if m.fire_ticks % 20 == 0 && !m.fluids.in_lava {
                m.damage(1.0, Damage::of(DamageKind::OnFire));
            }
            m.fire_ticks -= 1;
        }
    }
    // Entity.checkBelowWorld: LivingEntity.onBelowWorld hurts 4 a tick.
    if m.position.y < f64::from(crate::chunk::MIN_Y - 64) {
        m.damage(4.0, Damage::of(DamageKind::OutOfWorld));
    }
    m.first_tick = false;
    if m.alive() {
        // LivingEntity.baseTick: suffocation inside full blocks. Entity.isInWall
        // is false under noPhysics, which Vex.tick holds while it ticks.
        let (ex, ey, ez) = (
            m.position.x.floor() as i32,
            m.eye_y().floor() as i32,
            m.position.z.floor() as i32,
        );
        let at_eye = w.facts.get(w.world.block(ex, ey, ez));
        if at_eye.full_collision
            && at_eye.solid_render
            && m.width() < 1.0
            && m.species() != Species::Vex
        {
            m.damage(1.0, Damage::of(DamageKind::InWall));
        }
        // Air supply and drowning (WaterAnimal.handleAirSupply for aquatic mobs).
        if m.species() == Species::Axolotl {
            axolotl_air_supply(m, w);
        } else if m.kind.flags.needs_water {
            if !m.fluids.in_water {
                m.air -= 1;
                if m.air == -20 {
                    m.air = 0;
                    m.damage(2.0, Damage::of(DamageKind::Drown));
                }
            } else {
                m.air = 300;
            }
        } else if m.fluids.eye_in_water && !m.kind.flags.breathes_water {
            m.air -= 1;
            if m.air == -20 {
                m.air = 0;
                events.push(MobEvent::Drown {
                    mob: m.id,
                    at: [m.position.x, m.position.y, m.position.z],
                });
                m.damage(2.0, Damage::of(DamageKind::Drown));
            }
        } else if m.air < 300 {
            m.air = (m.air + 4).min(300);
        }

        if m.kind.flags.water_sensitive && (m.fluids.in_water || in_rain(m, w)) {
            m.damage(1.0, Damage::of(DamageKind::Drown));
        }
        freeze_tick(m, w);
    }
    tick_effects(m);
    if m.species() == Species::Illusioner {
        super::illusioner::tick(m, events);
    }
    if m.hurt_time > 0 {
        m.hurt_time -= 1;
    }
    if m.invulnerable > 0 {
        m.invulnerable -= 1;
    }
    if !m.alive() {
        m.death_time += 1;
    }
    // Hurt sounds of the damage above come before the ambient sound.
    events.append(&mut m.outbox);
    // Mob.baseTick: ambient sounds.
    if m.alive() {
        let roll = m.random.next_int_bound(1000).unwrap_or(999);
        let t = m.ambient_timer;
        m.ambient_timer += 1;
        if roll < t {
            m.ambient_timer = -m.ambient_interval();
            play_ambient(m, w, events);
        }
    }
    // Entity.baseTick ends with Leashable.tickLeash.
    super::leash::tick(m, w, events);
}
/// `LivingEntity.aiStep`'s "freezing" with `InsideBlockEffectType.FREEZE`:
/// powder snow builds `ticks_frozen` up, anywhere else it melts twice as
/// fast, and a fully frozen mob takes one freeze damage every 40 ticks.
fn freeze_tick(m: &mut Mob, w: &MobWorld) {
    let in_snow = in_powder_snow(m, w);
    if !in_snow && m.ticks_frozen == 0 {
        return;
    }
    let can_freeze = can_freeze(m, w);
    if in_snow && can_freeze {
        m.ticks_frozen =
            (m.ticks_frozen + 1).min(crate::motion::PlayerMotion::TICKS_REQUIRED_TO_FREEZE);
    } else {
        m.ticks_frozen = (m.ticks_frozen - 2).max(0);
    }
    if m.age.is_multiple_of(40) && m.is_fully_frozen() && can_freeze {
        m.damage(1.0, Damage::of(DamageKind::Freeze));
    }
}
/// Whether the mob's box (shrunk by `1.0E-5`, as in `Entity.checkInsideBlocks`)
/// touches powder snow.
fn in_powder_snow(m: &Mob, w: &MobWorld) -> bool {
    let b = m.bounds().deflate(Vec3::new(1e-5, 1e-5, 1e-5));
    let (min, max) = (b.min_position(), b.max_position());
    for x in min.x.floor() as i32..=max.x.floor() as i32 {
        for y in min.y.floor() as i32..=max.y.floor() as i32 {
            for z in min.z.floor() as i32..=max.z.floor() as i32 {
                if &*w.facts.get(w.world.block(x, y, z)).name == "minecraft:powder_snow" {
                    return true;
                }
            }
        }
    }
    false
}
/// `LivingEntity.canFreeze`: not a `#freeze_immune_entity_types` mob and no
/// worn piece in `#freeze_immune_wearables`.
fn can_freeze(m: &Mob, w: &MobWorld) -> bool {
    if w.data
        .entity_has("minecraft:freeze_immune_entity_types", &m.kind.id)
    {
        return false;
    }
    let immune = w
        .items
        .and_then(|i| i.item_tags.get("minecraft:freeze_immune_wearables"));
    ![&m.extra.head, &m.extra.chest, &m.extra.legs, &m.extra.feet]
        .into_iter()
        .flatten()
        .any(|item| immune.is_some_and(|t| t.contains(&**item)))
}
/// `Mob.playAmbientSound` on demand: a spawn egg or a mob bucket just placed `m`.
pub fn ambient_sound(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    play_ambient(m, w, events);
}
fn play_ambient(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if super::axolotl::is_playing_dead(m) {
        return;
    }
    let sound = match m.species() {
        // Warden.getAmbientSound: by anger, silent while roaring or digging.
        Species::Warden => match super::warden::ambient_sound(m) {
            Some(sound) => sound,
            None => return,
        },
        Species::Zombie | Species::Husk if m.is_baby() => m.kind.sounds[0],
        Species::Drowned if m.fluids.in_water => "entity.drowned.ambient_water",
        // Nautilus/ZombieNautilus.getAmbientSound: by age, under water or on land.
        Species::Nautilus | Species::ZombieNautilus => super::nautilus::sounds(m)[0],
        Species::Enderman if m.ai.target.is_some() => "entity.enderman.scream",
        Species::Wolf if m.ai.target.is_some() => {
            super::finalize::variant_sound(m, "growl").unwrap_or("entity.wolf.growl")
        }
        Species::Wolf if m.random.next_int_bound(3).unwrap_or(1) == 0 => {
            let kind = if m.extra.tame && m.health < m.attrs.max_health / 2.0 {
                "whine"
            } else {
                "pant"
            };
            super::finalize::variant_sound(m, kind).unwrap_or("entity.wolf.pant")
        }
        Species::Parrot => super::parrot::ambient(m, w),
        // Cat.getAmbientSound: purr in love, purreow one time in four, else ambient.
        Species::Cat if m.extra.tame && m.extra.in_love > 0 => {
            super::finalize::variant_sound(m, "purr").unwrap_or("entity.cat.purr")
        }
        Species::Cat if m.extra.tame && m.random.next_int_bound(4).unwrap_or(1) == 0 => {
            super::finalize::variant_sound(m, "purreow").unwrap_or("entity.cat.purreow")
        }
        Species::Cat if !m.extra.tame => {
            super::finalize::variant_sound(m, "stray_ambient").unwrap_or("entity.cat.stray_ambient")
        }
        Species::PolarBear if m.is_baby() => "entity.polar_bear.ambient_baby",
        Species::Dolphin if !m.fluids.in_water => "entity.dolphin.ambient",
        Species::Guardian if !m.fluids.in_water => "entity.guardian.ambient_land",
        Species::ElderGuardian if !m.fluids.in_water => "entity.elder_guardian.ambient_land",
        Species::Axolotl if !m.fluids.in_water => "entity.axolotl.idle_air",
        Species::Bat if m.extra.flag && m.random.next_int_bound(4).unwrap_or(1) != 0 => return,
        Species::Allay if m.extra.main_hand.is_some() => "entity.allay.ambient_with_item",
        // Strider.getAmbientSound: quiet while panicking or tempted.
        Species::Strider if m.ai.panicking || m.ai.goals.is_running("Tempt") => return,
        // HappyGhast: its own voice, never pitched up.
        Species::HappyGhast => {
            let sound = super::happy_ghast::sounds(m)[0];
            events.push(MobEvent::Sound(
                sound,
                m.center(),
                super::happy_ghast::volume(m),
                1.0,
            ));
            return;
        }
        // Sniffer.getAmbientSound.
        Species::Sniffer => match super::sniffer::ambient(m) {
            Some(s) => s,
            None => return,
        },
        // Breeze.playAmbientSound: a local sound, nothing drawn.
        Species::Breeze => {
            if let Some(s) = super::breeze::ambient(m) {
                let at = [m.position.x, m.position.y, m.position.z];
                events.push(MobEvent::LocalSound(s, at, 1.0, 1.0));
            }
            return;
        }
        // Creaking.getAmbientSound: silent while awake.
        Species::Creaking if m.extra.flag => return,
        // Shulker.playAmbientSound: silent while closed.
        Species::Shulker if m.extra.counter == 0 => return,
        Species::ZombifiedPiglin if m.ai.target.is_some() => "entity.zombified_piglin.angry",
        Species::Piglin if m.ai.target.is_some() => "entity.piglin.angry",
        Species::Hoglin if m.ai.target.is_some() => "entity.hoglin.angry",
        _ => voice_sounds(m)[0],
    };
    let _ = w;
    if !sound.is_empty() {
        m.voice(sound, events);
    }
}
/// LivingEntity.aiStep (and Mob/AgeableMob/Animal additions).
fn ai_step(m: &mut Mob, w: &MobWorld, level_random: &mut LegacyRandom, events: &mut Vec<MobEvent>) {
    if m.species() == Species::Parrot {
        super::parrot::ai_step(m, w, level_random, events);
    }
    // Witch.aiStep runs before super.aiStep().
    if m.species() == Species::Witch {
        let far = m
            .target_info(w)
            .is_some_and(|t| t.position.distance_squared(m.position) > 121.0);
        super::witch::ai_step(m, events, far);
    }
    if m.species() == Species::Fox {
        super::fox::pre_ai_step(m);
    }
    // AbstractHorse.aiStep: the tail swishes now and then (moveTail).
    if super::equine::is_equine(m.species()) {
        super::equine::tail_tick(m);
    }
    // Monster.aiStep: updateNoActionTime (Raider: always +2).
    if is_monster(m.species())
        && (super::raider::is_raider(m.species()) || super::hostile::mob_brightness(m, w) > 0.5)
    {
        m.ai.no_action_time += 2;
    }
    if super::creaking::is_creaking(m) {
        super::creaking::update(m, w, events);
    }
    if m.ai.no_jump_delay > 0 {
        m.ai.no_jump_delay -= 1;
    }
    let v = m.velocity;
    m.velocity = Vec3::new(
        if v.x.abs() < 0.003 { 0.0 } else { v.x },
        if v.y.abs() < 0.003 { 0.0 } else { v.y },
        if v.z.abs() < 0.003 { 0.0 } else { v.z },
    );
    m.ai.xxa *= 0.98;
    m.ai.zza *= 0.98;
    // LivingEntity.isImmobile: no goals or input (a biting, stunned or
    // roaring ravager, an eating or rearing horse). A mannequin is an Avatar, not a Mob: LivingEntity's
    // serverAiStep has no goals, so it only falls and is pushed.
    let immobile = (m.species() == Species::Ravager && super::ravager::immobile(m))
        || super::equine::immobile(m)
        || m.species() == Species::Mannequin;
    if m.species() == Species::Piglin && m.alive() {
        super::piglin::celebrate(m, w);
    }
    if m.alive() && !immobile {
        server_ai_step(m, w, level_random, events);
    } else {
        m.ai.jumping = false;
        m.ai.xxa = 0.0;
        m.ai.zza = 0.0;
    }
    m.handle_jumping(w);
    let input = Vec3::new(
        f64::from(m.ai.xxa),
        f64::from(m.ai.yya),
        f64::from(m.ai.zza),
    );
    m.travel(w, input, events);
    push_entities(m, w, events);
    // Spider.tick: climbing while pressed against a wall.
    if m.kind.flags.climber {
        m.climbing = m.horizontal_collision;
    }
    // AgeableMob.aiStep: a forced age-up's particles (`sendParticles`, so the
    // players are sent them), then the age.
    if m.extra.forced_age_timer > 0 {
        if m.extra.forced_age_timer % 4 == 0 {
            let (w, h) = (f64::from(m.width()), f64::from(m.height()));
            let r = &mut m.random;
            let x = m.position.x + w * (2.0 * r.next_double() - 1.0);
            let y = m.position.y + h * r.next_double() + 0.5;
            let z = m.position.z + w * (2.0 * r.next_double() - 1.0);
            // sendParticles(HAPPY_VILLAGER, x, y, z, 1, 0, 0, 0, 0): exactly there, at rest.
            events.push(MobEvent::LevelParticles {
                kind: "happy_villager",
                at: [x, y, z],
                spread: [0.0; 3],
                speed: 0.0,
                count: 1,
            });
        }
        m.extra.forced_age_timer -= 1;
    }
    if m.alive() {
        if m.extra.age < 0 {
            let (bw, bh, _) = m.dims();
            m.extra.age += 1;
            // setAge at the age boundary: refreshDimensions.
            if m.extra.age >= 0 {
                m.fudge_position_after_size_change(w, bw, bh);
                // Turtle.ageBoundaryReached: a grown turtle drops a scute.
                if m.species() == Species::Turtle {
                    events.push(MobEvent::GiftLoot {
                        mob: m.id,
                        table: "minecraft:gameplay/turtle_grow",
                        at: [m.position.x, m.position.y, m.position.z],
                    });
                }
            }
        } else if m.extra.age > 0 {
            m.extra.age -= 1;
        }
        // Animal.aiStep
        if m.extra.age != 0 {
            m.extra.in_love = 0;
        }
        if m.extra.in_love > 0 {
            m.extra.in_love -= 1;
            if m.extra.in_love % 10 == 0 {
                // The heart's speed and getRandomX/Y/Z, drawn even on the server.
                let (w, h) = (f64::from(m.width()), f64::from(m.height()));
                let r = &mut m.random;
                let velocity = [
                    r.next_gaussian() * 0.02,
                    r.next_gaussian() * 0.02,
                    r.next_gaussian() * 0.02,
                ];
                let x = m.position.x + w * (2.0 * r.next_double() - 1.0);
                let y = m.position.y + h * r.next_double() + 0.5;
                let z = m.position.z + w * (2.0 * r.next_double() - 1.0);
                events.push(MobEvent::ParticleAt {
                    kind: "heart",
                    at: [x, y, z],
                    velocity,
                });
            }
        }
    }
    burn_undead(m, w);
    species_ai_step(m, w, level_random, events);
}
impl Mob {
    /// LivingEntity.isPushable (and the push it takes from `Entity.push(Entity)`):
    /// bats and shulkers are never moved by others.
    pub fn pushable(&self) -> bool {
        self.alive()
            && !matches!(self.species(), Species::Bat | Species::Shulker)
            && (!self.kind.flags.climber || !self.climbing)
            && (self.species() != Species::Creaking || super::creaking::can_move(self))
    }
}
/// LivingEntity.onClimbable for the pusher (Spider.onClimbable: climbing a wall).
fn on_climbable(m: &Mob, w: &MobWorld) -> bool {
    if m.kind.flags.climber {
        return m.climbing;
    }
    let (x, y, z) = (
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    );
    w.has_tag("minecraft:climbable", w.block_name(x, y, z))
}
/// LivingEntity.pushEntities: every pushable entity overlapping the box is
/// pushed apart from this one (`doPush(e)` = `e.push(this)`); pushes of other
/// mobs are `Push` events their tick picks up.
fn push_entities(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if m.species() == Species::Bat {
        return;
    }
    let bounds = m.bounds();
    // The pusher's own isPushable, checked by `e.push(this)` before moving it.
    let self_pushable = m.alive()
        && m.species() != Species::Shulker
        && !on_climbable(m, w)
        && (m.species() != Species::Creaking || super::creaking::can_move(m));
    // Each with whether it rides a vehicle (`Entity.isPassenger`).
    let mut targets: Vec<(Who, Vec3, bool)> = Vec::new();
    if let Some(p) = &w.player
        // Entity.push: passengers of the same vehicle never push each other.
        && m.rider.is_none()
        && p.alive
        && !p.spectator
        && p.bounds.intersects(bounds)
    {
        targets.push((Who::Player, p.position, p.passenger));
    }
    for o in w.others_near(bounds) {
        if o.id != m.id && o.alive && o.pushable && o.bounds.intersects(bounds) {
            targets.push((Who::Mob(o.id), o.position, o.passenger));
        }
    }
    // EntitySelector.pushableBy(this): the teams' collision rules.
    let own = w.teams.of(Who::Mob(m.id));
    targets.retain(|(who, ..)| crate::scoreboard::pushable_by(own, w.teams.of(*who), true));
    // Cramming: a quarter of the ticks spent with `maxEntityCramming` or more
    // others around that are not riding anything hurts it by 6.
    let limit = w.max_entity_cramming;
    if limit > 0
        && targets.len() as i64 > i64::from(limit) - 1
        && m.random.next_int_bound(4).unwrap_or(1) == 0
        && targets.iter().filter(|t| !t.2).count() as i64 > i64::from(limit) - 1
    {
        m.damage(6.0, Damage::of(DamageKind::Cramming));
    }
    for (who, at, _) in targets {
        // Warden.doPush: touching it angers it, then the push itself.
        if m.species() == Species::Warden {
            super::warden::do_push(m, w, who, events);
        }
        // Entity.push(Entity) with `this` the other entity.
        let (mut xa, mut za) = (m.position.x - at.x, m.position.z - at.z);
        let mut dd = xa.abs().max(za.abs());
        if dd < f64::from(0.01_f32) {
            continue;
        }
        dd = dd.sqrt();
        xa /= dd;
        za /= dd;
        let pow = (1.0 / dd).min(1.0);
        xa *= pow;
        za *= pow;
        xa *= f64::from(0.05_f32);
        za *= f64::from(0.05_f32);
        // A guest's server-side push never reaches its client (ServerPlayer
        // motion is synced only when hurt): its client pushes it itself.
        if who != Who::Player || w.player.as_ref().is_none_or(|p| !p.remote) {
            events.push(MobEvent::Push {
                target: who,
                velocity: [-xa, 0.0, -za],
            });
        }
        if self_pushable {
            m.velocity = Vec3::new(m.velocity.x + xa, m.velocity.y, m.velocity.z + za);
        }
    }
}
/// `Mob.aiStep` -> `Mob.burnUndead`: undead burn in daylight. It runs with
/// or without AI (`NoAI` only skips `serverAiStep`).
fn burn_undead(m: &mut Mob, w: &MobWorld) {
    if m.kind.flags.burns_in_day && m.alive() && sun_burn_tick(m, w) && m.extra.head.is_none() {
        m.ignite_for_ticks(160);
    }
}
fn sun_burn_tick(m: &mut Mob, w: &MobWorld) -> bool {
    // Mob.isSunBurnTick: the monsters_burn attribute, not the sky darkness.
    if !w.monsters_burn {
        return false;
    }
    let (x, y, z) = (
        m.position.x.floor() as i32,
        m.eye_y().floor() as i32,
        m.position.z.floor() as i32,
    );
    let br = w.brightness(x, y, z);
    let wet = m.fluids.in_water || in_rain(m, w);
    br > 0.5 && m.random.next_float() * 30.0 < (br - 0.4) * 2.0 && !wet && w.can_see_sky(x, y, z)
}
/// Mob.serverAiStep
fn server_ai_step(
    m: &mut Mob,
    w: &MobWorld,
    level_random: &mut LegacyRandom,
    events: &mut Vec<MobEvent>,
) {
    m.ai.no_action_time += 1;
    // A happy ghast set up for its age (ageBoundaryReached runs at once).
    if m.species() == Species::HappyGhast {
        super::happy_ghast::ensure_setup(m);
    }
    // A saddled mount follows its rider instead of its goals.
    if super::equine::controlled(m) {
        super::equine::ridden_step(m);
        return;
    }
    super::riders::drive(m, w);
    let tick_count = m.age as i64;
    let id_based = tick_count + i64::from(m.id);
    let mut targets = std::mem::take(&mut m.ai.targets);
    let mut goals = std::mem::take(&mut m.ai.goals);
    {
        let mut cx = Cx {
            w,
            lr: level_random,
            events,
        };
        if id_based % 2 != 0 && tick_count > 1 {
            targets.tick_running(m, &mut cx, false);
            goals.tick_running(m, &mut cx, false);
        } else {
            targets.tick(m, &mut cx);
            goals.tick(m, &mut cx);
        }
    }
    m.ai.targets = targets;
    m.ai.goals = goals;
    // A creaking looked at stands still: its navigation, move, look and jump
    // controls wait (CreakingPathNavigation and the Creaking*Control).
    let frozen = super::creaking::is_creaking(m) && !super::creaking::can_move(m);
    // The bee's navigation waits while it pollinates.
    if !frozen && !super::bee::pollinating(m) {
        m.nav_tick(w);
    }
    if super::monster_brain::has_brain(m.species()) {
        // Breeze/Creaking.customServerAiStep: the brain, then updateActivity.
        super::monster_brain::tick(m, w, level_random, events);
    }
    if m.species() == Species::Villager {
        // Villager.customServerAiStep: the brain, then the raid sound roll.
        super::villager_brain::tick(m, w, level_random, events);
        let _ = m.random.next_int_bound(100);
    }
    if m.species() == Species::Warden {
        // Warden.customServerAiStep: the brain and its bookkeeping.
        super::warden::server_ai_step(m, w, level_random, events);
    }
    // Sniffer and HappyGhast.customServerAiStep: the brain (a happy ghast's
    // only while it is a ghastling), then updateActivity.
    if m.species() == Species::Sniffer
        || m.species() == Species::HappyGhast && m.is_baby()
        || m.species() == Species::CopperGolem
        || m.species() == Species::Axolotl
    {
        super::animal_brain::tick(m, w, level_random, events);
    }
    {
        let mut cx = Cx {
            w,
            lr: level_random,
            events,
        };
        custom_server_ai_step(m, &mut cx);
    }
    if frozen {
        m.ai.jumping = false;
        return;
    }
    if !super::axolotl::is_playing_dead(m) {
        m.move_control_tick(w, events);
        // BeeLookControl: still while angry.
        if !(m.species() == Species::Bee && super::bee::angry(m, w)) {
            m.look_control_tick();
        }
    }
    m.jump_control_tick();
}
/// `Armadillo.pickNextScuteDropTime`: 5 to 10 minutes.
fn next_scute_time(m: &mut Mob) -> i32 {
    m.random.next_int_bound(20 * 60 * 5).unwrap_or(0) + 20 * 60 * 5
}
/// `Armadillo.customServerAiStep`: the scute timer keeps counting down and is
/// only reset once `shouldDropLoot` (not a baby, `mob_drops` on) lets it shed.
fn armadillo_server_ai_step(m: &mut Mob, mob_drops: bool, events: &mut Vec<MobEvent>) {
    if !m.alive() {
        return;
    }
    m.extra.egg_time -= 1;
    if m.extra.egg_time <= 0 && !m.is_baby() && mob_drops {
        let pitch = (m.random.next_float() - m.random.next_float()) * 0.2 + 1.0;
        events.push(MobEvent::ShedScute {
            mob: m.id,
            at: [m.position.x, m.position.y, m.position.z],
            pitch,
        });
        m.extra.egg_time = next_scute_time(m);
    }
}
fn custom_server_ai_step(m: &mut Mob, cx: &mut Cx) {
    match m.species() {
        Species::Rabbit => super::rabbit::server_ai_step(m),
        Species::Cat | Species::Ocelot => super::tame::feline_server_ai_step(m),
        // Armadillo.customServerAiStep: the brain and its activity, then the scute.
        Species::Armadillo => {
            super::armadillo::server_ai_step(m, cx);
            armadillo_server_ai_step(m, cx.w.mob_drops, cx.events);
        }
        Species::Sheep => {}
        Species::Zombie
        | Species::Husk
        | Species::Drowned
        | Species::ZombieVillager
        | Species::Skeleton
        | Species::Stray
        | Species::Bogged
        | Species::Parched
        | Species::WitherSkeleton
        | Species::ZombifiedPiglin => undead::custom_server_ai_step(m, cx),
        Species::Vindicator => super::illager::vindicator_server_ai_step(m),
        Species::Enderman => super::enderman::custom_server_ai_step(m, cx),
        Species::Blaze => super::nether::blaze_server_ai_step(m, cx),
        Species::ElderGuardian => super::guardian::elder_server_ai_step(m, cx),
        Species::Wither if m.alive() => super::wither::custom_server_ai_step(m, cx.w, cx.events),
        // SpellcasterIllager.customServerAiStep.
        Species::Evoker if m.extra.counter > 0 => m.extra.counter -= 1,
        Species::Bat => super::bat::server_ai_step(m, cx.w, cx.events),
        Species::Bee => super::bee::server_ai_step(m, cx),
        Species::Frog => super::frog::server_ai_step(m, cx.w),
        _ => {}
    }
}
/// Endermite.aiStep: `counter` is the life; an endermite not required to
/// persist is discarded at 2400.
fn endermite_lifetime(m: &mut Mob) {
    if !m.persistent {
        m.extra.counter += 1;
    }
    if m.extra.counter >= 2400 {
        m.removed = true;
    }
}
fn species_ai_step(
    m: &mut Mob,
    w: &MobWorld,
    level_random: &mut LegacyRandom,
    events: &mut Vec<MobEvent>,
) {
    if family(m.species()) == Family::Aquatic {
        aquatic::ai_step(m, w, events);
    }
    match m.species() {
        Species::Enderman => super::enderman::ai_step(m, w, events),
        // Endermite.aiStep: `counter` is the life; an endermite not required
        // to persist is discarded at 2400.
        Species::Endermite => endermite_lifetime(m),
        Species::Wolf => super::tame::ai_step(m, w, events),
        Species::Cat => super::tame::cat_ai_step(m, events),
        Species::Blaze => super::nether::blaze_ai_step(m, events),
        Species::IronGolem => super::golem::iron_ai_step(m),
        Species::Piglin | Species::PiglinBrute | Species::Hoglin => {
            super::piglin::ai_step(m, w, events)
        }
        Species::Zoglin if m.extra.anim_ticks > 0 => m.extra.anim_ticks -= 1,
        s if super::equine::is_equine(s) => super::equine::ai_step(m, w, events),
        Species::Phantom => super::phantom::flap_sound(m, events),
        Species::Warden => super::warden::ai_step(m, events),
        Species::Guardian | Species::ElderGuardian => super::guardian::ai_step(m, w, events),
        Species::Vex => super::vex::ai_step(m, w),
        Species::Ravager => super::ravager::ai_step(m, w, events),
        Species::Wither => super::wither::ai_step(m, w, events),
        Species::Allay => super::allay::ai_step(m),
        Species::Bee => super::bee::ai_step(m, w),
        Species::PolarBear => super::polar_bear::ai_step(m, w),
        Species::Breeze => super::breeze::ai_step(m),
        Species::Sniffer => super::sniffer::ai_step(m, events),
        Species::HappyGhast => super::happy_ghast::ai_step(m, w),
        Species::Villager => super::villager::ai_step(m, w, events),
        Species::WanderingTrader => {
            super::villager::ai_step(m, w, events);
            super::trader::ai_step(m, w);
        }
        Species::Shulker => {
            super::shulker::tick(m, w, events);
            super::shulker::ai_step(m, w, level_random, events);
        }
        Species::SnowGolem => super::golem::snow_ai_step(m, w, events),
        Species::CopperGolem => super::copper_golem::ai_step(m, w, events),
        Species::Slime | Species::MagmaCube => super::cube::ai_step(m, w, events),
        Species::SulfurCube => {
            super::cube::ai_step(m, w, events);
            super::sulfur_cube::ai_step(m, w, events);
        }
        Species::Nautilus | Species::ZombieNautilus => super::nautilus::ai_step(m, w, events),
        Species::Chicken => farm::chicken_ai_step(m, w, events),
        Species::Armadillo => super::armadillo::tick(m),
        Species::Frog => super::frog::tick(m),
        Species::Goat => super::goat::tick(m),
        Species::Fox => super::fox::ai_step(m, w, events),
        Species::Turtle => super::turtle::ai_step(m, w, events),
        Species::Sheep if m.extra.anim_ticks > 0 => {
            m.extra.anim_ticks -= 1;
        }
        _ => {}
    }
}
/// `Entity.checkDespawn` alone, for a mob outside the entity-ticking
/// chunks. Returns false when the mob is gone.
pub fn despawn_check(m: &mut Mob, w: &MobWorld) -> bool {
    if m.removed {
        return false;
    }
    if !check_despawn(m, w) {
        m.removed = true;
        return false;
    }
    true
}
/// `Mob.requiresCustomPersistence` and its overrides: a passenger or a leashed
/// mob, a raider in a raid, an enderman carrying a block, a tame nautilus, a
/// sulfur cube holding a block or let out of a bucket. The spawn caps do not
/// count these mobs either (`NaturalSpawner.createState`).
pub fn requires_custom_persistence(m: &Mob) -> bool {
    super::riders::keeps_alive(m)
        || m.extra.leash.is_some()
        || super::raider::state(m).is_some_and(|s| s.raid_id.is_some())
        || m.species() == Species::Enderman && m.extra.carried.is_some()
        || super::nautilus::is_nautilus(m.species()) && m.extra.tame
        || m.species() == Species::SulfurCube
            && (super::sulfur_cube::has_body(m) || m.extra.scratch[3] != 0)
}
/// Mob.checkDespawn, before the tick (players are the only despawn
/// reference). Returns false when the mob is discarded.
fn check_despawn(m: &mut Mob, w: &MobWorld) -> bool {
    // A mannequin is a LivingEntity, which never despawns.
    if m.species() == Species::Mannequin {
        return true;
    }
    // Mob.checkDespawn discards disallowed types even when persistent or NoAI.
    if w.difficulty.effective == 0.0 && !super::spawn_rules::allowed_in_peaceful(m.species()) {
        return false;
    }
    let Some(p) = w.nearest_player else {
        return true;
    };
    let d = m.distance_sqr(p);
    let persistent = m.persistent || requires_custom_persistence(m);
    // removeWhenFarAway: animals, villagers, golems, patrols and raiders,
    // zombie villagers being cured and ridden mobs stay.
    let creature = m.kind.category == "creature";
    let removable = !(never_removed_when_far(m.species())
        || creature && m.kind.flags.animal && !m.extra.jockey
        || super::raider::keeps_alive(m, d)
        || super::zombie_villager::keeps_alive(m)
        || super::riders::keeps_alive(m));
    // MobCategory.getDespawnDistance / getNoDespawnDistance
    let despawn = if m.kind.category == "water_ambient" {
        64.0
    } else {
        128.0
    };
    if !persistent && d > despawn * despawn && removable {
        return false;
    }
    if !persistent
        && m.ai.no_action_time > 600
        && m.random.next_int_bound(800).unwrap_or(1) == 0
        && d > 32.0 * 32.0
        && removable
    {
        return false;
    }
    if d < 32.0 * 32.0 {
        m.ai.no_action_time = 0;
    }
    true
}
/// The types whose `removeWhenFarAway` is always false (`Villager`,
/// `WanderingTrader`, `AbstractGolem`: iron, snow and copper golems and
/// shulkers; `Allay`, `Warden`), and the wither, whose `checkDespawn` never
/// removes it for distance.
fn never_removed_when_far(s: Species) -> bool {
    matches!(
        s,
        Species::Villager
            | Species::WanderingTrader
            | Species::IronGolem
            | Species::SnowGolem
            | Species::CopperGolem
            | Species::Shulker
            | Species::Allay
            | Species::Warden
            | Species::Wither
    )
}
/// Movement speed bonus multiplier (baby zombies are 50% faster, etc.).
pub fn speed_bonus(m: &Mob) -> f64 {
    match m.species() {
        Species::Zombie
        | Species::Husk
        | Species::Drowned
        | Species::ZombieVillager
        | Species::ZombifiedPiglin
        | Species::Piglin
            if m.is_baby() =>
        {
            1.5
        }
        _ => 1.0,
    }
}
/// The ambient, hurt and death sounds of species that pick them by state
/// (`getHurtSound` / `getDeathSound` overrides); the type's otherwise.
pub fn voice_sounds(m: &Mob) -> [&'static str; 3] {
    match m.species() {
        Species::Nautilus | Species::ZombieNautilus => super::nautilus::sounds(m),
        Species::SulfurCube => super::sulfur_cube::sounds(m),
        Species::Armadillo => super::armadillo::sounds(m),
        // Panda.getAmbientSound.
        Species::Panda => {
            let ambient = if super::panda::is_aggressive(m) {
                "entity.panda.aggressive_ambient"
            } else if super::panda::is_worried(m) {
                "entity.panda.worried_ambient"
            } else {
                m.kind.sounds[0]
            };
            [ambient, m.kind.sounds[1], m.kind.sounds[2]]
        }
        // Horse.getAmbientSound, getHurtSound and getDeathSound by age.
        Species::Horse if m.is_baby() => [
            "entity.baby_horse.ambient",
            "entity.baby_horse.hurt",
            "entity.baby_horse.death",
        ],
        // Drowned, Guardian and ElderGuardian: the hurt and death sounds by
        // water or land.
        Species::Drowned if m.fluids.in_water => [
            "entity.drowned.ambient_water",
            "entity.drowned.hurt_water",
            "entity.drowned.death_water",
        ],
        Species::Guardian if !m.fluids.in_water => [
            m.kind.sounds[0],
            "entity.guardian.hurt_land",
            "entity.guardian.death_land",
        ],
        Species::ElderGuardian if !m.fluids.in_water => [
            m.kind.sounds[0],
            "entity.elder_guardian.hurt_land",
            "entity.elder_guardian.death_land",
        ],
        // Slime and MagmaCube.getHurtSound and getDeathSound: tiny ones.
        Species::Slime if m.extra.size <= 1 => [
            m.kind.sounds[0],
            "entity.slime.hurt_small",
            "entity.slime.death_small",
        ],
        Species::MagmaCube if m.extra.size <= 1 => [
            m.kind.sounds[0],
            "entity.magma_cube.hurt_small",
            "entity.magma_cube.death_small",
        ],
        // Shulker.getHurtSound: a closed shell.
        Species::Shulker if m.extra.counter == 0 => [
            m.kind.sounds[0],
            "entity.shulker.hurt_closed",
            m.kind.sounds[2],
        ],
        // Turtle.getHurtSound and getDeathSound by age.
        Species::Turtle if m.is_baby() => [
            m.kind.sounds[0],
            "entity.turtle.hurt_baby",
            "entity.turtle.death_baby",
        ],
        // SkeletonHorse.getAmbientSound: its own under water.
        Species::SkeletonHorse if m.fluids.eye_in_water => [
            "entity.skeleton_horse.ambient_water",
            m.kind.sounds[1],
            m.kind.sounds[2],
        ],
        // The mob's sound variant set, the baby one for a baby.
        Species::Pig | Species::Cow | Species::Chicken | Species::Wolf | Species::Cat => {
            use super::finalize::variant_sound as v;
            match (v(m, "ambient"), v(m, "hurt"), v(m, "death")) {
                (Some(a), Some(h), Some(d)) => [a, h, d],
                _ => m.kind.sounds,
            }
        }
        _ => m.kind.sounds,
    }
}
/// `Animal.mobInteract` with its food in hand: an adult that can fall in
/// love does, a baby grows up by a tenth, and either plays its eating sound
/// (`Strider.mobInteract` plays its own afterwards). `None` when the animal
/// does not take the food; otherwise the sound to play, if any.
pub fn feed_animal(
    m: &mut Mob,
    level: &mut impl RandomSource,
) -> Option<Option<(&'static str, f32, f32)>> {
    // Armadillo.mobInteract: a ball takes no food.
    if super::armadillo::scared(m) {
        return None;
    }
    if m.extra.age == 0 && m.extra.in_love <= 0 {
        m.extra.in_love = 600;
        m.extra.love_player = true;
    } else if m.extra.age < 0 {
        // AgeableMob.ageUp(getSpeedUpSecondsWhenFeeding(-age), true): a tenth.
        let secs = ((-m.extra.age / 20) as f32 * 0.1) as i32;
        age_up(m, secs, true);
    } else {
        return None;
    }
    let mut sound = eating_sound(m, level);
    if m.species() == Species::Strider && !m.silent {
        let pitch = 1.0 + (m.random.next_float() - m.random.next_float()) * 0.2;
        sound = Some(("entity.strider.eat", 1.0, pitch));
    }
    Some(sound)
}
/// `AgeableMob.ageUp(seconds, forced)`.
pub fn age_up(m: &mut Mob, seconds: i32, forced: bool) {
    let age = m.extra.age;
    let grown = (age + seconds * 20).min(0);
    m.extra.age = grown;
    if forced {
        m.extra.forced_age += grown - age;
        if m.extra.forced_age_timer == 0 {
            m.extra.forced_age_timer = 40;
        }
    }
    if m.extra.age == 0 {
        m.extra.age = m.extra.forced_age;
    }
}
/// `Animal.playEatingSound` (empty) and its overrides, as (sound, volume,
/// pitch); `level` is the level's random. Pigs and cats play their sound
/// variant's set.
pub fn eating_sound(
    m: &mut Mob,
    level: &mut impl RandomSource,
) -> Option<(&'static str, f32, f32)> {
    // Mth.randomBetween(level.getRandom(), 0.8, 1.2).
    let mut between = || level.next_float() * 0.4 + 0.8;
    let (sound, volume, pitch, entity_sound) = match m.species() {
        // makeSound: getSoundVolume, getVoicePitch.
        Species::Armadillo | Species::Pig | Species::SulfurCube => {
            let sound = match m.species() {
                Species::Armadillo => "entity.armadillo.eat",
                Species::Pig => {
                    super::finalize::variant_sound(m, "eat").unwrap_or("entity.pig.eat")
                }
                _ => "entity.small_sulfur_cube.eat",
            };
            let pitch = m.voice_pitch();
            (sound, sound_volume(m), pitch, true)
        }
        // playSound(sound, 1, 1).
        Species::Cat => (
            super::finalize::variant_sound(m, "eat").unwrap_or("entity.cat.eat"),
            1.0,
            1.0,
            true,
        ),
        Species::Fox => ("entity.fox.eat", 1.0, 1.0, true),
        // level().playSound(null, this, sound, NEUTRAL, volume, pitch).
        Species::Frog => ("entity.frog.eat", 2.0, 1.0, false),
        Species::Goat if m.extra.screaming => ("entity.goat.screaming.eat", 1.0, between(), false),
        Species::Goat => ("entity.goat.eat", 1.0, between(), false),
        Species::Sniffer => ("entity.sniffer.eat", 1.0, between(), false),
        _ => return None,
    };
    // Entity.playSound is skipped for a silent entity.
    if entity_sound && m.silent {
        return None;
    }
    Some((sound, volume, pitch))
}
pub fn sound_volume(m: &Mob) -> f32 {
    match m.species() {
        Species::Ghast => 5.0,
        Species::HappyGhast => super::happy_ghast::volume(m),
        Species::Warden => 4.0,
        Species::Slime | Species::MagmaCube | Species::SulfurCube => {
            0.4 * m.extra.size.max(1) as f32
        }
        Species::Bat => 0.1,
        Species::Squid
        | Species::GlowSquid
        | Species::Cow
        | Species::Mooshroom
        | Species::Bee
        | Species::Allay
        | Species::Wolf => 0.4,
        s if super::equine::is_equine(s) => 0.8,
        _ => 1.0,
    }
}
/// PathfinderMob.getWalkTargetValue (Animal and Monster overrides).
pub fn walk_target_value(m: &Mob, w: &MobWorld, p: [i32; 3]) -> f32 {
    let light = || w.brightness(p[0], p[1], p[2]) - 0.5;
    // Guardian.getWalkTargetValue: water first.
    if family(m.species()) == Family::Guardian && w.fluid(p[0], p[1], p[2]) == Fluid::Water {
        return 10.0 + light();
    }
    // HappyGhast.getWalkTargetValue: open air, best just above the ground.
    if m.species() == Species::HappyGhast {
        let air = |y: i32| w.facts.is_air(w.world.block(p[0], y, p[2]));
        return if !air(p[1]) {
            0.0
        } else if air(p[1] - 1) && !air(p[1] - 2) {
            10.0
        } else {
            5.0
        };
    }
    if m.species() == Species::Strider {
        return super::strider::walk_target_value(m, w, p);
    }
    if m.species() == Species::Turtle {
        return super::turtle::walk_target_value(m, w, p, light());
    }
    // Hoglin.getWalkTargetValue: nowhere near a repellent, crimson nylium best.
    if m.species() == Species::Hoglin {
        let near = m.extra.repellent.is_some_and(|r| {
            let d = |i: usize| i64::from(r[i] - p[i]);
            d(0) * d(0) + d(1) * d(1) + d(2) * d(2) < 64
        });
        return if near {
            -1.0
        } else if w.block_name(p[0], p[1] - 1, p[2]) == "minecraft:crimson_nylium" {
            10.0
        } else {
            0.0
        };
    }
    // Bee.getWalkTargetValue: any air block.
    if m.species() == Species::Bee {
        return if w.facts.is_air(w.world.block(p[0], p[1], p[2])) {
            10.0
        } else {
            0.0
        };
    }
    // AbstractNautilus, Axolotl and Creaking.getWalkTargetValue: everywhere alike.
    if matches!(
        m.species(),
        Species::Nautilus | Species::ZombieNautilus | Species::Axolotl | Species::Creaking
    ) {
        return 0.0;
    }
    if m.hostile()
        && !matches!(
            m.species(),
            Species::Slime | Species::MagmaCube | Species::Ghast | Species::Phantom
        )
    {
        return -light();
    }
    if m.kind.flags.animal {
        if w.block_name(p[0], p[1] - 1, p[2]) == "minecraft:grass_block" {
            return 10.0;
        }
        return light();
    }
    0.0
}
/// Species damage filters (LivingEntity.hurtServer overrides).
pub fn accepts_damage(m: &mut Mob, source: &Damage, _amount: f32) -> bool {
    match m.species() {
        // Allay.hurtServer: its liked player cannot hurt it.
        Species::Allay if m.extra.tame && source.attacker == Some(Who::Player) => false,
        Species::Creaking => super::creaking::accepts_damage(m, source),
        Species::Wither => super::wither::accepts(m, source.kind, source.attacker, false),
        Species::Warden => super::warden::accepts_damage(m, source),
        Species::Shulker => super::shulker::accepts(m, source),
        Species::SulfurCube => super::sulfur_cube::accepts_damage(m, source, _amount),
        // Bat.hurtServer: a hit wakes it.
        Species::Bat => {
            super::bat::hurt(m);
            true
        }
        Species::Bee => {
            super::bee::hurt(m);
            true
        }
        // Panda.hurtServer: up first, whatever the damage.
        Species::Panda => {
            super::panda::hurt(m);
            true
        }
        Species::Silverfish => {
            super::hostile::silverfish_hurt(m, source);
            true
        }
        Species::Enderman => super::enderman::on_damage(m, source),
        Species::WitherSkeleton | Species::Blaze | Species::MagmaCube | Species::Strider
            if source.is_fire() =>
        {
            false
        }
        _ => true,
    }
}
/// Reactions after taking damage (sounds, anger).
pub fn after_hurt(m: &mut Mob, source: &Damage) {
    m.ai.no_action_time = 0;
    match m.species() {
        Species::Enderman => super::enderman::after_hurt(m, source),
        Species::Armadillo => super::armadillo::after_hurt(m, source),
        Species::Shulker => super::shulker::after_hurt(m),
        Species::Warden => super::warden::hurt(m, source),
        Species::Wolf => super::tame::after_hurt(m, source),
        Species::Nautilus | Species::ZombieNautilus => super::nautilus::after_hurt(m, source),
        Species::Zombie
        | Species::Husk
        | Species::Drowned
        | Species::ZombieVillager
        | Species::ZombifiedPiglin => super::undead::after_hurt(m, source),
        // NeutralMob: an iron golem hit by the player stays angry a while.
        // PiglinAi.wasHurtBy: drop what is being admired, no admiring for
        // a while after a player's hit, and anger.
        Species::Piglin if source.attacker == Some(Who::Player) => {
            // stopHoldingOffHandItem(.., false): a gold ingot is not bartered
            // and is gone; anything else ends its admiring next tick, as if
            // the time ran out, and the game side puts it away.
            if m.extra.off_hand.as_deref() == Some("minecraft:gold_ingot") {
                m.extra.off_hand = None;
                m.extra.using_item = 0;
            } else {
                m.extra.using_item =
                    i32::from(m.extra.off_hand.is_some() && m.extra.using_item > 0);
            }
            m.extra.egg_time = 400;
            m.extra.anger = 600;
        }
        Species::IronGolem if source.attacker == Some(Who::Player) => {
            m.extra.anger = 400 + m.random.next_int_bound(380).unwrap_or(0);
        }
        _ => {}
    }
}
/// Mob.doHurtTarget: attack damage (+ species extras) to the target.
pub fn do_hurt_target(m: &mut Mob, who: Who, cx: &mut Cx) {
    if m.species() == Species::IronGolem {
        return super::golem::iron_hurt_target(m, who, cx);
    }
    if m.species() == Species::Bee {
        return super::bee::sting(m, who, cx);
    }
    if matches!(m.species(), Species::Hoglin | Species::Zoglin) {
        return super::piglin::hoglin_hurt_target(m, who, cx);
    }
    if m.species() == Species::Ravager {
        super::ravager::on_attack(m, cx.events);
    }
    if m.species() == Species::Panda {
        super::panda::bit(m);
    }
    if super::creaking::is_creaking(m) {
        super::creaking::on_attack(m, cx.events);
    }
    let mut amount = m.attrs.attack_damage;
    let from = [m.position.x, m.position.y, m.position.z];
    let source = Damage::mob(m.id, from);
    let first_event = cx.events.len();
    match m.species() {
        // CaveSpider.doHurtTarget: poison for 7 s on normal, 15 s on hard.
        Species::CaveSpider if who == Who::Player && cx.w.difficulty.base >= 2 => {
            cx.events.push(MobEvent::EffectPlayer {
                effect: "minecraft:poison",
                ticks: if cx.w.difficulty.base == 2 { 7 } else { 15 } * 20,
                amplifier: 0,
            })
        }
        // Husk.doHurtTarget: hunger for 140 ticks per whole point of the
        // local difficulty, only with an empty main hand.
        Species::Husk if who == Who::Player && m.extra.main_hand.is_none() => {
            let ticks = 140 * cx.w.difficulty.effective as i32;
            if ticks > 0 {
                cx.events.push(MobEvent::EffectPlayer {
                    effect: "minecraft:hunger",
                    ticks,
                    amplifier: 0,
                })
            }
        }
        Species::WitherSkeleton if who == Who::Player => cx.events.push(MobEvent::EffectPlayer {
            effect: "minecraft:wither",
            ticks: 200,
            amplifier: 0,
        }),
        _ => {}
    }
    if amount <= 0.0 {
        amount = 1.0;
    }
    // The held weapon's enchantments: Sharpness, Smite, Bane of Arthropods,
    // Fire Aspect and Knockback.
    let target_species = match who {
        Who::Mob(id) => cx.w.seen(id).map(|s| s.species),
        Who::Player => None,
    };
    amount += super::gear::melee_bonus(m, target_species);
    let burn = super::gear::fire_aspect(m);
    if burn > 0 {
        cx.events.push(MobEvent::Ignite {
            target: who,
            seconds: burn,
        });
    }
    let knock = super::gear::knockback(m);
    if knock > 0 {
        let (dx, dz) = match who {
            Who::Player => cx.w.player.as_ref().map_or((0.0, 0.0), |p| {
                (p.position.x - m.position.x, p.position.z - m.position.z)
            }),
            Who::Mob(id) => cx.w.seen(id).map_or((0.0, 0.0), |s| {
                (s.position.x - m.position.x, s.position.z - m.position.z)
            }),
        };
        let len = (dx * dx + dz * dz).sqrt().max(1e-5);
        let k = f64::from(knock) * 0.5;
        cx.events.push(MobEvent::Push {
            target: who,
            velocity: [dx / len * k, 0.1, dz / len * k],
        });
    }
    // Zombies set their target on fire while burning (Zombie.doHurtTarget).
    if m.fire_ticks > 0 && matches!(m.species(), Species::Zombie) && m.random.next_float() < 0.3 {
        cx.events.push(MobEvent::Ignite {
            target: who,
            seconds: 2,
        });
    }
    match who {
        // The effect picked above goes with the hit: it applies only when
        // the player takes damage (a shield-blocked hit gives none).
        Who::Player => {
            let effect = cx.events[first_event..]
                .iter()
                .position(|e| matches!(e, MobEvent::EffectPlayer { .. }))
                .map(|k| cx.events.remove(first_event + k));
            cx.events.push(match effect {
                Some(MobEvent::EffectPlayer {
                    effect,
                    ticks,
                    amplifier,
                }) => MobEvent::AttackPlayerWithEffect {
                    amount,
                    from,
                    effect,
                    ticks,
                    amplifier,
                },
                _ => MobEvent::AttackPlayer(amount, from),
            });
        }
        Who::Mob(id) => cx.events.push(MobEvent::AttackMob {
            target: id,
            amount,
            source,
        }),
    }
    // Mob.doHurtTarget ends with playAttackSound once the hit landed.
    if super::creaking::is_creaking(m) {
        m.voice("entity.creaking.attack", cx.events);
    }
    // Panda.playAttackSound: playSound(PANDA_BITE, 1.0, 1.0).
    if m.species() == Species::Panda && !m.silent {
        let at = [m.position.x, m.position.y, m.position.z];
        cx.events
            .push(MobEvent::Sound("entity.panda.bite", at, 1.0, 1.0));
    }
    // Axolotl.playAttackSound: playSound(AXOLOTL_ATTACK, 1.0, 1.0).
    if m.species() == Species::Axolotl {
        let at = [m.position.x, m.position.y, m.position.z];
        cx.events
            .push(MobEvent::Sound("entity.axolotl.attack", at, 1.0, 1.0));
    }
    let _ = Fluid::None;
}
/// Mob.ate (sheep regrow wool, babies grow).
pub fn ate(m: &mut Mob) {
    if m.species() == Species::Sheep {
        m.extra.sheared = false;
        if m.is_baby() {
            // AgeableMob.ageUp(60)
            m.extra.age = (m.extra.age + 60 * 20).min(0);
        }
    }
}
/// Species-specific travel (fish, squid, dolphins swim with their own rules).
pub fn custom_travel(m: &mut Mob, w: &MobWorld, input: Vec3, events: &mut Vec<MobEvent>) -> bool {
    if m.species() == Species::Drowned {
        return undead::drowned_travel(m, w, input, events);
    }
    match family(m.species()) {
        Family::Aquatic => aquatic::travel(m, w, input, events),
        Family::Guardian => super::guardian::travel(m, w, input, events),
        // Shulkers never move on their own.
        Family::Shulker => true,
        Family::Allay => super::allay::travel(m, w, input, events),
        Family::Vex => super::vex::travel(m),
        Family::Nautilus => super::nautilus::travel(m, w, input, events),
        _ => false,
    }
}

/// `Entity.applyEffectsFromBlocks` for the flowers that give an effect:
/// `WitherRoseBlock.entityInside` (wither for 40 ticks on a living mob that
/// can take it, not in peaceful) and `EyeblossomBlock.entityInside` (poison
/// for 25 ticks on a bee, when the open flower attracts bees). The blocks
/// are those the mob's box touches where it started the tick and where it
/// ended.
fn flower_contact_effects(m: &mut Mob, w: &MobWorld) {
    if !m.alive() || w.difficulty.effective == 0.0 {
        return;
    }
    let half = f64::from(m.width()) / 2.0 - 1.0e-5;
    let height = f64::from(m.height()) - 1.0e-5;
    let mut visited: Vec<[i32; 3]> = Vec::new();
    for at in [m.previous, m.position] {
        let cells = |lo: f64, hi: f64| lo.floor() as i32..=hi.floor() as i32;
        for x in cells(at.x - half, at.x + half) {
            for y in cells(at.y + 1.0e-5, at.y + height) {
                for z in cells(at.z - half, at.z + half) {
                    let p = [x, y, z];
                    if visited.contains(&p) {
                        continue;
                    }
                    match w.block_name(x, y, z) {
                        "minecraft:wither_rose" => {
                            // isInvulnerableTo(wither): the invulnerable time
                            // after a hit; WitherBoss and WitherSkeleton
                            // refuse the effect (`canBeAffected`).
                            if m.invulnerable <= 0
                                && !m.invulnerable_flag
                                && !matches!(m.species(), Species::Wither | Species::WitherSkeleton)
                            {
                                m.add_effect("minecraft:wither", 40, 0);
                            }
                        }
                        "minecraft:open_eyeblossom"
                            if m.species() == Species::Bee
                                && super::bee::attracts_bees(w, p)
                                && !m.has_effect("minecraft:poison") =>
                        {
                            m.add_effect("minecraft:poison", 25, 0);
                        }
                        _ => continue,
                    }
                    visited.push(p);
                }
            }
        }
    }
}

/// MobEffect ticking for the effects mobs receive (poison, wither, regeneration).
fn tick_effects(m: &mut Mob) {
    let mut effects = std::mem::take(&mut m.extra.effects);
    for (effect, ticks, amp) in &mut effects {
        let interval = |base: i32| (base >> (*amp).max(0)).max(1);
        // MobEffectInstance.tickServer: an infinite (-1) effect times its
        // ticks by the mob's tickCount and never counts down.
        let infinite = *ticks == -1;
        let count = if infinite { m.age as i32 } else { *ticks };
        match *effect {
            "minecraft:poison"
                if count % interval(25) == 0 && !m.kind.flags.undead && m.health > 1.0 =>
            {
                m.damage(1.0, Damage::of(DamageKind::Magic));
            }
            "minecraft:wither" if count % interval(40) == 0 => {
                m.damage(1.0, Damage::of(DamageKind::Magic));
            }
            "minecraft:regeneration"
                if count % interval(50) == 0 && !m.kind.flags.undead && m.alive() =>
            {
                m.health = (m.health + 1.0).min(m.attrs.max_health);
            }
            _ => {}
        }
        if !infinite {
            *ticks -= 1;
        }
    }
    effects.retain(|e| e.1 > 0 || e.1 == -1);
    m.extra
        .hidden_effects
        .retain(|h| effects.iter().any(|e| e.0 == *h));
    m.extra.effects = effects;
}

/// `Entity.getMovementEmission` is not NONE (silverfish, endermites,
/// shulkers, guardians and breezes emit only events, no step sounds).
pub fn emits_movement(m: &Mob) -> bool {
    !matches!(m.species(), Species::Shulker)
}
/// The movement emission includes sounds.
pub fn movement_sounds(m: &Mob) -> bool {
    !matches!(
        m.species(),
        Species::Silverfish
            | Species::Endermite
            | Species::Guardian
            | Species::ElderGuardian
            | Species::Breeze
    )
}
/// `playStepSound`: the species' footstep, or the block's step sound at 15%.
pub fn step_sound(
    m: &Mob,
    w: &MobWorld,
    pos: [i32; 3],
    state: horizon_world::registry::BlockStateId,
) -> Option<(&'static str, f32, f32)> {
    use Species as S;
    let baby = m.is_baby();
    let quiet = |s: &'static str| Some((s, 0.15, 1.0));
    match m.species() {
        // Silent footsteps, or only movement events.
        S::Allay
        | S::Bat
        | S::Bee
        | S::Cod
        | S::Salmon
        | S::TropicalFish
        | S::Pufferfish
        | S::Tadpole
        | S::HappyGhast
        | S::Nautilus
        | S::ZombieNautilus
        | S::Silverfish
        | S::Endermite
        | S::Guardian
        | S::ElderGuardian
        | S::Breeze => None,
        S::Armadillo => quiet("entity.armadillo.step"),
        S::Chicken | S::Cow | S::Pig | S::Wolf => {
            super::finalize::variant_sound(m, "step").and_then(quiet)
        }
        S::Mooshroom => quiet("entity.cow.step"),
        S::Llama | S::TraderLlama => quiet("entity.llama.step"),
        S::Frog => quiet("entity.frog.step"),
        S::Goat => quiet("entity.goat.step"),
        S::Panda => quiet("entity.panda.step"),
        S::Parrot => quiet("entity.parrot.step"),
        S::PolarBear => quiet("entity.polar_bear.step"),
        S::Sheep => quiet("entity.sheep.step"),
        S::Sniffer => quiet("entity.sniffer.step"),
        S::Turtle if baby => quiet("entity.turtle.shamble_baby"),
        S::Turtle => quiet("entity.turtle.shamble"),
        S::Creaking => quiet("entity.creaking.step"),
        S::Hoglin => quiet("entity.hoglin.step"),
        S::Zoglin => quiet("entity.zoglin.step"),
        S::Piglin => quiet("entity.piglin.step"),
        S::PiglinBrute => quiet("entity.piglin_brute.step"),
        S::Ravager => quiet("entity.ravager.step"),
        S::Skeleton => quiet("entity.skeleton.step"),
        S::Stray => quiet("entity.stray.step"),
        S::Bogged => quiet("entity.bogged.step"),
        S::Parched => quiet("entity.parched.step"),
        S::WitherSkeleton => quiet("entity.wither_skeleton.step"),
        S::Spider | S::CaveSpider => quiet("entity.spider.step"),
        S::Zombie | S::ZombifiedPiglin => quiet("entity.zombie.step"),
        S::Husk => quiet("entity.husk.step"),
        S::Drowned => quiet("entity.drowned.step"),
        S::ZombieVillager => quiet("entity.zombie_villager.step"),
        // Camel.playStepSound and CamelHusk.playStepSound: sand and concrete
        // powder have their own step.
        S::CamelHusk | S::Camel => {
            let sand = w.has_tag(
                "minecraft:camel_sand_step_sound_blocks",
                &w.facts.get(state).name,
            );
            Some(match (m.species(), sand) {
                (S::Camel, true) => ("entity.camel.step_sand", 1.0, 1.0),
                (S::Camel, false) => ("entity.camel.step", 1.0, 1.0),
                (_, true) => ("entity.camel_husk.step_sand", 0.4, 1.0),
                (_, false) => ("entity.camel_husk.step", 0.4, 1.0),
            })
        }
        S::IronGolem => Some(("entity.iron_golem.step", 1.0, 1.0)),
        // CopperGolem.playStepSound: its oxidation level's step.
        S::CopperGolem => Some((super::copper_golem::sound(m, "step"), 1.0, 1.0)),
        S::Strider if m.fluids.in_lava => Some(("entity.strider.step_lava", 1.0, 1.0)),
        S::Strider => Some(("entity.strider.step", 1.0, 1.0)),
        S::Warden => Some(("entity.warden.step", 10.0, 1.0)),
        _ => {
            // Entity.playStepSound: the block's sound type.
            let _ = pos;
            let sounds = &w.facts.get(state).sounds;
            let step: &'static str = super::ai::intern_sound(&sounds.step);
            Some((step, sounds.volume * 0.15, sounds.pitch))
        }
    }
}
/// A block sound id as a static string (the block facts intern them).
pub fn intern_sound(s: &std::sync::Arc<str>) -> &'static str {
    use std::{collections::HashSet, sync::Mutex};
    static NAMES: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
    let mut names = NAMES.lock().unwrap_or_else(|e| e.into_inner());
    let set = names.get_or_insert_with(HashSet::new);
    let key = s.strip_prefix("minecraft:").unwrap_or(s);
    if let Some(found) = set.get(key) {
        return found;
    }
    let leaked: &'static str = Box::leak(key.to_owned().into_boxed_str());
    set.insert(leaked);
    leaked
}
/// `getSwimSound`.
pub fn swim_sound(m: &Mob) -> &'static str {
    match m.species() {
        Species::Drowned => "entity.drowned.swim",
        Species::Axolotl => "entity.axolotl.swim",
        Species::Dolphin => "entity.dolphin.swim",
        Species::Cod
        | Species::Salmon
        | Species::TropicalFish
        | Species::Pufferfish
        | Species::Tadpole => "entity.fish.swim",
        Species::Nautilus if m.is_baby() => "entity.baby_nautilus.swim",
        Species::Nautilus => "entity.nautilus.swim",
        Species::ZombieNautilus => "entity.zombie_nautilus.swim",
        Species::Turtle => "entity.turtle.swim",
        // Monster.getSwimSound, and Hoglin's own override of it.
        s if s == Species::Hoglin || is_monster(s) => "entity.hostile.swim",
        _ => "entity.generic.swim",
    }
}

#[cfg(test)]
mod bucket_axolotl_tests {
    use super::*;
    #[test]
    fn bucket_axolotl_uses_aquatic_not_generic_land_ai() {
        assert_eq!(family(Species::Axolotl), Family::Aquatic);
    }
}
#[cfg(test)]
mod despawn_tests {
    use super::*;
    /// Village villagers are placed without PersistenceRequired (unlike the
    /// village's iron golem), so they must never despawn for distance.
    #[test]
    fn villagers_traders_and_golems_never_despawn_for_distance() {
        for s in [
            Species::Villager,
            Species::WanderingTrader,
            Species::IronGolem,
            Species::SnowGolem,
            Species::CopperGolem,
            Species::Shulker,
        ] {
            assert!(never_removed_when_far(s), "{s:?}");
        }
        for s in [Species::Zombie, Species::ZombieVillager, Species::Pillager] {
            assert!(!never_removed_when_far(s), "{s:?}");
        }
    }
}

#[cfg(test)]
mod frog_tests {
    use super::*;
    use crate::mobs::MobTypes;
    use crate::mobs::control::MoveKind;
    use std::sync::Arc;

    /// `Frog()`: `new SmoothSwimmingMoveControl<>(this, 85, 10, 0.02F, 0.1F, true)`.
    /// The frog's movement speed is 1.0, so the controller's 0.1 outside-water
    /// speed is what keeps a walking frog at vanilla's pace. The plain ground
    /// controller walks it ten times too fast, which is the jitter, the
    /// teleporting and the flying of #2213.
    #[test]
    fn a_frog_walks_at_the_vanilla_speed() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let data = MobTypes::load(&root).unwrap();
        let kind = Arc::new(data.types.get("minecraft:frog").unwrap().clone());
        assert_eq!(kind.speed, 1.0, "the frog's movement speed attribute");
        let mut m = Mob::new(1, kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
        register(&mut m);
        assert_eq!(
            m.ai.mov.kind,
            MoveKind::SmoothSwimming {
                max_turn_x: 85.0,
                max_turn_y: 10.0,
                in_water: 0.02,
                outside: 0.1,
                gravity: true,
            }
        );
    }
}

#[cfg(test)]
mod poison_effect_tests {
    use super::*;
    use crate::mobs::MobTypes;
    use std::sync::Arc;

    #[test]
    fn poison_effect_damages_living_mobs_but_not_undead() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let data = MobTypes::load(&root).unwrap();

        // Pig is not undead: should take damage from poison.
        let pig_kind = Arc::new(data.types.get("minecraft:pig").unwrap().clone());
        let mut pig = Mob::new(1, pig_kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
        pig.add_effect("poison", 100, 0);
        assert_eq!(pig.extra.effects[0].0, "minecraft:poison");
        let initial_pig_health = pig.health;
        tick_effects(&mut pig);
        assert_eq!(
            pig.health,
            initial_pig_health - 1.0,
            "living mob should take 1.0 damage from poison"
        );

        // Zombie is undead: immune to poison damage.
        let zombie_kind = Arc::new(data.types.get("minecraft:zombie").unwrap().clone());
        let mut zombie = Mob::new(2, zombie_kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
        zombie.add_effect("poison", 100, 0);
        let initial_zombie_health = zombie.health;
        tick_effects(&mut zombie);
        assert_eq!(
            zombie.health, initial_zombie_health,
            "undead mobs are immune to poison"
        );
    }
}
