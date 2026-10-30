//! The illusioner (`Illusioner`): a bow-wielding spellcaster that turns
//! invisible behind four mirror images and blinds its target.
use super::{
    Mob, MobEvent, Species, Who,
    goals::{
        Avoid, AvoidEntity, Cx, Float, Goal, HurtByTarget, LookAtPlayer, NearestTarget, Stroll,
        goal_clone, reduced,
    },
    illager::CastingSpell,
    undead::BowAttack,
};
use horizon_core::random::RandomSource;

const INVISIBILITY: &str = "minecraft:invisibility";
const BLINDNESS: &str = "minecraft:blindness";
/// `Illusioner.NUM_ILLUSIONS`.
pub const COPIES: usize = 4;

/// `Illusioner.clientSideIllusionTicks` and the two offset sets.
#[derive(Clone, Debug, Default)]
pub struct Illusions {
    pub ticks: i32,
    pub offsets: [[[f64; 3]; COPIES]; 2],
}

pub const MIRROR: u8 = 5;
pub const BLINDNESS_SPELL: u8 = 4;

pub fn register(m: &mut Mob) {
    let g = &mut m.ai.goals;
    g.add(0, Float);
    g.add(1, CastingSpell);
    g.add(
        3,
        AvoidEntity::new(Avoid::Species(vec![Species::Creaking]), 8.0, 1.0, 1.2),
    );
    g.add(4, Spell::new(MIRROR));
    g.add(5, Spell::new(BLINDNESS_SPELL));
    g.add(6, BowAttack::new(0.5, 20, 15.0));
    g.add(8, Stroll::new(0.6));
    g.add(9, LookAtPlayer::new(3.0).probability(1.0));
    g.add(10, LookAtPlayer::mobs(8.0));
    let mut hurt = HurtByTarget::new().alert();
    hurt.ignore = vec![
        Species::Pillager,
        Species::Vindicator,
        Species::Witch,
        Species::Evoker,
        Species::Illusioner,
        Species::Vex,
        Species::Ravager,
    ];
    m.ai.targets.add(1, hurt);
    m.ai.targets.add(2, NearestTarget::player(true));
    m.ai.targets.add(
        3,
        NearestTarget::species(&[Species::Villager, Species::WanderingTrader], false),
    );
    m.ai.targets
        .add(3, NearestTarget::species(&[Species::IronGolem], false));
}

/// `Illusioner.finalizeSpawn`: a bow in the main hand.
pub fn finalize(m: &mut Mob) {
    m.extra.set_main_hand(Some("minecraft:bow".into()));
}

/// `IllusionerMirrorSpellGoal` and `IllusionerBlindnessSpellGoal`.
#[derive(Clone, Debug)]
struct Spell {
    spell: u8,
    warmup: i32,
    next: i64,
    last_target: Option<Who>,
}
impl Spell {
    fn new(spell: u8) -> Self {
        Self {
            spell,
            warmup: 0,
            next: 0,
            last_target: None,
        }
    }
    /// (casting time, interval, prepare sound).
    fn timing(&self) -> (i32, i64, &'static str) {
        if self.spell == MIRROR {
            (20, 340, "entity.illusioner.prepare_mirror")
        } else {
            (20, 180, "entity.illusioner.prepare_blindness")
        }
    }
}
impl Goal for Spell {
    goal_clone!();
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !m.target_info(cx.w).is_some_and(|t| t.alive)
            || m.extra.counter > 0
            || i64::from(m.age) < self.next
        {
            return false;
        }
        if self.spell == MIRROR {
            !m.has_effect(INVISIBILITY)
        } else {
            m.ai.target != self.last_target && cx.w.difficulty.effective > 2.0
        }
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.target_info(cx.w).is_some_and(|t| t.alive) && self.warmup > 0
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        let (casting, interval, sound) = self.timing();
        self.warmup = reduced(20);
        m.extra.counter = casting;
        self.next = i64::from(m.age) + interval;
        cx.events.push(MobEvent::Sound(sound, m.center(), 1.0, 1.0));
        m.extra.variant2 = self.spell;
        if self.spell == BLINDNESS_SPELL {
            self.last_target = m.ai.target;
        }
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.warmup -= 1;
        if self.warmup != 0 {
            return;
        }
        if self.spell == MIRROR {
            m.add_effect(INVISIBILITY, 1200, 0);
        } else if m.ai.target == Some(Who::Player) {
            cx.events.push(MobEvent::EffectPlayer {
                effect: BLINDNESS,
                ticks: 400,
                amplifier: 0,
            });
        }
        cx.events.push(MobEvent::Sound(
            "entity.illusioner.cast_spell",
            m.center(),
            1.0,
            1.0,
        ));
    }
}

/// `Entity.isInvisible` of an illusioner on a client: the shared flag stands
/// in for the effect the server holds, which draws the mirror images.
pub fn set_invisible(m: &mut Mob, on: bool) {
    m.extra.effects.retain(|e| e.0 != INVISIBILITY);
    if on {
        m.extra.effects.push((INVISIBILITY, i32::MAX, 0));
    }
}

/// The client-side half of `Illusioner.aiStep`: new image offsets when
/// hurt and once a minute, with a puff of cloud and a sound.
pub fn tick(m: &mut Mob, events: &mut Vec<MobEvent>) {
    if !m.has_effect(INVISIBILITY) {
        return;
    }
    if step(m) {
        events.push(MobEvent::Particles {
            kind: "minecraft:cloud",
            at: [m.position.x, m.position.y + 0.9, m.position.z],
            count: 16,
        });
        events.push(MobEvent::Sound(
            "entity.illusioner.mirror_move",
            m.center(),
            1.0,
            1.0,
        ));
    }
}

/// `Illusioner.aiStep` on a client of a server: invisible is the synched
/// shared flag. The puff is drawn here exactly (sixteen `addParticle`s); the
/// sound comes from the server.
pub fn client_tick(m: &mut Mob, events: &mut Vec<MobEvent>) {
    if !m.invisible || !step(m) {
        return;
    }
    let (w, h) = (f64::from(m.width()), f64::from(m.height()));
    for _ in 0..16 {
        // getRandomX(0.5), getRandomY() and getZ(0.5), which is not random.
        let x = m.position.x + w * (2.0 * m.random.next_double() - 1.0) * 0.5;
        let y = m.position.y + h * m.random.next_double();
        events.push(MobEvent::ParticleAt {
            kind: "cloud",
            at: [x, y, m.position.z + w * 0.5],
            velocity: [0.0; 3],
        });
    }
}

/// The image offsets' clock of an invisible illusioner. True when the images
/// jumped to new offsets (hurt, or once a minute), which the puff goes with.
fn step(m: &mut Mob) -> bool {
    let hurt = m.hurt_time;
    let age = m.age;
    let mut il = m.extra.illusions.take().unwrap_or_default();
    il.ticks = (il.ticks - 1).max(0);
    let jumped = hurt == 1 || age.is_multiple_of(1200);
    if jumped {
        il.ticks = 3;
        for i in 0..COPIES {
            il.offsets[0][i] = il.offsets[1][i];
            let r = &mut m.random;
            let x = (-6 + r.next_int_bound(13).unwrap_or(0)) as f64 * 0.5;
            let y = (r.next_int_bound(6).unwrap_or(0) - 4).max(0) as f64;
            let z = (-6 + r.next_int_bound(13).unwrap_or(0)) as f64 * 0.5;
            il.offsets[1][i] = [x, y, z];
        }
    } else if hurt == 9 {
        il.ticks = 3;
        for i in 0..COPIES {
            il.offsets[0][i] = il.offsets[1][i];
            il.offsets[1][i] = [0.0; 3];
        }
    }
    m.extra.illusions = Some(il);
    jumped
}

/// `Illusioner.getIllusionOffsets`.
pub fn offsets(m: &Mob, partial: f32) -> [[f64; 3]; COPIES] {
    let Some(il) = &m.extra.illusions else {
        return [[0.0; 3]; COPIES];
    };
    if il.ticks <= 0 {
        return il.offsets[1];
    }
    let scale = (f64::from(il.ticks as f32 - partial) / 3.0).powf(0.25);
    let mut out = [[0.0; 3]; COPIES];
    for (i, o) in out.iter_mut().enumerate() {
        for k in 0..3 {
            o[k] = il.offsets[1][i][k] * (1.0 - scale) + il.offsets[0][i][k] * scale;
        }
    }
    out
}

/// The wobble `IllusionerRenderer.submit` adds to each image.
pub fn wobble(i: usize, age: f32) -> [f64; 3] {
    let i = i as f32;
    [
        f64::from((i + age * 0.5).cos()) * 0.025,
        f64::from((i + age * 0.75).cos()) * 0.0125,
        f64::from((i + age * 0.7).cos()) * 0.025,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settled_offsets_are_the_new_set() {
        let il = Illusions {
            ticks: 0,
            offsets: [[[1.0; 3]; COPIES], [[2.0; 3]; COPIES]],
        };
        let mut v = [[0.0; 3]; COPIES];
        v.copy_from_slice(&il.offsets[1]);
        assert_eq!(v[0], [2.0; 3]);
    }
    #[test]
    fn wobble_is_small() {
        assert!(wobble(1, 10.0).iter().all(|c| c.abs() <= 0.025));
    }
}
