//! Exact component predicates nested in a partial predicate's NBT
//! (`container`, `bundle_contents`: their item predicates' `components`).
//!
//! Vanilla's client decodes the predicate's NBT with its codec and hashes the
//! result (`DataComponentExactPredicate.CODEC` is `DataComponentType.VALUE_MAP_CODEC`,
//! a `Codec.dispatchedMap` of component type to that type's own codec). The
//! tree is what the codecs encode, so it is walked as NBT; the one thing NBT
//! does not say is which bytes are a `Codec.BOOL` (`NbtOps.createBoolean`
//! writes a byte, `HashOps` hashes a boolean). Those are the fields the
//! codecs of each component declare, listed by component below.
use super::gameplay::PREDICATE_BOOLS;
use super::*;
use horizon_world::nbt::{Compound, Tag};

/// What a component's codec holds that NBT cannot tell apart.
struct Shape {
    /// Keys whose byte value is a `Codec.BOOL`.
    bools: Vec<&'static str>,
    /// Keys whose list holds `Codec.BOOL`s (`CustomModelData.flags`).
    bool_lists: &'static [&'static str],
    /// Whether the value holds item stacks, whose `components` are a patch.
    stacks: bool,
}
impl Shape {
    fn plain() -> Self {
        Self::of(&[])
    }
    fn of(bools: &[&'static str]) -> Self {
        Self {
            bools: bools.to_vec(),
            bool_lists: &[],
            stacks: false,
        }
    }
    fn with(mut self, more: &[&'static str]) -> Self {
        self.bools.extend_from_slice(more);
        self
    }
    fn stacks(mut self) -> Self {
        self.stacks = true;
        self
    }
}

/// `MobEffectInstance.Details.MAP_CODEC`.
const EFFECT: [&str; 3] = ["ambient", "show_particles", "show_icon"];

/// The shape of the persistent component `name` (without `minecraft:`), or
/// `None` for one with no persistent codec: `DataComponentType.PERSISTENT_CODEC`
/// refuses it, so no predicate holds it.
fn shape(name: &str) -> Option<Shape> {
    Some(match name {
        "creative_slot_lock" | "map_post_processing" | "additional_trade_cost" => return None,
        // `ComponentSerialization.CODEC`, and records that hold text components.
        "custom_name"
        | "item_name"
        | "lore"
        | "attribute_modifiers"
        | "instrument"
        | "provides_trim_material"
        | "jukebox_playable"
        | "painting/variant" => Shape::of(&TEXT_BOOLS),
        "use_effects" => Shape::of(&["can_sprint", "interact_vibrations"]),
        "tooltip_display" => Shape::of(&["hide_tooltip"]),
        "food" => Shape::of(&["can_always_eat"]),
        "consumable" => {
            Shape::of(&["has_consume_particles", "directional_particles"]).with(&EFFECT)
        }
        "death_protection" => Shape::of(&["directional_particles"]).with(&EFFECT),
        "tool" => Shape::of(&["can_destroy_blocks_in_creative", "correct_for_drops"]),
        "equippable" => Shape::of(&[
            "dispensable",
            "swappable",
            "damage_on_hurt",
            "equip_on_interact",
            "can_be_sheared",
        ]),
        "piercing_weapon" => Shape::of(&["deals_knockback", "dismounts"]),
        "lodestone_tracker" => Shape::of(&["tracked"]),
        "firework_explosion" | "fireworks" => Shape::of(&["has_trail", "has_twinkle"]),
        "potion_contents" => Shape::of(&EFFECT),
        "trim" => Shape::of(&["decal"]).with(&TEXT_BOOLS),
        "block_transformer" => Shape::of(&["update_from_neighbors", "consume_on_use"]),
        "written_book_content" => Shape::of(&["resolved"]).with(&TEXT_BOOLS),
        "sign_text_front" | "sign_text_back" => Shape::of(&["has_glowing_text"]).with(&TEXT_BOOLS),
        "custom_model_data" => Shape {
            bool_lists: &["flags"],
            ..Shape::plain()
        },
        "container"
        | "bundle_contents"
        | "charged_projectiles"
        | "use_remainder"
        | "sulfur_cube_content" => Shape::plain().stacks(),
        // The rest hold no boolean (`custom_data` and the entity and block entity
        // data are raw NBT, which is hashed as it is).
        _ => Shape::plain(),
    })
}

fn units(key: &[u16]) -> String {
    String::from_utf16_lossy(key)
}

/// `DataComponentExactPredicate.CODEC` (or, for `stacks`, a
/// `DataComponentPatch.CODEC`, whose `!name` entries remove a component) as a
/// map of the component's name to its value.
fn components(map: &Compound, names: &dyn Names) -> Option<HashCode> {
    let mut entries = Vec::with_capacity(map.len());
    for (key, value) in map.iter() {
        let key = units(&key.0);
        let hashed = match key.strip_prefix('!') {
            Some(_) => HashOps::empty_map(),
            None => {
                let short = key.strip_prefix("minecraft:")?;
                match (short, value) {
                    // `enchantment_glint_override` is a bare `Codec.BOOL`.
                    ("enchantment_glint_override", Tag::Byte(b)) => {
                        HashOps::create_boolean(*b != 0)
                    }
                    _ => walk(value, &shape(short)?, names)?,
                }
            }
        };
        entries.push((string(&key), hashed));
    }
    Some(HashOps::create_map(entries))
}

/// A partial predicate's NBT: its booleans, and the `components` of the item
/// predicates inside it.
pub(super) fn predicate(tag: &Tag, names: &dyn Names) -> Option<HashCode> {
    walk(tag, &Shape::of(&PREDICATE_BOOLS).stacks(), names)
}

/// `tag` as the hash of the value its codec encoded, by `shape`.
fn walk(tag: &Tag, shape: &Shape, names: &dyn Names) -> Option<HashCode> {
    Some(match tag {
        Tag::List(items) => HashOps::create_list(
            items
                .iter()
                .map(|t| walk(t, shape, names))
                .collect::<Option<Vec<_>>>()?,
        ),
        Tag::Compound(map) => {
            let mut entries = Vec::with_capacity(map.len());
            for (key, value) in map.iter() {
                let name = units(&key.0);
                let hashed = match value {
                    Tag::Compound(inner) if shape.stacks && name == "components" => {
                        components(inner, names)?
                    }
                    Tag::Byte(b) if shape.bools.contains(&name.as_str()) => {
                        HashOps::create_boolean(*b != 0)
                    }
                    Tag::List(items) if shape.bool_lists.contains(&name.as_str()) => {
                        HashOps::create_list(items.iter().map(|t| match t {
                            Tag::Byte(b) => HashOps::create_boolean(*b != 0),
                            other => HashOps::from_nbt(other, &[]),
                        }))
                    }
                    _ => walk(value, shape, names)?,
                };
                entries.push((HashOps::create_string_units(&key.0), hashed));
            }
            HashOps::create_map(entries)
        }
        other => HashOps::from_nbt(other, &[]),
    })
}
