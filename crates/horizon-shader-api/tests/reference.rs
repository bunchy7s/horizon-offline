//! The stages' WGSL against the Rust definitions: the preludes compile, the
//! frame's WGSL layout is the uploaded struct's, and
//! docs/api/shaders-reference.md is up to date. Rewrite the reference with
//! `HORIZON_WRITE_DOCS=1 cargo test -p horizon-shader-api --test reference`.
use horizon_shader_api::{Frame, Pack, StageKind, compose, reference};
use std::path::Path;

const DOC: &str = "docs/api/shaders-reference.md";

fn root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn validate(text: &str) -> naga::Module {
    let module = naga::front::wgsl::parse_str(text)
        .unwrap_or_else(|e| panic!("{}\n{text}", e.emit_to_string(text)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .unwrap_or_else(|e| panic!("{}\n{text}", e.emit_to_string(text)));
    module
}

/// Vanilla's look, written as each stage.
fn vanilla(kind: StageKind) -> &'static str {
    match kind {
        StageKind::Sky => "fn sky(input: SkyInput) -> vec4<f32> { return input.color; }\n",
        StageKind::Material => {
            "fn material(input: MaterialInput) -> Surface {\n    \
             return Surface(input.albedo * input.tint * vec4<f32>(input.lightmap, 1.0));\n}\n"
        }
        StageKind::Vertex => {
            "fn vertex(input: VertexInput) -> vec3<f32> { return input.position; }\n"
        }
        StageKind::Weather => {
            "fn weather(input: WeatherInput) -> vec4<f32> { return input.color; }\n"
        }
        StageKind::Post => "fn post(input: PostInput) -> vec4<f32> { return input.color; }\n",
    }
}

#[test]
fn every_stage_compiles_with_vanilla_s_function() {
    for kind in StageKind::ALL {
        let text = format!("{}{}{}", kind.prelude(), vanilla(kind), kind.entries());
        let module = validate(&text);
        assert!(
            module
                .entry_points
                .iter()
                .all(|e| e.name.starts_with("horizon_")),
            "{kind:?}"
        );
    }
}

#[test]
fn the_frame_s_wgsl_layout_is_the_uploaded_struct_s() {
    let text = format!("{}{}", StageKind::Sky.prelude(), vanilla(StageKind::Sky));
    let module = validate(&text);
    let (_, ty) = module
        .types
        .iter()
        .find(|(_, t)| t.name.as_deref() == Some("Frame"))
        .expect("Frame in the prelude");
    let naga::TypeInner::Struct { members, span } = &ty.inner else {
        panic!("Frame is a struct")
    };
    assert_eq!(*span as usize, std::mem::size_of::<Frame>());
    let wgsl: Vec<_> = members
        .iter()
        .map(|m| (m.name.clone().unwrap(), m.offset as usize))
        .collect();
    let rust: Vec<_> = Frame::WGSL
        .fields
        .iter()
        .zip(Frame::OFFSETS)
        .map(|(f, &o)| (f.name.to_owned(), o))
        .collect();
    assert_eq!(wgsl, rust);
}

#[test]
fn the_sample_pack_s_stages_compile() {
    let dir = root().join("crates/horizon-shader-api/tests/samples/schema-1");
    let pack = Pack::read(&dir).unwrap();
    let m = &pack.manifest;
    let stages = [
        (StageKind::Sky, m.sky.iter().collect::<Vec<_>>()),
        (StageKind::Material, m.material.iter().collect()),
        (StageKind::Post, m.post.iter().collect()),
    ];
    for (kind, list) in stages {
        for stage in list {
            let composed = compose(&pack, kind, &stage.file);
            validate(&composed.text);
            // Errors point into the pack's own file.
            let part = composed.parts.iter().find(|(f, _)| *f == stage.file);
            let part = part.unwrap().1.clone();
            let needle = format!("fn {}(", kind.function());
            let at = part.start + composed.text[part].find(&needle).unwrap();
            let source = pack.source(&stage.file).unwrap();
            let line = source.lines().position(|l| l.starts_with(&needle)).unwrap() + 1;
            assert_eq!(composed.locate(at), (stage.file.as_str(), line, 1));
        }
    }
}

#[test]
fn the_reference_is_up_to_date() {
    let path = root().join(DOC);
    let want = reference::markdown();
    if std::env::var_os("HORIZON_WRITE_DOCS").is_some() {
        std::fs::write(&path, &want).unwrap();
    }
    let have = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        have == want,
        "{DOC} is out of date: HORIZON_WRITE_DOCS=1 cargo test -p horizon-shader-api --test reference"
    );
}
