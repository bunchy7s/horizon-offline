//! Item stacks in NBT (`ItemStack.CODEC`): id, count and components, alone or in
//! slot lists.
use horizon_client_core::nbt_value::{number, string, text};
use horizon_world::nbt::Tag;
use std::sync::Arc;

pub fn encode_item(stack: &crate::items::ItemStack) -> Vec<(&'static str, Tag)> {
    let mut entries = vec![
        ("id", string(&stack.item)),
        ("count", Tag::Int(stack.count as i32)),
    ];
    if let Some(components) = crate::item_data::encode_components(stack) {
        entries.push(("components", components));
    }
    entries
}
/// ItemStack.CODEC (id, count, components).
pub fn item_to_nbt(stack: &crate::items::ItemStack) -> Tag {
    Tag::compound(encode_item(stack))
}
/// ItemStack.CODEC's count range is independent of the item's stack limit;
/// an out-of-range count uses the field's default of one.
pub fn item_from_nbt(tag: &Tag) -> Option<crate::items::ItemStack> {
    let id = text(tag.get("id"))?;
    if id == "minecraft:air" {
        return None;
    }
    let count = number(tag.get("count"))
        .or_else(|| number(tag.get("Count")))
        .filter(|count| (1..=99).contains(count))
        .unwrap_or(1) as u32;
    let (damage, extra) = crate::item_data::decode_components(tag.get("components"));
    Some(crate::items::ItemStack {
        item: Arc::from(id.as_str()),
        count,
        damage,
        extra,
    })
}
pub fn encode_items(slots: &[Option<crate::items::ItemStack>]) -> Tag {
    Tag::List(
        slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let mut entries = vec![("Slot", Tag::Byte(i as i8))];
                entries.extend(encode_item(s.as_ref()?));
                Some(Tag::compound(entries))
            })
            .collect(),
    )
}
pub fn decode_items(tag: Option<&Tag>, size: usize) -> Vec<Option<crate::items::ItemStack>> {
    let mut slots = vec![None; size];
    if let Some(Tag::List(items)) = tag {
        for it in items {
            let slot = number(it.get("Slot")).unwrap_or(0) as u8 as usize;
            if let Some(cell) = slots.get_mut(slot) {
                *cell = item_from_nbt(it);
            }
        }
    }
    slots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nbt_count_defaults_to_one_outside_codec_range() {
        for item in ["minecraft:stone", "minecraft:diamond_sword"] {
            for (raw, expected) in [
                (None, 1),
                (Some(Tag::Int(-1)), 1),
                (Some(Tag::Int(0)), 1),
                (Some(Tag::Int(1)), 1),
                (Some(Tag::Int(2)), 2),
                (Some(Tag::Int(64)), 64),
                (Some(Tag::Int(99)), 99),
                (Some(Tag::Int(100)), 1),
                (Some(Tag::Int(255)), 1),
                (Some(Tag::Int(256)), 1),
                (Some(Tag::Int(i32::MAX)), 1),
                (Some(Tag::Byte(1)), 1),
                (Some(Tag::Float(1.5_f32.to_bits())), 1),
                (Some(string("1")), 1),
            ] {
                let mut tag = Tag::compound([("id", string(item))]);
                if let (Tag::Compound(map), Some(raw)) = (&mut tag, &raw) {
                    map.insert("count".into(), raw.clone());
                }
                let stack = item_from_nbt(&tag).unwrap();
                assert_eq!(stack.count, expected, "{item}: {raw:?}");
                assert_eq!(
                    number(item_to_nbt(&stack).get("count")),
                    Some(i64::from(expected))
                );
            }
        }
    }

    #[test]
    fn runtime_counts_remain_independent_of_the_nbt_decode_range() {
        let stack = crate::items::ItemStack::new("minecraft:diamond_sword", 1000);
        assert_eq!(stack.count, 1000);
        assert_eq!(number(item_to_nbt(&stack).get("count")), Some(1000));
    }
}
