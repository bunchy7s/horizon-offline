//! Leads (`Leashable`): who holds a mob's lead and the elastic physics of
//! `Leashable.tickLeash` (a pull once the rope stretches past six blocks,
//! the lead snapping past twelve, `PathfinderMob`'s walk toward the holder
//! while the rope is slack).
use super::{Mob, MobEvent, MobWorld, Species, Who};
use horizon_core::geometry::Vec3;
use horizon_world::nbt::Tag;

/// `Leashable.LEASH_ELASTIC_DIST`.
pub const ELASTIC: f64 = 6.0;
/// `Leashable.LEASH_TOO_FAR_DIST`.
pub const SNAP: f64 = 12.0;

/// What holds the other end of a lead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Holder {
    Player,
    Mob(u32),
    /// A boat or raft (`AbstractBoat` is `Leashable`, and any entity can be a holder).
    Boat(u32),
    /// A `LeashFenceKnotEntity` on the fence at this block.
    Knot([i32; 3]),
}
impl Holder {
    pub fn who(self) -> Option<Who> {
        match self {
            Holder::Player => Some(Who::Player),
            Holder::Mob(id) => Some(Who::Mob(id)),
            Holder::Boat(_) | Holder::Knot(_) => None,
        }
    }
}
/// A holder read from a save that is not in the world yet (the entity may
/// load later): the holder's UUID, or the fence knot's block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pending {
    Uuid([i32; 4]),
    Knot([i32; 3]),
}
/// `Leashable.LeashData.CODEC`: `{UUID: [I;..]}` for an entity holder or the
/// fence's block position as an int array.
pub fn pending_from_nbt(tag: Option<&Tag>) -> Option<Pending> {
    match tag? {
        Tag::IntArray(p) if p.len() == 3 => Some(Pending::Knot([p[0], p[1], p[2]])),
        t @ Tag::Compound(_) => crate::save::uuid_from_tag(t.get("UUID")).map(Pending::Uuid),
        _ => None,
    }
}
/// The `leash` tag for a holder: the holder entity's UUID, or the knot's block.
pub fn to_nbt(holder: Holder, uuid: Option<[i32; 4]>) -> Option<Tag> {
    match holder {
        Holder::Knot(p) => Some(Tag::IntArray(p.to_vec())),
        _ => uuid.map(|u| Tag::compound([("UUID", Tag::IntArray(u.to_vec()))])),
    }
}
/// Where the other end of the lead is this tick.
#[derive(Clone, Copy, Debug)]
pub struct HolderState {
    pub position: Vec3,
    pub center: Vec3,
    pub width: f64,
    pub height: f64,
    pub yaw: f32,
    pub velocity: Vec3,
}
/// `LeashFenceKnotEntity`: 0.375 wide, 0.5 tall, centred a block's 0.375 up.
pub fn knot_state(pos: [i32; 3]) -> HolderState {
    let position = Vec3::new(
        f64::from(pos[0]) + 0.5,
        f64::from(pos[1]) + 0.375,
        f64::from(pos[2]) + 0.5,
    );
    HolderState {
        position,
        center: Vec3::new(position.x, position.y + 0.25, position.z),
        width: 0.375,
        height: 0.5,
        yaw: 0.0,
        velocity: Vec3::ZERO,
    }
}
/// The holder's state, or `None` when it is gone.
pub fn holder_state(w: &MobWorld, holder: Holder) -> Option<HolderState> {
    match holder {
        Holder::Player => {
            let p = w.player.as_ref()?;
            let size = p.bounds.max_position().subtract(p.bounds.min_position());
            Some(HolderState {
                position: p.position,
                center: Vec3::new(p.position.x, p.position.y + size.y / 2.0, p.position.z),
                width: size.x,
                height: size.y,
                yaw: p.yaw,
                velocity: p.velocity,
            })
        }
        Holder::Mob(id) => {
            let s = w.seen(id).filter(|s| s.alive)?;
            let lo = s.bounds.min_position();
            let hi = s.bounds.max_position();
            Some(HolderState {
                position: s.position,
                center: Vec3::new(
                    (lo.x + hi.x) / 2.0,
                    (lo.y + hi.y) / 2.0,
                    (lo.z + hi.z) / 2.0,
                ),
                width: hi.x - lo.x,
                height: hi.y - lo.y,
                yaw: s.yaw,
                velocity: s.velocity,
            })
        }
        Holder::Boat(id) => w
            .leash_boats
            .iter()
            .find(|(boat, _)| *boat == id)
            .map(|(_, state)| *state),
        Holder::Knot(pos) => {
            let fence = w.has_tag("minecraft:fences", w.block_name(pos[0], pos[1], pos[2]));
            fence.then(|| knot_state(pos))
        }
    }
}
/// `Entity.yRot(angle)` on a vector (rotation about the Y axis).
fn rot_y(v: [f64; 3], angle: f64) -> [f64; 3] {
    let (s, c) = (angle.sin(), angle.cos());
    [v[0] * c + v[2] * s, v[1], v[2] * c - v[0] * s]
}
/// `Leashable.computeDampenedSpringInteraction` for the single attachment
/// of a mob (0, 0.5, 0.5 of its box) and its holder (0, 0.5, 0): the force
/// toward the holder and the torque on the mob, or `None` while slack.
pub fn spring(
    position: Vec3,
    yaw: f32,
    size: (f64, f64),
    holder: &HolderState,
    own_motion: Vec3,
) -> Option<([f64; 3], f64)> {
    let (w, h) = size;
    let lever = rot_y([0.0, 0.5 * h, 0.5 * w], -f64::from(yaw.to_radians()));
    let object = [
        position.x + lever[0],
        position.y + lever[1],
        position.z + lever[2],
    ];
    let arm = rot_y(
        [0.0, 0.5 * holder.height, 0.0],
        -f64::from(holder.yaw.to_radians()),
    );
    let pivot = [
        holder.position.x + arm[0],
        holder.position.y + arm[1],
        holder.position.z + arm[2],
    ];
    let d = [
        pivot[0] - object[0],
        pivot[1] - object[1],
        pivot[2] - object[2],
    ];
    let distance = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if distance < ELASTIC {
        return None;
    }
    let scale = (distance - ELASTIC) / distance;
    let mut force = [d[0] * scale, d[1] * scale, d[2] * scale];
    let torque = lever[2] * force[0] - lever[0] * force[2];
    let same_direction =
        own_motion.x * force[0] + own_motion.y * force[1] + own_motion.z * force[2] >= 0.0;
    if same_direction {
        for f in &mut force {
            *f *= f64::from(0.3_f32);
        }
    }
    Some((force, torque))
}
/// `Leashable.checkElasticInteractions` for an entity of this size at
/// `position` turned `yaw`, hanging on `holder`: the summed force toward it
/// and the torque, or `None` while every spring is slack. With `quad` (a
/// holder that `supportQuadLeashAsHolder` and a leashee that supports one)
/// the four corners pull, a quarter each.
pub fn elastic_pull(
    position: Vec3,
    yaw: f32,
    size: (f64, f64),
    holder: &HolderState,
    motion: Vec3,
    quad: bool,
) -> Option<([f64; 3], f64)> {
    // SHARED_QUAD_ATTACHMENT_POINTS, ENTITY_ATTACHMENT_POINT, LEASHER_ATTACHMENT_POINT.
    const SHARED_QUAD: [[f64; 3]; 4] = [
        [-0.5, 0.5, 0.5],
        [-0.5, 0.5, -0.5],
        [0.5, 0.5, -0.5],
        [0.5, 0.5, 0.5],
    ];
    const OWN: [[f64; 3]; 1] = [[0.0, 0.5, 0.5]];
    const THEIRS: [[f64; 3]; 1] = [[0.0, 0.5, 0.0]];
    let (own, theirs): (&[[f64; 3]], &[[f64; 3]]) = if quad {
        (&SHARED_QUAD, &SHARED_QUAD)
    } else {
        (&OWN, &THEIRS)
    };
    let (w, h) = size;
    let own_yaw = f64::from(yaw.to_radians());
    let holder_yaw = f64::from(holder.yaw.to_radians());
    let mut force = [0.0; 3];
    let mut torque = 0.0;
    let mut any = false;
    for (mine, other) in own.iter().zip(theirs) {
        let lever = rot_y([mine[0] * w, mine[1] * h, mine[2] * w], -own_yaw);
        let object = [
            position.x + lever[0],
            position.y + lever[1],
            position.z + lever[2],
        ];
        let arm = rot_y(
            [
                other[0] * holder.width,
                other[1] * holder.height,
                other[2] * holder.width,
            ],
            -holder_yaw,
        );
        let pivot = [
            holder.position.x + arm[0],
            holder.position.y + arm[1],
            holder.position.z + arm[2],
        ];
        let d = [
            pivot[0] - object[0],
            pivot[1] - object[1],
            pivot[2] - object[2],
        ];
        let distance = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if distance < ELASTIC {
            continue;
        }
        // Vec3.normalize, then scale(distance - slack).
        let length = distance;
        let unit = if length < 1.0e-5 {
            [0.0; 3]
        } else {
            [d[0] / length, d[1] / length, d[2] / length]
        };
        let pull = distance - ELASTIC;
        let mut f = [unit[0] * pull, unit[1] * pull, unit[2] * pull];
        let t = lever[2] * f[0] - lever[0] * f[2];
        if motion.x * f[0] + motion.y * f[1] + motion.z * f[2] >= 0.0 {
            for v in &mut f {
                *v *= f64::from(0.3_f32);
            }
        }
        for k in 0..3 {
            force[k] += f[k];
        }
        torque += t;
        any = true;
    }
    if !any {
        return None;
    }
    if quad {
        for v in &mut force {
            *v *= 0.25;
        }
        torque *= 0.25;
    }
    Some((force, torque))
}
/// `Entity.checkFallDistanceAccumulation`.
fn check_fall_distance(m: &mut Mob) {
    if m.velocity.y > -0.5 && m.fall_distance > 1.0 {
        m.fall_distance = 1.0;
    }
}
/// Whether a mob can be put on a lead (`Mob.canBeLeashed` and overrides).
pub fn can_be_leashed(m: &Mob) -> bool {
    use Species as S;
    match m.species() {
        S::Hoglin | S::Zoglin => true,
        S::Villager
        | S::WanderingTrader
        | S::Turtle
        | S::Panda
        | S::Bat
        | S::Cod
        | S::Salmon
        | S::TropicalFish
        | S::Pufferfish
        | S::Tadpole => false,
        S::Wolf => !super::tame::angry(m),
        _ => !m.hostile() && !m.removed,
    }
}
/// `Leashable.tickLeash` at the end of `Entity.baseTick`.
pub fn tick(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    let Some(holder) = m.extra.leash else {
        m.extra.leash_spin = 0.0;
        return;
    };
    // Leashable.tickLeash: a dying entity cannot interact with the level.
    if !m.alive() {
        events.push(MobEvent::LeashBreak {
            mob: m.id,
            drop: true,
            sound: false,
        });
        return;
    }
    let Some(state) = holder_state(w, holder) else {
        // The holder is gone: the lead drops.
        if !matches!(holder, Holder::Player) {
            events.push(MobEvent::LeashBreak {
                mob: m.id,
                drop: true,
                sound: false,
            });
        }
        return;
    };
    // PathfinderMob.whenLeashedTo: home is the holder's block, within the
    // elastic distance less one.
    let elastic = if m.species() == super::Species::HappyGhast {
        10.0
    } else {
        ELASTIC
    };
    m.ai.home = Some((
        [
            state.position.x.floor() as i32,
            state.position.y.floor() as i32,
            state.position.z.floor() as i32,
        ],
        elastic as i32 - 1,
    ));
    // Leashable.whenLeashedTo: `leashHolder.notifyLeashHolder(this)`.
    if let Holder::Mob(id) = holder
        && supports_quad_leash(m.species())
    {
        events.push(MobEvent::NotifyLeashHolder { holder: id });
    }
    let b = m.bounds();
    let (lo, hi) = (b.min_position(), b.max_position());
    let center = Vec3::new(
        (lo.x + hi.x) / 2.0,
        (lo.y + hi.y) / 2.0,
        (lo.z + hi.z) / 2.0,
    );
    let distance = ((state.center.x - center.x).powi(2)
        + (state.center.y - center.y).powi(2)
        + (state.center.z - center.z).powi(2))
    .sqrt();
    if distance > SNAP {
        events.push(MobEvent::LeashBreak {
            mob: m.id,
            drop: true,
            sound: true,
        });
        return;
    }
    let stretched = distance > ELASTIC - state.width - f64::from(m.width());
    let pulled = stretched
        && spring(
            m.position,
            m.yaw,
            (f64::from(m.width()), f64::from(m.height())),
            &state,
            m.velocity,
        )
        .is_some_and(|(force, torque)| {
            m.extra.leash_spin += 10.0 * torque;
            let rel = Vec3::new(
                state.velocity.x - m.velocity.x,
                state.velocity.y - m.velocity.y,
                state.velocity.z - m.velocity.z,
            );
            m.velocity = Vec3::new(
                m.velocity.x + force[0] * 0.8 + rel.x * 0.11,
                m.velocity.y + force[1] * 0.2 + rel.y * 0.11,
                m.velocity.z + force[2] * 0.8 + rel.z * 0.11,
            );
            check_fall_distance(m);
            super::equine::on_elastic_leash_pull(m, w, events);
            true
        });
    if !pulled {
        close_range(m, w, &state);
    }
    m.yaw -= m.extra.leash_spin as f32;
    let friction = if m.on_ground {
        0.6 * 0.91
    } else if m.is_in_liquid() {
        0.8
    } else {
        0.91
    };
    m.extra.leash_spin *= friction;
}
/// The lead comes off (`Mob.onLeashRemoved`): no more home.
pub fn remove(m: &mut Mob) -> Option<Holder> {
    let was = m.extra.leash.take();
    if was.is_some() {
        m.ai.home = None;
    }
    was
}
/// `PathfinderMob.closeRangeLeashBehaviour`: walk to within two blocks of
/// the holder unless panicking.
fn close_range(m: &mut Mob, w: &MobWorld, holder: &HolderState) {
    if m.ai.panicking {
        return;
    }
    let d = Vec3::new(
        holder.position.x - m.position.x,
        holder.position.y - m.position.y,
        holder.position.z - m.position.z,
    );
    let dist = d.length();
    let len = d.length();
    let scale = if len > 0.0 {
        (dist - 2.0).max(0.0) / len
    } else {
        0.0
    };
    let target = [
        m.position.x + d.x * scale,
        m.position.y + d.y * scale,
        m.position.z + d.z * scale,
    ];
    let speed = follow_speed(m);
    m.nav_move_to(w, target, speed);
}
/// `followLeashSpeed`: 1.0, 2.0 for llamas.
pub fn follow_speed(m: &Mob) -> f64 {
    if super::llama::is_llama(m.species()) {
        2.0
    } else {
        1.0
    }
}
/// `Mob.leashable` position of the rope on a mob (`getLeashOffset`).
pub fn leash_offset(m: &Mob) -> [f64; 3] {
    if super::llama::is_llama(m.species()) {
        // Llama.getLeashOffset.
        [0.0, 0.75 * m.eye_height(), f64::from(m.width()) * 0.5]
    } else if m.species() == super::Species::Wolf {
        // Wolf.getLeashOffset: 0.6 of the eye height.
        [
            0.0,
            f64::from(0.6_f32 * m.eye_height() as f32),
            f64::from(m.width() * 0.4),
        ]
    } else if m.species() == super::Species::Allay {
        // Allay.getLeashOffset.
        [0.0, m.eye_height() * 0.6, f64::from(m.width()) * 0.1]
    } else if matches!(
        m.species(),
        super::Species::Camel | super::Species::CamelHusk
    ) {
        super::camel::leash_offset(m)
    } else if m.species() == super::Species::Bee {
        // Bee.getLeashOffset: half the eye height, a fifth of the width ahead.
        [0.0, 0.5 * m.eye_height(), f64::from(m.width() * 0.2)]
    } else {
        [0.0, m.eye_height(), f64::from(m.width() * 0.4)]
    }
}
/// `Leashable.createQuadLeashOffsets`: four attachment points around an
/// entity of this size, front-left first, turning clockwise from above.
pub fn create_quad_leash_offsets(
    width: f32,
    height: f32,
    front_offset: f64,
    front_back: f64,
    left_right: f64,
    up: f64,
) -> [[f64; 3]; 4] {
    let front = front_offset * f64::from(width);
    let front_back = front_back * f64::from(width);
    let left_right = left_right * f64::from(width);
    let y = up * f64::from(height);
    [
        [-left_right, y, front_back + front],
        [-left_right, y, -front_back + front],
        [left_right, y, -front_back + front],
        [left_right, y, front_back + front],
    ]
}
/// `Leashable.supportQuadLeash`: `AbstractHorse` (horses, donkeys, mules, the
/// undead horses, camels), `Sniffer` and `AbstractBoat` say yes; `Llama`
/// overrides `AbstractHorse` back to no. Boats are not mobs here.
pub fn supports_quad_leash(s: Species) -> bool {
    use Species as S;
    matches!(
        s,
        S::Horse
            | S::Donkey
            | S::Mule
            | S::SkeletonHorse
            | S::ZombieHorse
            | S::Camel
            | S::CamelHusk
            | S::Sniffer
    )
}
/// `Leashable.getQuadLeashOffsets` of a mob for which `supports_quad_leash`:
/// `AbstractHorse`'s, with the overrides of `AbstractChestedHorse` and
/// `ZombieHorse`, `Camel` and `Sniffer`.
pub fn quad_leash_offsets(m: &Mob) -> [[f64; 3]; 4] {
    use Species as S;
    let (front, front_back, left_right, up) = match m.species() {
        S::Donkey | S::Mule | S::ZombieHorse => (0.04, 0.41, 0.18, 0.73),
        S::Camel | S::CamelHusk => (0.02, 0.48, 0.25, 0.82),
        S::Sniffer => (-0.01, 0.63, 0.38, 1.15),
        S::Horse | S::SkeletonHorse => (0.04, 0.52, 0.23, 0.87),
        _ => (0.0, 0.5, 0.5, 0.5),
    };
    create_quad_leash_offsets(m.width(), m.height(), front, front_back, left_right, up)
}
/// `AbstractBoat.getLeashOffset` for a boat or raft (1.375 by 0.5625).
pub fn boat_leash_offset() -> [f64; 3] {
    let (width, height) = (1.375_f32, 0.5625_f32);
    [
        0.0,
        f64::from(0.88_f32 * height),
        f64::from(0.64_f32 * width),
    ]
}
/// `AbstractBoat.getQuadLeashOffsets`.
pub fn boat_quad_leash_offsets() -> [[f64; 3]; 4] {
    create_quad_leash_offsets(1.375, 0.5625, 0.0, 0.64, 0.382, 0.88)
}
/// `Entity.supportQuadLeashAsHolder` and `getQuadLeashHolderOffsets`: the
/// `Ghast` family holds a quad lead, the happy ghast at its own points.
pub fn quad_holder_offsets(m: &Mob) -> Option<[[f64; 3]; 4]> {
    match m.species() {
        Species::HappyGhast => Some(create_quad_leash_offsets(
            m.width(),
            m.height(),
            -0.03125,
            0.4375,
            0.46875,
            0.03125,
        )),
        Species::Ghast => Some(create_quad_leash_offsets(
            m.width(),
            m.height(),
            0.0,
            0.5,
            0.5,
            0.0,
        )),
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quad_leash_points_are_vanillas() {
        // Leashable.createQuadLeashOffsets for a horse (1.3964844 x 1.6).
        let p = create_quad_leash_offsets(1.5, 1.6, 0.04, 0.52, 0.23, 0.87);
        let (y, fb, lr, f) = (
            0.87 * f64::from(1.6_f32),
            0.52 * 1.5,
            0.23 * 1.5,
            0.04 * 1.5,
        );
        let near = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9);
        assert!(near(p[0], [-lr, y, fb + f]));
        assert!(near(p[1], [-lr, y, -fb + f]));
        assert!(near(p[2], [lr, y, -fb + f]));
        assert!(near(p[3], [lr, y, fb + f]));
    }
    #[test]
    fn only_horses_camels_and_sniffers_support_a_quad_lead() {
        assert!(supports_quad_leash(Species::Horse));
        assert!(supports_quad_leash(Species::Camel));
        assert!(supports_quad_leash(Species::Sniffer));
        // Llama.supportQuadLeash is false.
        assert!(!supports_quad_leash(Species::Llama));
        assert!(!supports_quad_leash(Species::TraderLlama));
        assert!(!supports_quad_leash(Species::Cow));
    }
    #[test]
    fn a_boats_lead_hangs_at_its_front_and_corners() {
        let p = boat_leash_offset();
        assert!((p[1] - 0.88 * 0.5625).abs() < 1e-6 && (p[2] - 0.64 * 1.375).abs() < 1e-6);
        let q = boat_quad_leash_offsets();
        // Front-left first, as createQuadLeashOffsets orders them.
        assert!(q[0][0] < 0.0 && q[0][2] > 0.0 && q[2][0] > 0.0 && q[2][2] < 0.0);
        assert!((q[0][2] - 0.64 * 1.375).abs() < 1e-6 && (q[3][0] - 0.382 * 1.375).abs() < 1e-6);
    }
    #[test]
    fn rotating_about_y_matches_vanilla() {
        let v = rot_y([0.0, 1.0, 1.0], std::f64::consts::FRAC_PI_2);
        assert!((v[0] - 1.0).abs() < 1e-9 && v[2].abs() < 1e-9);
    }
    fn holder(x: f64) -> HolderState {
        let position = Vec3::new(x, 64.0, 0.0);
        HolderState {
            position,
            center: Vec3::new(x, 64.9, 0.0),
            width: 0.6,
            height: 1.8,
            yaw: 0.0,
            velocity: Vec3::ZERO,
        }
    }
    #[test]
    fn a_slack_rope_does_not_pull() {
        let pulled = spring(
            Vec3::new(0.0, 64.0, 0.0),
            0.0,
            (0.9, 1.87),
            &holder(3.0),
            Vec3::ZERO,
        );
        assert!(pulled.is_none());
    }
    #[test]
    fn a_stretched_rope_pulls_toward_the_holder() {
        let (force, _) = spring(
            Vec3::new(0.0, 64.0, 0.0),
            0.0,
            (0.9, 1.87),
            &holder(10.0),
            Vec3::ZERO,
        )
        .expect("past the slack");
        // Moving away from nothing: the same-direction damping applies.
        assert!(force[0] > 0.0);
        assert!(force[0] < 10.0 - ELASTIC);
    }
    #[test]
    fn one_spring_matches_the_mob_spring() {
        let at = Vec3::new(1.0, 64.0, 2.0);
        let (a, ta) = spring(at, 30.0, (1.375, 0.5625), &holder(10.0), Vec3::ZERO).unwrap();
        let (b, tb) =
            elastic_pull(at, 30.0, (1.375, 0.5625), &holder(10.0), Vec3::ZERO, false).unwrap();
        assert!((0..3).all(|k| (a[k] - b[k]).abs() < 1e-9));
        assert!((ta - tb).abs() < 1e-9);
    }
    #[test]
    fn a_quad_lead_pulls_a_quarter_from_each_corner() {
        // A holder straight ahead pulls every corner alike, so the quad force
        // is the mean of four springs, not their sum.
        let at = Vec3::new(0.0, 64.0, 0.0);
        let (quad, _) = elastic_pull(at, 0.0, (1.0, 1.0), &holder(10.0), Vec3::ZERO, true).unwrap();
        let (single, _) =
            elastic_pull(at, 0.0, (1.0, 1.0), &holder(10.0), Vec3::ZERO, false).unwrap();
        assert!(quad[0] > 0.0 && quad[0] <= single[0] * 1.5);
        assert!(elastic_pull(at, 0.0, (1.0, 1.0), &holder(3.0), Vec3::ZERO, true).is_none());
    }
    #[test]
    fn moving_with_the_pull_damps_it() {
        let at = Vec3::new(0.0, 64.0, 0.0);
        let still = spring(
            at,
            0.0,
            (0.9, 1.87),
            &holder(10.0),
            Vec3::new(-0.1, 0.0, 0.0),
        )
        .expect("pull")
        .0;
        let following = spring(
            at,
            0.0,
            (0.9, 1.87),
            &holder(10.0),
            Vec3::new(0.1, 0.0, 0.0),
        )
        .expect("pull")
        .0;
        assert!((following[0] - still[0] * 0.3_f32 as f64).abs() < 1e-9);
    }
    #[test]
    fn leash_tags_round_trip() {
        let knot = to_nbt(Holder::Knot([1, 2, 3]), None).expect("knot tag");
        assert_eq!(
            pending_from_nbt(Some(&knot)),
            Some(Pending::Knot([1, 2, 3]))
        );
        let held = to_nbt(Holder::Player, Some([4, 5, 6, 7])).expect("uuid tag");
        assert_eq!(
            pending_from_nbt(Some(&held)),
            Some(Pending::Uuid([4, 5, 6, 7]))
        );
        assert_eq!(to_nbt(Holder::Mob(1), None), None);
    }
    #[test]
    fn a_knot_sits_on_the_fence_post() {
        let k = knot_state([10, 64, -3]);
        assert_eq!(k.position.x, 10.5);
        assert_eq!(k.position.y, 64.375);
        assert_eq!(k.center.y, 64.625);
    }
}
