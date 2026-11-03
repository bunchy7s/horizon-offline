//! Fluids held by item stacks for native mods (docs/native-mods.md): vanilla's
//! buckets, and a mod's own containers (`native_item_fluid_tank`), read,
//! filled and drained in millibuckets. NeoForge's
//! `Capabilities.FluidHandler.ITEM` acts on one item of a stack at a time and
//! may swap the item for another; so do these. They take a stack and answer
//! without changing it: the caller shrinks its stack by one and hands out the
//! changed container they return. They hold no world state, so they work on
//! any side.
use super::Game;
use horizon_client::items::ItemStack;
use horizon_world::nbt::{Compound, JavaString, Tag};

/// What one item of a stack holds (`native_item_fluid`).
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NativeItemFluid {
    /// `minecraft:water` or `minecraft:lava`; none when it is empty.
    pub(crate) fluid: Option<String>,
    /// In millibuckets: a bucket is a thousand.
    pub(crate) amount: i32,
    pub(crate) capacity: i32,
}

const CUSTOM_DATA: &str = "minecraft:custom_data";
/// The key under `minecraft:custom_data` that keeps a mod container's
/// contents: `{Fluid: "minecraft:water", Amount: 250}`.
const TANK: &str = "horizon_fluid";
/// A bucket's capacity, vanilla's `FluidType.BUCKET_VOLUME`.
const BUCKET: i32 = 1000;
const MOD_CAPACITY: std::ops::RangeInclusive<i32> = 1..=1_000_000;

/// The bucket items: (empty, full), by the fluid they hold. A bucket is all
/// or nothing, as `BucketItem` is. Milk, powder snow and fish buckets hold
/// no fluid Horizon models.
const BUCKETS: [(&str, &str); 2] = [
    ("minecraft:water", "minecraft:water_bucket"),
    ("minecraft:lava", "minecraft:lava_bucket"),
];

fn is_fluid(name: &str) -> bool {
    BUCKETS.iter().any(|(fluid, _)| *fluid == name)
}

/// The contents a mod container's stack keeps in its `custom_data`: none and
/// 0 when empty, or when what is there isn't a tank.
fn read_tank(stack: &ItemStack) -> (Option<String>, i32) {
    let Some(Tag::Compound(data)) = stack.data().and_then(|d| d.other(CUSTOM_DATA)) else {
        return (None, 0);
    };
    let Some(Tag::Compound(tank)) = data.get(&JavaString::from(TANK)) else {
        return (None, 0);
    };
    let amount = match tank.get(&JavaString::from("Amount")) {
        Some(Tag::Int(amount)) => (*amount).max(0),
        _ => 0,
    };
    match tank.get(&JavaString::from("Fluid")) {
        Some(Tag::String(fluid)) if amount > 0 => (Some(fluid.to_string_lossy()), amount),
        _ => (None, 0),
    }
}

/// One item of `stack` holding `amount` of `fluid`, its other data kept. An
/// empty tank carries nothing, so empty containers stack.
fn with_tank(stack: &ItemStack, fluid: &str, amount: i32) -> ItemStack {
    let mut one = stack.clone();
    one.count = 1;
    let mut data = match one.data().and_then(|d| d.other(CUSTOM_DATA)) {
        Some(Tag::Compound(data)) => data.clone(),
        _ => Compound::new(),
    };
    if amount > 0 {
        data.insert(
            JavaString::from(TANK),
            Tag::compound([
                ("Fluid", Tag::String(JavaString::from(fluid))),
                ("Amount", Tag::Int(amount)),
            ]),
        );
    } else {
        data.remove(&JavaString::from(TANK));
    }
    let value = (!data.is_empty()).then_some(Tag::Compound(data));
    one.data_mut().set_other(CUSTOM_DATA, value);
    // What a server sent for the stack no longer describes it; kept, an
    // emptied container would not stack with a plain one.
    one.data_mut().wire = Default::default();
    one.tidy();
    one
}

/// `stack` as a fresh single `item`, as `BucketItem` makes its result.
fn bucket(item: &str) -> ItemStack {
    ItemStack::new(item, 1)
}

/// What one item of `stack` holds, if it is a container: a bucket, or an item
/// whose mod gave it `capacity`.
fn contents(stack: &ItemStack, capacity: Option<i32>) -> Option<NativeItemFluid> {
    if stack.count == 0 {
        return None;
    }
    if &*stack.item == "minecraft:bucket" {
        return Some(NativeItemFluid {
            fluid: None,
            amount: 0,
            capacity: BUCKET,
        });
    }
    if let Some((fluid, _)) = BUCKETS.iter().find(|(_, full)| *full == &*stack.item) {
        return Some(NativeItemFluid {
            fluid: Some((*fluid).to_owned()),
            amount: BUCKET,
            capacity: BUCKET,
        });
    }
    let capacity = capacity?;
    let (fluid, amount) = read_tank(stack);
    Some(NativeItemFluid {
        fluid,
        amount,
        capacity,
    })
}

/// How much of `amount` of `fluid` one item of `stack` takes, and the item
/// it becomes.
fn fill_one(
    stack: &ItemStack,
    capacity: Option<i32>,
    fluid: &str,
    amount: i32,
) -> Option<(i32, ItemStack)> {
    let held = contents(stack, capacity)?;
    let mut unchanged = stack.clone();
    unchanged.count = 1;
    if !is_fluid(fluid) || amount <= 0 {
        return Some((0, unchanged));
    }
    if &*stack.item == "minecraft:bucket" {
        // All or nothing: a bucket takes a whole bucket.
        let full = BUCKETS.iter().find(|(f, _)| *f == fluid)?.1;
        return Some(if amount >= BUCKET {
            (BUCKET, bucket(full))
        } else {
            (0, unchanged)
        });
    }
    if capacity.is_none() || held.fluid.as_deref().is_some_and(|f| f != fluid) {
        return Some((0, unchanged));
    }
    let take = (held.capacity - held.amount).max(0).min(amount);
    if take == 0 {
        return Some((0, unchanged));
    }
    Some((take, with_tank(stack, fluid, held.amount + take)))
}

/// The fluid and how much one item of `stack` gives of at most `max`, and
/// the item it becomes.
fn drain_one(
    stack: &ItemStack,
    capacity: Option<i32>,
    max: i32,
) -> Option<(String, i32, ItemStack)> {
    let held = contents(stack, capacity)?;
    let fluid = held.fluid?;
    if max <= 0 {
        return None;
    }
    if capacity.is_none() || BUCKETS.iter().any(|(_, full)| *full == &*stack.item) {
        // A full bucket gives its whole bucket or nothing.
        return (max >= BUCKET).then(|| (fluid, BUCKET, bucket("minecraft:bucket")));
    }
    let give = held.amount.min(max);
    Some((
        fluid.clone(),
        give,
        with_tank(stack, &fluid, held.amount - give),
    ))
}

// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// The capacity a mod gave item `item`, clamped; none for any other item.
    fn native_item_tank_capacity(&self, item: &str) -> Option<i32> {
        if item.starts_with("minecraft:") {
            return None;
        }
        let mut capacity = None;
        self.native_item_fluid_tank(item, &mut capacity);
        capacity.map(|c| c.clamp(*MOD_CAPACITY.start(), *MOD_CAPACITY.end()))
    }
    /// `Capabilities.FluidHandler.ITEM`, read: the fluid, amount and capacity
    /// of one item of `stack`. A bucket holds a bucket of water or lava or
    /// nothing; a mod's container what it keeps in its `custom_data`. None
    /// for any other item, and for an empty stack.
    pub(in crate::game) fn native_item_fluid(&self, stack: &ItemStack) -> Option<NativeItemFluid> {
        contents(stack, self.native_item_tank_capacity(&stack.item))
    }
    /// Fills one item of `stack` with up to `amount` millibuckets of `fluid`
    /// (`minecraft:water`, `minecraft:lava`), as `IFluidHandler.fill`: a
    /// bucket takes a whole bucket or nothing and becomes the full one; a
    /// mod's container takes what fits of one fluid. The amount it takes, and
    /// the single item it becomes (the item itself when it took nothing). The
    /// stack given is not changed: shrink it by one and hand out the result.
    /// None where the stack is no container.
    pub(in crate::game) fn native_item_fluid_fill(
        &self,
        stack: &ItemStack,
        fluid: &str,
        amount: i32,
    ) -> Option<(i32, ItemStack)> {
        fill_one(
            stack,
            self.native_item_tank_capacity(&stack.item),
            fluid,
            amount,
        )
    }
    /// Drains up to `max` millibuckets out of one item of `stack`, as
    /// `IFluidHandler.drain`: a full bucket gives its whole bucket or nothing
    /// and becomes an empty one; a mod's container gives what it holds, up to
    /// `max`. The fluid, the amount, and the single item it becomes. The stack
    /// given is not changed. None where it is no container, holds nothing, or
    /// `max` is less than a bucket's for a bucket.
    pub(in crate::game) fn native_item_fluid_drain(
        &self,
        stack: &ItemStack,
        max: i32,
    ) -> Option<(String, i32, ItemStack)> {
        drain_one(stack, self.native_item_tank_capacity(&stack.item), max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    const W: &str = "minecraft:water";
    const L: &str = "minecraft:lava";

    fn canister(count: u32) -> ItemStack {
        ItemStack::new("mymod:canister", count)
    }

    #[test]
    fn an_empty_bucket_takes_a_whole_bucket_or_nothing() {
        let empty = ItemStack::new("minecraft:bucket", 3);
        let (took, result) = fill_one(&empty, None, W, 1500).unwrap();
        assert_eq!(
            (took, &*result.item, result.count),
            (1000, "minecraft:water_bucket", 1)
        );
        let (took, result) = fill_one(&empty, None, L, 1000).unwrap();
        assert_eq!((took, &*result.item), (1000, "minecraft:lava_bucket"));
        let (took, result) = fill_one(&empty, None, W, 999).unwrap();
        assert_eq!(
            (took, &*result.item, result.count),
            (0, "minecraft:bucket", 1)
        );
        // Not a fluid Horizon models.
        assert_eq!(fill_one(&empty, None, "minecraft:milk", 1000).unwrap().0, 0);
    }

    #[test]
    fn a_full_bucket_gives_a_whole_bucket_and_takes_nothing() {
        let full = ItemStack::new("minecraft:lava_bucket", 1);
        assert_eq!(fill_one(&full, None, L, 1000).unwrap().0, 0);
        assert_eq!(drain_one(&full, None, 999), None);
        let (fluid, took, result) = drain_one(&full, None, 5000).unwrap();
        assert_eq!(
            (fluid.as_str(), took, &*result.item),
            (L, 1000, "minecraft:bucket")
        );
        let empty = ItemStack::new("minecraft:bucket", 1);
        assert_eq!(drain_one(&empty, None, 1000), None);
    }

    #[test]
    fn other_items_are_no_containers() {
        let stone = ItemStack::new("minecraft:stone", 1);
        assert_eq!(contents(&stone, None), None);
        assert_eq!(fill_one(&stone, None, W, 1000), None);
        assert_eq!(drain_one(&stone, None, 1000), None);
        assert_eq!(contents(&ItemStack::new("minecraft:bucket", 0), None), None);
        // A mod item nobody gave a capacity is none either.
        assert_eq!(contents(&canister(1), None), None);
    }

    #[test]
    fn a_mod_container_keeps_one_fluid_up_to_its_capacity() {
        let stack = canister(4);
        let (took, filled) = fill_one(&stack, Some(750), W, 500).unwrap();
        assert_eq!((took, filled.count), (500, 1));
        assert_eq!(read_tank(&filled), (Some(W.to_owned()), 500));
        // Only the room is taken, and only of the same fluid.
        let (took, full) = fill_one(&filled, Some(750), W, 500).unwrap();
        assert_eq!(took, 250);
        assert_eq!(read_tank(&full), (Some(W.to_owned()), 750));
        assert_eq!(fill_one(&full, Some(750), W, 1).unwrap().0, 0);
        assert_eq!(fill_one(&filled, Some(750), L, 100).unwrap().0, 0);
        assert_eq!(
            contents(&full, Some(750)),
            Some(NativeItemFluid {
                fluid: Some(W.to_owned()),
                amount: 750,
                capacity: 750
            })
        );
    }

    #[test]
    fn draining_a_mod_container_empties_it_back_to_a_plain_stack() {
        let (_, filled) = fill_one(&canister(1), Some(1000), L, 300).unwrap();
        let (fluid, took, rest) = drain_one(&filled, Some(1000), 100).unwrap();
        assert_eq!((fluid.as_str(), took), (L, 100));
        assert_eq!(read_tank(&rest), (Some(L.to_owned()), 200));
        let (_, took, empty) = drain_one(&rest, Some(1000), 5000).unwrap();
        assert_eq!(took, 200);
        // Nothing left over, so it stacks with a container never filled.
        assert_eq!(empty, canister(1));
        assert_eq!(drain_one(&empty, Some(1000), 100), None);
    }

    #[test]
    fn an_emptied_container_from_a_server_stacks_with_a_plain_one() {
        let mut sent = canister(2);
        sent.data_mut().wire =
            horizon_client::item_data::Wire::new(canister(2), Arc::from(vec![1u8]));
        let (_, filled) = fill_one(&sent, Some(100), W, 40).unwrap();
        let (_, _, emptied) = drain_one(&filled, Some(100), 40).unwrap();
        assert!(emptied.same_item_same_components(&canister(1)));
    }

    #[test]
    fn a_container_keeps_the_rest_of_its_custom_data() {
        let mut stack = canister(1);
        stack.data_mut().set_other(
            CUSTOM_DATA,
            Some(Tag::compound([(
                "owner",
                Tag::String(JavaString::from("Alex")),
            )])),
        );
        let (_, filled) = fill_one(&stack, Some(1000), W, 10).unwrap();
        let Some(Tag::Compound(data)) = filled.data().and_then(|d| d.other(CUSTOM_DATA)) else {
            panic!("no custom data");
        };
        assert!(data.contains_key(&JavaString::from("owner")));
        assert!(data.contains_key(&JavaString::from(TANK)));
        let (_, _, emptied) = drain_one(&filled, Some(1000), 10).unwrap();
        assert_eq!(emptied, stack);
    }
}
