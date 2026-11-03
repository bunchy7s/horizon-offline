//! The decode-time bounds vanilla's processor codecs check, which a data pack
//! that vanilla refuses must not get past here either.
use horizon_world::{
    DATA_VERSION,
    nbt::Tag,
    registry::BlockRegistry,
    worldgen::{
        block_feature::FeatureBlockBindings,
        structure::{
            StateTransforms, StructureResources,
            processor::{Processor, ProcessorList},
        },
    },
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

fn names() -> [&'static str; 4] {
    [
        "minecraft:air",
        "minecraft:cave_air",
        "minecraft:void_air",
        "minecraft:stone",
    ]
}

fn registry() -> &'static BlockRegistry {
    let names = names();
    let states = names
        .iter()
        .map(|name| Tag::compound([("id", Tag::String((*name).into()))]))
        .collect();
    let defaults = names
        .iter()
        .enumerate()
        .map(|(id, name)| ((*name).into(), Tag::Int(id as i32)))
        .collect();
    let registry = BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        ("states", Tag::List(states)),
        ("defaults", Tag::Compound(defaults)),
    ]))
    .unwrap();
    Box::leak(Box::new(registry))
}

/// A registry whose states rotate and mirror onto themselves, so a processor
/// list decodes without a prepared resource bundle.
fn resources() -> StructureResources {
    let registry = registry();
    let states = registry.len();
    let table = json!({
        "rotate": vec![json!([0, 0, 0]); states],
        "mirror": vec![json!([0, 0]); states],
        "full_block": vec![json!(false); states],
    });
    let transforms = StateTransforms::from_json(&table.to_string(), states).unwrap();
    StructureResources::new(
        registry,
        transforms,
        Path::new("."),
        BTreeMap::new(),
        &BTreeMap::new(),
    )
}

fn list(json: Value) -> Result<ProcessorList, horizon_world::worldgen::DecodeError> {
    let resources = resources();
    let bindings = FeatureBlockBindings::new(resources.registry, BTreeSet::new()).unwrap();
    ProcessorList::decode(&json.to_string(), &resources, &bindings)
}

/// A capping processor's `limit` is `IntProviders.POSITIVE_CODEC` in vanilla:
/// a provider whose range reaches 0 or below fails the pack, it is not run as
/// a processor that never keeps a block.
#[test]
fn a_capping_limit_must_be_positive() {
    let capped = |limit: Value| {
        json!({"processors": [{
            "processor_type": "minecraft:capped",
            "delegate": {"processor_type": "minecraft:nop"},
            "limit": limit,
        }]})
    };
    for bad in [
        json!(0),
        json!(-1),
        json!({"type": "minecraft:uniform", "min_inclusive": 0, "max_inclusive": 3}),
        json!({"type": "minecraft:biased_to_bottom", "min_inclusive": -2, "max_inclusive": 2}),
        json!({"type": "minecraft:weighted_list", "distribution": [
            {"data": {"type": "minecraft:constant", "value": 0}, "weight": 1}]}),
    ] {
        assert!(list(capped(bad.clone())).is_err(), "{bad}");
    }
    for good in [
        json!(1),
        json!(7),
        json!({"type": "minecraft:uniform", "min_inclusive": 1, "max_inclusive": 3}),
    ] {
        let decoded = list(capped(good.clone())).unwrap_or_else(|e| panic!("{good}: {e}"));
        let Processor::Capped { limit, .. } = &decoded.0[0] else {
            panic!("{good}: not a capping processor");
        };
        assert!(limit.bounds()[0] >= 1, "{good}");
    }
}

/// `BlockRotProcessor`'s `integrity` is `Codec.floatRange(0, 1)`: a fraction
/// outside it is refused at decode instead of keeping or rotting every block.
#[test]
fn block_rot_integrity_is_a_fraction() {
    let rot = |integrity: Value| json!({"processors": [{"processor_type": "minecraft:block_rot", "integrity": integrity}]});
    for bad in [
        json!(-1),
        json!(-0.01),
        json!(1.01),
        json!(2),
        json!("half"),
    ] {
        assert!(list(rot(bad.clone())).is_err(), "{bad}");
    }
    // The field is required, as a plain `fieldOf` is.
    assert!(list(json!({"processors": [{"processor_type": "minecraft:block_rot"}]})).is_err());
    for good in [json!(0), json!(0.5), json!(1)] {
        let decoded = list(rot(good.clone())).unwrap_or_else(|e| panic!("{good}: {e}"));
        let Processor::BlockRot { integrity, .. } = &decoded.0[0] else {
            panic!("{good}: not a block_rot processor");
        };
        assert!((0. ..=1.).contains(integrity), "{good}");
    }
}
