//! `Zombie.hurtServer`'s reinforcements: a hurt zombie on Hard may call
//! another of its kind from 7 to 40 blocks away to fight its target.
use super::Game;
use horizon_client::mobs::{
    MobType, Species, Who,
    finalize::{self, Attr, Op},
    spawn_rules::{self, Reason},
    spawn_selection::{self, NaturalLevel},
    spawner::SpawnerLevel,
};
use horizon_core::random::{LegacyRandom, RandomSource};
use horizon_world::food::Difficulty;

/// `Zombie.REINFORCEMENT_ATTEMPTS`.
const ATTEMPTS: usize = 50;
/// `Zombie.REINFORCEMENT_RANGE_MIN` and `REINFORCEMENT_RANGE_MAX`.
const RANGE_MIN: i32 = 7;
const RANGE_MAX: i32 = 40;

/// One axis of the offset: `Mth.nextInt(random, 7, 40) * Mth.nextInt(random, -1, 1)`.
fn offset(random: &mut LegacyRandom) -> i32 {
    let distance = random
        .next_int_between_inclusive(RANGE_MIN, RANGE_MAX)
        .unwrap_or(RANGE_MIN);
    distance * random.next_int_between_inclusive(-1, 1).unwrap_or(0)
}

impl Game<'_> {
    /// `Zombie.hurtServer` after the hit: on Hard, with the zombie's chance
    /// and monsters spawning, a copy of it appears out of sight and takes
    /// `target`; both then call less often.
    pub(super) fn call_reinforcements(&mut self, caller: u32, target: Who) {
        let Some(index) = self.mobs.iter().position(|m| m.id == caller) else {
            return;
        };
        if self.world.health_rules.difficulty != Difficulty::Hard {
            return;
        }
        let zombie = &mut self.mobs[index];
        if f64::from(zombie.random.next_float()) >= zombie.attrs.spawn_reinforcements {
            return;
        }
        // ServerLevel.isSpawningMonsters.
        if !self.ui.rules.bool("spawn_mobs") || !self.ui.rules.bool("spawn_monsters") {
            return;
        }
        let kind = zombie.kind.clone();
        let from = [
            zombie.position.x.floor() as i32,
            zombie.position.y.floor() as i32,
            zombie.position.z.floor() as i32,
        ];
        let mut own = std::mem::replace(&mut zombie.random, LegacyRandom::new(0));
        let mut rules = std::mem::replace(&mut self.world.random, LegacyRandom::new(0));
        let spot = self.reinforcement_spot(&kind, from, &mut own, &mut rules);
        self.mobs[index].random = own;
        self.world.random = rules;
        let Some(spot) = spot else {
            return;
        };
        let Some(id) = self.spawn_mob_reason(&kind.id, spot.map(f64::from), false, false, false)
        else {
            return;
        };
        if let Some(reinforcement) = self.mobs.iter_mut().find(|m| m.id == id) {
            reinforcement.ai.target = Some(target);
            // ZOMBIE_REINFORCEMENT_CALLEE_CHARGE.
            finalize::add_modifier(
                reinforcement,
                Attr::SpawnReinforcements,
                "minecraft:reinforcement_callee_charge",
                -f64::from(0.05f32),
                Op::AddValue,
            );
        }
        // The caller's charge grows by 0.05 each time it calls.
        let caller = &mut self.mobs[index];
        let charged = finalize::modifier_amount(
            caller,
            Attr::SpawnReinforcements,
            "minecraft:reinforcement_caller_charge",
        )
        .unwrap_or(0.0);
        finalize::add_modifier(
            caller,
            Attr::SpawnReinforcements,
            "minecraft:reinforcement_caller_charge",
            charged - 0.05,
            Op::AddValue,
        );
    }
    /// The block `Zombie.hurtServer` puts the reinforcement at: the first of
    /// 50 random offsets with a valid spawn surface, no alive player within
    /// 7 blocks, and room for the zombie standing on the block's corner.
    fn reinforcement_spot(
        &self,
        kind: &MobType,
        from: [i32; 3],
        own: &mut LegacyRandom,
        rules: &mut LegacyRandom,
    ) -> Option<[i32; 3]> {
        let level = self.natural_level();
        let (width, height) = (kind.width as f32, kind.height as f32);
        for _ in 0..ATTEMPTS {
            let [x, y, z] = from.map(|v| v + offset(own));
            if !spawn_rules::is_spawn_position_ok(kind.species, &level, x, y, z)
                || !spawn_rules::check_spawn_rules(
                    kind.species,
                    &level,
                    Reason::Reinforcement,
                    rules,
                    x,
                    y,
                    z,
                )
            {
                continue;
            }
            // setPos(x, y, z): the box is centred on the block's corner.
            let bounds =
                spawn_selection::spawn_box(f64::from(x), f64::from(y), f64::from(z), width, height);
            // Drowned.canSpawnInLiquids.
            let wet_ok = kind.species == Species::Drowned
                || !spawn_selection::contains_any_liquid(&level, bounds);
            if !level.alive_player_near(f64::from(x), f64::from(y), f64::from(z), 7.0)
                && level.unobstructed(bounds)
                && level.no_collision(bounds)
                && wet_ok
            {
                return Some([x, y, z]);
            }
        }
        None
    }
}
