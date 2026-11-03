//! Native component registration, value limits and content identity.
extern crate horizon_client_lib as horizon_client;
use horizon_client::{
    content_identity,
    mod_content::{ContentMod, accept_mod, build, known_components},
};
use horizon_protocol::{
    buf::NBT_QUOTA,
    item::custom::{self, InstallError},
};
use horizon_world::nbt::Tag;
use std::fs;

#[test]
fn registration_limits_and_defaults_are_bound_to_the_connection() {
    assert_eq!(
        custom::install(122, vec!["minecraft:no".into()]),
        Err(InstallError::InvalidName)
    );
    assert_eq!(
        custom::install(122, vec!["sample:x".into(), "sample:x".into()]),
        Err(InstallError::DuplicateName)
    );
    custom::install(122, vec!["sample:z".into(), "sample:a".into()]).unwrap();
    assert_eq!(custom::id("sample:a"), Some(122));
    assert_eq!(custom::id("sample:z"), Some(123));
    assert_eq!(
        custom::install(122, vec![]),
        Err(InstallError::AlreadyInstalled)
    );

    assert!(custom::validate(&Tag::Int(42)).is_ok());
    assert!(custom::validate(&Tag::End).is_err());
    assert!(custom::validate(&Tag::Double((-0.0_f64).to_bits())).is_err());
    assert!(custom::validate(&Tag::Float(0x7fc0_0001)).is_err());
    assert!(custom::validate(&Tag::Float(0x7fc0_0000)).is_ok());
    assert!(custom::validate(&Tag::ByteArray(vec![0; NBT_QUOTA])).is_err());
    let mut deep = Tag::Int(1);
    for _ in 0..600 {
        deep = Tag::List(vec![deep]);
    }
    assert!(custom::validate(&deep).is_err());
    assert!(custom::validate(&Tag::List(vec![Tag::Int(1), Tag::String("x".into())])).is_ok());

    let dir =
        std::env::temp_dir().join(format!("horizon-component-identity-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (name, value) in [("a", 1), ("b", 2), ("c", 1)] {
        let root = dir.join(name);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("facts.json"), br#"{"states":[]}"#).unwrap();
        std::fs::write(
            root.join("items.json"),
            format!(r#"{{"sample:item":{{"sample:a":{{"value":{value}}}}}}}"#),
        )
        .unwrap();
    }
    let a = content_identity::identity(&dir.join("a")).unwrap();
    let b = content_identity::identity(&dir.join("b")).unwrap();
    let c = content_identity::identity(&dir.join("c")).unwrap();
    assert_ne!(a, b);
    assert_eq!(a, c);
    let scratch = std::env::temp_dir().join(format!(
        "horizon-component-isolation-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&scratch);
    let make = |id: &str, patch: &str, block: bool| {
        let kind = if block { "block" } else { "item" };
        let dir = scratch.join(id);
        let path = dir.join(format!("data/{id}/horizon/{kind}/thing.json"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(dir.join("horizon-mod.toml"), format!("id = \"{id}\"\nname = \"{id}\"\nversion = \"1.0\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\nnamespaces = []\n")).unwrap();
        fs::write(
            path,
            format!(r#"{{"template":"minecraft:stone","components":{patch}}}"#),
        )
        .unwrap();
        let package = horizon_mods::Package::read(&dir).unwrap();
        ContentMod {
            id: id.into(),
            content: horizon_mods::Content::read(&dir, &package.manifest).unwrap(),
        }
    };
    let base = scratch.join("base");
    fs::create_dir_all(&base).unwrap();
    fs::write(
        base.join("items.json"),
        r#"{"minecraft:stone":{"minecraft:repairable":{}}}"#,
    )
    .unwrap();
    let known = known_components(&base).unwrap();
    let good = make(
        "good",
        r#"{"sample:a":{"value":5},"!minecraft:repairable":{}}"#,
        false,
    );
    let removal = make("remove", r#"{"!sample:a":{}}"#, true);
    let bad = make("bad", r#"{"missing:type":{}}"#, false);
    let bad_block = make("badblock", r#"{"!missing:type":{}}"#, true);
    let mut oversized = make("large", r#"{"sample:a":1}"#, false);
    oversized.content.items[0]
        .components
        .insert("sample:a".into(), serde_json::json!("x".repeat(2_097_153)));
    let mut accepted = Vec::new();
    for (candidate, valid) in [
        (good, true),
        (bad, false),
        (removal, true),
        (bad_block, false),
        (oversized, false),
    ] {
        assert_eq!(accept_mod(&mut accepted, candidate, &known).is_ok(), valid);
    }
    assert_eq!(
        accepted.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["good", "remove"]
    );
    fs::write(
        base.join("facts.json"),
        r#"{"states":[["minecraft:stone"]],"defaults":{"minecraft:stone":0}}"#,
    )
    .unwrap();
    fs::write(
        base.join("gameplay-facts.json"),
        r#"{"states":[{"item":"minecraft:stone"}]}"#,
    )
    .unwrap();
    let blockstates = base.join("assets/minecraft/blockstates");
    fs::create_dir_all(&blockstates).unwrap();
    fs::write(blockstates.join("stone.json"), "{}").unwrap();
    let out = scratch.join("out");
    build(&base, &accepted, &out).unwrap();
    let items: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("items.json")).unwrap()).unwrap();
    assert_eq!(
        items["good:thing"]["sample:a"],
        serde_json::json!({"value":5})
    );
    assert!(items["good:thing"].get("minecraft:repairable").is_none());
    assert!(items.get("bad:thing").is_none());
    let catalog =
        horizon_client::items::Items::catalog(&out, &horizon_world::loot::PackOverlay::NONE)
            .unwrap();
    assert_eq!(
        catalog.get("good:thing").unwrap().custom_components["sample:a"],
        Tag::compound([("value", Tag::Int(5))])
    );
    fs::remove_dir_all(scratch).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}
