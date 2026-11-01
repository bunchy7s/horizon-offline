use horizon_client::{item_sync, items::ItemStack, lan::guest::stack_to_network};
use horizon_items::item_nbt::{item_from_nbt, item_to_nbt};
use horizon_protocol::{
    buf::{Codec, Reader, Writer},
    item::{DelimitedSlot, Hashed, Slot, Stack, Template, component, custom},
    registry::{Builtin, Dynamic, Ids, SyncedNames},
};
use horizon_world::nbt::Tag;

const FIRST_CUSTOM_ID: i32 = 122;

#[test]
fn custom_components_survive_save_wire_and_nested_stacks() {
    custom::install(
        FIRST_CUSTOM_ID,
        vec!["sample:charge".into(), "sample:mode".into()],
    )
    .unwrap();
    let mut component_names = vec![String::new(); FIRST_CUSTOM_ID as usize];
    component_names[horizon_protocol::item::component::BUNDLE_CONTENTS as usize] =
        "minecraft:bundle_contents".into();
    component_names[horizon_protocol::item::component::CONTAINER as usize] =
        "minecraft:container".into();
    let builtin = Builtin {
        registries: [
            (
                "minecraft:item".into(),
                Ids::from_names(vec!["minecraft:stone".into()]),
            ),
            (
                "minecraft:data_component_type".into(),
                Ids::from_names(component_names),
            ),
        ]
        .into(),
    };
    let dynamic = Dynamic {
        custom_components: true,
        ..Default::default()
    };
    let names = SyncedNames {
        builtin: &builtin,
        dynamic: &dynamic,
    };
    let value = Tag::compound([("charge", Tag::Int(42))]);
    let mut item = ItemStack::new("minecraft:stone", 1);
    item.data_mut()
        .set_other("sample:charge", Some(value.clone()));
    item.data_mut()
        .set_other("!sample:mode", Some(Tag::Compound(Default::default())));

    let saved = item_from_nbt(&item_to_nbt(&item)).unwrap();
    assert!(item.same_item_same_components(&saved));

    let encode = |item: &ItemStack| {
        stack_to_network(|_| 0, |_, name| custom::id(name), &Some(item.clone())).unwrap()
    };
    let vanilla = stack_to_network(|_| 0, |_, _| None, &Some(saved.clone())).unwrap();
    assert!(vanilla.added.is_empty());
    assert!(vanilla.removed.is_empty());
    let stack = encode(&saved);
    assert!(!custom::permitted(&stack, false).unwrap());
    assert!(custom::permitted(&stack, true).unwrap());
    let mut removed = Stack::new(0, 1);
    removed.removed.push(FIRST_CUSTOM_ID + 1);
    assert!(!custom::permitted(&removed, false).unwrap());
    for kind in [
        component::USE_REMAINDER,
        component::POT_DECORATIONS,
        component::CONTAINER,
        component::SULFUR_CUBE_CONTENT,
    ] {
        let mut bytes = Writer::new();
        if kind == component::CONTAINER {
            bytes.varint(1);
        }
        if kind == component::POT_DECORATIONS || kind == component::CONTAINER {
            bytes.bool(true);
        }
        Template::write(&removed, &mut bytes);
        if kind == component::POT_DECORATIONS {
            for _ in 0..3 {
                bytes.bool(false);
            }
        }
        let mut outer = Stack::new(0, 1);
        outer
            .added
            .push((kind, horizon_protocol::item::Component::Raw(bytes.data)));
        assert!(!custom::permitted(&outer, false).unwrap(), "kind {kind}");
        assert!(custom::permitted(&outer, true).unwrap());
    }
    let hash = Hashed::create(&stack, &names);
    assert_eq!(hash.added.len(), 1);
    assert_eq!(hash.removed, [FIRST_CUSTOM_ID + 1]);
    assert_eq!(
        hash.added[0].1,
        horizon_protocol::item::Component::Nbt(value)
            .hash(FIRST_CUSTOM_ID, &names)
            .unwrap()
    );

    let mut writer = Writer::new();
    Slot::write(&Some(stack.clone()), &mut writer);
    let received = Slot::read(&mut Reader::new(&writer.data)).unwrap().unwrap();
    let client = item_sync::from_network(&received, &builtin, &dynamic);
    assert!(saved.same_item_same_components(&client));
    assert_eq!(hash, Hashed::create(&encode(&client), &names));

    let mut writer = Writer::new();
    DelimitedSlot::write(&Some(encode(&client)), &mut writer);
    let returned = DelimitedSlot::read(&mut Reader::new(&writer.data))
        .unwrap()
        .unwrap();
    assert!(
        saved.same_item_same_components(&item_sync::from_network(&returned, &builtin, &dynamic))
    );

    let mut writer = Writer::new();
    Template::write(&stack, &mut writer);
    let template: Stack = Template::read(&mut Reader::new(&writer.data)).unwrap();
    assert!(
        saved.same_item_same_components(&item_sync::from_network(&template, &builtin, &dynamic))
    );

    let mut bundle = Stack::new(0, 1);
    bundle.added.push((
        horizon_protocol::item::component::BUNDLE_CONTENTS,
        horizon_protocol::item::Component::Stacks(vec![stack]),
    ));
    assert!(!custom::permitted(&bundle, false).unwrap());
    let mut writer = Writer::new();
    Slot::write(&Some(bundle), &mut writer);
    let received = Slot::read(&mut Reader::new(&writer.data)).unwrap().unwrap();
    let received = item_sync::from_network(&received, &builtin, &dynamic);
    assert!(saved.same_item_same_components(&received.data().unwrap().bundle_contents[0]));

    let mut container = ItemStack::new("minecraft:stone", 1);
    container.data_mut().container.push((2, saved.clone()));
    let mut writer = Writer::new();
    Slot::write(&Some(encode(&container)), &mut writer);
    let received = Slot::read(&mut Reader::new(&writer.data)).unwrap().unwrap();
    let (added, removed) =
        horizon_protocol::item::WirePatch::decode(received.wire.0.as_ref().unwrap()).unwrap();
    let cached = Stack {
        added,
        removed,
        ..Stack::new(received.item, received.count)
    };
    assert!(!custom::permitted(&cached, false).unwrap());
    let received = item_sync::from_network(&received, &builtin, &dynamic);
    assert!(saved.same_item_same_components(&received.data().unwrap().container[0].1));

    let mut malformed = Writer::new();
    malformed.varint(1);
    malformed.varint(0);
    malformed.varint(1);
    malformed.varint(0);
    malformed.varint(FIRST_CUSTOM_ID);
    malformed.byte_array(&[0, 0]);
    assert!(DelimitedSlot::read(&mut Reader::new(&malformed.data)).is_err());
}
