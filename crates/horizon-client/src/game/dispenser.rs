//! Dispenser behaviours: `DispenserBlock.getDispenseMethod(..).dispense` with
//! every `DispenseItemBehavior` registration of vanilla that Horizon has the
//! entities for. A behaviour returns what is left in the slot and the level
//! event it plays (`DefaultDispenseItemBehavior.dispense` then plays that
//! event and the smoke animation, event 2000).
use super::Game;
use horizon_client::{
    Result,
    collisions::SceneCollisions,
    containers::BlockEntity,
    items::ItemStack,
    mobs::{Mob, Species, sulfur_cube},
    particles::{ParticleOptions, ParticleType},
    projectiles::{Projectile, ProjectileKind},
};
use horizon_core::{
    geometry::{Aabb, Direction, Vec3},
    position::BlockPos,
    random::{LegacyRandom, RandomSource},
};
use horizon_world::{
    fluid::FluidKind,
    level::{BucketContent, PlaceContext, PlaceOutcome},
    registry::BlockStateId,
};
use std::sync::Arc;

type Pos = (i32, i32, i32);

/// The sound event of a plain click, a failed dispense and a launched projectile.
const CLICK: i32 = 1000;
const FAIL: i32 = 1001;
const LAUNCH: i32 = 1002;

fn step(d: Direction) -> [i32; 3] {
    match d {
        Direction::Down => [0, -1, 0],
        Direction::Up => [0, 1, 0],
        Direction::North => [0, 0, -1],
        Direction::South => [0, 0, 1],
        Direction::West => [-1, 0, 0],
        Direction::East => [1, 0, 0],
    }
}
fn offset(p: Pos, d: Direction) -> Pos {
    let s = step(d);
    (p.0 + s[0], p.1 + s[1], p.2 + s[2])
}
/// `Direction.get3DDataValue`.
fn data_value(d: Direction) -> i32 {
    match d {
        Direction::Down => 0,
        Direction::Up => 1,
        Direction::North => 2,
        Direction::South => 3,
        Direction::West => 4,
        Direction::East => 5,
    }
}
/// `Direction.toYRot` (vertical directions give 270 through `data2d = -1`).
fn y_rot(d: Direction) -> f32 {
    match d {
        Direction::South => 0.0,
        Direction::West => 90.0,
        Direction::North => 180.0,
        Direction::East => 270.0,
        _ => 270.0,
    }
}
fn block_box(p: Pos) -> Aabb {
    Aabb::new(
        Vec3::new(f64::from(p.0), f64::from(p.1), f64::from(p.2)),
        Vec3::new(f64::from(p.0 + 1), f64::from(p.1 + 1), f64::from(p.2 + 1)),
    )
}
fn overlaps(a: Aabb, b: Aabb) -> bool {
    let (a0, a1, b0, b1) = (
        a.min_position(),
        a.max_position(),
        b.min_position(),
        b.max_position(),
    );
    a0.x < b1.x && b0.x < a1.x && a0.y < b1.y && b0.y < a1.y && a0.z < b1.z && b0.z < a1.z
}
/// `ItemStack.shrink(1)`; the slot is empty at zero.
fn shrink(mut stack: ItemStack) -> Option<ItemStack> {
    stack.count -= 1;
    (stack.count > 0).then_some(stack)
}
/// A dispenser's answer: the slot's new content and the level event played.
type Dispensed = (Option<ItemStack>, i32);

thread_local! {
    /// `OptionalDispenseItemBehavior.success` of the registered behaviours
    /// that leave it unset on some paths (brush, TNT, honeycomb, chest).
    /// Vanilla keeps one behaviour instance per item for the whole run, so
    /// there the flag carries over from that item's previous dispense.
    static SUCCESS: std::cell::RefCell<std::collections::HashMap<&'static str, bool>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}
fn set_success(behaviour: &'static str, success: bool) {
    SUCCESS.with(|s| s.borrow_mut().insert(behaviour, success));
}
/// The level event `OptionalDispenseItemBehavior.playSound` plays for the flag.
fn optional_event(behaviour: &'static str) -> i32 {
    if SUCCESS.with(|s| s.borrow().get(behaviour).copied().unwrap_or(true)) {
        CLICK
    } else {
        FAIL
    }
}

/// Entities `getEntitiesOfClass(.., AABB(pos))` can find.
#[derive(Clone, Copy)]
enum Target {
    Player,
    Mob(usize),
    Stand(usize),
}

/// `RandomSource.create()`: the unseeded random an entity owns
/// (`RandomSupport.generateUniqueSeed`). Draws of an entity's own random never
/// touch the level's stream.
pub(super) fn entity_random() -> LegacyRandom {
    use std::sync::atomic::{AtomicI64, Ordering};
    static UNIQUIFIER: AtomicI64 = AtomicI64::new(8_682_522_807_148_012);
    const STEP: i64 = 1_181_783_497_276_652_981;
    // seedUniquifier.updateAndGet(s -> s * STEP)
    let mut current = UNIQUIFIER.load(Ordering::Relaxed);
    let seed = loop {
        let next = current.wrapping_mul(STEP);
        match UNIQUIFIER.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => break next,
            Err(actual) => current = actual,
        }
    };
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as i64);
    LegacyRandom::new(seed ^ nanos)
}
/// `new ItemEntity(level, x, y, z, stack)`'s motion, from the item's own random.
pub(super) fn new_item_motion() -> [f64; 3] {
    let mut r = entity_random();
    let x = r.next_double() * 0.2 - 0.1;
    let z = r.next_double() * 0.2 - 0.1;
    [x, 0.2, z]
}

/// `EquipmentSlot`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Slot {
    MainHand,
    OffHand,
    Feet,
    Legs,
    Chest,
    Head,
    Body,
    Saddle,
}
impl Slot {
    pub(super) fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "mainhand" => Self::MainHand,
            "offhand" => Self::OffHand,
            "feet" => Self::Feet,
            "legs" => Self::Legs,
            "chest" => Self::Chest,
            "head" => Self::Head,
            "body" => Self::Body,
            "saddle" => Self::Saddle,
            _ => return None,
        })
    }
    /// The player's armour array index (head, chest, legs, feet).
    fn armor_index(self) -> Option<usize> {
        match self {
            Self::Head => Some(0),
            Self::Chest => Some(1),
            Self::Legs => Some(2),
            Self::Feet => Some(3),
            _ => None,
        }
    }
    /// `ArmorStand` equipment index (mainhand, offhand, feet, legs, chest, head).
    fn stand_index(self) -> Option<usize> {
        match self {
            Self::MainHand => Some(0),
            Self::OffHand => Some(1),
            Self::Feet => Some(2),
            Self::Legs => Some(3),
            Self::Chest => Some(4),
            Self::Head => Some(5),
            _ => None,
        }
    }
}

fn equine(s: Species) -> bool {
    matches!(
        s,
        Species::Horse
            | Species::Donkey
            | Species::Mule
            | Species::SkeletonHorse
            | Species::ZombieHorse
            | Species::Camel
            | Species::CamelHusk
            | Species::Llama
            | Species::TraderLlama
    )
}
/// Mobs whose body slot Horizon keeps in `extra.head` (no helmet slot is drawn).
pub(super) fn body_in_head(s: Species) -> bool {
    equine(s)
        || matches!(s, Species::Wolf | Species::HappyGhast | Species::SulfurCube)
        || horizon_client::mobs::nautilus::is_nautilus(s)
}
/// What a mob holds in a slot.
pub(super) fn mob_slot_item(m: &Mob, slot: Slot) -> Option<Arc<str>> {
    let x = &m.extra;
    let body = body_in_head(m.species());
    match slot {
        Slot::MainHand => x.main_hand.clone(),
        Slot::OffHand => x.off_hand.clone(),
        Slot::Feet => x.feet.clone(),
        Slot::Legs => x.legs.clone(),
        Slot::Chest => x.chest.clone(),
        Slot::Head => x.head.clone().filter(|_| !body),
        Slot::Body => x.head.clone().filter(|_| body),
        Slot::Saddle => m.saddle_item().map(|s| s.item),
    }
}
/// `LivingEntity.canUseSlot` and its overrides.
fn mob_can_use_slot(m: &Mob, slot: Slot) -> bool {
    let s = m.species();
    let grown = m.alive() && !m.is_baby();
    match s {
        Species::Pig | Species::Strider if slot == Slot::Saddle => grown,
        Species::Horse
        | Species::SkeletonHorse
        | Species::ZombieHorse
        | Species::Llama
        | Species::TraderLlama => true,
        _ if equine(s) && slot == Slot::Saddle => grown && m.extra.tame,
        _ if horizon_client::mobs::nautilus::is_nautilus(s)
            && matches!(slot, Slot::Saddle | Slot::Body) =>
        {
            grown && m.extra.tame
        }
        Species::SulfurCube | Species::HappyGhast if slot == Slot::Body => grown,
        _ => true,
    }
}
/// `Mob.canDispenserEquipIntoSlot` and its overrides.
fn mob_can_dispenser_equip(m: &Mob, slot: Slot) -> bool {
    let s = m.species();
    let loot = m.extra.pick_up_loot;
    match s {
        Species::Pig | Species::Strider => slot == Slot::Saddle || loot,
        _ if equine(s) => matches!(slot, Slot::Body | Slot::Saddle) && m.extra.tame || loot,
        _ if horizon_client::mobs::nautilus::is_nautilus(s) => {
            matches!(slot, Slot::Body | Slot::Saddle) || loot
        }
        Species::SulfurCube | Species::HappyGhast => slot == Slot::Body,
        Species::Dolphin | Species::Panda | Species::Fox => slot == Slot::MainHand && loot,
        Species::Allay => false,
        _ => loot,
    }
}
/// `getEquipSound`: the saddle overrides of pigs, striders, horses, camels and nautili.
fn equip_sound(m: &Mob, slot: Slot, default: Option<String>) -> Option<String> {
    if slot != Slot::Saddle {
        return default;
    }
    let s = m.species();
    Some(match s {
        Species::Pig => "minecraft:entity.pig.saddle".into(),
        Species::Strider => "minecraft:entity.strider.saddle".into(),
        Species::Camel => "minecraft:entity.camel.saddle".into(),
        Species::CamelHusk => "minecraft:entity.camel_husk.saddle".into(),
        _ if equine(s) => "minecraft:entity.horse.saddle".into(),
        _ if horizon_client::mobs::nautilus::is_nautilus(s) => {
            if m.fluids.eye_in_water {
                "minecraft:item.nautilus_saddle_underwater_equip".into()
            } else {
                "minecraft:item.nautilus_saddle_equip".into()
            }
        }
        _ => return default,
    })
}
pub(super) fn mob_set_slot(m: &mut Mob, slot: Slot, stack: ItemStack) {
    let item: Arc<str> = stack.item.clone();
    let x = &mut m.extra;
    match slot {
        Slot::MainHand => x.main_hand = Some(item),
        Slot::OffHand => x.off_hand = Some(item),
        Slot::Feet => x.feet = Some(item),
        Slot::Legs => x.legs = Some(item),
        Slot::Chest => x.chest = Some(item),
        Slot::Head | Slot::Body => x.head = Some(item),
        Slot::Saddle => m.set_saddle(Some(stack)),
    }
}

impl Game<'_> {
    fn prop(&self, state: BlockStateId, key: &str) -> Option<String> {
        self.pack
            .assets
            .registry
            .state(state)?
            .properties
            .get(key)
            .cloned()
    }
    /// The same block with other properties (`withProperty`).
    pub(super) fn with_props(
        &self,
        state: BlockStateId,
        props: &[(&str, &str)],
    ) -> Option<BlockStateId> {
        let mut s = self.pack.assets.registry.state(state)?.clone();
        for (k, v) in props {
            s.properties.insert((*k).to_owned(), (*v).to_owned());
        }
        self.pack.assets.registry.resolve(&s).ok()
    }
    fn block_at(&self, p: Pos) -> BlockStateId {
        self.world.block(p.0, p.1, p.2)
    }
    fn dispenser_block_name(&self, p: Pos) -> std::sync::Arc<str> {
        self.res.facts().get(self.block_at(p)).name.clone()
    }
    fn in_block_tag(&self, tag: &str, block: &str) -> bool {
        self.res
            .items()
            .block_tags
            .get(tag)
            .is_some_and(|s| s.contains(block))
    }
    fn water_at(&self, p: Pos) -> bool {
        matches!(
            self.res.behaviours().fluid_kind(self.block_at(p)),
            FluidKind::Water | FluidKind::FlowingWater
        )
    }
    fn center(p: Pos) -> [f64; 3] {
        [
            f64::from(p.0) + 0.5,
            f64::from(p.1) + 0.5,
            f64::from(p.2) + 0.5,
        ]
    }
    fn pos_of(p: Pos) -> BlockPos {
        BlockPos::new(p.0, p.1, p.2)
    }

    // ---- level events (LevelEventHandler) ----

    /// The dispenser level events: 1000 click, 1001 fail, 1002 launch, 1004
    /// firework shoot, 1018 blaze shoot and 2000 smoke shot out of the front.
    pub(super) fn dispenser_event(&mut self, id: i32, pos: Pos, data: i32) {
        super::dispense_fixture::record(|| format!("L {id} {},{},{} {data}", pos.0, pos.1, pos.2));
        let c = Self::center(pos);
        match id {
            1000 => self
                .audio
                .play("minecraft:block.dispenser.dispense", Some(c), 1.0, 1.0),
            1001 => self
                .audio
                .play("minecraft:block.dispenser.fail", Some(c), 1.0, 1.2),
            1002 => self
                .audio
                .play("minecraft:block.dispenser.launch", Some(c), 1.0, 1.2),
            1004 => self
                .audio
                .play("minecraft:entity.firework_rocket.shoot", Some(c), 1.0, 1.2),
            // LevelEventHandler runs on the client: its pitch draws come from
            // the client level's random, never the server stream.
            1018 => {
                let r = self.particles.random();
                let pitch = (r.next_float() - r.next_float()) * 0.2 + 1.0;
                self.audio
                    .play("minecraft:entity.blaze.shoot", Some(c), 2.0, pitch);
            }
            1051 => {
                let pitch = 0.4 / (self.particles.random().next_float() * 0.4 + 0.8);
                self.audio
                    .play("minecraft:entity.wind_charge.throw", Some(c), 0.5, pitch);
            }
            1049 => self
                .audio
                .play("minecraft:block.crafter.craft", Some(c), 1.0, 1.0),
            1050 => self
                .audio
                .play("minecraft:block.crafter.fail", Some(c), 1.0, 1.0),
            // LevelEventHandler.shootParticles: smoke for dispensers, white
            // smoke out of a crafter's front.
            2000 | 2010 => {
                let particle = if id == 2010 {
                    ParticleType::WhiteSmoke
                } else {
                    ParticleType::Smoke
                };
                let s = match data {
                    0 => [0, -1, 0],
                    1 => [0, 1, 0],
                    2 => [0, 0, -1],
                    3 => [0, 0, 1],
                    4 => [-1, 0, 0],
                    _ => [1, 0, 0],
                };
                for _ in 0..10 {
                    let r = self.particles.random();
                    let pow = r.next_double() * 0.2 + 0.01;
                    let (sx, sy, sz) = (f64::from(s[0]), f64::from(s[1]), f64::from(s[2]));
                    let p = [
                        f64::from(pos.0)
                            + sx * 0.6
                            + 0.5
                            + sx * 0.01
                            + (r.next_double() - 0.5) * sz * 0.5,
                        f64::from(pos.1)
                            + sy * 0.6
                            + 0.5
                            + sy * 0.01
                            + (r.next_double() - 0.5) * sy * 0.5,
                        f64::from(pos.2)
                            + sz * 0.6
                            + 0.5
                            + sz * 0.01
                            + (r.next_double() - 0.5) * sx * 0.5,
                    ];
                    let v = [
                        sx * pow + r.next_gaussian() * 0.01,
                        sy * pow + r.next_gaussian() * 0.01,
                        sz * pow + r.next_gaussian() * 0.01,
                    ];
                    self.particles.add(&ParticleOptions::simple(particle), p, v);
                }
            }
            _ => {}
        }
    }

    // ---- DefaultDispenseItemBehavior ----

    /// `DispenserBlock.getDispensePosition(source, scale, offset)`.
    fn dispense_position(pos: Pos, facing: Direction, scale: f64, offset: [f64; 3]) -> Vec3 {
        let s = step(facing);
        let c = Self::center(pos);
        Vec3::new(
            c[0] + scale * f64::from(s[0]) + offset[0],
            c[1] + scale * f64::from(s[1]) + offset[1],
            c[2] + scale * f64::from(s[2]) + offset[2],
        )
    }
    /// `DefaultDispenseItemBehavior.spawnItem` at `getDispensePosition`.
    pub(super) fn spawn_dispensed(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
        accuracy: i32,
    ) {
        let s = step(facing);
        let p = Self::dispense_position(pos, facing, 0.7, [0.0; 3]);
        let mut at = [p.x, p.y, p.z];
        at[1] -= if s[1] != 0 { 0.125 } else { 0.156_25 };
        let r = &mut self.world.random;
        let pow = r.next_double() * 0.1 + 0.2;
        let spread = 0.017_227_5 * f64::from(accuracy);
        let v = [
            r.triangle(f64::from(s[0]) * pow, spread),
            r.triangle(0.2, spread),
            r.triangle(f64::from(s[2]) * pow, spread),
        ];
        self.spawn_item(stack, at, v, 0);
    }
    /// `DefaultDispenseItemBehavior.execute`: drop one item.
    fn default_execute(
        &mut self,
        pos: Pos,
        facing: Direction,
        mut stack: ItemStack,
    ) -> Option<ItemStack> {
        let one = ItemStack {
            count: 1,
            ..stack.clone()
        };
        stack.count -= 1;
        self.spawn_dispensed(pos, facing, one, 6);
        (stack.count > 0).then_some(stack)
    }
    /// `new DefaultDispenseItemBehavior().dispense(..)` including its events.
    pub(super) fn dispense_default(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Option<ItemStack> {
        let left = self.default_execute(pos, facing, stack);
        self.dispenser_event(CLICK, pos, 0);
        self.dispenser_event(2000, pos, data_value(facing));
        left
    }
    /// `DefaultDispenseItemBehavior.consumeWithRemainder`: the remainder goes
    /// into the dispenser (`insertItem`) or is dropped.
    fn consume_with_remainder(
        &mut self,
        pos: Pos,
        facing: Direction,
        mut stack: ItemStack,
        remainder: ItemStack,
    ) -> Option<ItemStack> {
        stack.count -= 1;
        if stack.count == 0 {
            return Some(remainder);
        }
        let mut rest = remainder;
        if let Some(entity) = self.world.block_entities.get_mut(&pos) {
            let max = self
                .res
                .items()
                .get(&rest.item)
                .map_or(64, |i| i.max_stack)
                .max(1);
            for cell in entity.slots_mut() {
                match cell {
                    None => {
                        let n = rest.count.min(max);
                        *cell = Some(ItemStack {
                            count: n,
                            ..rest.clone()
                        });
                        rest.count -= n;
                    }
                    Some(e) if e.same_item_same_components(&rest) => {
                        let n = rest.count.min(max.saturating_sub(e.count));
                        e.count += n;
                        rest.count -= n;
                    }
                    Some(_) => {}
                }
                if rest.count == 0 {
                    break;
                }
            }
        }
        if rest.count > 0 {
            self.spawn_dispensed(pos, facing, rest, 6);
            self.dispenser_event(CLICK, pos, 0);
            self.dispenser_event(2000, pos, data_value(facing));
        }
        Some(stack)
    }
    /// `ItemStack.hurtAndBreak(amount)` without an entity.
    fn hurt_and_break(&mut self, mut stack: ItemStack, amount: u32) -> Option<ItemStack> {
        let max = self
            .res
            .items()
            .get(&stack.item)
            .map_or(0, |i| i.max_damage);
        if max == 0 {
            return Some(stack);
        }
        stack.damage += self.durability_loss(&stack, amount);
        if stack.damage >= max {
            return shrink(ItemStack { damage: 0, ..stack });
        }
        Some(stack)
    }

    // ---- the registry ----

    /// `DispenserBlock.getDispenseMethod(..).dispense(source, stack)`: what the
    /// slot holds afterwards.
    pub(super) fn dispense_item(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Option<ItemStack>> {
        let item = stack.item.to_string();
        let (left, event) = match item.as_str() {
            "minecraft:arrow"
            | "minecraft:tipped_arrow"
            | "minecraft:spectral_arrow"
            | "minecraft:egg"
            | "minecraft:blue_egg"
            | "minecraft:brown_egg"
            | "minecraft:snowball"
            | "minecraft:experience_bottle"
            | "minecraft:splash_potion"
            | "minecraft:lingering_potion"
            | "minecraft:firework_rocket"
            | "minecraft:fire_charge"
            | "minecraft:wind_charge" => self.dispense_projectile(pos, facing, stack),
            "minecraft:armor_stand" => self.dispense_armor_stand(pos, facing, stack),
            "minecraft:chest" => self.dispense_chest(pos, facing, stack),
            "minecraft:lava_bucket"
            | "minecraft:water_bucket"
            | "minecraft:powder_snow_bucket"
            | "minecraft:salmon_bucket"
            | "minecraft:cod_bucket"
            | "minecraft:pufferfish_bucket"
            | "minecraft:tropical_fish_bucket"
            | "minecraft:axolotl_bucket"
            | "minecraft:tadpole_bucket"
            | "minecraft:sulfur_cube_bucket" => self.dispense_filled_bucket(pos, facing, stack)?,
            "minecraft:bucket" => self.dispense_empty_bucket(pos, facing, stack)?,
            "minecraft:flint_and_steel" => self.dispense_flint_and_steel(pos, facing, stack)?,
            "minecraft:bone_meal" => self.dispense_bone_meal(pos, facing, stack)?,
            "minecraft:tnt" => self.dispense_tnt(pos, facing, stack),
            "minecraft:wither_skeleton_skull" => self.dispense_wither_skull(pos, facing, stack)?,
            "minecraft:carved_pumpkin" => self.dispense_carved_pumpkin(pos, facing, stack)?,
            "minecraft:glass_bottle" => self.dispense_glass_bottle(pos, facing, stack)?,
            "minecraft:glowstone" => self.dispense_glowstone(pos, facing, stack)?,
            "minecraft:shears" => self.dispense_shears(pos, facing, stack)?,
            "minecraft:brush" => self.dispense_brush(pos, facing, stack),
            "minecraft:honeycomb" => self.dispense_honeycomb(pos, facing, stack)?,
            "minecraft:potion" => self.dispense_potion(pos, facing, stack)?,
            _ if item.ends_with("shulker_box") => self.dispense_shulker_box(pos, facing, stack)?,
            _ if super::boats::any_boat_wood(&item).is_some() => {
                self.dispense_boat(pos, facing, stack)
            }
            _ if super::minecarts::CartKind::from_id(&item).is_some() => {
                self.dispense_minecart(pos, facing, stack)
            }
            _ => self.dispense_default_method(pos, facing, stack)?,
        };
        self.dispenser_event(event, pos, 0);
        self.dispenser_event(2000, pos, data_value(facing));
        Ok(left)
    }
    /// `DispenserBlock.getDefaultDispenseMethod`: equipment, spawn eggs, or a drop.
    fn dispense_default_method(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let equippable = self
            .res
            .items()
            .get(&stack.item)
            .is_some_and(|i| i.equip_slot.is_some());
        if equippable {
            let mut stack = stack;
            if self.dispense_equipment(pos, facing, &mut stack) {
                return Ok((Some(stack).filter(|s| s.count > 0), CLICK));
            }
            return Ok((self.default_execute(pos, facing, stack), CLICK));
        }
        let swallowable = self
            .res
            .items()
            .item_tags
            .get("minecraft:sulfur_cube_swallowable")
            .is_some_and(|t| t.contains(&*stack.item));
        if swallowable {
            let front = offset(pos, facing);
            if self.sulfur_cube_swallow(front, &stack) {
                return Ok((shrink(stack), CLICK));
            }
            return Ok((self.default_execute(pos, facing, stack), CLICK));
        }
        if stack.item.ends_with("_spawn_egg") {
            return self.dispense_spawn_egg(pos, facing, stack);
        }
        Ok((self.default_execute(pos, facing, stack), CLICK))
    }

    // ---- entities ----

    /// Entities whose box overlaps the block at `p`, in the order the level
    /// added them (an entity section keeps insertion order; ids grow with it).
    fn targets_at(&self, p: Pos) -> Vec<Target> {
        let block = block_box(p);
        let mut found: Vec<(u32, Target)> = self
            .mobs
            .iter()
            .enumerate()
            .filter(|(_, m)| !m.removed && overlaps(m.bounds(), block))
            .map(|(i, m)| (m.id, Target::Mob(i)))
            .collect();
        found.extend(
            self.armor_stands
                .iter()
                .enumerate()
                .filter(|(_, s)| overlaps(s.bounds(), block))
                .map(|(i, s)| (s.id, Target::Stand(i))),
        );
        found.sort_by_key(|(id, _)| *id);
        let mut out: Vec<Target> = found.into_iter().map(|(_, t)| t).collect();
        if let Some(v) = self.player_view()
            && v.alive
            && !self.spectator()
            && overlaps(v.bounds, block)
        {
            out.push(Target::Player);
        }
        out
    }
    /// `EntityType.getYOffset(level, pos, false, box)`: the entity box set on
    /// top of the block (`setPos(x + 0.5, y + 1, z + 0.5)`) falls one block
    /// against the collision shapes overlapping that block alone.
    fn drop_offset(&self, width: f64, height: f64, p: Pos) -> f64 {
        use horizon_world::{
            block_collision_scan::BlockCollisionScan, collision_context::CollisionContext,
        };
        let h = width / 2.0;
        let (x, z) = (f64::from(p.0) + 0.5, f64::from(p.2) + 0.5);
        let y = f64::from(p.1) + 1.0;
        let entity = Aabb::new(
            Vec3::new(x - h, y, z - h),
            Vec3::new(x + h, y + height, z + h),
        );
        let cell = Aabb::new(
            Vec3::new(f64::from(p.0), f64::from(p.1), f64::from(p.2)),
            Vec3::new(
                f64::from(p.0) + 1.0,
                f64::from(p.1) + 1.0,
                f64::from(p.2) + 1.0,
            ),
        );
        let mut scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        let mut amount = -1.0;
        // Level.getCollisions(null, aabb): CollisionContext.empty().
        if let Ok(scan) = BlockCollisionScan::new(
            &mut scene,
            CollisionContext::empty(false),
            cell,
            false,
            1_000_000,
        ) {
            for hit in scan.flatten() {
                amount = hit
                    .shape
                    .collide(horizon_core::geometry::Axis::Y, entity, amount);
            }
        }
        1.0 + amount
    }
    /// `EntityType.spawn(.., pos, reason, tryMoveDown, false)` for a mob.
    pub(super) fn spawn_mob_at(&mut self, kind: &str, p: Pos, try_move_down: bool) -> Option<u32> {
        let name = if kind.contains(':') {
            kind.to_owned()
        } else {
            format!("minecraft:{kind}")
        };
        let t = self.res.mob_types().get(&name)?.clone();
        let y_off = if try_move_down {
            self.drop_offset(t.width, t.height, p)
        } else {
            0.0
        };
        let at = [
            f64::from(p.0) + 0.5,
            f64::from(p.1) + y_off,
            f64::from(p.2) + 0.5,
        ];
        let id = self.spawn_mob(&name, at, true)?;
        // finalizeSpawn's riders (chicken jockeys, spider jockeys...) come
        // with a dispensed mob as with any spawn that is not natural.
        self.spawn_jockeys(id, false);
        Some(id)
    }
    /// `EquipmentDispenseItemBehavior.dispenseEquipment`: the first living
    /// entity in front that `canEquipWithDispenser` gets one of the item.
    fn dispense_equipment(&mut self, pos: Pos, facing: Direction, stack: &mut ItemStack) -> bool {
        let Some(info) = self.res.items().get(&stack.item) else {
            return false;
        };
        let (Some(slot), true) = (
            info.equip_slot.as_deref().and_then(Slot::from_name),
            info.equip_dispensable,
        ) else {
            return false;
        };
        let allowed = info.equip_allowed.clone();
        // Equippable.equipSound defaults to the generic armour sound.
        let sound = info
            .equip_sound
            .clone()
            .or_else(|| Some("minecraft:item.armor.equip_generic".into()));
        let front = offset(pos, facing);
        let one = ItemStack {
            count: 1,
            ..stack.clone()
        };
        let allows = |id: &str| allowed.as_ref().is_none_or(|a| a.contains(id));
        // LivingEntity.canEquipWithDispenser: alive, the slot usable, the
        // item's allowed entities, the slot empty and the dispenser rule.
        let target = self.targets_at(front).into_iter().find(|t| match *t {
            // Player.canUseSlot: every slot; the armour and the off hand are
            // the ones an equippable item can reach.
            Target::Player => {
                allows("minecraft:player")
                    && match slot {
                        Slot::OffHand => self.offhand.is_none(),
                        _ => slot.armor_index().is_some_and(|i| self.armor[i].is_none()),
                    }
            }
            Target::Stand(i) => slot.stand_index().is_some_and(|s| {
                allows("minecraft:armor_stand") && self.armor_stands[i].equipment[s].is_none()
            }),
            Target::Mob(i) => {
                let m = &self.mobs[i];
                m.alive()
                    && mob_can_use_slot(m, slot)
                    && allows(&m.kind.id)
                    && mob_slot_item(m, slot).is_none()
                    && mob_can_dispenser_equip(m, slot)
            }
        });
        let Some(target) = target else {
            return false;
        };
        stack.count -= 1;
        // setItemSlot: onEquipItem plays the equip sound and the equip event.
        let (at, sound) = match target {
            Target::Player => {
                // Player.setItemSlot: onEquipItem plays the sound and the event.
                let name = match slot.armor_index() {
                    Some(i) => {
                        self.armor[i] = Some(one.clone());
                        ["head", "chest", "legs", "feet"][i]
                    }
                    None => {
                        self.offhand = Some(one.clone());
                        "offhand"
                    }
                };
                self.player_equipped(name, Some(&one));
                self.apply_equipment();
                return true;
            }
            Target::Stand(i) => {
                let stand = &mut self.armor_stands[i];
                if let Some(s) = slot.stand_index() {
                    stand.equipment[s] = Some(one);
                }
                (Some(stand.position), sound)
            }
            Target::Mob(i) => {
                let m = &mut self.mobs[i];
                mob_set_slot(m, slot, one);
                // setGuaranteedDrop / setPersistenceRequired.
                m.persistent = true;
                let sound = if m.silent {
                    None
                } else {
                    equip_sound(m, slot, sound)
                };
                (Some(m.position), sound)
            }
        };
        if let Some(at) = at {
            if let Some(sound) = sound {
                self.audio.play(&sound, Some([at.x, at.y, at.z]), 1.0, 1.0);
            }
            let who = match target {
                Target::Mob(i) => Some(horizon_client::mobs::Who::Mob(self.mobs[i].id)),
                Target::Player | Target::Stand(_) => None,
            };
            let ctx = self.who_context(who, None);
            self.post_game_event("minecraft:equip", [at.x, at.y, at.z], ctx);
        }
        true
    }
    /// After `EntityType.spawn` / `MobBucketItem.spawn`: `mob.playAmbientSound()`.
    fn mob_ambient(&mut self, id: u32) -> Result<()> {
        let Some(i) = self.mobs.iter().position(|m| m.id == id) else {
            return Ok(());
        };
        let mut m = self.mobs.remove(i);
        let mut events = Vec::new();
        {
            let w = self.mob_world_base(&[]);
            horizon_client::mobs::ai::ambient_sound(&mut m, &w, &mut events);
        }
        self.mobs.insert(i, m);
        self.handle_mob_events(events)
    }
    /// A mob a bucket emptied: `setFromBucket` keeps it, then it calls out.
    pub(super) fn mob_placed(&mut self, id: u32) -> Result<()> {
        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
            m.persistent = true;
        }
        self.mob_ambient(id)
    }
    fn dispense_spawn_egg(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let kind = stack.item.trim_end_matches("_spawn_egg").to_owned();
        let front = offset(pos, facing);
        let Some(id) = self.spawn_mob_at(&kind, front, facing != Direction::Up) else {
            return Ok((Some(stack), CLICK));
        };
        self.mob_ambient(id)?;
        // SpawnEggItemBehavior: the entity was placed by the dispenser.
        self.post_block_game_event("minecraft:entity_place", [pos.0, pos.1, pos.2], None, None);
        Ok((shrink(stack), CLICK))
    }

    // ---- ProjectileDispenseBehavior ----

    fn dispense_projectile(&mut self, pos: Pos, facing: Direction, stack: ItemStack) -> Dispensed {
        let item = stack.item.to_string();
        let s = step(facing);
        let dir = [f64::from(s[0]), f64::from(s[1]), f64::from(s[2])];
        // ProjectileItem.DispenseConfig per item: position, uncertainty, power, event.
        let (at, uncertainty, power, event) = match item.as_str() {
            "minecraft:firework_rocket" => (
                Self::dispense_position(pos, facing, 0.500_009_999_999_747_4, [0.0; 3]),
                1.0,
                0.5,
                1004,
            ),
            "minecraft:fire_charge" => (
                Self::dispense_position(pos, facing, 1.0, [0.0; 3]),
                6.666_666_5,
                1.0,
                1018,
            ),
            "minecraft:wind_charge" => (
                Self::dispense_position(pos, facing, 1.0, [0.0; 3]),
                6.666_666_5,
                1.0,
                1051,
            ),
            "minecraft:experience_bottle"
            | "minecraft:splash_potion"
            | "minecraft:lingering_potion" => (
                Self::dispense_position(pos, facing, 0.7, [0.0, 0.1, 0.0]),
                3.0,
                1.375,
                LAUNCH,
            ),
            _ => (
                Self::dispense_position(pos, facing, 0.7, [0.0, 0.1, 0.0]),
                6.0,
                1.1,
                LAUNCH,
            ),
        };
        self.shoot_projectile_item(at, dir, &stack, power, uncertainty);
        (shrink(stack), event)
    }

    /// `ProjectileItem.asProjectile` and `Projectile.spawnProjectileUsingShoot`:
    /// one of `stack` fired from `at` along `dir`.
    pub(super) fn shoot_projectile_item(
        &mut self,
        at: Vec3,
        dir: [f64; 3],
        stack: &ItemStack,
        power: f32,
        uncertainty: f32,
    ) {
        let item = stack.item.to_string();
        let one = ItemStack {
            count: 1,
            ..stack.clone()
        };
        // Projectile.shoot spreads the shot with the projectile's own random.
        let mut own = entity_random();
        match item.as_str() {
            "minecraft:experience_bottle"
            | "minecraft:splash_potion"
            | "minecraft:lingering_potion" => {
                let mut shot = Projectile::new(ProjectileKind::Snowball, at, None);
                shot.shoot(&mut own, dir, power, uncertainty);
                self.launch_thrown(one, at, shot.velocity, None);
            }
            "minecraft:firework_rocket" => {
                let mut shot = Projectile::new(ProjectileKind::Snowball, at, None);
                shot.shoot(&mut own, dir, power, uncertainty);
                self.launch_angled_rocket(at, shot.velocity, &one, None);
            }
            _ => {
                let kind = match item.as_str() {
                    "minecraft:arrow" => ProjectileKind::Arrow {
                        base_damage: 2.0,
                        critical: false,
                        effect: None,
                    },
                    "minecraft:spectral_arrow" => ProjectileKind::Arrow {
                        base_damage: 2.0,
                        critical: false,
                        effect: Some(("minecraft:glowing", 200)),
                    },
                    "minecraft:tipped_arrow" => ProjectileKind::Arrow {
                        base_damage: 2.0,
                        critical: false,
                        effect: tipped_effect(stack),
                    },
                    "minecraft:fire_charge" | "minecraft:wind_charge" => {
                        // asProjectile aims with the level's random first; the
                        // shot along the facing then replaces that motion.
                        let r = &mut self.world.random;
                        let _ = (
                            r.triangle(dir[0], 0.114_85),
                            r.triangle(dir[1], 0.114_85),
                            r.triangle(dir[2], 0.114_85),
                        );
                        if item == "minecraft:wind_charge" {
                            ProjectileKind::WindCharge
                        } else {
                            ProjectileKind::SmallFireball
                        }
                    }
                    "minecraft:egg" | "minecraft:blue_egg" | "minecraft:brown_egg" => {
                        ProjectileKind::Egg
                    }
                    _ => ProjectileKind::Snowball,
                };
                let mut shot = Projectile::new(kind, at, None);
                // ArrowItem/SpectralArrowItem.asProjectile: dispensed arrows
                // may be picked up.
                if let Some(a) = shot.arrow.as_mut() {
                    a.set_item(&one);
                    a.pickup = horizon_client::arrows::Pickup::Allowed;
                }
                shot.shoot(&mut own, dir, power, uncertainty);
                self.add_projectile(shot);
            }
        }
    }

    // ---- entities placed by items ----

    fn dispense_armor_stand(&mut self, pos: Pos, facing: Direction, stack: ItemStack) -> Dispensed {
        let front = offset(pos, facing);
        let foot = Vec3::new(
            f64::from(front.0) + 0.5,
            f64::from(front.1),
            f64::from(front.2) + 0.5,
        );
        // EntityType.create rolls a yaw from the level random before the stack config turns it.
        let _ = self.world.random.next_float();
        self.next_entity += 1;
        let stand =
            horizon_client::entities::ArmorStand::new(self.next_entity, foot, y_rot(facing));
        self.world.modified.insert((front.0 >> 4, front.2 >> 4));
        self.armor_stands.push(stand);
        (shrink(stack), CLICK)
    }
    /// A tame donkey, mule or llama in front without a chest gets it.
    fn dispense_chest(&mut self, pos: Pos, facing: Direction, stack: ItemStack) -> Dispensed {
        let front = offset(pos, facing);
        for target in self.targets_at(front) {
            let Target::Mob(i) = target else { continue };
            let m = &mut self.mobs[i];
            if matches!(
                m.species(),
                Species::Donkey | Species::Mule | Species::Llama | Species::TraderLlama
            ) && m.alive()
                && m.extra.tame
                && !m.extra.chested
            {
                m.extra.chested = true;
                set_success("chest", true);
                return (shrink(stack), CLICK);
            }
        }
        (
            self.default_execute(pos, facing, stack),
            optional_event("chest"),
        )
    }
    /// `BoatDispenseItemBehavior`.
    fn dispense_boat(&mut self, pos: Pos, facing: Direction, stack: ItemStack) -> Dispensed {
        let Some((wood, chest)) =
            super::boats::any_boat_wood(&stack.item).map(|(w, c)| (w.to_owned(), c))
        else {
            return (self.dispense_default(pos, facing, stack), CLICK);
        };
        let front = offset(pos, facing);
        let s = step(facing);
        let c = Self::center(pos);
        // Boat width 1.375.
        let outside = 0.5625 + 1.375 / 2.0;
        let (x, y, z) = (
            c[0] + f64::from(s[0]) * outside,
            c[1] + f64::from(s[1]) * f64::from(1.125f32),
            c[2] + f64::from(s[2]) * outside,
        );
        let below = (front.0, front.1 - 1, front.2);
        let y_offset = if self.water_at(front) {
            1.0
        } else if self.res.facts().is_air(self.block_at(front)) && self.water_at(below) {
            0.0
        } else {
            return (self.dispense_default(pos, facing, stack), CLICK);
        };
        if !self.boat_models.has(&wood, chest) {
            return (self.dispense_default(pos, facing, stack), CLICK);
        }
        self.next_entity += 1;
        let mut boat = super::boats::Boat::new(
            self.next_entity,
            wood.as_str().into(),
            Vec3::new(x, y + y_offset, z),
            y_rot(facing),
        );
        if chest {
            boat = boat.with_chest();
        }
        self.world.modified.insert((front.0 >> 4, front.2 >> 4));
        self.boats.push(boat);
        (shrink(stack), CLICK)
    }
    /// `MinecartDispenseItemBehavior`.
    fn dispense_minecart(&mut self, pos: Pos, facing: Direction, stack: ItemStack) -> Dispensed {
        let Some(kind) = super::minecarts::CartKind::from_id(&stack.item) else {
            return (self.dispense_default(pos, facing, stack), CLICK);
        };
        let s = step(facing);
        let c = Self::center(pos);
        let (x, base_y, z) = (
            c[0] + f64::from(s[0]) * 1.125,
            c[1].floor() + f64::from(s[1]),
            c[2] + f64::from(s[2]) * 1.125,
        );
        let front = offset(pos, facing);
        let y_offset = if let Some(rail) = self.rail(front.0, front.1, front.2) {
            if rail.shape.is_slope() { 0.6 } else { 0.1 }
        } else {
            if !self.res.facts().is_air(self.block_at(front)) {
                return (self.dispense_default(pos, facing, stack), CLICK);
            }
            let below = (front.0, front.1 - 1, front.2);
            let Some(rail) = self.rail(below.0, below.1, below.2) else {
                return (self.dispense_default(pos, facing, stack), CLICK);
            };
            if facing != Direction::Down && rail.shape.is_slope() {
                -0.4
            } else {
                -0.9
            }
        };
        self.next_entity += 1;
        let cart = super::minecarts::Minecart::new(
            self.next_entity,
            kind,
            Vec3::new(x, base_y + y_offset, z),
        );
        self.world.modified.insert((front.0 >> 4, front.2 >> 4));
        self.carts.push(cart);
        (shrink(stack), CLICK)
    }

    // ---- buckets ----

    /// The filled bucket behaviour: `DispensibleContainerItem.emptyContents`.
    fn dispense_filled_bucket(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let item = stack.item.to_string();
        let target = offset(pos, facing);
        let (content, sound, mob) =
            horizon_client::items::bucket_contents(&item).expect("filled bucket dispatcher");
        let target_pos = Self::pos_of(target);
        let emptied = self
            .with_level(|b, level, ticks| b.bucket_empty(level, ticks, target_pos, content, sound));
        self.process_level_events()?;
        if !emptied {
            return Ok((self.dispense_default(pos, facing, stack), CLICK));
        }
        if self.dim == super::Dim::Nether
            && matches!(
                content,
                BucketContent::Fluid(FluidKind::Water) | BucketContent::MobWater
            )
        {
            // The water evaporates in a cloud of smoke.
            let c = Self::center(target);
            for _ in 0..8 {
                let r = self.particles.random();
                let p = [
                    f64::from(target.0) + (r.next_double() * 2.0 - 1.0),
                    f64::from(target.1) + (r.next_double() * 2.0 - 1.0),
                    f64::from(target.2) + (r.next_double() * 2.0 - 1.0),
                ];
                let _ = c;
                self.particles.add(
                    &ParticleOptions::simple(ParticleType::LargeSmoke),
                    p,
                    [0.0; 3],
                );
            }
        }
        if let Some(kind) = mob {
            // MobBucketItem.checkExtraContent: the mob (from the bucket, so
            // persistent) calls out, then ENTITY_PLACE.
            if let Some(id) = self.spawn_mob_at(kind, target, true) {
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                    horizon_client::entity_nbt::load_bucket_data(m, &stack);
                }
                self.mob_placed(id)?;
            }
            self.post_block_game_event(
                "minecraft:entity_place",
                [target.0, target.1, target.2],
                None,
                None,
            );
        }
        let bucket = ItemStack::new("minecraft:bucket", 1);
        Ok((
            self.consume_with_remainder(pos, facing, stack, bucket),
            CLICK,
        ))
    }
    /// The empty bucket: `BucketPickup.pickupBlock`.
    fn dispense_empty_bucket(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = Self::pos_of(offset(pos, facing));
        let picked = self.with_level(|b, level, ticks| b.bucket_pickup(level, ticks, target));
        self.process_level_events()?;
        match picked {
            Some(filled) => {
                self.post_block_game_event(
                    "minecraft:fluid_pickup",
                    [target.x, target.y, target.z],
                    None,
                    None,
                );
                let filled = ItemStack::new(filled, 1);
                Ok((
                    self.consume_with_remainder(pos, facing, stack, filled),
                    CLICK,
                ))
            }
            None => Ok((self.default_execute(pos, facing, stack), CLICK)),
        }
    }

    // ---- fire, bone meal and TNT ----

    fn dispense_flint_and_steel(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = offset(pos, facing);
        let target_pos = Self::pos_of(target);
        let state = self.block_at(target);
        let name = self.dispenser_block_name(target);
        let mut success = true;
        // tryIgniteExplosiveEntities: the first sulfur cube that can explode is primed.
        let cube = self.targets_at(target).into_iter().find_map(|t| match t {
            Target::Mob(i) => {
                let m = &self.mobs[i];
                (m.species() == Species::SulfurCube
                    && m.alive()
                    && !sulfur_cube::primed(m)
                    && sulfur_cube::archetype(m).is_some_and(|a| a.explosion.is_some()))
                .then_some(i)
            }
            _ => None,
        });
        if let Some(i) = cube {
            // primeTime(false): the fuse is lit only while TNT explodes; the
            // ignition counts either way.
            if self.ui.rules.bool("tnt_explodes") && sulfur_cube::prime(&mut self.mobs[i], false) {
                let m = &mut self.mobs[i];
                let (at, id) = (m.position, m.id);
                let mut events = Vec::new();
                m.voice("entity.tnt.primed", &mut events);
                self.handle_mob_events(events)?;
                let ctx = self.who_context(Some(horizon_client::mobs::Who::Mob(id)), None);
                self.post_game_event("minecraft:prime_fuse", [at.x, at.y, at.z], ctx);
            }
            return Ok((self.hurt_and_break(stack, 1), CLICK));
        }
        if self
            .with_level(|b, level, ticks| b.fire_can_be_placed_at(level, ticks, target_pos, facing))
        {
            let fire =
                self.with_level(|b, level, ticks| b.fire_state_for(level, ticks, target_pos));
            self.level_set([target.0, target.1, target.2], fire)?;
            self.post_block_game_event(
                "minecraft:block_place",
                [target.0, target.1, target.2],
                None,
                None,
            );
        } else if self.can_light(state, &name) {
            if let Some(lit) = self.with_props(state, &[("lit", "true")]) {
                self.level_set([target.0, target.1, target.2], lit)?;
            }
            self.post_block_game_event(
                "minecraft:block_change",
                [target.0, target.1, target.2],
                None,
                None,
            );
        } else if &*name == "minecraft:tnt" {
            if self.ui.rules.bool("tnt_explodes") {
                self.spawn_primed_tnt(target_pos, false);
                self.with_level(|b, level, ticks| b.remove_block(level, ticks, target_pos, false));
                self.process_level_events()?;
            } else {
                success = false;
            }
        }
        let event = if success { CLICK } else { FAIL };
        if success {
            return Ok((self.hurt_and_break(stack, 1), event));
        }
        Ok((Some(stack), event))
    }
    /// `CampfireBlock.canLight || CandleBlock.canLight || CandleCakeBlock.canLight`.
    pub(super) fn can_light(&self, state: BlockStateId, name: &str) -> bool {
        let lit = self.prop(state, "lit");
        let waterlogged = self.prop(state, "waterlogged");
        let dry_unlit = lit.as_deref() == Some("false") && waterlogged.as_deref() == Some("false");
        ((self.in_block_tag("minecraft:campfires", name)
            || self.in_block_tag("minecraft:candles", name))
            && dry_unlit)
            || (self.in_block_tag("minecraft:candle_cakes", name)
                && lit.as_deref() == Some("false"))
    }
    fn dispense_bone_meal(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = offset(pos, facing);
        let target_pos = Self::pos_of(target);
        let grown = self.with_level(|b, level, ticks| {
            b.grow_crop(level, ticks, target_pos)
                || b.grow_water_plant(level, ticks, target_pos, None)
        });
        if grown {
            self.level_events
                .push(horizon_world::level::LevelEvent::LevelEvent {
                    id: 1505,
                    pos: target_pos,
                    data: 15,
                });
        }
        self.process_level_events()?;
        if grown {
            Ok((shrink(stack), CLICK))
        } else {
            Ok((Some(stack), FAIL))
        }
    }
    fn dispense_tnt(&mut self, pos: Pos, facing: Direction, stack: ItemStack) -> Dispensed {
        if !self.ui.rules.bool("tnt_explodes") {
            set_success("tnt", false);
            return (Some(stack), FAIL);
        }
        let front = offset(pos, facing);
        if self.sulfur_cube_swallow(front, &stack) {
            return (shrink(stack), optional_event("tnt"));
        }
        set_success("tnt", true);
        // new PrimedTnt(level, x + 0.5, y, z + 0.5, null), its sound and ENTITY_PLACE.
        let target = Self::pos_of(front);
        let rot = self.world.random.next_double() * f64::from(std::f32::consts::TAU);
        self.add_primed_tnt(target, rot, 80);
        self.audio.play(
            "minecraft:entity.tnt.primed",
            Some([
                f64::from(target.x) + 0.5,
                f64::from(target.y),
                f64::from(target.z) + 0.5,
            ]),
            1.0,
            1.0,
        );
        self.post_block_game_event(
            "minecraft:entity_place",
            [front.0, front.1, front.2],
            None,
            None,
        );
        (shrink(stack), CLICK)
    }

    // ---- blocks placed by dispensers ----

    fn dispense_wither_skull(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = offset(pos, facing);
        let free = self.res.facts().is_air(self.block_at(target))
            && target.1 >= horizon_client::chunk::MIN_Y + 2
            && self.ui.difficulty != 0
            && self.can_build_wither(target);
        if free {
            let rotation = match facing {
                Direction::Up | Direction::Down => 0,
                other => {
                    // RotationSegment.convertToSegment: the 2D data value * 4.
                    (match other {
                        Direction::South => 0,
                        Direction::West => 1,
                        Direction::North => 2,
                        _ => 3,
                    }) * 4
                }
            };
            let skull = self
                .pack
                .assets
                .registry
                .default_state("minecraft:wither_skeleton_skull")
                .and_then(|s| self.with_props(s, &[("rotation", &rotation.to_string())]));
            if let Some(skull) = skull {
                self.level_set([target.0, target.1, target.2], skull)?;
                if let Some(entity) = BlockEntity::for_block("minecraft:wither_skeleton_skull") {
                    self.world.block_entities.insert(target, entity);
                }
                self.post_block_game_event(
                    "minecraft:block_place",
                    [target.0, target.1, target.2],
                    None,
                    None,
                );
                self.check_wither_spawn([target.0, target.1, target.2])?;
                return Ok((shrink(stack), CLICK));
            }
        }
        let mut stack = stack;
        let equipped = self.dispense_equipment(pos, facing, &mut stack);
        Ok((
            Some(stack).filter(|s| s.count > 0),
            if equipped { CLICK } else { FAIL },
        ))
    }
    /// `WitherSkullBlock.getOrCreateWitherBase().find(level, pos)`: the soul
    /// sand T below the row `head` is in.
    fn can_build_wither(&self, head: Pos) -> bool {
        let soul = |p: Pos| {
            matches!(
                &*self.dispenser_block_name(p),
                "minecraft:soul_sand" | "minecraft:soul_soil"
            )
        };
        let air = |p: Pos| self.res.facts().is_air(self.block_at(p));
        for (ax, az) in [(1, 0), (0, 1)] {
            for col in 0..3 {
                let left = (head.0 - ax * col, head.1, head.2 - az * col);
                let at = |c: i32, row: i32| (left.0 + ax * c, left.1 - row, left.2 + az * c);
                if (0..3).all(|c| soul(at(c, 1)))
                    && soul(at(1, 2))
                    && air(at(0, 2))
                    && air(at(2, 2))
                {
                    return true;
                }
            }
        }
        false
    }
    fn dispense_carved_pumpkin(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = offset(pos, facing);
        if self.res.facts().is_air(self.block_at(target))
            && self.can_spawn_golem([target.0, target.1, target.2])
        {
            if let Some(pumpkin) = self
                .pack
                .assets
                .registry
                .default_state("minecraft:carved_pumpkin")
            {
                let head = [target.0, target.1, target.2];
                self.set_golem_head(head, pumpkin)?;
                self.post_block_game_event("minecraft:block_place", head, None, None);
            }
            return Ok((shrink(stack), CLICK));
        }
        let mut stack = stack;
        let equipped = self.dispense_equipment(pos, facing, &mut stack);
        Ok((
            Some(stack).filter(|s| s.count > 0),
            if equipped { CLICK } else { FAIL },
        ))
    }
    /// `ShulkerBoxDispenseBehavior`: place it like a `DirectionalPlaceContext`.
    fn dispense_shulker_box(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let front = offset(pos, facing);
        let below_empty = self
            .res
            .facts()
            .is_air(self.block_at((front.0, front.1 - 1, front.2)));
        let clicked_face = if below_empty { facing } else { Direction::Up };
        let ctx = PlaceContext {
            clicked_pos: Self::pos_of(front),
            face: clicked_face,
            hit: Vec3::new(
                f64::from(front.0) + 0.5,
                f64::from(front.1),
                f64::from(front.2) + 0.5,
            ),
            inside: false,
            yaw: y_rot(facing),
            pitch: 0.0,
            sneaking: false,
            item: stack.item.to_string(),
            can_use_game_master_blocks: false,
        };
        // DirectionalPlaceContext places at the front block or nowhere.
        if !self.with_level(|b, level, ticks| b.directional_can_place(level, ticks, &ctx)) {
            return Ok((Some(stack), FAIL));
        }
        let outcome = self.with_level(|b, level, ticks| b.place_block(level, ticks, &ctx));
        let PlaceOutcome::Placed { pos: placed, state } = outcome else {
            self.process_level_events()?;
            return Ok((Some(stack), FAIL));
        };
        let target = (placed.x, placed.y, placed.z);
        let state = self.apply_item_block_state([target.0, target.1, target.2], state, &stack)?;
        if let Some(entity) = BlockEntity::for_block(&self.res.facts().get(state).name) {
            self.world.block_entities.insert(target, entity);
            self.apply_placed_item([target.0, target.1, target.2], &stack, false);
        }
        self.process_level_events()?;
        Ok((shrink(stack), CLICK))
    }

    // ---- bottles, glowstone, shears, brush, honeycomb ----

    fn dispense_glass_bottle(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = offset(pos, facing);
        let state = self.block_at(target);
        let name = self.dispenser_block_name(target);
        let honey = self
            .prop(state, "honey_level")
            .and_then(|v| v.parse::<i32>().ok());
        if self.in_block_tag("minecraft:beehives", &name)
            && let Some(level) = honey
            && level >= 5
        {
            // releaseBeesAndResetHoneyLevel.
            if let Some(reset) = self.with_props(state, &[("honey_level", "0")]) {
                self.level_set([target.0, target.1, target.2], reset)?;
            }
            self.dispenser_empty_hive(target, state);
            let bottle = ItemStack::new("minecraft:honey_bottle", 1);
            self.post_block_game_event("minecraft:fluid_pickup", [pos.0, pos.1, pos.2], None, None);
            return Ok((
                self.consume_with_remainder(pos, facing, stack, bottle),
                CLICK,
            ));
        }
        if self.water_at(target) {
            let water = ItemStack::new("minecraft:potion", 1).with_potion("minecraft:water");
            self.post_block_game_event("minecraft:fluid_pickup", [pos.0, pos.1, pos.2], None, None);
            return Ok((
                self.consume_with_remainder(pos, facing, stack, water),
                CLICK,
            ));
        }
        Ok((self.default_execute(pos, facing, stack), FAIL))
    }
    fn dispense_glowstone(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = offset(pos, facing);
        let state = self.block_at(target);
        if &*self.dispenser_block_name(target) == "minecraft:respawn_anchor" {
            let charge = self
                .prop(state, "charges")
                .and_then(|v| v.parse::<i32>().ok())
                .unwrap_or(0);
            if charge != 4 {
                if let Some(next) =
                    self.with_props(state, &[("charges", &(charge + 1).to_string())])
                {
                    self.level_set([target.0, target.1, target.2], next)?;
                    // RespawnAnchorBlock.charge: Context.of(null, newState).
                    self.post_block_game_event(
                        "minecraft:block_change",
                        [target.0, target.1, target.2],
                        None,
                        Some(next),
                    );
                }
                self.audio.play(
                    "minecraft:block.respawn_anchor.charge",
                    Some(Self::center(target)),
                    1.0,
                    1.0,
                );
                return Ok((shrink(stack), CLICK));
            }
            return Ok((Some(stack), FAIL));
        }
        Ok((self.default_execute(pos, facing, stack), CLICK))
    }
    fn dispense_shears(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = offset(pos, facing);
        let sheared = self.shear_beehive(target)? || self.shear_entity(target)?;
        if sheared {
            return Ok((self.hurt_and_break(stack, 1), CLICK));
        }
        Ok((Some(stack), FAIL))
    }
    /// The bees of a hive a dispenser harvested come out (`BEE_RELEASED`:
    /// not at night nor in the rain), through the front of its old state.
    fn dispenser_empty_hive(&mut self, target: Pos, old: BlockStateId) {
        let facing = self.prop(old, "facing");
        self.empty_hive(
            [target.0, target.1, target.2],
            horizon_client::beehive::facing_step(facing.as_deref()),
            horizon_client::beehive::Release::BeeReleased,
        );
    }
    fn shear_beehive(&mut self, target: Pos) -> Result<bool> {
        let state = self.block_at(target);
        let name = self.dispenser_block_name(target);
        let level = self
            .prop(state, "honey_level")
            .and_then(|v| v.parse::<i32>().ok());
        if !self.in_block_tag("minecraft:beehives", &name) || level.is_none_or(|l| l < 5) {
            return Ok(false);
        }
        self.audio.play(
            "minecraft:block.beehive.shear",
            Some(Self::center(target)),
            1.0,
            1.0,
        );
        // BeehiveBlock.dropHoneycomb (harvest/beehive), then
        // releaseBeesAndResetHoneyLevel and the shear event.
        let p = [target.0, target.1, target.2];
        for stack in self.sequence_loot(
            "minecraft:harvest/beehive",
            "minecraft:harvest/beehive",
            None,
        ) {
            self.pop_resource(p, stack);
        }
        if let Some(reset) = self.with_props(state, &[("honey_level", "0")]) {
            self.level_set(p, reset)?;
        }
        self.dispenser_empty_hive(target, state);
        self.post_block_game_event("minecraft:shear", p, None, None);
        Ok(true)
    }
    /// `LootTable.getRandomItems` of `table` in the context of `parent`: the
    /// draws come from `parent`'s server random sequence when it names one (a
    /// nested `loot_table` entry shares its parent's context), else the level's.
    pub(super) fn sequence_loot(
        &mut self,
        parent: &str,
        table: &str,
        held: Option<&ItemStack>,
    ) -> Vec<ItemStack> {
        match self.res.items().loot_sequence(parent) {
            Some(sequence) => {
                let random = self.rand_sequences.get(&sequence, self.level.seed);
                self.res.items().loot_table_from(table, false, held, random)
            }
            None => self
                .res
                .items()
                .loot_table(table, false, held, &mut self.world.random),
        }
    }
    /// `Entity.spawnAtLocation(level, stack, offset)`: an item at the entity's
    /// feet plus the offset, moving as `new ItemEntity` makes it.
    pub(super) fn spawn_at_location(
        &mut self,
        at: Vec3,
        y_offset: f64,
        stack: ItemStack,
    ) -> [f64; 3] {
        let v = new_item_motion();
        self.spawn_item(stack, [at.x, at.y + y_offset, at.z], v, 10);
        v
    }
    /// `Mob.convertTo(type, ConversionParams.single(this, false, false))`: a
    /// new mob of `kind` in the same place (no spawn finalization, so no
    /// random draws) keeps the name, flags, persistence and age; the old one
    /// is discarded.
    pub(super) fn convert_mob(&mut self, index: usize, kind: &str) -> Option<u32> {
        let t = self.res.mob_types().get(kind).cloned()?;
        let old = &self.mobs[index];
        self.next_entity += 1;
        let mut mob = Mob::new(self.next_entity, t, old.position, old.yaw);
        mob.previous = old.previous;
        mob.head_yaw = old.head_yaw;
        mob.body_yaw = old.body_yaw;
        mob.head_pitch = old.head_pitch;
        mob.persistent = old.persistent;
        mob.custom_name = old.custom_name.clone();
        mob.custom_name_text = old.custom_name_text.clone();
        mob.name_visible = old.name_visible;
        mob.invisible = old.invisible;
        mob.no_ai = old.no_ai;
        mob.silent = old.silent;
        mob.no_gravity = old.no_gravity;
        mob.invulnerable_flag = old.invulnerable_flag;
        mob.extra.age = if old.is_baby() { old.extra.age } else { 0 };
        for slot in [
            &mut mob.extra.main_hand,
            &mut mob.extra.off_hand,
            &mut mob.extra.head,
            &mut mob.extra.chest,
            &mut mob.extra.legs,
            &mut mob.extra.feet,
        ] {
            *slot = None;
        }
        let id = mob.id;
        self.mobs[index].removed = true;
        self.mobs.push(mob);
        Some(id)
    }
    /// `ShearsDispenseItemBehavior.tryShearEntity`: the first live, ready
    /// `Shearable` in the block is sheared.
    fn shear_entity(&mut self, target: Pos) -> Result<bool> {
        let shears = ItemStack::new("minecraft:shears", 1);
        for t in self.targets_at(target) {
            // Entity.shearOffAllLeashConnections comes first, for any entity.
            if self.shear_off_leashes(t) {
                return Ok(true);
            }
            let Target::Mob(i) = t else { continue };
            let m = &self.mobs[i];
            if !m.alive() {
                continue;
            }
            let at = m.position;
            let sound_at = Some([at.x, at.y, at.z]);
            match m.species() {
                Species::SulfurCube if sulfur_cube::has_body(m) => {
                    // SulfurCube.shear: the swallowed block comes out at the
                    // passenger attachment, and nothing is swallowed for 5 s.
                    let height = f64::from(m.height());
                    let id = m.id;
                    if let Some(body) = self.mobs[i].extra.head.take() {
                        self.spawn_at_location(at, height, ItemStack::new(&body, 1));
                    }
                    // setItemSlot(BODY, EMPTY): an unequip.
                    let ctx = self.who_context(Some(horizon_client::mobs::Who::Mob(id)), None);
                    self.post_game_event("minecraft:unequip", [at.x, at.y, at.z], ctx);
                    self.mobs[i].extra.ints[3] = sulfur_cube::PICKUP_TIMER;
                    self.audio
                        .play("minecraft:entity.sulfur_cube.eject", sound_at, 1.0, 1.0);
                }
                Species::Sheep if !m.extra.sheared && !m.is_baby() => {
                    let color =
                        horizon_client::mobs::render::DYE_NAMES[m.extra.color as usize & 15];
                    self.audio
                        .play("minecraft:entity.sheep.shear", sound_at, 1.0, 1.0);
                    let drops = self.sequence_loot(
                        "minecraft:shearing/sheep",
                        &format!("minecraft:shearing/sheep/{color}"),
                        Some(&shears),
                    );
                    for stack in drops {
                        for _ in 0..stack.count {
                            let one = ItemStack {
                                count: 1,
                                ..stack.clone()
                            };
                            self.spawn_at_location(at, 1.0, one);
                            // Sheep.shear nudges each item with the sheep's own random.
                            let r = &mut self.mobs[i].random;
                            let push = [
                                f64::from((r.next_float() - r.next_float()) * 0.1),
                                f64::from(r.next_float() * 0.05),
                                f64::from((r.next_float() - r.next_float()) * 0.1),
                            ];
                            if let Some(item) = self.dropped.last_mut() {
                                item.velocity = Vec3::new(
                                    item.velocity.x + push[0],
                                    item.velocity.y + push[1],
                                    item.velocity.z + push[2],
                                );
                            }
                        }
                    }
                    self.mobs[i].extra.sheared = true;
                }
                Species::Bogged if !m.extra.sheared => {
                    let height = f64::from(m.height());
                    self.audio
                        .play("minecraft:entity.bogged.shear", sound_at, 1.0, 1.0);
                    let drops = self.sequence_loot(
                        "minecraft:shearing/bogged",
                        "minecraft:shearing/bogged",
                        Some(&shears),
                    );
                    for stack in drops {
                        self.spawn_at_location(at, height, stack);
                    }
                    self.mobs[i].extra.sheared = true;
                }
                Species::SnowGolem if !m.extra.sheared => {
                    let eye = m.eye_height();
                    self.audio
                        .play("minecraft:entity.snow_golem.shear", sound_at, 1.0, 1.0);
                    self.mobs[i].extra.sheared = true;
                    let drops = self.sequence_loot(
                        "minecraft:shearing/snow_golem",
                        "minecraft:shearing/snow_golem",
                        Some(&shears),
                    );
                    for stack in drops {
                        self.spawn_at_location(at, eye, stack);
                    }
                }
                // CopperGolem.shear: the antenna's flower drops 1.5 up.
                Species::CopperGolem if self.golem_shearable(i) => {
                    self.audio
                        .play("minecraft:entity.copper_golem.shear", sound_at, 1.0, 1.0);
                    let antenna = self.mobs[i].saddle_item();
                    let id = self.mobs[i].id;
                    // setItemSlot(EQUIPMENT_SLOT_ANTENNA, EMPTY): an unequip.
                    self.mobs[i].set_saddle(None);
                    let ctx = self.who_context(Some(horizon_client::mobs::Who::Mob(id)), None);
                    self.post_game_event("minecraft:unequip", [at.x, at.y, at.z], ctx);
                    if let Some(antenna) = antenna {
                        self.spawn_at_location(at, 1.5, antenna);
                    }
                }
                Species::Mooshroom if !m.is_baby() => {
                    let variant = if m.extra.variant == 1 { "brown" } else { "red" };
                    let height = f64::from(m.height());
                    self.audio
                        .play("minecraft:entity.mooshroom.shear", sound_at, 1.0, 1.0);
                    // convertTo(COW): the cow takes its place, then the mushrooms drop.
                    self.convert_mob(i, "minecraft:cow");
                    let drops = self.sequence_loot(
                        "minecraft:shearing/mooshroom",
                        &format!("minecraft:shearing/mooshroom/{variant}"),
                        Some(&shears),
                    );
                    for stack in drops {
                        for _ in 0..stack.count {
                            let one = ItemStack {
                                count: 1,
                                ..stack.clone()
                            };
                            self.spawn_at_location(at, height, one);
                        }
                    }
                }
                _ => continue,
            }
            self.post_block_game_event(
                "minecraft:shear",
                [target.0, target.1, target.2],
                None,
                None,
            );
            return Ok(true);
        }
        Ok(false)
    }
    /// `Entity.shearOffAllLeashConnections(null)`: the entity's own lead and
    /// every lead held by it (within a 32 block box around its centre) drop;
    /// a shear game event at the entity and a snip at its block follow.
    fn shear_off_leashes(&mut self, t: Target) -> bool {
        use horizon_client::mobs::{Who, leash::Holder};
        let (holder, centre, own) = match t {
            Target::Mob(i) => {
                let m = &self.mobs[i];
                let b = m.bounds();
                (Holder::Mob(m.id), b.center(), Some(i))
            }
            Target::Player => {
                let Some(v) = self.player_view() else {
                    return false;
                };
                (Holder::Player, v.bounds.center(), None)
            }
            Target::Stand(_) => return false,
        };
        let area = Aabb::new(centre, centre).inflate(Vec3::new(16.0, 16.0, 16.0));
        let mut leashed: Vec<(u32, usize)> = self
            .mobs
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                !m.removed && m.extra.leash == Some(holder) && overlaps(m.bounds(), area)
            })
            .map(|(j, m)| (m.id, j))
            .collect();
        leashed.sort_unstable();
        let own_leashed = own.is_some_and(|i| self.mobs[i].extra.leash.is_some());
        if leashed.is_empty() && !own_leashed {
            return false;
        }
        // Leashable.dropLeash: a lead item at the leashed entity's feet.
        let drop = |game: &mut Self, j: usize| {
            if horizon_client::mobs::leash::remove(&mut game.mobs[j]).is_some() {
                let at = game.mobs[j].position;
                game.spawn_at_location(at, 0.0, ItemStack::new("minecraft:lead", 1));
            }
        };
        if let Some(i) = own.filter(|_| own_leashed) {
            drop(self, i);
        }
        for (_, j) in leashed {
            drop(self, j);
        }
        let (at, who) = match t {
            Target::Mob(i) => {
                let m = &self.mobs[i];
                (m.position, Who::Mob(m.id))
            }
            _ => (
                self.player.as_ref().map_or(Vec3::ZERO, |p| p.position),
                Who::Player,
            ),
        };
        let ctx = self.who_context(Some(who), None);
        self.post_game_event("minecraft:shear", [at.x, at.y, at.z], ctx);
        let block = [at.x.floor() + 0.5, at.y.floor() + 0.5, at.z.floor() + 0.5];
        self.audio
            .play("minecraft:item.shears.snip", Some(block), 1.0, 1.0);
        true
    }
    /// The brush behaviour: the first armadillo in front that `brushOffScute`s.
    fn dispense_brush(&mut self, pos: Pos, facing: Direction, stack: ItemStack) -> Dispensed {
        let target = offset(pos, facing);
        let armadillos: Vec<usize> = self
            .targets_at(target)
            .into_iter()
            .filter_map(|t| match t {
                Target::Mob(i) if self.mobs[i].species() == Species::Armadillo => Some(i),
                _ => None,
            })
            .collect();
        for i in armadillos {
            if self.mobs[i].is_baby() {
                continue;
            }
            let at = self.mobs[i].position;
            let drops = self.sequence_loot(
                "minecraft:brush/armadillo",
                "minecraft:brush/armadillo",
                Some(&stack),
            );
            for d in drops {
                self.spawn_at_location(at, 0.0, d);
            }
            self.audio.play(
                "minecraft:entity.armadillo.brush",
                Some([at.x, at.y, at.z]),
                1.0,
                1.0,
            );
            let who = horizon_client::mobs::Who::Mob(self.mobs[i].id);
            let ctx = self.who_context(Some(who), None);
            self.post_game_event("minecraft:entity_interact", [at.x, at.y, at.z], ctx);
            return (self.hurt_and_break(stack, 16), optional_event("brush"));
        }
        set_success("brush", false);
        (Some(stack), FAIL)
    }
    /// `SulfurCubeBlockDispenseItemBehavior.dispenseBlock`: the first sulfur
    /// cube in the block that `equipItem`s the stack swallows one.
    fn sulfur_cube_swallow(&mut self, target: Pos, stack: &ItemStack) -> bool {
        let cubes: Vec<usize> = self
            .targets_at(target)
            .into_iter()
            .filter_map(|t| match t {
                Target::Mob(i) if self.mobs[i].species() == Species::SulfurCube => Some(i),
                _ => None,
            })
            .collect();
        for i in cubes {
            let m = &self.mobs[i];
            if m.is_baby() || m.extra.head.as_deref() == Some(&*stack.item) {
                continue;
            }
            let (at, height) = (m.position, f64::from(m.height()));
            // The swallowed block comes out on top (the passenger attachment).
            if let Some(old) = self.mobs[i].extra.head.take() {
                self.spawn_at_location(at, height, ItemStack::new(&old, 1));
            }
            let m = &mut self.mobs[i];
            m.extra.head = Some(stack.item.clone());
            m.persistent = true;
            let (at, id) = (m.position, m.id);
            // setItemSlot: a body item without `equippable` posts UNEQUIP.
            let ctx = self.who_context(Some(horizon_client::mobs::Who::Mob(id)), None);
            self.post_game_event("minecraft:unequip", [at.x, at.y, at.z], ctx);
            self.audio.play(
                "minecraft:entity.sulfur_cube.absorb",
                Some([at.x, at.y, at.z]),
                1.0,
                1.0,
            );
            return true;
        }
        false
    }
    fn dispense_honeycomb(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = offset(pos, facing);
        let state = self.block_at(target);
        let name = self.dispenser_block_name(target);
        // HoneycombItem.WAXABLES: each copper block has a waxed twin.
        let waxed = name.strip_prefix("minecraft:").and_then(|n| {
            if n.starts_with("waxed_") {
                return None;
            }
            let twin = format!("minecraft:waxed_{n}");
            let mut s = self.pack.assets.registry.state(state)?.clone();
            self.pack.assets.registry.default_state(&twin)?;
            s.name = twin;
            self.pack.assets.registry.resolve(&s).ok()
        });
        let Some(waxed) =
            waxed.filter(|_| name.contains("copper") || name.contains("lightning_rod"))
        else {
            return Ok((
                self.default_execute(pos, facing, stack),
                optional_event("honeycomb"),
            ));
        };
        set_success("honeycomb", true);
        self.level_set([target.0, target.1, target.2], waxed)?;
        self.level_events
            .push(horizon_world::level::LevelEvent::LevelEvent {
                id: 3003,
                pos: Self::pos_of(target),
                data: 0,
            });
        self.process_level_events()?;
        self.audio.play(
            "minecraft:item.honeycomb.wax_on",
            Some(Self::center(target)),
            1.0,
            1.0,
        );
        Ok((shrink(stack), CLICK))
    }
    fn dispense_potion(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = offset(pos, facing);
        let water = stack.potion() == Some("minecraft:water");
        let name = self.dispenser_block_name(target);
        if !water || !self.in_block_tag("minecraft:convertible_to_mud", &name) {
            // The inner default behaviour dispenses, then the potion's own
            // events play on top.
            return Ok((self.dispense_default(pos, facing, stack), CLICK));
        }
        // The server picks the splash positions with the level random.
        for _ in 0..5 {
            let r = &mut self.world.random;
            let p = [
                f64::from(pos.0) + r.next_double(),
                f64::from(pos.1 + 1),
                f64::from(pos.2) + r.next_double(),
            ];
            self.particles
                .add(&ParticleOptions::simple(ParticleType::Splash), p, [0.0; 3]);
        }
        self.audio.play(
            "minecraft:item.bottle.empty",
            Some(Self::center(pos)),
            1.0,
            1.0,
        );
        self.post_block_game_event("minecraft:fluid_place", [pos.0, pos.1, pos.2], None, None);
        if let Some(mud) = self.pack.assets.registry.default_state("minecraft:mud") {
            self.level_set([target.0, target.1, target.2], mud)?;
        }
        let bottle = ItemStack::new("minecraft:glass_bottle", 1);
        Ok((
            self.consume_with_remainder(pos, facing, stack, bottle),
            CLICK,
        ))
    }
}
/// A tipped arrow's lasting effect at an eighth of the potion's duration
/// (`Arrow.doPostHurtEffects`).
pub(super) fn tipped_effect(stack: &ItemStack) -> Option<(&'static str, i32)> {
    stack
        .potion()
        .and_then(horizon_client::brewing::potion)
        .and_then(|def| {
            def.effects
                .iter()
                .find(|(id, _, _)| !horizon_client::brewing::effect(id).is_some_and(|e| e.instant))
                .map(|(id, duration, _)| {
                    (
                        horizon_client::brewing::namespaced(id),
                        (duration / 8).max(1),
                    )
                })
        })
}
