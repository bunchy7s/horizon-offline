//! Item stacks on the wire (`ItemStack.OPTIONAL_STREAM_CODEC`): the component
//! patch read back as vanilla's `components` compound, so the decoder the saved
//! and command item NBT use fills every field. Both sides use it: a client for
//! what its server sends, a server for a creative client's
//! `SetCreativeModeSlot`. The other direction is `lan::guest::stack_to_network`.
use crate::items::ItemStack;
use horizon_protocol::buf::{Codec, Reader};
use horizon_protocol::item::{Component, Profile, Stack, Template, component};
use horizon_protocol::registry::{Builtin, Dynamic};
use horizon_world::nbt::Tag;
use std::sync::Arc;

/// A stack from the wire: its item by the built-in registry's id, its
/// components by the synced registries' (enchantments, trims).
pub fn from_network(s: &Stack, builtin: &Builtin, synced: &Dynamic) -> ItemStack {
    let mut out = ItemStack {
        item: item_name(builtin, s.item),
        count: s.count.max(0) as u32,
        damage: 0,
        extra: None,
    };
    let components = components_tag(s, builtin, synced);
    let (damage, data) = crate::item_data::decode_components(components.as_ref());
    out.damage = damage;
    if let Some(data) = data {
        *out.data_mut() = *data;
    }
    out.tidy();
    // What it decoded to, to send the server's own patch back if that holds.
    if let Some(patch) = &s.wire.0 {
        let decoded = out.clone();
        out.data_mut().wire = crate::item_data::Wire::new(decoded, patch.clone());
    }
    out
}

/// `DyeColor.byId`: an unknown id is white (`OutOfBoundsStrategy.ZERO`).
fn dye_name(id: i32) -> &'static str {
    usize::try_from(id)
        .ok()
        .and_then(|i| crate::block_entity_render::DYES.get(i))
        .map_or("white", |d| d.0)
}

fn item_name(builtin: &Builtin, id: i32) -> Arc<str> {
    Arc::from(
        builtin
            .get("minecraft:item")
            .and_then(|i| i.name(id))
            .unwrap_or("minecraft:air"),
    )
}

/// The item's patched components as vanilla's `components` compound.
fn components_tag(s: &Stack, builtin: &Builtin, synced: &Dynamic) -> Option<Tag> {
    let name = |id: i32| builtin.get("minecraft:data_component_type")?.name(id);
    let mut entries: Vec<(&str, Tag)> = Vec::new();
    for (kind, c) in &s.added {
        let Some(key) = name(*kind) else {
            continue;
        };
        let value = match c {
            // `ItemContainerContents`: a shulker box's items, for its tooltip.
            Component::Raw(bytes) if *kind == component::CONTAINER => {
                let Some(slots) = container_tag(bytes, builtin, synced) else {
                    continue;
                };
                slots
            }
            // `TypedEntityData`: a block item's block entity, which the op
            // warning and a spawner's tooltip read.
            Component::Raw(bytes) if *kind == component::BLOCK_ENTITY_DATA => {
                let Some(data) = block_entity_data_tag(bytes, builtin) else {
                    continue;
                };
                data
            }
            // Read past, not decoded: the item keeps its prototype's value.
            Component::Raw(_) => continue,
            // Trim materials and patterns are synced registries, as enchantments are.
            Component::Trim { material, pattern } => {
                let (Some(material), Some(pattern)) = (
                    synced.name("minecraft:trim_material", *material),
                    synced.name("minecraft:trim_pattern", *pattern),
                ) else {
                    continue;
                };
                Tag::compound([
                    ("material", Tag::String(material.into())),
                    ("pattern", Tag::String(pattern.into())),
                ])
            }
            // Instruments are a synced registry: a goat horn names its own.
            Component::Instrument(id) => match synced.name("minecraft:instrument", *id) {
                Some(name) => Tag::String(name.into()),
                None => continue,
            },
            // BannerPatternLayers: each layer's pattern is a synced registry entry.
            Component::BannerPatterns(layers) => {
                let layers: Option<Vec<Tag>> = layers
                    .iter()
                    .map(|(pattern, color)| {
                        let pattern = synced.name("minecraft:banner_pattern", *pattern)?;
                        Some(Tag::compound([
                            ("pattern", Tag::String(pattern.into())),
                            ("color", Tag::String(dye_name(*color).into())),
                        ]))
                    })
                    .collect();
                let Some(layers) = layers else {
                    continue;
                };
                Tag::List(layers)
            }
            // A shield's `base_color`: the dye's id.
            Component::Int(v) if *kind == component::BASE_COLOR => Tag::String(dye_name(*v).into()),
            Component::Int(v) if *kind == component::RARITY => {
                let rarity = ["common", "uncommon", "rare", "epic"];
                Tag::String(rarity.get(*v as usize).copied().unwrap_or("common").into())
            }
            Component::Int(v) => Tag::Int(*v),
            Component::Unit => Tag::Compound(Default::default()),
            Component::Bool(v) => Tag::Byte(i8::from(*v)),
            Component::Text(t) | Component::Nbt(t) => t.clone(),
            Component::Ident(v) => Tag::String(v.as_str().into()),
            Component::Lore(lines) => Tag::List(lines.clone()),
            Component::WritableBook(pages) => Tag::compound([(
                "pages",
                Tag::List(
                    pages
                        .iter()
                        .map(|p| Tag::String(p.as_str().into()))
                        .collect(),
                ),
            )]),
            Component::WrittenBook(b) => Tag::compound([
                ("title", Tag::String(b.title.as_str().into())),
                ("author", Tag::String(b.author.as_str().into())),
                ("generation", Tag::Int(b.generation)),
                ("pages", Tag::List(b.pages.clone())),
                ("resolved", Tag::Byte(i8::from(b.resolved))),
            ]),
            Component::Fireworks { flight, explosions } => Tag::compound([
                ("flight_duration", Tag::Byte(*flight as i8)),
                (
                    "explosions",
                    Tag::List(explosions.iter().map(fireworks_tag).collect()),
                ),
            ]),
            Component::FireworkExplosion(e) => fireworks_tag(e),
            Component::LodestoneTracker { target, tracked } => {
                let mut entries = Vec::new();
                if let Some((dimension, pos)) = target {
                    entries.push((
                        "target",
                        Tag::compound([
                            ("dimension", Tag::String(dimension.as_str().into())),
                            ("pos", Tag::IntArray(pos.to_vec())),
                        ]),
                    ));
                }
                if !tracked {
                    entries.push(("tracked", Tag::Byte(0)));
                }
                Tag::compound(entries)
            }
            Component::Enchantments(list) => {
                // Enchantments are a synced (data-driven) registry: the
                // ids index the server's RegistryData, not the builtin
                // report, so a datapack enchantment names correctly.
                let levels = list.iter().filter_map(|(id, level)| {
                    let name = synced.name("minecraft:enchantment", *id)?;
                    Some((name, Tag::Int(*level)))
                });
                Tag::compound(levels)
            }
            Component::AttackRange(v) => Tag::compound([
                ("min_reach", Tag::Float(v[0].to_bits())),
                ("max_reach", Tag::Float(v[1].to_bits())),
                ("min_attack_momentum", Tag::Float(v[2].to_bits())),
                ("max_attack_momentum", Tag::Float(v[3].to_bits())),
                ("hitbox_margin", Tag::Float(v[4].to_bits())),
                ("mob_factor", Tag::Float(v[5].to_bits())),
            ]),
            Component::Profile(p) => profile_tag(p),
            // PotionContents' NBT: the potion by name and the custom colour.
            Component::Potion { potion, color, .. } => {
                let potion = potion.and_then(|id| builtin.get("minecraft:potion")?.name(id));
                Tag::compound(
                    potion
                        .map(|name| ("potion", Tag::String(name.into())))
                        .into_iter()
                        .chain(color.map(|c| ("custom_color", Tag::Int(c)))),
                )
            }
            // ItemStackTemplate.CODEC: id, count and components.
            Component::Stacks(stacks) => Tag::List(
                stacks
                    .iter()
                    .map(|t| template_tag(t, builtin, synced))
                    .collect(),
            ),
            Component::CustomModelData {
                floats,
                flags,
                strings,
                colors,
            } => Tag::compound([
                (
                    "floats",
                    Tag::List(floats.iter().map(|f| Tag::Float(f.to_bits())).collect()),
                ),
                (
                    "flags",
                    Tag::List(flags.iter().map(|b| Tag::Byte(i8::from(*b))).collect()),
                ),
                (
                    "strings",
                    Tag::List(
                        strings
                            .iter()
                            .map(|s| Tag::String(s.as_str().into()))
                            .collect(),
                    ),
                ),
                (
                    "colors",
                    Tag::List(colors.iter().map(|c| Tag::Int(*c)).collect()),
                ),
            ]),
            Component::TooltipDisplay { hide, hidden } => {
                let hidden = hidden
                    .iter()
                    .filter_map(|id| Some(Tag::String(name(*id)?.into())));
                Tag::compound([
                    ("hide_tooltip", Tag::Byte(i8::from(*hide))),
                    ("hidden_components", Tag::List(hidden.collect())),
                ])
            }
        };
        entries.push((key, value));
    }
    Some(Tag::compound(entries))
}

/// An `ItemStackTemplate` as its NBT codec writes it: id, count and components.
fn template_tag(t: &Stack, builtin: &Builtin, synced: &Dynamic) -> Tag {
    let mut entries = vec![
        (
            "id",
            Tag::String(item_name(builtin, t.item).as_ref().into()),
        ),
        ("count", Tag::Int(t.count)),
    ];
    if let Some(c) = components_tag(t, builtin, synced) {
        entries.push(("components", c));
    }
    Tag::compound(entries)
}

/// `TypedEntityData.streamCodec` of a block entity: its type's registry id,
/// then its data without the id, which the saved component keeps.
fn block_entity_data_tag(bytes: &[u8], builtin: &Builtin) -> Option<Tag> {
    let mut r = Reader::new(bytes);
    let id = r.varint().ok()?;
    let kind = builtin.get("minecraft:block_entity_type")?.name(id)?;
    let Tag::Compound(mut data) = r.nbt().ok()? else {
        return None;
    };
    data.insert("id".into(), Tag::String(kind.into()));
    Some(Tag::Compound(data))
}

/// `ItemContainerContents.STREAM_CODEC`, one optional template per slot, as
/// the saved `container` lists the slots that hold one.
fn container_tag(bytes: &[u8], builtin: &Builtin, synced: &Dynamic) -> Option<Tag> {
    let mut r = Reader::new(bytes);
    let mut slots = Vec::new();
    for slot in 0..r.count(256).ok()? {
        if r.bool().ok()? {
            let stack = Template::read(&mut r).ok()?;
            slots.push(Tag::compound([
                ("slot", Tag::Int(slot as i32)),
                ("item", template_tag(&stack, builtin, synced)),
            ]));
        }
    }
    Some(Tag::List(slots))
}

/// A wire `FireworkExplosion` as vanilla's NBT codec writes it
/// (`FireworkExplosion.Shape`'s BY_ID order).
fn fireworks_tag(e: &horizon_protocol::item::FireworkExplosion) -> Tag {
    const SHAPES: [&str; 5] = ["small_ball", "large_ball", "star", "creeper", "burst"];
    let ints = |c: &[i32]| Tag::List(c.iter().map(|c| Tag::Int(*c)).collect());
    Tag::compound([
        (
            "shape",
            Tag::String(
                SHAPES
                    .get(e.shape.max(0) as usize)
                    .copied()
                    .unwrap_or("small_ball")
                    .into(),
            ),
        ),
        ("colors", ints(&e.colors)),
        ("fade_colors", ints(&e.fade)),
        ("has_trail", Tag::Byte(i8::from(e.trail))),
        ("has_twinkle", Tag::Byte(i8::from(e.twinkle))),
    ])
}

/// A `minecraft:profile` component as the item data JSON the model reads.
fn profile_tag(p: &Profile) -> Tag {
    let properties = p
        .properties
        .iter()
        .map(|(name, value, signature)| {
            let mut entries: Vec<(&'static str, Tag)> = vec![
                ("name", Tag::String(name.as_str().into())),
                ("value", Tag::String(value.as_str().into())),
            ];
            if let Some(signature) = signature {
                entries.push(("signature", Tag::String(signature.as_str().into())));
            }
            Tag::compound(entries)
        })
        .collect::<Vec<_>>();
    let mut entries: Vec<(&'static str, Tag)> = Vec::new();
    if let Some(name) = &p.name {
        entries.push(("name", Tag::String(name.as_str().into())));
    }
    if let Some(id) = p.id {
        entries.push((
            "id",
            Tag::IntArray(vec![
                (id >> 96) as i32,
                (id >> 64) as i32,
                (id >> 32) as i32,
                id as i32,
            ]),
        ));
    }
    entries.push(("properties", Tag::List(properties)));
    Tag::compound(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_protocol::registry::Ids;

    #[test]
    fn merchant_water_cost_survives_the_offer_packet() {
        use horizon_protocol::{
            buf::{Codec, Reader, Writer},
            packets::play::{NetOffer, Offers},
        };
        let mut components = vec![String::new(); component::POTION_CONTENTS as usize + 1];
        components[component::POTION_CONTENTS as usize] = "minecraft:potion_contents".into();
        let builtin = Builtin {
            registries: [
                (
                    "minecraft:item".into(),
                    Ids::from_names(vec!["minecraft:potion".into(), "minecraft:emerald".into()]),
                ),
                (
                    "minecraft:potion".into(),
                    Ids::from_names(vec!["minecraft:water".into()]),
                ),
                (
                    "minecraft:data_component_type".into(),
                    Ids::from_names(components),
                ),
            ]
            .into(),
        };
        let dynamic = Dynamic::default();
        let water = ItemStack::new("minecraft:potion", 1).with_potion("minecraft:water");
        let wire = crate::lan::guest::stack_to_network(
            |name| builtin.get("minecraft:item").unwrap().id(name).unwrap(),
            |registry, name| builtin.get(registry)?.id(name),
            &Some(water.clone()),
        )
        .unwrap();
        let offers = vec![NetOffer {
            cost_a: wire.into(),
            result: Stack::new(1, 1),
            cost_b: Some(Stack::new(1, 2).into()),
            out_of_stock: false,
            uses: 0,
            max_uses: 2,
            xp: 0,
            special_price: 0,
            price_multiplier: 0.05,
            demand: 0,
        }];
        let mut w = Writer::new();
        Offers::write(&offers, &mut w);
        let mut r = Reader::new(&w.data);
        let decoded = Offers::read(&mut r).unwrap();
        assert_eq!(r.remaining(), 0);
        let cost = crate::merchant::ItemCost {
            stack: from_network(&decoded[0].cost_a.clone().into(), &builtin, &dynamic),
        };
        assert_eq!(cost.stack, water);
        assert_eq!(cost.stack.potion(), Some("minecraft:water"));
        let authoritative = crate::merchant::ItemCost {
            stack: water.clone(),
        };
        for payment in [
            water,
            ItemStack::new("minecraft:potion", 1).with_potion("minecraft:swiftness"),
            ItemStack::new("minecraft:potion", 1),
        ] {
            assert_eq!(cost.test(&payment), authoritative.test(&payment));
        }
        assert!(cost.test(&ItemStack::new("minecraft:potion", 1).with_potion("minecraft:water")));
        assert!(
            !cost.test(&ItemStack::new("minecraft:potion", 1).with_potion("minecraft:swiftness"))
        );
        let emerald = crate::merchant::ItemCost {
            stack: from_network(
                &decoded[0].cost_b.clone().unwrap().into(),
                &builtin,
                &dynamic,
            ),
        };
        assert!(emerald.test(&ItemStack::new("minecraft:emerald", 2)));
    }

    /// A bundle's contents cross the wire, so a server's client can draw
    /// its tooltip (BundleContents' stream codec: a list of templates).
    #[test]
    fn a_bundle_keeps_its_contents_across_the_wire() {
        let mut components = vec![String::new(); 60];
        components[component::BUNDLE_CONTENTS as usize] = "minecraft:bundle_contents".into();
        let items = ["minecraft:air", "minecraft:bundle", "minecraft:stone"];
        let builtin = Builtin {
            registries: [
                (
                    "minecraft:item".to_owned(),
                    Ids::from_names(items.iter().map(|s| s.to_string()).collect()),
                ),
                (
                    "minecraft:data_component_type".to_owned(),
                    Ids::from_names(components),
                ),
            ]
            .into(),
        };
        let dynamic = Dynamic {
            registries: Default::default(),
            ..Default::default()
        };
        let mut bundle = ItemStack::new("minecraft:bundle", 1);
        bundle.data_mut().bundle_contents = vec![ItemStack::new("minecraft:stone", 32)];
        let wire = crate::lan::guest::stack_to_network(
            |name| items.iter().position(|i| *i == name).unwrap_or(0) as i32,
            |_, _| None,
            &Some(bundle),
        )
        .unwrap();
        let back = from_network(&wire, &builtin, &dynamic);
        let contents = &back.data().expect("components").bundle_contents;
        assert_eq!(contents.len(), 1);
        assert_eq!(
            (&*contents[0].item, contents[0].count),
            ("minecraft:stone", 32)
        );
    }

    /// An ominous bottle's `ominous_bottle_amplifier` reaches the client
    /// through the packet's bytes, so its tooltip names the Bad Omen level.
    #[test]
    fn an_ominous_bottle_keeps_its_amplifier_across_the_wire() {
        use horizon_protocol::buf::{Codec, Reader, Writer};
        use horizon_protocol::item::Slot;
        let mut components = vec![String::new(); 80];
        components[component::OMINOUS_BOTTLE_AMPLIFIER as usize] =
            "minecraft:ominous_bottle_amplifier".into();
        let items = ["minecraft:air", "minecraft:ominous_bottle"];
        let builtin = Builtin {
            registries: [
                (
                    "minecraft:item".to_owned(),
                    Ids::from_names(items.iter().map(|s| s.to_string()).collect()),
                ),
                (
                    "minecraft:data_component_type".to_owned(),
                    Ids::from_names(components),
                ),
            ]
            .into(),
        };
        let dynamic = Dynamic {
            registries: Default::default(),
            ..Default::default()
        };
        for amplifier in [None, Some(0), Some(3)] {
            let mut bottle = ItemStack::new("minecraft:ominous_bottle", 1);
            if let Some(a) = amplifier {
                bottle.data_mut().ominous_amplifier = Some(a);
            }
            let wire = crate::lan::guest::stack_to_network(
                |name| items.iter().position(|i| *i == name).unwrap_or(0) as i32,
                |_, _| None,
                &Some(bottle),
            );
            let mut w = Writer::new();
            Slot::write(&wire, &mut w);
            let mut r = Reader::new(&w.data);
            let read = Slot::read(&mut r).unwrap().unwrap();
            assert_eq!(r.remaining(), 0);
            let back = from_network(&read, &builtin, &dynamic);
            assert_eq!(
                back.data().and_then(|d| d.ominous_amplifier),
                amplifier,
                "{amplifier:?}"
            );
        }
    }

    /// What a creative client sends (`stack_to_network`) comes back whole on
    /// the server: damage, both enchantment lists, repair cost, name, trim.
    #[test]
    fn a_creative_stack_keeps_its_components_across_the_wire() {
        let mut components = vec![String::new(); 60];
        for (id, name) in [
            (component::DAMAGE, "minecraft:damage"),
            (component::CUSTOM_NAME, "minecraft:custom_name"),
            (component::ITEM_NAME, "minecraft:item_name"),
            (component::ENCHANTMENTS, "minecraft:enchantments"),
            (component::POTION_CONTENTS, "minecraft:potion_contents"),
            (component::REPAIR_COST, "minecraft:repair_cost"),
            (
                component::STORED_ENCHANTMENTS,
                "minecraft:stored_enchantments",
            ),
            (component::TRIM, "minecraft:trim"),
        ] {
            components[id as usize] = name.to_owned();
        }
        let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let builtin = Builtin {
            registries: [
                (
                    "minecraft:item".to_owned(),
                    Ids::from_names(names(&[
                        "minecraft:air",
                        "minecraft:diamond_chestplate",
                        "minecraft:splash_potion",
                    ])),
                ),
                (
                    "minecraft:data_component_type".to_owned(),
                    Ids::from_names(components),
                ),
                (
                    "minecraft:potion".to_owned(),
                    Ids::from_names(names(&["minecraft:water", "minecraft:swiftness"])),
                ),
            ]
            .into(),
        };
        let synced = |n: &[&str]| n.iter().map(|s| (s.to_string(), None)).collect::<Vec<_>>();
        let dynamic = Dynamic {
            registries: [
                (
                    "minecraft:enchantment".to_owned(),
                    synced(&["minecraft:protection", "minecraft:sharpness"]),
                ),
                (
                    "minecraft:trim_material".to_owned(),
                    synced(&["minecraft:gold"]),
                ),
                (
                    "minecraft:trim_pattern".to_owned(),
                    synced(&["minecraft:sentry"]),
                ),
            ]
            .into(),
            ..Default::default()
        };
        let mut stack = ItemStack::new("minecraft:diamond_chestplate", 1);
        stack.damage = 5;
        stack.enchant("minecraft:protection", 4);
        let d = stack.data_mut();
        d.stored_enchantments
            .push((Arc::from("minecraft:sharpness"), 2));
        d.repair_cost = 9;
        d.custom_name = Some("Keeper".to_owned());
        // A translatable item name reaches the client as the same component.
        let item_name = Tag::compound([("translate", Tag::String("filled_map.monument".into()))]);
        d.other
            .push(("minecraft:item_name".into(), item_name.clone()));
        d.trim = Some((Arc::from("minecraft:gold"), Arc::from("minecraft:sentry")));
        let id = |registry: &str, name: &str| {
            builtin
                .get(registry)
                .and_then(|ids| ids.id(name))
                .or_else(|| dynamic.id(registry, name))
        };
        let wire = crate::lan::guest::stack_to_network(
            |name| {
                builtin
                    .get("minecraft:item")
                    .and_then(|i| i.id(name))
                    .unwrap_or(0)
            },
            id,
            &Some(stack.clone()),
        )
        .unwrap();
        let back = from_network(&wire, &builtin, &dynamic);
        assert_eq!(back.item, stack.item);
        assert_eq!(back.damage, 5);
        let d = back.data().expect("components");
        assert_eq!(d.enchantments, vec![(Arc::from("minecraft:protection"), 4)]);
        assert_eq!(
            d.stored_enchantments,
            vec![(Arc::from("minecraft:sharpness"), 2)]
        );
        assert_eq!(d.repair_cost, 9);
        assert_eq!(d.custom_name.as_deref(), Some("Keeper"));
        assert_eq!(d.other("minecraft:item_name"), Some(&item_name));
        assert_eq!(
            d.trim,
            Some((Arc::from("minecraft:gold"), Arc::from("minecraft:sentry")))
        );
        // PotionContents: the potion is a built-in registry, as a hub's
        // `SyncData::id` treats it, so the id has to come from there.
        let potion = ItemStack::new("minecraft:splash_potion", 1).with_potion("swiftness");
        let wire = crate::lan::guest::stack_to_network(
            |name| id("minecraft:item", name).unwrap_or(0),
            id,
            &Some(potion),
        )
        .unwrap();
        assert_eq!(
            from_network(&wire, &builtin, &dynamic).potion(),
            Some("minecraft:swiftness")
        );
    }
}
