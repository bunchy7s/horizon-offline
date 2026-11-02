//! Energy held by item stacks for native mods (docs/native-mods.md): a mod's
//! batteries and charged tools (`native_item_energy_buffer`), read, filled
//! and drained. Vanilla has no energy; the contract is NeoForge's
//! `Capabilities.Energy.ITEM`, an `EnergyHandler` reached through an
//! `ItemAccess`: it acts on one item of a stack at a time and may change it.
//! So do these, as the item fluid helpers (`native_item_fluids.rs`) do: they
//! take a stack and answer without changing it, and the caller shrinks its
//! stack by one and hands out the changed item they return. They hold no
//! world state, so they work on any side.
use super::Game;
use horizon_client::items::ItemStack;
use horizon_world::nbt::{Compound, JavaString, Tag};

/// The buffer a mod item declares (`native_item_energy_buffer`), as
/// NeoForge's `SimpleEnergyHandler` is built: its capacity and the most one
/// insert and one extract move.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemEnergyBuffer {
    pub capacity: i32,
    /// The most one insert moves; 0 takes none.
    pub max_insert: i32,
    /// The most one extract moves; 0 gives none.
    pub max_extract: i32,
}

/// What one item of a stack holds (`native_item_energy`).
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NativeItemEnergy {
    pub(crate) amount: i32,
    pub(crate) capacity: i32,
    /// The most one insert moves.
    pub(crate) max_insert: i32,
    /// The most one extract moves.
    pub(crate) max_extract: i32,
}

const CUSTOM_DATA: &str = "minecraft:custom_data";
/// The int under `minecraft:custom_data` that keeps the amount, as a mod
/// block's buffer keeps it under `energy`.
const ENERGY: &str = "horizon_energy";

/// The buffer as declared, or none when its capacity is under 1; the limits
/// are clamped to 0..=capacity, as a block's are.
fn checked(buffer: ItemEnergyBuffer) -> Option<ItemEnergyBuffer> {
    (buffer.capacity >= 1).then(|| ItemEnergyBuffer {
        capacity: buffer.capacity,
        max_insert: buffer.max_insert.clamp(0, buffer.capacity),
        max_extract: buffer.max_extract.clamp(0, buffer.capacity),
    })
}

/// The amount a stack keeps in its `custom_data`: anything but an int, or a
/// negative, reads as empty.
fn read_energy(stack: &ItemStack) -> i32 {
    let Some(Tag::Compound(data)) = stack.data().and_then(|d| d.other(CUSTOM_DATA)) else {
        return 0;
    };
    match data.get(&JavaString::from(ENERGY)) {
        Some(Tag::Int(energy)) => (*energy).max(0),
        _ => 0,
    }
}

/// One item of `stack`, unchanged.
fn one_of(stack: &ItemStack) -> ItemStack {
    let mut one = stack.clone();
    one.count = 1;
    one
}

/// One item of `stack` holding `energy`, its other data kept. An empty item
/// carries nothing, so empty batteries stack with new ones.
fn with_energy(stack: &ItemStack, energy: i32) -> ItemStack {
    let mut one = one_of(stack);
    let mut data = match one.data().and_then(|d| d.other(CUSTOM_DATA)) {
        Some(Tag::Compound(data)) => data.clone(),
        _ => Compound::new(),
    };
    if energy > 0 {
        data.insert(JavaString::from(ENERGY), Tag::Int(energy));
    } else {
        data.remove(&JavaString::from(ENERGY));
    }
    let value = (!data.is_empty()).then_some(Tag::Compound(data));
    one.data_mut().set_other(CUSTOM_DATA, value);
    // What a server sent for the stack no longer describes it; kept, an
    // emptied battery would not stack with a new one.
    one.data_mut().wire = Default::default();
    one.tidy();
    one
}

/// What one item of `stack` holds, if it has a buffer.
fn contents(stack: &ItemStack, buffer: Option<ItemEnergyBuffer>) -> Option<NativeItemEnergy> {
    let buffer = buffer?;
    (stack.count > 0).then(|| NativeItemEnergy {
        amount: read_energy(stack),
        capacity: buffer.capacity,
        max_insert: buffer.max_insert,
        max_extract: buffer.max_extract,
    })
}

/// `SimpleEnergyHandler.insert` on one item of `stack`: the room left (none
/// when it holds more than its capacity), the call's limit and what was
/// offered, whichever is least; and the item it becomes.
fn insert_one(
    stack: &ItemStack,
    buffer: Option<ItemEnergyBuffer>,
    offered: i32,
) -> Option<(i32, ItemStack)> {
    let held = contents(stack, buffer)?;
    let took = (held.capacity.saturating_sub(held.amount))
        .min(offered)
        .min(held.max_insert)
        .max(0);
    if took == 0 {
        return Some((0, one_of(stack)));
    }
    Some((took, with_energy(stack, held.amount + took)))
}

/// `SimpleEnergyHandler.extract` on one item of `stack`: what it holds, the
/// call's limit and what was asked, whichever is least; and the item it
/// becomes.
fn extract_one(
    stack: &ItemStack,
    buffer: Option<ItemEnergyBuffer>,
    asked: i32,
) -> Option<(i32, ItemStack)> {
    let held = contents(stack, buffer)?;
    let gave = held.amount.min(asked).min(held.max_extract).max(0);
    if gave == 0 {
        return Some((0, one_of(stack)));
    }
    Some((gave, with_energy(stack, held.amount - gave)))
}

// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// The buffer a mod gave item `item`, checked; none for any other item.
    fn native_item_energy_of(&self, item: &str) -> Option<ItemEnergyBuffer> {
        if item.starts_with("minecraft:") {
            return None;
        }
        let mut buffer = None;
        self.native_item_energy_buffer(item, &mut buffer);
        buffer.and_then(checked)
    }
    /// `Capabilities.Energy.ITEM`, read: the amount, capacity and limits per
    /// call of one item of `stack`, which keeps the amount in its
    /// `custom_data`. None for an item without a buffer, and for an empty
    /// stack.
    pub(in crate::game) fn native_item_energy(
        &self,
        stack: &ItemStack,
    ) -> Option<NativeItemEnergy> {
        contents(stack, self.native_item_energy_of(&stack.item))
    }
    /// `EnergyHandler.insert`: up to `amount` into one item of `stack`, no
    /// more than its room and its limit per call. The amount that went in,
    /// and the single item it becomes (the item itself when it took
    /// nothing). The stack given is not changed: shrink it by one and hand
    /// out the result, or leave the result unused to simulate. None where
    /// the item has no buffer.
    pub(in crate::game) fn native_item_energy_insert(
        &self,
        stack: &ItemStack,
        amount: i32,
    ) -> Option<(i32, ItemStack)> {
        insert_one(stack, self.native_item_energy_of(&stack.item), amount)
    }
    /// `EnergyHandler.extract`: up to `amount` out of one item of `stack`,
    /// no more than it holds and its limit per call. The amount that came
    /// out, and the single item it becomes. The stack given is not changed,
    /// as with `native_item_energy_insert`. None where the item has no
    /// buffer.
    pub(in crate::game) fn native_item_energy_extract(
        &self,
        stack: &ItemStack,
        amount: i32,
    ) -> Option<(i32, ItemStack)> {
        extract_one(stack, self.native_item_energy_of(&stack.item), amount)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn battery(count: u32) -> ItemStack {
        ItemStack::new("mymod:battery", count)
    }

    fn buffer(capacity: i32, max_insert: i32, max_extract: i32) -> Option<ItemEnergyBuffer> {
        checked(ItemEnergyBuffer {
            capacity,
            max_insert,
            max_extract,
        })
    }

    #[test]
    fn an_insert_stops_at_the_room_the_limit_and_the_offer() {
        let b = buffer(1000, 300, 1000);
        let (took, one) = insert_one(&battery(5), b, 200).unwrap();
        assert_eq!((took, one.count, read_energy(&one)), (200, 1, 200));
        let (took, one) = insert_one(&one, b, 5000).unwrap();
        assert_eq!((took, read_energy(&one)), (300, 500));
        let (_, one) = insert_one(&one, b, 300).unwrap();
        let (_, one) = insert_one(&one, b, 300).unwrap();
        assert_eq!(read_energy(&one), 1000);
        assert_eq!(insert_one(&one, b, 1).unwrap().0, 0);
        // Nothing offered, or a negative offer, moves nothing.
        assert_eq!(insert_one(&battery(1), b, 0).unwrap(), (0, battery(1)));
        assert_eq!(insert_one(&battery(1), b, -50).unwrap(), (0, battery(1)));
    }

    #[test]
    fn an_extract_stops_at_what_is_held_the_limit_and_the_ask() {
        let b = buffer(1000, 1000, 150);
        let (_, full) = insert_one(&battery(1), b, 400).unwrap();
        let (gave, rest) = extract_one(&full, b, 100).unwrap();
        assert_eq!((gave, read_energy(&rest)), (100, 300));
        let (gave, rest) = extract_one(&rest, b, 1000).unwrap();
        assert_eq!((gave, read_energy(&rest)), (150, 150));
        let (gave, empty) = extract_one(&rest, b, 1000).unwrap();
        assert_eq!(gave, 150);
        // Nothing left, so it is a new battery again and stacks with one.
        assert_eq!(empty, battery(1));
        assert_eq!(extract_one(&empty, b, 10).unwrap(), (0, battery(1)));
    }

    #[test]
    fn a_zero_limit_closes_that_way_only() {
        // A generator's output cell: it gives, and takes nothing.
        let source = buffer(500, 0, 500);
        assert_eq!(insert_one(&battery(1), source, 100).unwrap().0, 0);
        let sink = buffer(500, 500, 0);
        let (_, charged) = insert_one(&battery(1), sink, 100).unwrap();
        assert_eq!(extract_one(&charged, sink, 100).unwrap().0, 0);
        assert_eq!(extract_one(&charged, source, 100).unwrap().0, 100);
    }

    #[test]
    fn a_buffer_is_checked_as_a_blocks_is() {
        assert_eq!(buffer(0, 10, 10), None);
        assert_eq!(buffer(-5, 10, 10), None);
        assert_eq!(
            buffer(100, 500, -3),
            Some(ItemEnergyBuffer {
                capacity: 100,
                max_insert: 100,
                max_extract: 0,
            })
        );
    }

    #[test]
    fn items_without_a_buffer_hold_nothing() {
        assert_eq!(contents(&battery(1), None), None);
        assert_eq!(insert_one(&battery(1), None, 10), None);
        assert_eq!(extract_one(&battery(1), None, 10), None);
        assert_eq!(contents(&battery(0), buffer(100, 10, 10)), None);
    }

    #[test]
    fn what_a_stack_holds_reads_back_with_its_limits() {
        let b = buffer(2000, 50, 75);
        let (_, one) = insert_one(&battery(1), b, 50).unwrap();
        assert_eq!(
            contents(&one, b),
            Some(NativeItemEnergy {
                amount: 50,
                capacity: 2000,
                max_insert: 50,
                max_extract: 75,
            })
        );
        // A damaged or foreign value reads as empty.
        let mut odd = battery(1);
        odd.data_mut().set_other(
            CUSTOM_DATA,
            Some(Tag::compound([(
                ENERGY,
                Tag::String(JavaString::from("9")),
            )])),
        );
        assert_eq!(read_energy(&odd), 0);
        odd.data_mut()
            .set_other(CUSTOM_DATA, Some(Tag::compound([(ENERGY, Tag::Int(-40))])));
        assert_eq!(read_energy(&odd), 0);
    }

    #[test]
    fn more_than_the_capacity_takes_nothing_and_still_gives() {
        // A mod that shrank its capacity after the item was charged.
        let mut over = battery(1);
        over.data_mut()
            .set_other(CUSTOM_DATA, Some(Tag::compound([(ENERGY, Tag::Int(900))])));
        let b = buffer(500, 500, 500);
        assert_eq!(insert_one(&over, b, 100).unwrap().0, 0);
        let (gave, rest) = extract_one(&over, b, 1000).unwrap();
        assert_eq!((gave, read_energy(&rest)), (500, 400));
    }

    #[test]
    fn an_emptied_battery_from_a_server_stacks_with_a_new_one() {
        let b = buffer(100, 100, 100);
        let mut sent = battery(2);
        sent.data_mut().wire =
            horizon_client::item_data::Wire::new(battery(2), Arc::from(vec![1u8]));
        let (_, charged) = insert_one(&sent, b, 40).unwrap();
        let (_, emptied) = extract_one(&charged, b, 40).unwrap();
        assert!(emptied.same_item_same_components(&battery(1)));
    }

    #[test]
    fn a_battery_keeps_the_rest_of_its_custom_data() {
        let b = buffer(1000, 1000, 1000);
        let mut stack = battery(1);
        stack.data_mut().set_other(
            CUSTOM_DATA,
            Some(Tag::compound([(
                "owner",
                Tag::String(JavaString::from("Alex")),
            )])),
        );
        let (_, charged) = insert_one(&stack, b, 10).unwrap();
        let Some(Tag::Compound(data)) = charged.data().and_then(|d| d.other(CUSTOM_DATA)) else {
            panic!("no custom data");
        };
        assert!(data.contains_key(&JavaString::from("owner")));
        assert_eq!(data.get(&JavaString::from(ENERGY)), Some(&Tag::Int(10)));
        let (_, emptied) = extract_one(&charged, b, 10).unwrap();
        assert_eq!(emptied, stack);
    }
}
