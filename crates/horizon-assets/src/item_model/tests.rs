use super::*;

#[test]
fn identifier_adds_default_namespace() {
    assert_eq!(identifier("stone").unwrap(), "minecraft:stone");
    assert_eq!(identifier("a:b").unwrap(), "a:b");
}

#[test]
fn positive_modulo_wraps_negatives() {
    assert!((positive_modulo(-0.25, 1.0) - 0.75).abs() < 1e-6);
    assert!((positive_modulo(1.5, 1.0) - 0.5).abs() < 1e-6);
}

#[test]
fn display_context_names_round_trip() {
    for name in ["gui", "ground", "fixed", "head"] {
        assert_eq!(DisplayContext::by_name(name).unwrap().name(), name);
    }
}

#[test]
fn a_mods_tint_source_is_untinted_and_an_unknown_vanilla_one_fails() {
    let tint = |json: &str| TintSource::decode(&serde_json::from_str(json).unwrap());
    assert_eq!(
        tint(r#"{"type":"ae2:storage_cell_state"}"#).unwrap(),
        TintSource::Constant(-1)
    );
    assert!(tint(r#"{"type":"minecraft:nonsense"}"#).is_err());
}
