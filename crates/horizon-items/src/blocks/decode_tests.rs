use super::*;

#[test]
fn shape_identity_and_geometry_match_the_value_loader() -> crate::Result<()> {
    let inputs = [
        "[]",
        "[ ]",
        "[[0,0,0,1,1,1]]",
        "[ [0, 0, 0, 1, 1, 1] ]",
        "[[0.0,0,0,1,1,1]]",
        "[[0e0,0,0,1,1,1]]",
        "[[-0.0,0,0,1,1,1]]",
        "[[0,0,0,1,1,1,7]]",
        "[[0,0,0,1,1,1,8]]",
        "[[1,null,\"2\"],true,{}]",
        "[[1,null,\"\\u0032\"],true,{}]",
        "[{\"z\":0,\"a\":1}]",
        "[{\"a\":1,\"z\":0}]",
        "[[18446744073709551615,-9223372036854775808,1e20]]",
        "[{\"a\":2,\"a\":1,\"z\":0}]",
    ];
    let values = inputs
        .iter()
        .map(|s| serde_json::from_str::<Value>(s))
        .collect::<Result<Vec<_>, _>>()?;
    // Value equality merges signed zero; the legacy string keys do not.
    let keys: Vec<_> = values.iter().map(Value::to_string).collect();
    let mut shapes = Shapes::default();
    let actual = inputs
        .iter()
        .map(|s| {
            shapes.intern(Some(serde_json::from_str::<&RawValue>(s)?), || {
                panic!("array fallback")
            })
        })
        .collect::<crate::Result<Vec<_>>>()?;
    for (i, value) in values.iter().enumerate() {
        let expected: Vec<[f64; 6]> = value
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|b| {
                let b = b.as_array()?;
                Some(std::array::from_fn(|i| {
                    b.get(i).and_then(Value::as_f64).unwrap_or(0.0)
                }))
            })
            .collect();
        // Bit comparisons include negative zero.
        assert_eq!(
            actual[i]
                .iter()
                .flatten()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>(),
            expected
                .iter()
                .flatten()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>()
        );
        for j in 0..values.len() {
            assert_eq!(
                Arc::ptr_eq(&actual[i], &actual[j]),
                keys[i] == keys[j],
                "{i}, {j}"
            );
        }
    }
    for input in ["null", "false", "7", "{}", "\"shape\""] {
        let raw = serde_json::from_str(input)?;
        let a = shapes.intern(Some(raw), || vec![[7.0; 6]])?;
        let b = shapes.intern(Some(raw), || vec![[7.0; 6]])?;
        assert_eq!(&*a, &[[7.0; 6]]);
        assert!(!Arc::ptr_eq(&a, &b)); // Fallbacks were never interned.
    }
    Ok(())
}

#[test]
fn typed_fields_keep_defaults_duplicates_and_wrong_type_behavior() -> crate::Result<()> {
    let input = r#"{"states":[null,[],{"id":0,"id":2,"emission":4.5,"dampening":"15",
        "hardness":-0.0,"skylight_down":true,"offset":true,"outline_offset":"true",
        "item":42,"render_shape":false,"sound":{"volume":null,"pitch":2,"break":{"id":"a\u0062"},
        "step":false,"hit":{"id":2}},"outline":[[0,1]],"unknown":{"nested":true}}]}"#;
    let parsed: Gameplay<'_> = serde_json::from_str(input)?;
    let entries = parsed.states.0.unwrap();
    assert_eq!(entries[0].id, None);
    assert_eq!(entries[1].id, None);
    let e = &entries[2];
    assert_eq!(e.id, Some(2));
    assert_eq!(e.emission.as_f64(), Some(4.5));
    assert_eq!(e.dampening.as_f64().unwrap_or(0.0), 0.0);
    assert_eq!(e.hardness.as_f64().unwrap().to_bits(), (-0.0f64).to_bits());
    assert!(e.skylight_down && e.outline_offset.unwrap_or(e.offset));
    assert_eq!(e.item, None);
    assert_eq!(e.render_shape, None);
    assert_eq!((e.sound.volume, e.sound.pitch), (1.0, 2.0));
    assert_eq!(e.sound.r#break.id.as_deref(), Some("ab"));
    assert_eq!(e.sound.step.id, None);
    assert_eq!(e.sound.hit.id, None);
    assert_eq!(e.outline.unwrap().get(), "[[0,1]]");
    for input in ["null", "[]", "{}", r#"{"states":null}"#, r#"{"states":{}}"#] {
        let parsed: Gameplay<'_> = serde_json::from_str(input)?;
        assert!(parsed.states.0.is_none());
    }
    for input in [
        r#"{"states":["#,
        r#"{"states":[{"id":1e400}]}"#,
        "{} trailing",
        r#"{"unknown":1e400}"#,
    ] {
        let before = serde_json::from_str::<Value>(input).err().unwrap();
        let after = serde_json::from_str::<Gameplay<'_>>(input).err().unwrap();
        assert_eq!(before.classify(), after.classify());
    }
    Ok(())
}

#[test]
fn raw_shape_capture_keeps_original_json_errors() {
    let deep = format!("{}0{}", "[".repeat(130), "]".repeat(130));
    let inputs = [
        br#"{"states":[{"outline":[[1e400]],"outline":[]}]}"#.to_vec(),
        format!("{{\"states\":[{{\"outline\":{deep}}}]}}").into_bytes(),
        br#"{"states":[{"outline":[[0]]}]} trailing"#.to_vec(),
        b"{\"states\":[{\"outline\":[[\"\xff\"]]}]}".to_vec(),
    ];
    for input in inputs {
        let old = serde_json::from_slice::<Value>(&input).err().unwrap();
        let new = serde_json::from_slice::<Validate>(&input).err().unwrap();
        assert_eq!(
            (old.classify(), old.line(), old.column()),
            (new.classify(), new.line(), new.column())
        );
    }
}

#[test]
fn the_validation_error_still_fails_the_load_first() {
    // The typed decode reads this shape (raw), the validation pass rejects
    // its number, and the empty behaviour document would fail the load next.
    let facts = br#"{"states":[{"id":0,"outline":[[1e400]]}]}"#;
    let expected = serde_json::from_slice::<Value>(facts)
        .unwrap_err()
        .to_string();
    let root = std::env::temp_dir().join(format!("horizon-facts-validate-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("gameplay-facts.json"), facts).unwrap();
    let registry = horizon_world::registry::BlockRegistry::from_facts_json(
        &br#"{"states":[["minecraft:air",true,false,"","minecraft:air"]],"defaults":{"minecraft:air":0}}"#[..],
    )
    .unwrap();
    for parallel in [false, true] {
        let error = super::super::BlockFacts::load_with_projection(
            &root,
            &registry,
            &Value::Null,
            parallel,
        )
        .err()
        .unwrap();
        assert_eq!(error.to_string(), expected, "parallel: {parallel}");
    }
    std::fs::remove_dir_all(&root).unwrap();
}
