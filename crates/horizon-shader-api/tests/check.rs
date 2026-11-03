//! `check`: the sample pack passes, and each kind of mistake is reported
//! with the pack's own file and line.
use horizon_shader_api::{MANIFEST, Pack, Problem, StageKind, check};
use std::path::Path;

fn sample() -> Pack {
    Pack::read(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/samples/schema-1"
    )))
    .unwrap()
}

/// The sample with `file` replaced by `text`, checked.
fn problems(file: &str, text: &str) -> Vec<Problem> {
    let mut files = sample().files;
    files.insert(file.into(), text.as_bytes().to_vec());
    check(&Pack::from_files(files).unwrap()).expect_err("should fail")
}

fn one(file: &str, text: &str, needle: &str) -> Problem {
    let p = problems(file, text);
    assert!(
        p.iter().any(|p| p.message.contains(needle)),
        "{needle:?} not in {p:#?}"
    );
    p.into_iter().find(|p| p.message.contains(needle)).unwrap()
}

const GRADE: &str = "post/grade.wgsl";

#[test]
fn the_sample_passes() {
    let pack = sample();
    let report = check(&pack).unwrap();
    let labels: Vec<_> = report.stages.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(
        labels,
        ["sky.wgsl", "Terrain", "Colour grade", "post/vignette.wgsl"]
    );
    assert!(report.stages.iter().all(|s| s.expressions > 0));
    let post = &report.stages[2];
    assert!(post.texture_reads >= 2, "the entry point samples and loads");
    let sha = pack.sha256().unwrap();
    assert_eq!(sha.len(), 64);
    assert_eq!(sha, sample().sha256().unwrap());
}

#[test]
fn a_syntax_error_points_at_its_line() {
    let p = one(
        GRADE,
        "override saturation: f32 = 1.1;\noverride tonemap: u32 = 1u;\nfn post(input: PostInput) -> vec4<f32> {\n    return input.color +;\n}\n",
        "expected",
    );
    assert_eq!((p.file.as_str(), p.line), (GRADE, 4), "{p}");
    assert!(p.to_string().starts_with("post/grade.wgsl:4:"), "{p}");
}

#[test]
fn the_stage_function_must_have_its_signature() {
    let header = "override saturation: f32 = 1.1;\noverride tonemap: u32 = 1u;\n";
    one(
        GRADE,
        &format!("{header}fn post(input: PostInput) -> vec3<f32> {{ return input.color.rgb; }}\n"),
        "`fn post(PostInput) -> vec3<f32>` should be `fn post(input: PostInput) -> vec4<f32>`",
    );
    one(
        GRADE,
        &format!("{header}fn grade(input: PostInput) -> vec4<f32> {{ return input.color; }}\n"),
        "defines no `fn post(input: PostInput) -> vec4<f32>`",
    );
}

#[test]
fn a_pack_declares_no_bindings_entry_points_or_horizon_names() {
    let body = "override saturation: f32 = 1.1;\noverride tonemap: u32 = 1u;\nfn post(input: PostInput) -> vec4<f32> { return input.color; }\n";
    one(
        GRADE,
        &format!("@group(3) @binding(9) var extra: texture_2d<f32>;\n{body}"),
        "declares the binding `extra`",
    );
    one(
        GRADE,
        &format!(
            "{body}@fragment fn main() -> @location(0) vec4<f32> {{ return vec4<f32>(1.0); }}\n"
        ),
        "declares the entry point `main`",
    );
    one(
        GRADE,
        &format!("{body}fn horizon_mine() {{}}\n"),
        "`horizon_mine`: names starting with horizon_",
    );
}

#[test]
fn overrides_match_the_options() {
    let post = "fn post(input: PostInput) -> vec4<f32> { return input.color; }\n";
    one(
        GRADE,
        &format!("override saturation: i32 = 1;\noverride tonemap: u32 = 1u;\n{post}"),
        "option \"saturation\" is a float, so its override is `f32`",
    );
    one(
        GRADE,
        &format!(
            "override saturation: f32 = 1.1;\noverride tonemap: u32 = 1u;\noverride extra: f32;\n{post}"
        ),
        "`override extra` has no default and no [[options]] entry",
    );
    let p = one(
        GRADE,
        &format!("override saturation: f32 = 1.1;\n{post}"),
        "option \"tonemap\": no stage declares `override tonemap: u32`",
    );
    assert_eq!(p.file, MANIFEST);
}

#[test]
fn validation_errors_point_into_the_pack() {
    // A compute-only barrier in a fragment stage passes the parser, not the validator.
    let p = one(
        GRADE,
        "override saturation: f32 = 1.1;\noverride tonemap: u32 = 1u;\n\
         fn post(input: PostInput) -> vec4<f32> {\n    \
         workgroupBarrier();\n    \
         return input.color;\n}\n",
        "forbidden at this stage",
    );
    assert_eq!(p.file, GRADE, "{p}");
}

#[test]
fn texture_reads_have_a_budget() {
    let reads: String = (0..70)
        .map(|i| format!("    c += textureLoad(previous, vec2<i32>({i}, 0), 0);\n"))
        .collect();
    one(
        GRADE,
        &format!(
            "override saturation: f32 = 1.1;\noverride tonemap: u32 = 1u;\nfn post(input: PostInput) -> vec4<f32> {{\n    var c = input.color;\n{reads}    return c;\n}}\n"
        ),
        "texture reads; a stage may write 64",
    );
}

#[test]
fn reads_follows_the_stage_function_through_its_calls() {
    let material = |text: &str| {
        // The sample's `[reflection] option` is an override of the material.
        let text = format!("override reflections: bool = true;\n{text}");
        let mut files = sample().files;
        files.insert("material.wgsl".into(), text.into_bytes());
        let report = check(&Pack::from_files(files).unwrap()).unwrap();
        let stage = report.stages.into_iter().find(|s| s.label == "Terrain");
        stage.unwrap()
    };
    let sample = material(
        "fn material(input: MaterialInput) -> Surface {\n    return Surface(input.albedo);\n}\n",
    );
    assert!(!sample.reads("reflection"));
    let mirror = material(
        "fn mirrored(uv: vec2<f32>) -> vec4<f32> {\n    return textureSampleLevel(reflection, reflection_sampler, uv, 0.0);\n}\nfn material(input: MaterialInput) -> Surface {\n    return Surface(mix(input.albedo, mirrored(input.screen), 0.5));\n}\n",
    );
    assert!(mirror.reads("reflection"));
    assert!(mirror.reads("reflection_sampler"));
    assert!(!mirror.reads("opaque_color"));
}

/// A stage's `files` come before its own file, and no other stage sees them.
#[test]
fn a_stages_files_are_its_own() {
    let mut pack = sample();
    pack.files.insert(
        "only_post.wgsl".into(),
        b"fn only_post() -> f32 { return 1.0; }\n".to_vec(),
    );
    pack.manifest.post[0].files.push("only_post.wgsl".into());
    let file = pack.manifest.post[0].file.clone();
    let post = horizon_shader_api::compose(&pack, horizon_shader_api::StageKind::Post, &file);
    let names: Vec<_> = post.parts.iter().map(|p| p.0.as_str()).collect();
    let (extra, own) = (
        names.iter().position(|n| *n == "only_post.wgsl").unwrap(),
        names.iter().position(|n| *n == file).unwrap(),
    );
    assert!(extra < own, "{names:?}");
    let material = pack.manifest.material.clone().unwrap().file;
    let other =
        horizon_shader_api::compose(&pack, horizon_shader_api::StageKind::Material, &material);
    assert!(other.parts.iter().all(|p| p.0 != "only_post.wgsl"));
    assert!(pack.manifest.files().contains(&"only_post.wgsl"));
    check(&pack).unwrap();
}

#[test]
fn a_vertex_stage_checks_alone_and_joins_the_material() {
    let mut files = sample().files;
    let manifest = String::from_utf8(files[MANIFEST].clone()).unwrap();
    files.insert(
        MANIFEST.into(),
        format!("{manifest}\n[vertex]\nfile = \"wave.wgsl\"\n").into_bytes(),
    );
    files.insert(
        "wave.wgsl".into(),
        b"fn vertex(input: VertexInput) -> vec3<f32> {
    return input.position + vec3(0.0, sin(frame.time) * f32(input.material), 0.0);
}
"
        .to_vec(),
    );
    let report = check(&Pack::from_files(files).unwrap()).unwrap();
    let source = |kind| {
        let stage = report.stages.iter().find(|s| s.kind == kind).unwrap();
        stage.source.contains("fn vertex(input: VertexInput)")
    };
    assert!(source(StageKind::Vertex));
    assert!(source(StageKind::Material));
}

#[test]
fn a_material_reads_its_block_texture() {
    let mut files = sample().files;
    let pack = Pack::from_files(files.clone()).unwrap();
    let file = pack.manifest.material.as_ref().unwrap().file.clone();
    files.insert(
        file,
        b"override reflections: bool = true;
fn material(input: MaterialInput) -> Surface {
    let lod = block_texture_lod() + block_texture_size().x;
    return Surface(block_texture(input.uv) + block_texture_level(input.uv, lod));
}
"
        .to_vec(),
    );
    check(&Pack::from_files(files).unwrap()).unwrap();
}

#[test]
fn only_a_material_that_calls_set_surface_needs_its_target() {
    let calls = |pack: &Pack| {
        let report = check(pack).unwrap();
        let material = report.stages.iter().find(|s| s.kind == StageKind::Material);
        horizon_shader_api::stages::calls_set_surface(&material.unwrap().source)
    };
    let mut files = sample().files;
    let pack = Pack::from_files(files.clone()).unwrap();
    assert!(!calls(&pack));
    let material = pack.manifest.material.as_ref().unwrap().file.clone();
    files.insert(
        material,
        b"override reflections: bool = true;
fn material(input: MaterialInput) -> Surface {
    set_surface(input.normal, 0.5, 7.0);
    return Surface(input.albedo);
}
"
        .to_vec(),
    );
    assert!(calls(&Pack::from_files(files).unwrap()));
}
