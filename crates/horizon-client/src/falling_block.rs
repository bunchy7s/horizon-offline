//! FallingBlockEntity's impact damage, anvil wear, saved impact settings and
//! the cobwebs that hold it back.
use crate::{
    mobs::{Damage as MobDamage, DamageKind, Mob},
    save::{float, number, put},
};
use horizon_core::{
    geometry::{Aabb, Axis, Direction, Vec3, sweep_intersected_blocks},
    position::BlockPos,
};
use horizon_world::{chunk::BlockState, nbt::Tag};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Damage {
    pub hurt_entities: bool,
    pub cancel_drop: bool,
    pub per_distance: f32,
    pub max: i32,
}
impl Default for Damage {
    fn default() -> Self {
        Self {
            hurt_entities: false,
            cancel_drop: false,
            per_distance: 0.0,
            max: 40,
        }
    }
}
impl Damage {
    pub fn anvil() -> Self {
        Self {
            hurt_entities: true,
            per_distance: 2.0,
            ..Self::default()
        }
    }
    /// A block that starts to fall: anvils hurt, and a brushable block
    /// breaks where it lands (BrushableBlock.tick: disableDrop).
    pub fn falling(block: &str) -> Self {
        if block.ends_with("anvil") {
            Self::anvil()
        } else {
            Self {
                cancel_drop: block.starts_with("minecraft:suspicious_"),
                ..Self::default()
            }
        }
    }
    pub fn from_nbt(tag: &Tag, anvil: bool) -> Self {
        Self {
            hurt_entities: number(tag.get("HurtEntities")).map_or(anvil, |v| v != 0),
            cancel_drop: number(tag.get("CancelDrop")).is_some_and(|v| v != 0),
            per_distance: float(tag.get("FallHurtAmount")).unwrap_or(0.0) as f32,
            max: number(tag.get("FallHurtMax")).unwrap_or(40) as i32,
        }
    }
    pub fn save(self, tag: &mut Tag) {
        for (key, value) in [
            ("HurtEntities", Tag::Byte(i8::from(self.hurt_entities))),
            ("CancelDrop", Tag::Byte(i8::from(self.cancel_drop))),
            ("FallHurtAmount", Tag::Float(self.per_distance.to_bits())),
            ("FallHurtMax", Tag::Int(self.max)),
        ] {
            put(tag, key, value);
        }
    }
    pub fn impact(self, fall_distance: f64) -> Option<Impact> {
        if !self.hurt_entities {
            return None;
        }
        let steps = (fall_distance - 1.0).ceil() as i32;
        if steps < 0 {
            return None;
        }
        let amount = ((steps as f32 * self.per_distance).floor() as i32).min(self.max) as f32;
        Some(Impact {
            amount,
            wear_chance: if amount > 0.0 {
                0.05 + steps as f32 * 0.05
            } else {
                0.0
            },
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Impact {
    pub amount: f32,
    pub wear_chance: f32,
}

/// AnvilBlock.damage: advance wear without changing facing; false means broken.
pub fn wear_anvil(state: &mut BlockState) -> bool {
    state.name = match state.name.as_str() {
        "minecraft:anvil" => "minecraft:chipped_anvil",
        "minecraft:chipped_anvil" => "minecraft:damaged_anvil",
        _ => return false,
    }
    .into();
    true
}

pub fn bounds(position: Vec3) -> Aabb {
    let size = f64::from(0.98_f32);
    Aabb::new(
        Vec3::new(position.x - size / 2.0, position.y, position.z - size / 2.0),
        Vec3::new(
            position.x + size / 2.0,
            position.y + size,
            position.z + size / 2.0,
        ),
    )
}
/// `WebBlock.entityInside`'s `makeStuckInBlock` multiplier: the next move
/// goes at a quarter of the speed across, a twentieth down.
pub fn web_multiplier() -> Vec3 {
    Vec3::new(0.25, f64::from(0.05_f32), 0.25)
}
/// `Entity.applyEffectsFromBlocks` after a falling block's move this tick
/// (`from` to `to`; `original` the move it asked for, when the move was
/// recorded): whether `checkInsideBlocks` reaches a cobweb (`is_web`), whose
/// `entityInside` makes it stuck. Each movement sweeps its axes in
/// `Direction.axisStepOrder` within 16 iterations, then the end alone.
pub fn passes_cobweb(
    from: Vec3,
    to: Vec3,
    original: Option<Vec3>,
    mut is_web: impl FnMut(BlockPos) -> bool,
) -> bool {
    let mut stuck = false;
    // checkInsideBlocks(from, to, ..): the iterations used plus one.
    let mut check = |from: Vec3, to: Vec3, max: i32| {
        let d = f64::from(1.0e-5_f32);
        let target = bounds(to).deflate(Vec3::new(d, d, d));
        let mut iterations = 0;
        let _ = sweep_intersected_blocks(from, to, target, 1_000_000, |p, iteration| {
            if iteration >= max {
                return false;
            }
            iterations = iteration;
            // getEntityInsideCollisionShape is the whole block for cobwebs.
            stuck |= is_web(p);
            true
        });
        iterations + 1
    };
    let delta = to.subtract(from);
    let mut budget = 16;
    match original {
        Some(original) if delta.length_squared() > 0.0 => {
            let mut at = from;
            for axis in Axis::step_order(original) {
                let step = delta.component(axis);
                if step != 0.0 {
                    let positive = match axis {
                        Axis::X => Direction::East,
                        Axis::Y => Direction::Up,
                        Axis::Z => Direction::South,
                    };
                    let next = at.relative(positive, step);
                    budget -= check(at, next, budget);
                    at = next;
                }
            }
        }
        _ => budget -= check(from, to, 16),
    }
    if budget <= 0 {
        check(to, to, 1);
    }
    stuck
}
pub fn hurt_mobs(mobs: &mut [Mob], position: Vec3, amount: f32) {
    hurt_mobs_with_kind(mobs, position, amount, DamageKind::FallingBlock);
}

pub fn hurt_mobs_with_kind(mobs: &mut [Mob], position: Vec3, amount: f32, kind: DamageKind) {
    if amount <= 0.0 {
        return;
    }
    let bounds = bounds(position);
    let source = MobDamage {
        kind,
        attacker: None,
        from: Some([position.x, position.y, position.z]),
    };
    for mob in mobs
        .iter_mut()
        .filter(|m| m.alive() && bounds.intersects(m.bounds()))
    {
        mob.damage(amount, source);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_brushable_blocks_break_where_they_land() {
        assert!(Damage::falling("minecraft:suspicious_gravel").cancel_drop);
        assert!(Damage::falling("minecraft:suspicious_sand").cancel_drop);
        assert!(!Damage::falling("minecraft:gravel").cancel_drop);
        assert!(Damage::falling("minecraft:chipped_anvil").hurt_entities);
    }

    #[test]
    fn impact_uses_ceil_distance_capped_damage_and_positive_damage_wear() {
        let damage = Damage::anvil();
        assert!(Damage::default().impact(50.0).is_none());
        assert!(damage.impact(0.0).is_none());
        let short = damage.impact(1.0).unwrap();
        assert_eq!((short.amount, short.wear_chance), (0.0, 0.0));
        assert_eq!(damage.impact(1.01).unwrap().amount, 2.0);
        assert_eq!(damage.impact(5.0).unwrap().amount, 8.0);
        assert_eq!(damage.impact(100.0).unwrap().amount, 40.0);
        assert!(damage.impact(21.0).unwrap().wear_chance > 1.0);
    }
    #[test]
    fn damage_settings_round_trip_and_keep_vanilla_load_defaults() {
        let empty = Tag::Compound(Default::default());
        assert_eq!(Damage::from_nbt(&empty, false), Damage::default());
        let loaded_anvil = Damage::from_nbt(&empty, true);
        assert!(loaded_anvil.hurt_entities);
        assert_eq!(loaded_anvil.per_distance, 0.0);
        let damage = Damage {
            hurt_entities: true,
            cancel_drop: true,
            per_distance: 3.5,
            max: 17,
        };
        let mut tag = empty;
        damage.save(&mut tag);
        assert_eq!(Damage::from_nbt(&tag, false), damage);
    }
    #[test]
    fn anvil_wears_twice_then_breaks_preserving_facing() {
        for facing in ["north", "south", "east", "west"] {
            let mut state = BlockState {
                name: "minecraft:anvil".into(),
                properties: [("facing".into(), facing.into())].into(),
            };
            for name in ["minecraft:chipped_anvil", "minecraft:damaged_anvil"] {
                assert!(wear_anvil(&mut state));
                assert_eq!(state.name, name);
                assert_eq!(state.properties["facing"], facing);
            }
            assert!(!wear_anvil(&mut state));
        }
    }
    #[test]
    fn a_cobweb_on_the_path_or_at_the_end_holds_a_falling_block() {
        let web = BlockPos::new(0, 5, 0);
        let is_web = |p: BlockPos| p == web;
        let at = |y: f64| Vec3::new(0.5, y, 0.5);
        // Falling through the web's block in one move, landing in it, resting in it.
        assert!(passes_cobweb(
            at(7.2),
            at(3.1),
            Some(Vec3::new(0.0, -4.1, 0.0)),
            is_web
        ));
        assert!(passes_cobweb(
            at(6.5),
            at(5.5),
            Some(Vec3::new(0.0, -1.0, 0.0)),
            is_web
        ));
        assert!(passes_cobweb(at(5.5), at(5.5), None, is_web));
        // Above it, below it, or beside it.
        assert!(!passes_cobweb(
            at(8.0),
            at(6.01),
            Some(Vec3::new(0.0, -1.99, 0.0)),
            is_web
        ));
        assert!(!passes_cobweb(
            at(4.0),
            at(3.0),
            Some(Vec3::new(0.0, -1.0, 0.0)),
            is_web
        ));
        let beside = Vec3::new(1.5, 5.5, 0.5);
        assert!(!passes_cobweb(beside, beside, None, is_web));
    }
    #[test]
    #[ignore = "requires prepared entity models/data in HORIZON_TEST_RESOURCES"]
    fn impact_hurts_only_living_intersecting_noninvulnerable_mobs() {
        use crate::mobs::{Mob, MobTypes};
        use std::{path::Path, sync::Arc};
        let root = std::env::var_os("HORIZON_TEST_RESOURCES").expect("HORIZON_TEST_RESOURCES");
        let types = MobTypes::load(Path::new(&root)).unwrap();
        let cow = Arc::new(types.types["minecraft:cow"].clone());
        let mut mobs = vec![
            Mob::new(1, cow.clone(), Vec3::ZERO, 0.0),
            Mob::new(2, cow.clone(), Vec3::new(3.0, 0.0, 0.0), 0.0),
            Mob::new(3, cow.clone(), Vec3::ZERO, 0.0),
            Mob::new(4, cow, Vec3::ZERO, 0.0),
        ];
        mobs[2].invulnerable_flag = true;
        mobs[3].extra.head = Some("minecraft:carved_pumpkin".into());
        let initial = mobs.iter().map(|m| m.health).collect::<Vec<_>>();
        hurt_mobs(
            &mut mobs,
            Vec3::ZERO,
            Damage::anvil().impact(5.0).unwrap().amount,
        );
        assert_eq!(mobs[0].health, initial[0] - 8.0);
        assert_eq!(mobs[1].health, initial[1]);
        assert_eq!(mobs[2].health, initial[2]);
        assert_eq!(mobs[3].health, initial[3] - 6.0);
    }
}
