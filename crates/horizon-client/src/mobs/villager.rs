//! Villagers (`Villager`): professions, sleeping, the unhappy head shake,
//! food, gossip and the raid state. Their AI is the brain in
//! `villager_brain`.
use super::{
    Mob, MobEvent, MobWorld, Species,
    anim::{Poses, State},
};
use crate::items::ItemStack;
use horizon_core::geometry::trig::Mth;
use horizon_core::random::RandomSource;
use std::f32::consts::PI;

// Villager state: `variant` type, `variant2` profession (index into
// `render::PROFESSIONS`), `variant3` level, `home` the claimed job site,
// `bed` and `meeting` the claimed bed head and bell, `ints[0]` the unhappy
// counter, `ints[1]` the bed's facing while asleep (1 north, 2 south,
// 3 west, 4 east), `trade` the offers, experience and restocks.

pub const NONE: u8 = 0;
pub const NITWIT: u8 = 11;

/// `PoiTypes`: the job site block of each profession.
pub fn job_site_profession(block: &str) -> Option<u8> {
    Some(match block.trim_start_matches("minecraft:") {
        "blast_furnace" => 1,
        "smoker" => 2,
        "cartography_table" => 3,
        "brewing_stand" => 4,
        "composter" => 5,
        "barrel" => 6,
        "fletching_table" => 7,
        "cauldron" | "water_cauldron" | "lava_cauldron" | "powder_snow_cauldron" => 8,
        "lectern" => 9,
        "stonecutter" => 10,
        "loom" => 12,
        "smithing_table" => 13,
        "grindstone" => 14,
        _ => return None,
    })
}

pub fn register(m: &mut Mob) {
    if m.extra.variant3 == 0 {
        m.extra.variant3 = 1;
    }
    // Villager's constructor: its brain does everything, it has no goals.
    m.ai.nav.can_open_doors = true;
    m.ai.nav.can_float = true;
    m.set_required_path_length(48.0);
}

/// `LivingEntity.addParticlesAroundSelf`: five particles about the body.
pub fn particles_around(m: &mut Mob, kind: &'static str, events: &mut Vec<MobEvent>) {
    let (w, h) = (f64::from(m.width()), f64::from(m.height()));
    for _ in 0..5 {
        let at = [
            m.position.x + w * (2.0 * m.random.next_double() - 1.0),
            m.position.y + h * m.random.next_double() + 1.0,
            m.position.z + w * (2.0 * m.random.next_double() - 1.0),
        ];
        events.push(MobEvent::Particles { kind, at, count: 0 });
    }
}
/// The `villager_activity` / `baby_villager_activity` timeline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Activity {
    Idle,
    Work,
    Play,
    Meet,
    Rest,
}
pub fn activity(m: &Mob, day_time: i64) -> Activity {
    activity_at(m.is_baby(), day_time)
}
pub fn activity_at(baby: bool, day_time: i64) -> Activity {
    let t = day_time.rem_euclid(24000);
    if baby {
        match t {
            10..3000 | 6000..10000 => Activity::Idle,
            3000..6000 | 10000..12000 => Activity::Play,
            _ => Activity::Rest,
        }
    } else {
        match t {
            10..2000 | 11000..12000 => Activity::Idle,
            2000..9000 => Activity::Work,
            9000..11000 => Activity::Meet,
            _ => Activity::Rest,
        }
    }
}
pub fn sleeping(m: &Mob) -> bool {
    matches!(m.species(), Species::Villager | Species::WanderingTrader) && m.extra.ints[1] != 0
}
/// `LivingEntity.startSleeping`: lie in the bed (`setPosToBed`), whose
/// head becomes `occupied`.
pub fn start_sleeping(m: &mut Mob, p: [i32; 3], w: &MobWorld, events: &mut Vec<MobEvent>) -> bool {
    if w.prop(p[0], p[1], p[2], "occupied") == Some("true") {
        return false;
    }
    m.extra.ints[1] = bed_facing_code(w.prop(p[0], p[1], p[2], "facing"));
    m.nav_stop();
    let e = ext_mut(m);
    e.last_slept = Some(w.game_time);
    e.sleeping_pos = Some(p);
    events.push(MobEvent::BedOccupied {
        pos: p,
        occupied: true,
    });
    m.velocity = horizon_core::geometry::Vec3::ZERO;
    m.position = horizon_core::geometry::Vec3::new(
        f64::from(p[0]) + 0.5,
        f64::from(p[1]) + 0.6875,
        f64::from(p[2]) + 0.5,
    );
    true
}
/// `LivingEntity.stopSleeping` with `BedBlock.findStandUpPosition`: the bed
/// is no longer `occupied`, and the sleeper stands beside it where there is
/// room, else on top of it.
pub fn stop_sleeping(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    m.extra.ints[1] = 0;
    let bed = m
        .extra
        .villager
        .as_deref_mut()
        .and_then(|e| e.sleeping_pos.take());
    let Some(p) = bed else { return };
    if !w.block_name(p[0], p[1], p[2]).ends_with("_bed") {
        return;
    }
    events.push(MobEvent::BedOccupied {
        pos: p,
        occupied: false,
    });
    for (dx, dz) in [
        (1, 0),
        (-1, 0),
        (0, 1),
        (0, -1),
        (1, 1),
        (-1, -1),
        (1, -1),
        (-1, 1),
    ] {
        let (x, y, z) = (p[0] + dx, p[1], p[2] + dz);
        let free = |y| w.data.state(w.world.block(x, y, z)).top <= 0.0;
        if free(y) && free(y + 1) && w.solid(x, y - 1, z) {
            m.position = horizon_core::geometry::Vec3::new(
                f64::from(x) + 0.5,
                f64::from(y),
                f64::from(z) + 0.5,
            );
            return;
        }
    }
    // The bed's top (getSleepHeight) and 0.125 above it.
    m.position.y = f64::from(p[1]) + 0.5625 + 0.125;
}
/// `LivingEntity.getSleepingPos`: the bed head a sleeper lies in.
pub fn sleeping_pos(m: &Mob) -> Option<[i32; 3]> {
    ext(m).and_then(|e| e.sleeping_pos).filter(|_| sleeping(m))
}
/// `LivingEntityRenderer.sleepDirectionToRotation` and the bed's direction
/// (the body is shifted back from the pillow by the standing eye height).
/// A bed's `facing` as `sleep_pose` keeps it in `ints[1]`.
pub fn bed_facing_code(facing: Option<&str>) -> i32 {
    match facing {
        Some("north") => 1,
        Some("south") => 2,
        Some("west") => 3,
        _ => 4,
    }
}
pub fn sleep_pose(m: &Mob) -> Option<(f32, [f32; 2])> {
    Some(match m.extra.ints[1] {
        1 => (270.0, [0.0, -1.0]),
        2 => (90.0, [0.0, 1.0]),
        3 => (0.0, [-1.0, 0.0]),
        4 => (180.0, [1.0, 0.0]),
        _ => return None,
    })
}
/// `Villager.tick`: the unhappy counter and the gossip decay; a sleeper
/// hurt or without its bed wakes up (`LivingEntity.hurtServer`,
/// `LivingEntity.checkBedExists`), and lies the way its bed faces
/// (`getBedOrientation`). The brain does the rest.
pub fn ai_step(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if m.extra.ints[0] > 0 {
        m.extra.ints[0] -= 1;
    }
    if m.species() == Species::Villager {
        ext_mut(m).maybe_decay(w.game_time);
    }
    if sleeping(m) {
        let bed = sleeping_pos(m).filter(|p| w.block_name(p[0], p[1], p[2]).ends_with("_bed"));
        if let Some(p) = bed {
            m.extra.ints[1] = bed_facing_code(w.prop(p[0], p[1], p[2], "facing"));
        }
        if m.hurt_time > 0 || bed.is_none() {
            stop_sleeping(m, w, events);
            super::villager_brain::woken(m, w.game_time);
        }
    }
}
/// `Villager.setUnhappy`: shake the head for two seconds and say no.
pub fn set_unhappy(m: &mut Mob, events: &mut Vec<MobEvent>) {
    m.extra.ints[0] = 40;
    let sound = if m.species() == Species::WanderingTrader {
        "entity.wandering_trader.no"
    } else {
        "entity.villager.no"
    };
    events.push(MobEvent::Sound(
        sound,
        [m.position.x, m.position.y, m.position.z],
        1.0,
        1.0,
    ));
}
/// `VillagerModel.setupAnim`.
pub fn anim(m: &Mob, s: &State, p: &mut Poses) {
    let awake = !sleeping(m);
    p.yrot("head", if awake { s.y_rot.to_radians() } else { 0.0 });
    p.xrot("head", if awake { s.x_rot.to_radians() } else { 0.0 });
    if m.extra.ints[0] > 0 {
        p.zrot("head", 0.3 * (0.45 * s.age).msin());
        p.xrot("head", 0.4);
    } else {
        p.zrot("head", 0.0);
    }
    let leg = (s.pos * 0.6662).mcos() * 1.4 * s.speed * 0.5;
    p.xrot("right_leg", leg);
    p.xrot(
        "left_leg",
        (s.pos * 0.6662 + PI).mcos() * 1.4 * s.speed * 0.5,
    );
    p.yrot("right_leg", 0.0);
    p.yrot("left_leg", 0.0);
}

// ---- Raids: the raid activities and the gift for a hero.

/// No raid where the villager stands (`ServerLevel.getRaidAt` is null).
pub const RAID_NONE: u8 = 0;
/// A raid is near, before or between its waves.
pub const PRE_RAID: u8 = 1;
/// A raid is near and a wave is on.
pub const RAID: u8 = 2;
/// The raid there was won: the villagers celebrate.
pub const RAID_VICTORY: u8 = 3;
/// The raid there was lost (still found by `getRaidAt` until it stops).
pub const RAID_LOSS: u8 = 4;

/// Villager state beyond `Extra`'s scratch fields.
#[derive(Clone, Debug, Default)]
pub struct Ext {
    /// The raid `getRaidAt` finds where the villager stands (`RAID_NONE`
    /// ... `RAID_LOSS`), set by the game every tick.
    pub raid: u8,
    /// That raid has spawned a wave and is not between waves
    /// (`hasFirstWaveSpawned && !isBetweenWaves`).
    pub raid_wave: bool,
    /// What the villager remembers of players (`gossips`).
    pub gossips: crate::gossip::Gossips,
    pub last_gossip_time: i64,
    pub last_gossip_decay: i64,
    /// `foodLevel`: eaten but not yet digested, in food points.
    pub food_level: i32,
    /// The eight-slot inventory (`InventoryCarrier`).
    pub inventory: Vec<ItemStack>,
    /// `LAST_SLEPT` memory.
    pub last_slept: Option<i64>,
    /// `LivingEntity.getSleepingPos`: the bed head it lies in, saved as
    /// `sleeping_pos`.
    pub sleeping_pos: Option<[i32; 3]>,
    /// `GOLEM_DETECTED_RECENTLY` expires at this game time.
    pub golem_until: i64,
    /// The `last_hurt_by_time` of the player's hit already counted.
    pub hurt_seen: i32,
    /// A zombie villager's cure under way.
    pub cure: Option<super::zombie_villager::Cure>,
    /// The villager's brain, built at its first tick.
    pub brain: Option<Box<super::villager_brain::Brain>>,
}
pub fn ext(m: &Mob) -> Option<&Ext> {
    m.extra.villager.as_deref()
}
pub fn ext_mut(m: &mut Mob) -> &mut Ext {
    m.extra.villager.get_or_insert_with(Box::default)
}
/// The raid where the villager stands (`RAID_NONE` ... `RAID_LOSS`).
pub fn raid_status(m: &Mob) -> u8 {
    ext(m).map_or(RAID_NONE, |e| e.raid)
}
/// `ServerLevel.getRaidAt(blockPosition)` is a raid.
pub fn raid_nearby(m: &Mob) -> bool {
    ext(m).is_some_and(|e| e.raid != RAID_NONE)
}
/// `ResetRaidStatus`: no raid, a stopped one or a lost one.
pub fn raid_over_or_lost(m: &Mob) -> bool {
    matches!(raid_status(m), RAID_NONE | RAID_LOSS)
}
/// `SetRaidStatus`: `Some(true)` while a wave is on
/// (`hasFirstWaveSpawned && !isBetweenWaves`), `Some(false)` otherwise.
pub fn raid_state(m: &Mob) -> Option<bool> {
    let e = ext(m)?;
    (e.raid != RAID_NONE).then_some(e.raid_wave)
}
/// `Villager.getVillagerXp`.
pub fn xp(m: &Mob) -> i32 {
    m.extra.trade.as_ref().map_or(0, |t| t.xp)
}
/// `Villager.setVillagerData(withProfession)`: a new profession forgets the
/// offers (`offers = null`), so the next `getOffers` rolls the new
/// profession's trades.
pub fn set_profession(m: &mut Mob, profession: u8) {
    if m.extra.variant2 != profession {
        m.extra.variant2 = profession;
        if let Some(t) = m.extra.trade.as_mut() {
            t.offers.clear();
            t.unset = true;
        }
    }
}
/// `VillagerProfession.workSound`.
pub fn work_sound(profession: u8) -> Option<&'static str> {
    Some(match super::render::PROFESSIONS[profession as usize % 15] {
        "armorer" => "entity.villager.work_armorer",
        "butcher" => "entity.villager.work_butcher",
        "cartographer" => "entity.villager.work_cartographer",
        "cleric" => "entity.villager.work_cleric",
        "farmer" => "entity.villager.work_farmer",
        "fisherman" => "entity.villager.work_fisherman",
        "fletcher" => "entity.villager.work_fletcher",
        "leatherworker" => "entity.villager.work_leatherworker",
        "librarian" => "entity.villager.work_librarian",
        "mason" => "entity.villager.work_mason",
        "shepherd" => "entity.villager.work_shepherd",
        "toolsmith" => "entity.villager.work_toolsmith",
        "weaponsmith" => "entity.villager.work_weaponsmith",
        _ => return None,
    })
}
/// A `CropBlock` at `p`.
pub fn crop(w: &MobWorld, p: [i32; 3]) -> bool {
    matches!(
        w.block_name(p[0], p[1], p[2]),
        "minecraft:wheat"
            | "minecraft:carrots"
            | "minecraft:potatoes"
            | "minecraft:beetroots"
            | "minecraft:torchflower_crop"
    )
}
/// `CropBlock.isMaxAge` for the block at `p`.
pub fn ripe_crop(w: &MobWorld, p: [i32; 3]) -> bool {
    let max = match w.block_name(p[0], p[1], p[2]) {
        "minecraft:wheat" | "minecraft:carrots" | "minecraft:potatoes" => "7",
        "minecraft:beetroots" => "3",
        // TorchflowerCropBlock.getMaxAge is 2, past its age property (0-1):
        // the crop is never at max age (age 2 is the torchflower itself).
        _ => return false,
    };
    w.prop(p[0], p[1], p[2], "age") == Some(max)
}

/// The gift loot table of a villager (`GiveGiftToHero.getLootTableToThrow`).
pub fn gift_table(m: &Mob) -> String {
    if m.is_baby() {
        return "minecraft:gameplay/hero_of_the_village/baby_gift".into();
    }
    let profession = super::render::PROFESSIONS[m.extra.variant2 as usize % 15];
    let name = match profession {
        "none" | "nitwit" => "unemployed",
        other => other,
    };
    format!("minecraft:gameplay/hero_of_the_village/{name}_gift")
}
// ---- Gossip, food, breeding and golems.

/// `VillagerFood` nutrition (`DataComponents.VILLAGER_FOOD`).
pub fn food_points(item: &str) -> i32 {
    match item {
        "minecraft:bread" => 4,
        "minecraft:potato" | "minecraft:carrot" | "minecraft:beetroot" => 1,
        _ => 0,
    }
}
/// The farmer's `requestedItems` (`VillagerProfession.FARMER`). Food is picked
/// up and traded separately, through its villager food value.
pub fn farmer_items() -> &'static [&'static str] {
    &[
        "minecraft:wheat",
        "minecraft:wheat_seeds",
        "minecraft:beetroot_seeds",
        "minecraft:bone_meal",
    ]
}
/// A villager's `requestedItems` by profession.
pub fn requested_items(m: &Mob) -> &'static [&'static str] {
    if super::render::PROFESSIONS[m.extra.variant2 as usize % 15] == "farmer" {
        farmer_items()
    } else {
        &[]
    }
}
pub fn is_farmer(m: &Mob) -> bool {
    super::render::PROFESSIONS[m.extra.variant2 as usize % 15] == "farmer"
}
/// `SimpleContainer(8)`: stacks the villager carries.
const INVENTORY_SLOTS: usize = 8;

impl Ext {
    /// `countFoodPointsInInventory`.
    pub fn food_in_inventory(&self) -> i32 {
        self.inventory
            .iter()
            .map(|s| food_points(&s.item) * s.count as i32)
            .sum()
    }
    /// `hasExcessFood` and `wantsMoreFood`.
    pub fn has_excess_food(&self) -> bool {
        self.food_in_inventory() >= 24
    }
    pub fn wants_more_food(&self) -> bool {
        self.food_in_inventory() < 12
    }
    fn hungry(&self) -> bool {
        self.food_level < 12
    }
    /// `Villager.eatAndDigestFood`: eat until full, then digest 12 points.
    pub fn eat_and_digest(&mut self) {
        if self.hungry() {
            let mut slot = 0;
            while slot < self.inventory.len() && self.hungry() {
                let points = food_points(&self.inventory[slot].item);
                if points > 0 {
                    while self.inventory[slot].count > 0 && self.hungry() {
                        self.food_level += points;
                        self.inventory[slot].count -= 1;
                    }
                    if self.inventory[slot].count == 0 {
                        self.inventory.remove(slot);
                        continue;
                    }
                }
                slot += 1;
            }
        }
        self.food_level -= 12;
    }
    /// Whether the stack fits (`SimpleContainer.canAddItem`).
    pub fn can_add(&self, stack: &ItemStack, max_stack: u32) -> bool {
        self.inventory.len() < INVENTORY_SLOTS
            || self
                .inventory
                .iter()
                .any(|s| s.same_item_same_components(stack) && s.count < max_stack)
    }
    /// `SimpleContainer.addItem`: merge into stacks, then free slots; the
    /// part that did not fit is returned.
    pub fn add_item(&mut self, mut stack: ItemStack, max_stack: u32) -> Option<ItemStack> {
        for s in &mut self.inventory {
            if stack.count > 0 && s.same_item_same_components(&stack) && s.count < max_stack {
                let n = (max_stack - s.count).min(stack.count);
                s.count += n;
                stack.count -= n;
            }
        }
        while stack.count > 0 && self.inventory.len() < INVENTORY_SLOTS {
            let n = stack.count.min(max_stack);
            self.inventory.push(ItemStack {
                count: n,
                ..stack.clone()
            });
            stack.count -= n;
        }
        (stack.count > 0).then_some(stack)
    }
    /// `SimpleContainer.removeItemType`: take up to `count` of the item from
    /// the first stacks that hold it.
    pub fn remove_item(&mut self, item: &str, mut count: u32) {
        let mut slot = 0;
        while slot < self.inventory.len() && count > 0 {
            let s = &mut self.inventory[slot];
            if &*s.item == item {
                let n = s.count.min(count);
                s.count -= n;
                count -= n;
                if s.count == 0 {
                    self.inventory.remove(slot);
                    continue;
                }
            }
            slot += 1;
        }
    }
    pub fn count_item(&self, item: &str) -> u32 {
        self.inventory
            .iter()
            .filter(|s| &*s.item == item)
            .map(|s| s.count)
            .sum()
    }
    /// `TradeWithVillager.throwHalfStack`: the half of a big matching stack
    /// (or whatever is over 24) leaves the inventory.
    pub fn take_half_stack(
        &mut self,
        wanted: impl Fn(&ItemStack) -> bool,
        max_stack: impl Fn(&str) -> u32,
    ) -> Option<ItemStack> {
        for i in 0..self.inventory.len() {
            let s = &self.inventory[i];
            if s.count == 0 || !wanted(s) {
                continue;
            }
            let count = if s.count > max_stack(&s.item) / 2 {
                s.count / 2
            } else if s.count > 24 {
                s.count - 24
            } else {
                continue;
            };
            let taken = ItemStack { count, ..s.clone() };
            self.inventory[i].count -= count;
            if self.inventory[i].count == 0 {
                self.inventory.remove(i);
            }
            return Some(taken);
        }
        None
    }
    /// `Villager.wantsToSpawnGolem`: slept within the last day and no golem
    /// seen lately.
    pub fn wants_golem(&self, now: i64) -> bool {
        self.last_slept.is_some_and(|t| now - t < 24000) && now >= self.golem_until
    }
    /// `maybeDecayGossip`: once a day.
    pub fn maybe_decay(&mut self, now: i64) {
        if self.last_gossip_decay == 0 {
            self.last_gossip_decay = now;
        } else if now >= self.last_gossip_decay + 24000 {
            self.gossips.decay();
            self.last_gossip_decay = now;
        }
    }
    /// `Villager.gossip` timing: hearing again waits 1200 ticks.
    pub fn can_gossip(&self, now: i64) -> bool {
        now < self.last_gossip_time || now >= self.last_gossip_time + 1200
    }
}
/// What a villager ticking later reads of another one this tick (vanilla
/// reads the other villager itself).
#[derive(Clone, Debug, Default)]
pub struct Seen {
    pub gossips: crate::gossip::Gossips,
    pub last_gossip_time: i64,
    /// `countFoodPointsInInventory`.
    pub food_points: i32,
    /// The `INTERACTION_TARGET` memory (play tag's chases).
    pub interaction_target: Option<super::Who>,
    /// The `PATH` memory, when not done: its previous and next nodes.
    pub path_nodes: Option<(Option<[i32; 3]>, [i32; 3])>,
    /// `LAST_SLEPT` and whether `GOLEM_DETECTED_RECENTLY` is remembered.
    pub last_slept: Option<i64>,
    pub golem_detected: bool,
}
impl Seen {
    /// `Villager.wantsToSpawnGolem`.
    pub fn wants_golem(&self, now: i64) -> bool {
        self.last_slept.is_some_and(|t| now - t < 24000) && !self.golem_detected
    }
    /// `Villager.wantsMoreFood`.
    pub fn wants_more_food(&self) -> bool {
        self.food_points < 12
    }
}
pub fn seen(m: &Mob) -> Seen {
    let e = ext(m);
    let (interaction_target, path_nodes, golem_detected, last_slept) =
        super::villager_brain::seen_memories(m);
    Seen {
        gossips: e.map(|e| e.gossips.clone()).unwrap_or_default(),
        last_gossip_time: e.map_or(0, |e| e.last_gossip_time),
        food_points: e.map_or(0, Ext::food_in_inventory),
        interaction_target,
        path_nodes,
        last_slept: last_slept.or(e.and_then(|e| e.last_slept)),
        golem_detected,
    }
}
/// What `TradeWithVillager.tick` hands over: stacks tossed from the first
/// villager's hand (`from`, its eye height) toward the second (`to`), and
/// whether the gossip happened (`spawnGolemIfNeeded` follows).
pub struct Meeting {
    pub thrown: Vec<ItemStack>,
    pub from: horizon_core::geometry::Vec3,
    pub eye: f64,
    pub to: horizon_core::geometry::Vec3,
    pub gossiped: bool,
}
/// `TradeWithVillager.tick` for villagers `a` and `b` (indices into `mobs`):
/// `Villager.gossip` (at most once in 1200 ticks for both), then surplus
/// food and wanted wares taken out of `a`'s inventory.
pub fn meet(
    mobs: &mut [Mob],
    a: usize,
    b: usize,
    now: i64,
    max_stack: impl Fn(&str) -> u32,
) -> Meeting {
    let ready = ext(&mobs[a]).is_none_or(|e| e.can_gossip(now))
        && ext(&mobs[b]).is_none_or(|e| e.can_gossip(now));
    if ready {
        let source = ext(&mobs[b]).map(|e| e.gossips.clone()).unwrap_or_default();
        let me = &mut mobs[a];
        let mut random = me.random.clone();
        let e = ext_mut(me);
        e.gossips.transfer_from(&source, &mut random, 10);
        e.last_gossip_time = now;
        me.random = random;
        ext_mut(&mut mobs[b]).last_gossip_time = now;
    }
    // Surplus food goes to a hungry neighbour (farmers hand it to anyone).
    let farmer = is_farmer(&mobs[a]);
    let wants_more = ext(&mobs[b]).is_none_or(Ext::wants_more_food);
    let excess = ext(&mobs[a]).is_some_and(Ext::has_excess_food);
    let mut thrown = Vec::new();
    if excess
        && (farmer || wants_more)
        && let Some(s) =
            ext_mut(&mut mobs[a]).take_half_stack(|s| food_points(&s.item) > 0, &max_stack)
    {
        thrown.push(s);
    }
    // A farmer's wheat.
    if farmer
        && ext(&mobs[a]).is_some_and(|e| e.count_item("minecraft:wheat") > 32)
        && let Some(s) =
            ext_mut(&mut mobs[a]).take_half_stack(|s| &*s.item == "minecraft:wheat", &max_stack)
    {
        thrown.push(s);
    }
    // Wares the neighbour's profession asks for and ours does not.
    let theirs = requested_items(&mobs[b]);
    let mine = requested_items(&mobs[a]);
    let wanted: Vec<&str> = theirs
        .iter()
        .copied()
        .filter(|i| !mine.contains(i))
        .collect();
    if !wanted.is_empty()
        && let Some(s) =
            ext_mut(&mut mobs[a]).take_half_stack(|s| wanted.contains(&&*s.item), &max_stack)
    {
        thrown.push(s);
    }
    Meeting {
        thrown,
        from: mobs[a].position,
        eye: mobs[a].eye_y(),
        to: mobs[b].position,
        gossiped: ready,
    }
}
/// `Villager.spawnGolemIfNeeded` up to the spawn: villager `i` wants a golem
/// and so do at least `needed` of the villagers whose boxes meet its own grown
/// by 10 blocks. Returns all of those villagers: once the golem stands, they
/// remember it ([`super::villager_brain::golem_detected`]).
pub fn golem_summoners(mobs: &[Mob], i: usize, now: i64, needed: usize) -> Option<Vec<usize>> {
    if !ext(&mobs[i]).is_some_and(|e| e.wants_golem(now)) {
        return None;
    }
    let area = mobs[i]
        .bounds()
        .inflate(horizon_core::geometry::Vec3::new(10.0, 10.0, 10.0));
    let near: Vec<usize> = mobs
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            m.species() == Species::Villager && !m.removed && m.bounds().intersects(area)
        })
        .map(|(j, _)| j)
        .collect();
    let wanting = near
        .iter()
        .filter(|&&j| ext(&mobs[j]).is_some_and(|e| e.wants_golem(now)))
        .take(5)
        .count();
    (wanting >= needed).then_some(near)
}
/// `SpawnUtil.Strategy.LEGACY_IRON_GOLEM`'s refused floors.
fn golem_floor_ok(name: &str) -> bool {
    !matches!(
        name,
        "minecraft:cobweb"
            | "minecraft:cactus"
            | "minecraft:glass_pane"
            | "minecraft:conduit"
            | "minecraft:ice"
            | "minecraft:tnt"
            | "minecraft:glowstone"
            | "minecraft:beacon"
            | "minecraft:sea_lantern"
            | "minecraft:frosted_ice"
            | "minecraft:tinted_glass"
            | "minecraft:glass"
    ) && !name.ends_with("_stained_glass_pane")
        && !name.ends_with("_stained_glass")
        && !name.ends_with("leaves")
}
/// One attempt of `SpawnUtil.trySpawnMob(IRON_GOLEM, ..., 8 across, 6 up and
/// down, LEGACY_IRON_GOLEM)`: a column drawn near `start`, searched down from
/// 6 above for a solid floor (or powder snow) under air or a liquid block.
/// Returns the block the golem would stand in.
fn golem_attempt<L: super::spawn_rules::SpawnLevel>(
    l: &L,
    start: [i32; 3],
    random: &mut impl RandomSource,
) -> Option<[i32; 3]> {
    let dx = random.next_int_bound(17).unwrap_or(8) - 8;
    let dz = random.next_int_bound(17).unwrap_or(8) - 8;
    let (x, z) = (start[0] + dx, start[2] + dz);
    let name = |y: i32| -> &str { &l.facts().get(l.state(x, y, z)).name };
    let mut y = start[1] + 6;
    let mut above = l.state(x, y, z);
    for _ in 0..=12 {
        y -= 1;
        let here = l.state(x, y, z);
        let above_name = &*l.facts().get(above).name;
        // BlockState.liquid(): a water or lava block (not a waterlogged one).
        let open = l.facts().is_air(above)
            || above_name == "minecraft:water"
            || above_name == "minecraft:lava";
        let solid = l.mob_data().state(here).flags & super::data::SOLID != 0
            || name(y) == "minecraft:powder_snow";
        if golem_floor_ok(name(y)) && open && solid {
            return Some([x, y + 1, z]);
        }
        above = here;
    }
    None
}
/// `IronGolem.checkSpawnObstruction`: a floor with a full top face, its feet
/// free whatever fluid is there, and the two blocks above free.
fn golem_fits<L: super::spawn_rules::SpawnLevel>(l: &L, [x, y, z]: [i32; 3]) -> bool {
    use super::spawn_rules::is_valid_empty_spawn;
    // entityCanStandOn: Block.isFaceFull(collision shape, UP).
    let floor = l.mob_data().state(l.state(x, y - 1, z));
    let stands = floor.flags & super::data::STURDY_TOP != 0 && floor.top >= 1.0;
    // isValidEmptySpawnBlock with Fluids.EMPTY: open water is free too.
    let feet = is_valid_empty_spawn(Species::IronGolem, l, x, y, z)
        || &*l.facts().get(l.state(x, y, z)).name == "minecraft:water";
    stands
        && feet
        && is_valid_empty_spawn(Species::IronGolem, l, x, y + 1, z)
        && is_valid_empty_spawn(Species::IronGolem, l, x, y + 2, z)
}
/// `SpawnUtil.trySpawnMob(IRON_GOLEM, MOB_SUMMONED, level, start, 10, 8, 6,
/// LEGACY_IRON_GOLEM, false)`: up to ten columns tried. In each spot found
/// `create` makes the golem (`EntityType.create`: the yaw drawn here, then its
/// `finalizeSpawn` draws, see [`finalize_summoned_golem`]); it is kept only
/// when `IronGolem.checkSpawnObstruction` passes, and discarded otherwise.
pub fn summon_golem<L: super::spawn_rules::SpawnLevel, R: RandomSource, T>(
    l: &L,
    start: [i32; 3],
    random: &mut R,
    mut create: impl FnMut(&mut R, horizon_core::geometry::Vec3, f32) -> T,
) -> Option<T> {
    for _ in 0..10 {
        let Some([x, y, z]) = golem_attempt(l, start, random) else {
            continue;
        };
        // Mth.wrapDegrees(nextFloat() * 360).
        let mut yaw = random.next_float() * 360.0;
        if yaw >= 180.0 {
            yaw -= 360.0;
        }
        let at =
            horizon_core::geometry::Vec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5);
        let golem = create(random, at, yaw);
        if golem_fits(l, [x, y, z]) {
            return Some(golem);
        }
    }
    None
}
/// `Mob.finalizeSpawn` for a summoned iron golem, from the level's random:
/// the follow range spawn bonus and left-handedness.
pub fn finalize_summoned_golem(m: &mut Mob, random: &mut impl RandomSource) {
    super::finalize::mob_spawn(m, random);
}
/// `Villager.canBreed`: enough food, awake and an adult.
pub fn can_breed(m: &Mob) -> bool {
    m.species() == Species::Villager
        && m.extra.age == 0
        && !sleeping(m)
        && ext(m).map_or(0, |e| e.food_level + e.food_in_inventory()) >= 12
}

/// The saved fields (`FoodLevel`, `Gossips`, `LastGossipDecay`, `Inventory`,
/// `LivingEntity`'s `sleeping_pos` and the sleep and golem memories).
pub fn write_nbt(m: &Mob, map: &mut horizon_world::nbt::Compound) {
    use horizon_world::nbt::Tag;
    if let Some(p) = sleeping_pos(m) {
        map.insert("sleeping_pos".into(), Tag::IntArray(p.to_vec()));
    }
    let Some(e) = ext(m) else { return };
    map.insert(
        "FoodLevel".into(),
        Tag::Byte(e.food_level.clamp(-128, 127) as i8),
    );
    map.insert("Gossips".into(), e.gossips.to_nbt());
    map.insert("LastGossipDecay".into(), Tag::Long(e.last_gossip_decay));
    map.insert(
        "Inventory".into(),
        Tag::List(e.inventory.iter().map(crate::save::item_to_nbt).collect()),
    );
}
/// The memories of `Brain.memories` the villager keeps besides positions.
pub fn write_memories(m: &Mob, memories: &mut Vec<(&'static str, horizon_world::nbt::Tag)>) {
    use horizon_world::nbt::Tag;
    let Some(e) = ext(m) else { return };
    if let Some(t) = e.last_slept {
        memories.push((
            "minecraft:last_slept",
            Tag::compound([("value", Tag::Long(t))]),
        ));
    }
}
pub fn read_nbt(m: &mut Mob, tag: &horizon_world::nbt::Tag) {
    use horizon_world::nbt::Tag;
    let int = |t: Option<&Tag>| match t {
        Some(Tag::Int(v)) => Some(i64::from(*v)),
        Some(Tag::Byte(v)) => Some(i64::from(*v)),
        Some(Tag::Short(v)) => Some(i64::from(*v)),
        Some(Tag::Long(v)) => Some(*v),
        _ => None,
    };
    let last_slept = int(tag
        .get("Brain")
        .and_then(|b| b.get("memories"))
        .and_then(|b| b.get("minecraft:last_slept"))
        .and_then(|j| j.get("value")));
    let e = ext_mut(m);
    e.food_level = int(tag.get("FoodLevel")).unwrap_or(0) as i32;
    e.gossips = tag
        .get("Gossips")
        .map(crate::gossip::Gossips::from_nbt)
        .unwrap_or_default();
    e.last_gossip_decay = int(tag.get("LastGossipDecay")).unwrap_or(0);
    e.last_slept = last_slept;
    // LivingEntity.readAdditionalSaveData: asleep in the bed again (the
    // pose; `ai_step` turns it the way the bed faces, or wakes it when the
    // bed is gone, as `baseTick`'s first tick does).
    e.sleeping_pos = match tag.get("sleeping_pos") {
        Some(Tag::IntArray(a)) if a.len() == 3 => Some([a[0], a[1], a[2]]),
        _ => None,
    };
    let asleep = e.sleeping_pos.is_some();
    if let Some(Tag::List(items)) = tag.get("Inventory") {
        e.inventory = items
            .iter()
            .filter_map(crate::save::item_from_nbt)
            .take(INVENTORY_SLOTS)
            .collect();
    }
    if asleep {
        m.extra.ints[1] = bed_facing_code(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn villager() -> Mob {
        use crate::mobs::{Anim, Brain, Flags, MobType, Part, Species};
        use std::{collections::HashMap, sync::Arc};
        let kind = Arc::new(MobType {
            id: "minecraft:villager".into(),
            species: Species::Villager,
            width: 0.6,
            height: 1.95,
            eye_height: 1.62,
            category: "misc".into(),
            max_health: 20.0,
            speed: 0.5,
            flying_speed: 0.02,
            attack_damage: 0.0,
            attack_knockback: 0.0,
            follow_range: 48.0,
            step_height: 0.6,
            armor: 0.0,
            armor_toughness: 0.0,
            knockback_resistance: 0.0,
            jump_strength: 0.42,
            attributes: Arc::default(),
            gravity: 0.08,
            safe_fall_distance: 3.0,
            fall_damage_multiplier: 1.0,
            water_efficiency: 0.0,
            water_slowdown: 0.8,
            air_speed: 0.02,
            baby_scale: 0.5,
            head_rot_speed: 10.0,
            max_head_x_rot: 40.0,
            max_head_y_rot: 75.0,
            ambient_interval: 80,
            flags: Flags::default(),
            model: Arc::new(Part::parse("root", &serde_json::json!({}))),
            layers: HashMap::new(),
            texture: String::new(),
            brain: Brain::Passive,
            anim: Anim::Villager,
            sounds: [""; 3],
            step_sound: None,
            food: Arc::default(),
            tempt: Arc::default(),
        });
        Mob::new(1, kind, horizon_core::geometry::Vec3::ZERO, 0.0)
    }
    /// A sleeper's box is 0.2 high, but `LivingEntityRenderer` moves it
    /// toward the foot of the bed by its standing eye height, so its head
    /// lies on the pillow.
    #[test]
    fn a_sleeper_lies_by_its_standing_eye_height() {
        let mut m = villager();
        m.extra.ints[1] = bed_facing_code(Some("east"));
        assert!(sleeping(&m));
        assert_eq!(m.eye_height(), f64::from(0.2_f32));
        assert_eq!(m.awake_dims().2, f64::from(1.62_f32));
    }
    /// `LivingEntity.add/readAdditionalSaveData`: a sleeper is saved with
    /// its bed (`sleeping_pos`) and loads asleep in it.
    #[test]
    fn a_sleeper_is_saved_asleep_in_its_bed() {
        let mut m = villager();
        m.extra.ints[1] = bed_facing_code(Some("north"));
        ext_mut(&mut m).sleeping_pos = Some([3, 64, -7]);
        let mut map = horizon_world::nbt::Compound::new();
        write_nbt(&m, &mut map);
        let mut loaded = villager();
        read_nbt(&mut loaded, &horizon_world::nbt::Tag::Compound(map));
        assert!(sleeping(&loaded));
        assert_eq!(sleeping_pos(&loaded), Some([3, 64, -7]));
        let mut awake = villager();
        let mut map = horizon_world::nbt::Compound::new();
        write_nbt(&awake, &mut map);
        assert!(!map.contains_key(&"sleeping_pos".into()));
        read_nbt(&mut awake, &horizon_world::nbt::Tag::Compound(map));
        assert!(!sleeping(&awake));
    }
    #[test]
    fn a_new_profession_rolls_new_offers() {
        let mut m = villager();
        set_profession(&mut m, 1);
        // Offers made for the first profession.
        m.extra.trade.get_or_insert_default().unset = false;
        // The same profession keeps them.
        set_profession(&mut m, 1);
        assert!(!m.extra.trade.as_ref().unwrap().unset);
        // A new one forgets them (offers = null), so the next getOffers
        // (ensure_offers) rolls the new profession's trades.
        set_profession(&mut m, 2);
        let t = m.extra.trade.as_ref().unwrap();
        assert!(t.offers.is_empty());
        assert!(t.unset);
        assert_eq!(m.extra.variant2, 2);
    }
    /// The reported sequence: a villager works at a bench, the bench is
    /// destroyed (its job site is gone, so `ResetProfession` makes it
    /// unemployed) and it takes another bench's job. The new profession's
    /// trades are offered again.
    ///
    /// `Villager.setVillagerData` drops the offers (`offers = null`) when the
    /// profession changes, so `AbstractVillager.getOffers` rolls the new
    /// profession's trade set the next time the offers are asked for. A
    /// villager that keeps its old offers marked as made never trades again:
    /// it shakes its head instead of opening the trading menu.
    #[test]
    fn a_villager_that_loses_its_job_site_offers_its_new_profession_trades() {
        let mut m = villager();
        // The composter's job: the villager is a farmer.
        set_profession(&mut m, 5);
        assert_eq!(m.extra.variant2, 5);
        // Its trades were made, so the offers are there.
        let trade = m.extra.trade.get_or_insert_default();
        trade.unset = false;
        trade.offers = vec![crate::merchant::MerchantOffer {
            base_cost_a: crate::merchant::ItemCost {
                stack: ItemStack::new("minecraft:wheat", 20),
            },
            cost_b: None,
            result: ItemStack::new("minecraft:emerald", 1),
            uses: 0,
            max_uses: 16,
            reward_exp: true,
            special_price_diff: 0,
            demand: 0,
            price_multiplier: 0.0,
            xp: 1,
        }];
        // The work bench is destroyed: the job site is gone, so the villager
        // is unemployed and its offers are dropped (`offers = null`).
        set_profession(&mut m, NONE);
        let t = m.extra.trade.as_ref().unwrap();
        assert!(t.offers.is_empty());
        assert!(t.unset);
        // It takes the smithing table's job. The new profession's trades are
        // rolled by the next `getOffers` (ensure_offers).
        set_profession(&mut m, 13);
        let t = m.extra.trade.as_ref().unwrap();
        assert!(t.offers.is_empty());
        assert!(t.unset);
        assert_eq!(m.extra.variant2, 13);
    }

    fn stack(item: &str, count: u32) -> ItemStack {
        ItemStack::new(item, count)
    }
    #[test]
    fn eating_fills_the_food_level_then_digests() {
        let mut e = Ext {
            inventory: vec![stack("minecraft:bread", 3), stack("minecraft:carrot", 5)],
            ..Default::default()
        };
        assert_eq!(e.food_in_inventory(), 17);
        // Hungry below 12: three breads fill it exactly, then 12 digest away.
        e.eat_and_digest();
        assert_eq!(e.food_level, 0);
        assert_eq!(e.inventory.len(), 1);
        assert_eq!(e.inventory[0].count, 5);
        // Five carrots are not enough to be full again, and are all eaten.
        e.eat_and_digest();
        assert_eq!(e.food_level, 5 - 12);
        assert!(e.inventory.is_empty());
    }
    #[test]
    fn willingness_to_breed_needs_twelve_food_points() {
        let mut e = Ext::default();
        assert!(e.wants_more_food() && !e.has_excess_food());
        e.inventory.push(stack("minecraft:bread", 6));
        assert!(e.has_excess_food() && !e.wants_more_food());
        e.food_level = -5;
        assert_eq!(e.food_level + e.food_in_inventory(), 19);
    }
    #[test]
    fn the_inventory_has_eight_stacking_slots() {
        let mut e = Ext::default();
        assert!(e.add_item(stack("minecraft:bread", 100), 64).is_none());
        assert_eq!(e.inventory.len(), 2);
        assert_eq!(e.inventory[0].count, 64);
        for i in 0..6 {
            assert!(
                e.add_item(stack(&format!("minecraft:item_{i}"), 1), 64)
                    .is_none()
            );
        }
        assert_eq!(e.inventory.len(), 8);
        // Full: another kind is refused, the same kind tops up the stack.
        assert!(!e.can_add(&stack("minecraft:stick", 1), 64));
        let left = e.add_item(stack("minecraft:bread", 40), 64).unwrap();
        assert_eq!(left.count, 40 - 28);
    }
    #[test]
    fn half_stacks_are_thrown() {
        let mut e = Ext {
            inventory: vec![
                stack("minecraft:bread", 10),
                stack("minecraft:wheat", 50),
                stack("minecraft:wheat", 20),
            ],
            ..Default::default()
        };
        let max = |_: &str| 64;
        // Bread is under half a stack and under 24: nothing.
        assert!(
            e.take_half_stack(|s| &*s.item == "minecraft:bread", max)
                .is_none()
        );
        // 50 wheat is over half a stack: 25 go.
        let t = e
            .take_half_stack(|s| &*s.item == "minecraft:wheat", max)
            .unwrap();
        assert_eq!(t.count, 25);
        assert_eq!(e.count_item("minecraft:wheat"), 25 + 20);
    }
    #[test]
    fn golems_need_sleep_and_no_golem_nearby() {
        let mut e = Ext::default();
        assert!(!e.wants_golem(100));
        e.last_slept = Some(0);
        assert!(e.wants_golem(23_999));
        assert!(!e.wants_golem(24_000));
        e.last_slept = Some(1000);
        e.golem_until = 2000 + 599;
        assert!(!e.wants_golem(2500));
        assert!(e.wants_golem(2700));
    }
    #[test]
    fn gossip_decays_once_a_day_and_heard_gossip_waits() {
        let player = [1, 2, 3, 4];
        let mut e = Ext::default();
        e.gossips
            .add(player, crate::gossip::GossipType::MajorNegative, 25);
        e.maybe_decay(100);
        assert_eq!(e.last_gossip_decay, 100);
        e.maybe_decay(24_099);
        assert_eq!(
            e.gossips
                .get(player, crate::gossip::GossipType::MajorNegative),
            25
        );
        e.maybe_decay(24_100);
        assert_eq!(
            e.gossips
                .get(player, crate::gossip::GossipType::MajorNegative),
            15
        );
        e.last_gossip_time = 5000;
        assert!(!e.can_gossip(5100));
        assert!(e.can_gossip(6200));
        assert!(e.can_gossip(4000));
    }
    #[test]
    fn villager_food_values() {
        assert_eq!(food_points("minecraft:bread"), 4);
        assert_eq!(food_points("minecraft:beetroot"), 1);
        assert_eq!(food_points("minecraft:apple"), 0);
    }
}
