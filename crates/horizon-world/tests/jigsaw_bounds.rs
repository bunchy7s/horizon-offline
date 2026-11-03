//! The depth and reach bounds `JigsawStructure`'s codec puts on a structure.
//! A structure carrying others is not registered by vanilla, so it must not
//! place pieces here either.
use horizon_world::worldgen::structure::start::{StructureKind, StructureRegistry};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn kind(fields: Value) -> StructureKind {
    let mut structure = json!({
        "type": "minecraft:jigsaw",
        "start_pool": "x:pool",
        "start_height": {"absolute": 64},
        "biomes": "#minecraft:is_overworld",
        "step": "surface_structures",
    });
    for (key, value) in fields.as_object().expect("an object of fields") {
        structure[key] = value.clone();
    }
    let structures = BTreeMap::from([("x:test".to_owned(), structure.to_string())]);
    let registry = StructureRegistry::new(&structures, &BTreeMap::new(), BTreeMap::new(), &|_| {
        Ok(BTreeSet::new())
    })
    .expect("the structure decodes");
    registry.structures.into_iter().next().unwrap().kind
}

fn places(fields: Value) -> bool {
    matches!(kind(fields), StructureKind::Jigsaw(_))
}

/// `JigsawStructure.CODEC` reads `size` with `Codec.intRange(0, 20)`.
#[test]
fn a_jigsaw_structures_size_stops_at_twenty() {
    assert!(places(json!({"size": 0})));
    assert!(places(json!({"size": 20})));
    // Absent, the field is read as a depth of 0, as before.
    assert!(places(json!({})));
    for bad in [
        json!({"size": 21}),
        json!({"size": 200}),
        json!({"size": -1}),
    ] {
        assert!(!places(bad.clone()), "{bad}");
    }
}

/// `JigsawStructure.MaxDistance` is `horizontal` `Codec.intRange(1, 128)`
/// beside `vertical` `ExtraCodecs.intRange(1, DimensionType.Y_SIZE)` (4064),
/// given either as one number for both or as an object of the two.
#[test]
fn a_jigsaw_structures_reach_stays_inside_vanillas() {
    for good in [
        json!({"max_distance_from_center": 1}),
        json!({"max_distance_from_center": 128}),
        json!({"max_distance_from_center": {"horizontal": 1, "vertical": 1}}),
        json!({"max_distance_from_center": {"horizontal": 128, "vertical": 4064}}),
        // Absent, the reach stays at the 80 by 80 it had.
        json!({}),
    ] {
        assert!(places(good.clone()), "{good}");
    }
    for bad in [
        json!({"max_distance_from_center": 0}),
        json!({"max_distance_from_center": 129}),
        json!({"max_distance_from_center": -8}),
        json!({"max_distance_from_center": {"horizontal": 0, "vertical": 100}}),
        json!({"max_distance_from_center": {"horizontal": 64, "vertical": 0}}),
        json!({"max_distance_from_center": {"horizontal": 128, "vertical": 4065}}),
    ] {
        assert!(!places(bad.clone()), "{bad}");
    }
}
