//! Original GLSL initializer-list controls; no shader-pack assets or algorithms.
use horizon_shaders::{Origin, Preprocessed, Target, translate_composite};

fn stage(code: &str) -> Preprocessed {
    Preprocessed {
        code: code.into(),
        files: vec!["original.fsh".into()],
        lines: code
            .lines()
            .enumerate()
            .map(|(i, _)| Origin {
                file: 0,
                line: i as u32 + 1,
            })
            .collect(),
        version: Some((440, Some("compatibility".into()))),
        ..Default::default()
    }
}

fn translated(fragment: &str) -> Result<horizon_shaders::Program, Vec<horizon_shaders::Error>> {
    translate_composite(
        &stage("void main() { gl_Position = gl_Vertex; }"),
        &stage(fragment),
        Target::Window,
    )
}

fn assert_count(fragment: &str, expected: i32) {
    let program = translated(fragment).unwrap_or_else(|e| panic!("{e:?}"));
    let count = program
        .fragment
        .constants
        .iter()
        .find_map(|(_, c)| (c.name.as_deref() == Some("count")).then_some(c))
        .expect("original count constant");
    assert!(
        matches!(program.fragment.global_expressions[count.init], naga::Expression::Literal(naga::Literal::I32(n)) if n == expected)
    );
    for module in [&program.vertex, &program.fragment] {
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(module)
        .unwrap();
        let words =
            naga::back::spv::write_vec(module, &info, &naga::back::spv::Options::default(), None)
                .unwrap();
        assert!(!words.is_empty());
    }
}

#[test]
fn unsized_global_brace_initializer_has_constant_length() {
    assert_count(
        "const int values[] = {2, 5, 8}; const int count = values.length(); void main() { gl_FragColor = vec4(float(count), float(values[2]), 0.0, 1.0); }",
        3,
    );
}

#[test]
fn sized_brace_initializer_control() {
    assert_count(
        "const int values[3] = {2, 5, 8}; const int count = values.length(); void main() { gl_FragColor = vec4(float(count), float(values[2]), 0.0, 1.0); }",
        3,
    );
}

#[test]
fn unsized_constructor_initializer_control() {
    assert_count(
        "const int values[] = int[](2, 5, 8); const int count = values.length(); void main() { gl_FragColor = vec4(float(count), float(values[2]), 0.0, 1.0); }",
        3,
    );
}

#[test]
fn trailing_comma_does_not_add_an_element() {
    assert_count(
        "const int values[] = {2, 5, 8,}; const int count = values.length(); void main() { gl_FragColor = vec4(float(count)); }",
        3,
    );
}

#[test]
fn nested_aggregate_elements_are_counted_once() {
    assert_count(
        "const ivec2 values[] = {{2, 5}, {8, 13}}; const int count = values.length(); void main() { gl_FragColor = vec4(float(count), vec2(values[1]), 1.0); }",
        2,
    );
}

#[test]
fn inner_constructors_do_not_supply_the_outer_length() {
    assert_count(
        "const ivec3 values[] = {ivec3(2, 5, 8), ivec3(13), ivec3(21, 34, 55), ivec3(89)}; const int count = values.length(); void main() { gl_FragColor = vec4(float(count)); }",
        4,
    );
}

#[test]
fn arrays_of_arrays_keep_the_outer_element_count() {
    assert_count(
        "const int values[][2] = {{1, 2}, {3, 4}, {5, 6}}; const int count = values.length(); void main() { gl_FragColor = vec4(float(count)); }",
        3,
    );
}

#[test]
fn local_brace_arrays_have_a_fixed_length() {
    let program = translated("void main() { const int values[] = {2, 5, 8}; gl_FragColor = vec4(float(values.length())); }").unwrap_or_else(|e| panic!("{e:?}"));
    assert!(program.fragment.types.iter().any(|(_, ty)| matches!(ty.inner, naga::TypeInner::Array { size: naga::ArraySize::Constant(n), .. } if n.get() == 3)));
}

#[test]
fn invalid_aggregate_initializers_still_fail() {
    for source in [
        "const int values[2] = {1, 2, 3}; void main() { gl_FragColor = vec4(float(values.length())); }",
        "const int values[] = {}; void main() { gl_FragColor = vec4(float(values.length())); }",
        "const int values[]; const int count = values.length(); void main() { gl_FragColor = vec4(float(count)); }",
    ] {
        assert!(
            translated(source).is_err(),
            "accepted invalid source: {source}"
        );
    }
}

#[test]
fn runtime_storage_array_remains_dynamically_sized() {
    let program = horizon_shaders::translate_compute(&stage("layout(local_size_x=1) in; layout(std430,binding=0) buffer Data { int values[]; }; void main() { if (values.length() > 0) values[0] = values.length(); }")).unwrap_or_else(|e| panic!("{e:?}"));
    assert!(program.module.types.iter().any(|(_, ty)| matches!(
        ty.inner,
        naga::TypeInner::Array {
            size: naga::ArraySize::Dynamic,
            ..
        }
    )));
}
