//! Pandas (`Panda`): the genes and what they show, the sit, lie-on-back, roll
//! and sneeze states with their goals, the unhappy and eat counters, and the
//! client side of the three amounts `PandaModel.setupAnim` reads.
//!
//! `extra.variant` and `extra.variant2` are the main and the hidden gene
//! (`MAIN_GENE_ID`, `HIDDEN_GENE_ID`); the rest is [`State`], whose `flags`,
//! `unhappy`, `sneeze` and `eat` are the synched data a server sends.
use super::{
    DamageKind, Mob, MobEvent, MobWorld, Species, Who,
    body_anim::Panda as Pose,
    finalize::{self, Attr},
    goals::{
        Avoid, AvoidEntity, Breed, Cx, Float, FollowParent, Goal, HurtByTarget, JUMP, LOOK, MOVE,
        MeleeAttack, Panic, RandomLookAround, Stroll, Tempt, goal_clone, reduced,
    },
};
use horizon_core::{
    geometry::{
        Vec3,
        trig::{DEG_TO_RAD, Mth},
    },
    jmath,
    random::{LegacyRandom, RandomSource},
};

/// `Panda.Gene` ids (the order of `Gene.values()`).
pub const LAZY: u8 = 1;
pub const WORRIED: u8 = 2;
pub const PLAYFUL: u8 = 3;
pub const WEAK: u8 = 5;
pub const AGGRESSIVE: u8 = 6;
/// `Gene.getSerializedName`, by id.
pub const GENES: [&str; 7] = [
    "normal",
    "lazy",
    "worried",
    "playful",
    "brown",
    "weak",
    "aggressive",
];

/// `DATA_ID_FLAGS`.
pub const FLAG_SNEEZE: u8 = 2;
pub const FLAG_ROLL: u8 = 4;
pub const FLAG_SIT: u8 = 8;
pub const FLAG_ON_BACK: u8 = 16;

/// The crumbs of one bite: where and how fast each one flies.
pub type Crumbs = Vec<([f64; 3], [f64; 3])>;

#[derive(Clone, Debug, Default)]
pub struct State {
    /// `DATA_ID_FLAGS` (sneezing, rolling, sitting, on its back).
    pub flags: u8,
    /// `UNHAPPY_COUNTER`, `SNEEZE_COUNTER` and `EAT_COUNTER`.
    pub unhappy: i32,
    pub sneeze: i32,
    pub eat: i32,
    got_bamboo: bool,
    did_bite: bool,
    /// `rollCounter`, which a client counts for itself.
    pub roll_counter: i32,
    roll_delta: [f64; 2],
    /// `sitAmount`, `onBackAmount` and `rollAmount` with the tick before.
    sit_amount: [f32; 2],
    on_back_amount: [f32; 2],
    roll_amount: [f32; 2],
    /// `isScared` as of the last tick (the model and the item layer read it).
    scared: bool,
    /// `PandaLookAtPlayerGoal.lookAt`, which the breed goal can set.
    look_at: Option<Who>,
    /// What the game saw on the ground: any food within 6 blocks, and the
    /// first within 8.
    pub food_near: bool,
    pub food_at: Option<[f64; 3]>,
    /// The size of the stack in its mouth (the mob holds one item id).
    pub held_count: u32,
    /// What a window draws of the last bite and sneeze, taken by `effects`.
    crumbs: Option<Crumbs>,
    sneezed: Option<([f64; 3], [f64; 3])>,
}

fn st(m: &mut Mob) -> &mut State {
    m.extra.panda.get_or_insert_with(Default::default)
}
pub fn get(m: &Mob) -> Option<&State> {
    m.extra.panda.as_deref()
}
fn flag(m: &Mob, f: u8) -> bool {
    get(m).is_some_and(|s| s.flags & f != 0)
}
fn set_flag(m: &mut Mob, f: u8, on: bool) {
    let s = st(m);
    if on {
        s.flags |= f;
    } else {
        s.flags &= !f;
    }
}

// ---- genes ----------------------------------------------------------------

/// `Panda.getVariant`: the gene a panda shows.
pub fn variant(m: &Mob) -> u8 {
    finalize::panda_variant(m.extra.variant, m.extra.variant2)
}
pub fn is_lazy(m: &Mob) -> bool {
    variant(m) == LAZY
}
pub fn is_worried(m: &Mob) -> bool {
    variant(m) == WORRIED
}
pub fn is_playful(m: &Mob) -> bool {
    variant(m) == PLAYFUL
}
pub fn is_weak(m: &Mob) -> bool {
    variant(m) == WEAK
}
/// `Panda.isAggressive`: the gene, not the mob flag.
pub fn is_aggressive(m: &Mob) -> bool {
    variant(m) == AGGRESSIVE
}
/// `Panda.Gene.getRandom`.
fn random_gene(random: &mut impl RandomSource) -> u8 {
    finalize::panda_gene(random.next_int_bound(16).unwrap_or(0))
}
/// `Panda.setAttributes`: a weak panda has 10 health, a lazy one is slow.
pub fn set_attributes(m: &mut Mob) {
    if is_weak(m) {
        finalize::set_base(m, Attr::MaxHealth, 10.0);
    }
    if is_lazy(m) {
        finalize::set_base(m, Attr::MovementSpeed, f64::from(0.07_f32));
    }
}
/// `Panda.getBreedOffspring`: `setGeneFromParents`, then `setAttributes`, on
/// a baby the game made (its attributes go back to the type's first).
pub fn inherit(child: &mut Mob, parent: &Mob, partner: &Mob) {
    let r = &mut child.random;
    let one_of = |p: &Mob, r: &mut LegacyRandom| {
        if r.next_boolean() {
            p.extra.variant
        } else {
            p.extra.variant2
        }
    };
    let (mut main, mut hidden) = if r.next_boolean() {
        let main = one_of(parent, r);
        (main, one_of(partner, r))
    } else {
        let main = one_of(partner, r);
        (main, one_of(parent, r))
    };
    if r.next_int_bound(32).unwrap_or(1) == 0 {
        main = random_gene(r);
    }
    if r.next_int_bound(32).unwrap_or(1) == 0 {
        hidden = random_gene(r);
    }
    child.extra.variant = main;
    child.extra.variant2 = hidden;
    for a in [Attr::MaxHealth, Attr::MovementSpeed] {
        if let Some(value) = finalize::default_value(child, a) {
            finalize::set_base(child, a, value);
        }
    }
    set_attributes(child);
    child.health = finalize::max_health(child);
}
/// `Panda.Gene.CODEC` for `MainGene` and `HiddenGene`.
pub fn gene_from_name(name: &str) -> u8 {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    GENES.iter().position(|g| *g == name).unwrap_or(0) as u8
}

// ---- state ----------------------------------------------------------------

pub fn is_sitting(m: &Mob) -> bool {
    flag(m, FLAG_SIT)
}
fn is_rolling(m: &Mob) -> bool {
    flag(m, FLAG_ROLL)
}
/// `Panda.isOnBack`.
pub fn is_on_back(m: &Mob) -> bool {
    flag(m, FLAG_ON_BACK)
}
fn is_eating(m: &Mob) -> bool {
    get(m).is_some_and(|s| s.eat > 0)
}
/// `Panda.isScared`: a worried panda in a thunderstorm.
fn is_scared(m: &Mob, thundering: bool) -> bool {
    is_worried(m) && thundering
}
/// `Panda.canPerformAction`.
pub fn can_perform_action(m: &Mob, thundering: bool) -> bool {
    !is_on_back(m) && !is_scared(m, thundering) && !is_eating(m) && !is_rolling(m) && !is_sitting(m)
}
fn sit(m: &mut Mob, on: bool) {
    set_flag(m, FLAG_SIT, on);
}
fn eat(m: &mut Mob, on: bool) {
    st(m).eat = i32::from(on);
}
/// `Panda.sneeze`: ending it resets the counter.
fn sneeze(m: &mut Mob, on: bool) {
    set_flag(m, FLAG_SNEEZE, on);
    if !on {
        st(m).sneeze = 0;
    }
}
/// `Panda.tryToSit`.
pub fn try_to_sit(m: &mut Mob) {
    if !m.fluids.in_water {
        m.ai.zza = 0.0;
        m.nav_stop();
        sit(m, true);
    }
}
/// `Panda.mobInteract` with bamboo in hand while it has a target.
pub fn got_bamboo(m: &mut Mob) {
    st(m).got_bamboo = true;
}
/// `Panda.hurtServer`: any hurt gets it up.
pub fn hurt(m: &mut Mob) {
    sit(m, false);
}
/// `Panda.doHurtTarget`: a bite that is not an aggressive panda's.
pub fn bit(m: &mut Mob) {
    if !is_aggressive(m) {
        st(m).did_bite = true;
    }
}
/// `Panda.setOnBack(false)` from a click on a panda on its back.
pub fn get_up(m: &mut Mob) -> bool {
    let was = is_on_back(m);
    if was {
        set_flag(m, FLAG_ON_BACK, false);
    }
    was
}
/// `Panda.eat(true)` at the first bite of what a player gave it.
pub fn start_eating(m: &mut Mob) {
    eat(m, true);
}
/// `isScared` as of its last tick, for a click or a sneeze.
pub fn scared(m: &Mob) -> bool {
    get(m).is_some_and(|s| s.scared)
}
/// `setItemSlot(MAINHAND, stack)` and `setGuaranteedDrop`, or just the size
/// of a stack it was given.
pub fn hold(m: &mut Mob, item: &str, count: u32, guaranteed: bool) {
    finalize::set_item(m, super::gear::MAIN, item);
    if guaranteed {
        finalize::set_drop_chance(m, super::gear::MAIN, 2.0);
    }
    st(m).held_count = count;
}
/// What its goals look for on the ground, found by the game each tick.
pub fn set_food(m: &mut Mob, near: bool, at: Option<[f64; 3]>) {
    let s = st(m);
    s.food_near = near;
    s.food_at = at;
}
/// The size of the stack in its mouth, for what it drops.
pub fn held_count(m: &Mob) -> u32 {
    get(m).map_or(1, |s| s.held_count.max(1))
}

// ---- synched data ---------------------------------------------------------

/// `DATA_ID_FLAGS` from a server: a sneeze that ends is a sneeze that was
/// done (`afterSneeze`'s particle).
pub fn set_flags(m: &mut Mob, flags: u8) {
    if flag(m, FLAG_SNEEZE) && flags & FLAG_SNEEZE == 0 {
        let at = [m.position.x, m.position.y, m.position.z];
        let particle = sneeze_particle(at, m.width(), m.body_yaw, m.eye_y(), m.velocity);
        st(m).sneezed = Some(particle);
    }
    st(m).flags = flags;
}
/// `UNHAPPY_COUNTER` and `SNEEZE_COUNTER` from a server.
pub fn set_unhappy(m: &mut Mob, unhappy: i32) {
    st(m).unhappy = unhappy;
}
pub fn set_sneeze(m: &mut Mob, sneeze: i32) {
    st(m).sneeze = sneeze;
}
/// `EAT_COUNTER` from a server: a bite every five.
pub fn set_eat(m: &mut Mob, eat: i32) {
    let bite = eat > 0 && eat % 5 == 0 && st(m).eat != eat && m.extra.main_hand.is_some();
    st(m).eat = eat;
    if bite {
        let crumbs = crumbs(m);
        st(m).crumbs = Some(crumbs);
    }
}
/// What the window draws since the last call: the crumbs of a bite and the
/// sneeze's particle (position, velocity).
pub fn effects(m: &mut Mob) -> (Option<Crumbs>, Option<([f64; 3], [f64; 3])>) {
    match m.extra.panda.as_deref_mut() {
        Some(s) => (s.crumbs.take(), s.sneezed.take()),
        None => (None, None),
    }
}

// ---- the tick --------------------------------------------------------------

/// `Entity.playSound`: at the feet, never from a silent mob.
fn play(m: &Mob, sound: &'static str, volume: f32, pitch: f32, events: &mut Vec<MobEvent>) {
    if !m.silent {
        let at = [m.position.x, m.position.y, m.position.z];
        events.push(MobEvent::Sound(sound, at, volume, pitch));
    }
}
fn lerp(a: f32, o: f32, n: f32) -> f32 {
    o + a * (n - o)
}
/// `updateSitAmount`, `updateOnBackAnimation` and `updateRollAmount`.
fn step_amount(amount: &mut [f32; 2], on: bool) {
    amount[0] = amount[1];
    amount[1] = if on {
        (amount[1] + 0.15).min(1.0)
    } else {
        (amount[1] - 0.19).max(0.0)
    };
}
fn update_amounts(m: &mut Mob) {
    let (sit, back, roll) = (is_sitting(m), is_on_back(m), is_rolling(m));
    let s = st(m);
    step_amount(&mut s.sit_amount, sit);
    step_amount(&mut s.on_back_amount, back);
    step_amount(&mut s.roll_amount, roll);
}
/// `Panda.handleRoll`; a client counts the roll and leaves the motion to the
/// server.
fn handle_roll(m: &mut Mob, server: bool) {
    let counter = {
        let s = st(m);
        s.roll_counter += 1;
        s.roll_counter
    };
    if counter > 32 {
        set_flag(m, FLAG_ROLL, false);
    } else if server {
        let movement = m.velocity;
        if counter == 1 {
            let angle = m.yaw * DEG_TO_RAD;
            let multiplier = if m.is_baby() { 0.1_f32 } else { 0.2_f32 };
            let delta = [
                movement.x + f64::from(-angle.msin() * multiplier),
                movement.z + f64::from(angle.mcos() * multiplier),
            ];
            st(m).roll_delta = delta;
            m.velocity = Vec3::new(delta[0], 0.27, delta[1]);
        } else if counter != 7 && counter != 15 && counter != 23 {
            let [x, z] = st(m).roll_delta;
            m.velocity = Vec3::new(x, movement.y, z);
        } else {
            let y = if m.on_ground { 0.27 } else { movement.y };
            m.velocity = Vec3::new(0.0, y, 0.0);
        }
    }
}
fn rnd(m: &mut Mob, bound: i32) -> i32 {
    m.random.next_int_bound(bound).unwrap_or(0)
}
/// `Panda.handleEating`: the bite's sound and crumbs, and the end of a meal.
fn handle_eating(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    let holds = m.extra.main_hand.is_some();
    if !is_eating(m) && is_sitting(m) && !is_scared(m, w.thundering) && holds && rnd(m, 80) == 1 {
        eat(m, true);
    } else if !holds || !is_sitting(m) {
        eat(m, false);
    }
    if !is_eating(m) {
        return;
    }
    // addEatingParticles
    if st(m).eat % 5 == 0 {
        let volume = 0.5 + 0.5 * rnd(m, 2) as f32;
        let pitch = (m.random.next_float() - m.random.next_float()) * 0.2 + 1.0;
        play(m, "entity.panda.eat", volume, pitch, events);
        if holds {
            let crumbs = crumbs(m);
            st(m).crumbs = Some(crumbs);
        }
    }
    let count = st(m).eat;
    if count > 80 && rnd(m, 20) == 1 {
        let eats_from_ground = m.extra.main_hand.as_deref().is_some_and(|item| {
            w.items
                .and_then(|i| i.item_tags.get("minecraft:panda_eats_from_ground"))
                .is_some_and(|tag| tag.contains(item))
        });
        if count > 100 && eats_from_ground {
            super::drops::clear_slot(m, "mainhand");
            sit(m, false);
        }
        eat(m, false);
        return;
    }
    st(m).eat += 1;
}
/// The six crumbs of `addEatingParticles`, drawn as the server draws them.
fn crumbs(m: &mut Mob) -> Crumbs {
    let (x_rot, y_rot) = (-m.head_pitch * DEG_TO_RAD, -m.yaw * DEG_TO_RAD);
    let body = -m.body_yaw * DEG_TO_RAD;
    let (x, y, z) = (m.position.x, m.eye_y() + 1.0, m.position.z);
    (0..6)
        .map(|_| {
            let r = &mut m.random;
            let mut f = || f64::from(r.next_float());
            let v = Vec3::new((f() - 0.5) * 0.1, f() * 0.1 + 0.1, (f() - 0.5) * 0.1)
                .rotate_x(x_rot)
                .rotate_y(y_rot);
            let p = Vec3::new((f() - 0.5) * 0.8, -f() * 0.6 - 0.3, 1.0 + (f() - 0.5) * 0.4)
                .rotate_y(body);
            ([p.x + x, p.y + y, p.z + z], [v.x, v.y + 0.05, v.z])
        })
        .collect()
}
/// `afterSneeze`'s particle, from the nose.
fn sneeze_particle(
    at: [f64; 3],
    width: f32,
    body_yaw: f32,
    eye: f64,
    movement: Vec3,
) -> ([f64; 3], [f64; 3]) {
    let reach = f64::from(width + 1.0) * 0.5;
    let angle = body_yaw * DEG_TO_RAD;
    (
        [
            at[0] - reach * f64::from(angle.msin()),
            eye - f64::from(0.1_f32),
            at[2] + reach * f64::from(angle.mcos()),
        ],
        [movement.x, 0.0, movement.z],
    )
}
/// `Panda.afterSneeze`: the particle, the sound, a start for the grown pandas
/// about, and a slime ball one sneeze in 700.
fn after_sneeze(m: &mut Mob, events: &mut Vec<MobEvent>) {
    let at = [m.position.x, m.position.y, m.position.z];
    let particle = sneeze_particle(at, m.width(), m.body_yaw, m.eye_y(), m.velocity);
    st(m).sneezed = Some(particle);
    play(m, "entity.panda.sneeze", 1.0, 1.0, events);
    events.push(MobEvent::PandaSneezed { mob: m.id });
    events.push(MobEvent::GiftLoot {
        mob: m.id,
        table: "minecraft:gameplay/panda_sneeze",
        at,
    });
}
/// `Panda.tick` after `super.tick()`.
pub fn tick(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    let thundering = w.thundering;
    if is_worried(m) {
        if thundering && !m.fluids.in_water {
            sit(m, true);
            eat(m, false);
        } else if !is_eating(m) {
            sit(m, false);
        }
    }
    if m.ai.target.is_none() {
        let s = st(m);
        s.got_bamboo = false;
        s.did_bite = false;
    }
    let unhappy = st(m).unhappy;
    if unhappy > 0 {
        if let Some(t) = m.target_info(w) {
            m.turn_to([t.position.x, t.eye_y, t.position.z], 90.0, 90.0);
        }
        if unhappy == 29 || unhappy == 14 {
            play(m, "entity.panda.cant_breed", 1.0, 1.0, events);
        }
        st(m).unhappy -= 1;
    }
    if flag(m, FLAG_SNEEZE) {
        let count = {
            let s = st(m);
            s.sneeze += 1;
            s.sneeze
        };
        if count > 20 {
            sneeze(m, false);
            after_sneeze(m, events);
        } else if count == 1 {
            play(m, "entity.panda.pre_sneeze", 1.0, 1.0, events);
        }
    }
    if is_rolling(m) {
        handle_roll(m, true);
    } else {
        st(m).roll_counter = 0;
    }
    if is_sitting(m) {
        m.head_pitch = 0.0;
    }
    st(m).scared = is_scared(m, thundering);
    update_amounts(m);
    handle_eating(m, w, events);
    // updateOnBackAnimation and updateRollAmount ran with the others.
}
/// `Panda.tick` for a panda a client only displays: the flags and counters
/// are the server's; the amounts and the roll count are the client's own.
pub fn client_tick(m: &mut Mob, thundering: bool) {
    if is_rolling(m) {
        handle_roll(m, false);
    } else {
        st(m).roll_counter = 0;
    }
    if is_sitting(m) {
        m.head_pitch = 0.0;
    }
    st(m).scared = is_scared(m, thundering);
    update_amounts(m);
}

// ---- drawing ---------------------------------------------------------------

/// `PandaRenderer.getTextureLocation`.
pub fn texture(m: &Mob) -> String {
    let v = usize::from(variant(m)).min(GENES.len() - 1);
    let gene = GENES[v];
    match (m.is_baby(), gene) {
        (false, "normal") => "entity/panda/panda".into(),
        (false, g) => format!("entity/panda/panda_{g}"),
        (true, "normal") => "entity/panda/panda_baby".into(),
        (true, g) => format!("entity/panda/{g}_panda_baby"),
    }
}
/// What `PandaRenderState` gives `PandaModel.setupAnim`.
pub fn pose(m: &Mob, partial: f32) -> Pose {
    let Some(s) = get(m) else {
        return Pose::default();
    };
    let amount = |a: &[f32; 2]| lerp(partial, a[0], a[1]);
    Pose {
        unhappy: s.unhappy > 0,
        sneezing: s.flags & FLAG_SNEEZE != 0,
        sneeze_time: s.sneeze,
        eating: s.eat > 0,
        scared: s.scared,
        sit_amount: amount(&s.sit_amount),
        lie_on_back_amount: amount(&s.on_back_amount),
        roll_amount: if m.is_baby() {
            0.0
        } else {
            amount(&s.roll_amount)
        },
    }
}
/// `PandaRenderer.setupRotations` after the common ones: the roll over, the
/// sit up and the lie on its back, and a scared panda's shiver.
pub fn rotations(m: &Mob, partial: f32) -> glam::Mat4 {
    use glam::{Mat4, Vec3};
    let Some(s) = get(m) else {
        return Mat4::IDENTITY;
    };
    let baby = m.is_baby();
    let x_rot = m.previous_head_pitch + (m.head_pitch - m.previous_head_pitch) * partial;
    let mut t = Mat4::IDENTITY;
    let translate = |y: f32| Mat4::from_translation(Vec3::new(0.0, y, 0.0));
    let tilt = |degrees: f32| Mat4::from_rotation_x(degrees.to_radians());
    if s.roll_counter > 0 {
        let time = s.roll_counter as f32 + partial;
        let (pos, frac) = (time.floor() as i32, time.fract());
        let next = pos + 1;
        let y = if baby { 0.3 } else { 0.8 };
        // getAngle: the angle between this step and the next, except in the
        // last step of a quarter.
        let angle = |this: f32, next_angle: f32, threshold: i32| {
            if next < threshold {
                lerp(frac, this, next_angle)
            } else {
                this
            }
        };
        let quarter = pos / 8;
        if (0..4).contains(&quarter) {
            let base = 90.0 * quarter as f32;
            let step = |p: i32| base + 90.0 * (p - quarter * 8) as f32 / 7.0;
            let a = angle(step(pos), step(next), (quarter + 1) * 8);
            let lift = match quarter {
                0 => (y + 0.2) * (a / 90.0),
                1 => y + 0.2 + (y - 0.2) * (a - 90.0) / 90.0,
                2 => y + y * (270.0 - a) / 90.0,
                _ => y * ((360.0 - a) / 90.0),
            };
            t = t * translate(lift) * tilt(-a);
        }
    }
    let sit = s.sit_amount;
    let sit = lerp(partial, sit[0], sit[1]);
    if sit > 0.0 {
        t = t * translate(0.8 * sit) * tilt(lerp(sit, x_rot, x_rot + 90.0)) * translate(-sit);
        if s.scared {
            let age = m.age as f32 + partial;
            let shake = (jmath::cos(f64::from(age * 1.25))
                * std::f64::consts::PI
                * f64::from(0.05_f32)) as f32;
            t *= Mat4::from_rotation_y(shake.to_radians());
            if baby {
                t *= Mat4::from_translation(Vec3::new(0.0, 0.8, 0.55));
            }
        }
    }
    let back = lerp(partial, s.on_back_amount[0], s.on_back_amount[1]);
    if back > 0.0 {
        let y = if baby { 0.5 } else { 1.3 };
        t = t * translate(y * back) * tilt(lerp(back, x_rot, x_rot + 180.0));
    }
    t
}
/// `PandaHoldsItemLayer`: where the item in its mouth sits, in the model's
/// space, while it sits and eats.
pub fn item_offset(m: &Mob, partial: f32) -> Option<glam::Mat4> {
    let s = get(m)?;
    if m.extra.main_hand.is_none() || s.flags & FLAG_SIT == 0 || s.scared {
        return None;
    }
    let (mut y, mut z) = (1.4_f32, -0.6_f32);
    if s.eat > 0 {
        let bite = ((m.age as f32 + partial) * 0.6).msin();
        z -= 0.2 * bite + 0.2;
        y -= 0.09 * bite;
    }
    Some(glam::Mat4::from_translation(glam::Vec3::new(0.1, y, z)))
}

// ---- goals -----------------------------------------------------------------

/// A goal that starts only when `when` allows it (`PandaAttackGoal`,
/// `PandaAvoidGoal`).
#[derive(Clone, Debug)]
struct When<G> {
    goal: G,
    when: fn(&Mob, &MobWorld) -> bool,
}
impl<G: Goal + Clone + 'static> Goal for When<G> {
    goal_clone!();
    fn flags(&self) -> u8 {
        self.goal.flags()
    }
    fn name(&self) -> &'static str {
        self.goal.name()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        (self.when)(m, cx.w) && self.goal.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.goal.can_continue(m, cx)
    }
    fn interruptable(&self) -> bool {
        self.goal.interruptable()
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.goal.start(m, cx)
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.goal.stop(m, cx)
    }
    fn every_tick(&self) -> bool {
        self.goal.every_tick()
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.goal.tick(m, cx)
    }
}
fn acts(m: &Mob, w: &MobWorld) -> bool {
    can_perform_action(m, w.thundering)
}
fn avoids(m: &Mob, w: &MobWorld) -> bool {
    is_worried(m) && acts(m, w)
}

/// `PandaPanicGoal`: panics at the environment only, and not while sitting.
#[derive(Clone, Debug)]
struct PandaPanic(Panic);
impl Goal for PandaPanic {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn name(&self) -> &'static str {
        "Panic"
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        environmental(m) && self.0.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if is_sitting(m) {
            m.nav_stop();
            false
        } else {
            self.0.can_continue(m, cx)
        }
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx)
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx)
    }
}
/// `DamageTypeTags.PANIC_ENVIRONMENTAL_CAUSES`, or burning.
fn environmental(m: &Mob) -> bool {
    m.fire_ticks > 0 && !m.kind.flags.fire_immune
        || m.last_damage.is_some_and(|(d, t)| {
            matches!(
                d.kind,
                DamageKind::OnFire
                    | DamageKind::InFire
                    | DamageKind::Lava
                    | DamageKind::Cactus
                    | DamageKind::LightningBolt
                    | DamageKind::HotFloor
            ) && i64::from(m.age) - t <= 40
        })
}

/// `PandaBreedGoal`: a panda in love finds no mate happy unless there is
/// bamboo about.
#[derive(Clone, Debug)]
struct PandaBreed {
    breed: Breed,
    unhappy_cooldown: i64,
}
impl PandaBreed {
    /// `canFindBamboo`: any bamboo within 7 blocks and 2 up.
    fn can_find_bamboo(m: &Mob, w: &MobWorld) -> bool {
        let at = [
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        ];
        (0..3).any(|y| {
            (-7..=7).any(|x| {
                (-7..=7)
                    .any(|z| w.block_name(at[0] + x, at[1] + y, at[2] + z) == "minecraft:bamboo")
            })
        })
    }
}
impl Goal for PandaBreed {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn name(&self) -> &'static str {
        "Breed"
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !self.breed.can_use(m, cx) || st(m).unhappy != 0 {
            return false;
        }
        if Self::can_find_bamboo(m, cx.w) {
            return true;
        }
        if self.unhappy_cooldown <= i64::from(m.age) {
            st(m).unhappy = 32;
            self.unhappy_cooldown = i64::from(m.age) + 600;
            // The nearest player it looks at while it sulks.
            let seen = m.can_see_player(cx.w, 8.0);
            st(m).look_at = seen.then_some(Who::Player);
        }
        false
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.breed.can_continue(m, cx)
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.breed.stop(m, cx)
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.breed.tick(m, cx)
    }
}

/// `PandaHurtByTargetGoal`: a panda that was fed bamboo or bit lets go.
#[derive(Clone, Debug)]
struct PandaHurtBy(HurtByTarget);
impl Goal for PandaHurtBy {
    goal_clone!();
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn name(&self) -> &'static str {
        self.0.name()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let s = st(m);
        if s.got_bamboo || s.did_bite {
            m.ai.target = None;
            false
        } else {
            self.0.can_continue(m, cx)
        }
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx)
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx)
    }
}

/// What `PandaSitGoal` and `PandaLieOnBackGoal` ask each tick of their run.
fn rest_goes_on(m: &mut Mob) -> bool {
    if !m.fluids.in_water && (is_lazy(m) || rnd(m, reduced(600)) != 1) {
        rnd(m, reduced(2000)) != 1
    } else {
        false
    }
}

/// `PandaSitGoal`: sit down with bamboo or cake in the mouth, or go to get
/// some from the ground.
#[derive(Clone, Debug, Default)]
struct PandaSit {
    cooldown: i64,
}
impl Goal for PandaSit {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if self.cooldown > i64::from(m.age)
            || m.is_baby()
            || m.fluids.in_water
            || !acts(m, cx.w)
            || st(m).unhappy > 0
        {
            return false;
        }
        m.extra.main_hand.is_some() || st(m).food_near
    }
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        rest_goes_on(m)
    }
    fn tick(&mut self, m: &mut Mob, _: &mut Cx) {
        if !is_sitting(m) && m.extra.main_hand.is_some() {
            try_to_sit(m);
        }
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        if m.extra.main_hand.is_none() {
            if let Some(at) = st(m).food_at {
                m.nav_move_to_entity(cx.w, at, 0, f64::from(1.2_f32));
            }
        } else {
            try_to_sit(m);
        }
        self.cooldown = 0;
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(item) = m.extra.main_hand.clone() {
            let count = held_count(m);
            cx.events.push(MobEvent::DropItem {
                item,
                count,
                at: [m.position.x, m.position.y, m.position.z],
            });
            super::drops::clear_slot(m, "mainhand");
            let wait = if is_lazy(m) {
                rnd(m, 50) + 10
            } else {
                rnd(m, 150) + 10
            };
            self.cooldown = i64::from(m.age) + i64::from(wait) * 20;
        }
        sit(m, false);
    }
}

/// `PandaLieOnBackGoal`: a lazy panda now and then lies down.
#[derive(Clone, Debug, Default)]
struct PandaLieOnBack {
    cooldown: i64,
}
impl Goal for PandaLieOnBack {
    goal_clone!();
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.cooldown < i64::from(m.age) && is_lazy(m) && acts(m, cx.w) && rnd(m, reduced(400)) == 1
    }
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        rest_goes_on(m)
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        set_flag(m, FLAG_ON_BACK, true);
        self.cooldown = 0;
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        set_flag(m, FLAG_ON_BACK, false);
        self.cooldown = i64::from(m.age) + 200;
    }
}

/// `PandaSneezeGoal`: a cub sneezes now and then, a weak one more.
#[derive(Clone, Debug)]
struct PandaSneeze;
impl Goal for PandaSneeze {
    goal_clone!();
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !(m.is_baby() && acts(m, cx.w)) {
            return false;
        }
        if is_weak(m) && rnd(m, reduced(500)) == 1 {
            true
        } else {
            rnd(m, reduced(6000)) == 1
        }
    }
    fn can_continue(&mut self, _: &mut Mob, _: &mut Cx) -> bool {
        false
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        sneeze(m, true);
    }
}

/// `PandaLookAtPlayerGoal`: looks at the nearest player, or at the one the
/// breed goal sulks at.
#[derive(Clone, Debug, Default)]
struct PandaLook {
    look_time: i32,
}
impl PandaLook {
    fn target(m: &Mob, cx: &Cx) -> Option<(Vec3, f64)> {
        match get(m)?.look_at? {
            Who::Player => {
                cx.w.player
                    .as_ref()
                    .filter(|p| p.alive)
                    .map(|p| (p.position, p.eye[1]))
            }
            Who::Mob(id) => {
                cx.w.seen(id)
                    .filter(|s| s.alive)
                    .map(|s| (s.position, s.eye_y))
            }
        }
    }
}
impl Goal for PandaLook {
    goal_clone!();
    fn flags(&self) -> u8 {
        LOOK
    }
    fn name(&self) -> &'static str {
        "LookAtPlayer"
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if m.random.next_float() >= 0.02 {
            return false;
        }
        if st(m).look_at.is_none() {
            let seen = m.can_see_player(cx.w, 6.0);
            st(m).look_at = seen.then_some(Who::Player);
        }
        acts(m, cx.w) && st(m).look_at.is_some()
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some((at, _)) = Self::target(m, cx) else {
            return false;
        };
        m.distance_sqr(at) <= 36.0 && self.look_time > 0
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        self.look_time = reduced(40 + rnd(m, 40));
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        st(m).look_at = None;
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some((at, eye)) = Self::target(m, cx) {
            m.look_at([at.x, eye, at.z]);
            self.look_time -= 1;
        }
    }
}

/// `PandaRollGoal`: a cub, or a playful panda, rolls over when it faces a
/// drop and now and then at random.
#[derive(Clone, Debug)]
struct PandaRoll;
impl Goal for PandaRoll {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK | JUMP
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !((m.is_baby() || is_playful(m)) && m.on_ground) || !acts(m, cx.w) {
            return false;
        }
        let angle = m.yaw * DEG_TO_RAD;
        let (x_dir, z_dir) = (-angle.msin(), angle.mcos());
        let step = |d: f32| if d.abs() > 0.5 { d.signum() as i32 } else { 0 };
        let (x, z) = (
            m.position.x.floor() as i32 + step(x_dir),
            m.position.z.floor() as i32 + step(z_dir),
        );
        let below = m.position.y.floor() as i32 - 1;
        if cx.w.facts.is_air(cx.w.world.block(x, below, z)) {
            return true;
        }
        if is_playful(m) && rnd(m, reduced(60)) == 1 {
            true
        } else {
            rnd(m, reduced(500)) == 1
        }
    }
    fn can_continue(&mut self, _: &mut Mob, _: &mut Cx) -> bool {
        false
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        set_flag(m, FLAG_ROLL, true);
    }
    fn interruptable(&self) -> bool {
        false
    }
}

/// `Panda.registerGoals`.
pub fn register(m: &mut Mob) {
    let food = m.kind.food.clone();
    let monsters = super::species::ALL
        .iter()
        .map(|d| d.species)
        .filter(|s| super::ai::is_monster(*s))
        .collect::<Vec<Species>>();
    let g = &mut m.ai.goals;
    g.add(0, Float);
    g.add(2, PandaPanic(Panic::new(2.0)));
    g.add(
        2,
        PandaBreed {
            breed: Breed::new(1.0),
            unhappy_cooldown: 0,
        },
    );
    g.add(
        3,
        When {
            goal: MeleeAttack::new(f64::from(1.2_f32), true),
            when: acts,
        },
    );
    g.add(4, Tempt::new(1.0, food, false));
    g.add(
        6,
        When {
            goal: AvoidEntity::new(Avoid::Player, 8.0, 2.0, 2.0),
            when: avoids,
        },
    );
    g.add(
        6,
        When {
            goal: AvoidEntity::new(Avoid::Species(monsters), 4.0, 2.0, 2.0),
            when: avoids,
        },
    );
    g.add(7, PandaSit::default());
    g.add(8, PandaLieOnBack::default());
    g.add(8, PandaSneeze);
    g.add(9, PandaLook::default());
    g.add(10, RandomLookAround::default());
    g.add(12, PandaRoll);
    g.add(13, FollowParent::new(1.25));
    g.add(14, Stroll::avoiding_water(1.0));
    // alertOther: only an aggressive panda joins in.
    m.ai.targets.add(
        1,
        PandaHurtBy(
            HurtByTarget::new()
                .alert()
                .alert_if(|s| s.species == Species::Panda && s.variant == AGGRESSIVE),
        ),
    );
}
