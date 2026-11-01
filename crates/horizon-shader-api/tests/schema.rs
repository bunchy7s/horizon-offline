//! Schema 1 of `horizon-shaders.toml` and the `.hshader` package.
use horizon_shader_api::{MANIFEST, Manifest, OptionKind, Pack, PageItem};
use std::{collections::BTreeMap, path::Path};

fn sample_dir() -> &'static Path {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/samples/schema-1"
    ))
}

fn sample_text() -> String {
    std::fs::read_to_string(sample_dir().join(MANIFEST)).unwrap()
}

fn refused(text: &str) -> String {
    Manifest::parse(text).expect_err("should be refused")
}

#[test]
fn reads_every_key_of_schema_1() {
    let m = Manifest::parse(&sample_text()).unwrap();
    assert_eq!(
        (
            m.id.as_str(),
            m.name.as_str(),
            m.version.as_str(),
            m.api.as_str()
        ),
        ("sample_pack", "Sample Pack", "1.0.0", "0.1")
    );
    assert_eq!(m.authors, ["Horizon"]);
    assert_eq!(m.license.as_deref(), Some("MIT"));
    assert_eq!(m.sky.as_ref().unwrap().file, "sky.wgsl");
    assert_eq!(
        m.material.as_ref().unwrap().name.as_deref(),
        Some("Terrain")
    );
    let post: Vec<_> = m.post.iter().map(|s| s.file.as_str()).collect();
    assert_eq!(post, ["post/grade.wgsl", "post/vignette.wgsl"]);
    assert_eq!(
        m.files(),
        [
            "common.wgsl",
            "sky.wgsl",
            "material.wgsl",
            "post/grade.wgsl",
            "post/vignette.wgsl"
        ]
    );
    let kinds: Vec<_> = m.options.iter().map(|o| (o.id.as_str(), &o.kind)).collect();
    assert_eq!(
        kinds,
        [
            (
                "saturation",
                &OptionKind::Float {
                    default: 1.1,
                    min: 0.0,
                    max: 2.0,
                    step: 0.05
                }
            ),
            ("reflections", &OptionKind::Bool { default: true }),
            ("vignette", &OptionKind::Bool { default: true }),
            (
                "samples",
                &OptionKind::Int {
                    default: 4,
                    min: 1,
                    max: 16
                }
            ),
            (
                "tonemap",
                &OptionKind::Choice {
                    values: vec!["None".into(), "Soft".into(), "Filmic".into()],
                    default: 1
                }
            ),
        ]
    );
    let types: Vec<_> = m.options.iter().map(|o| o.kind.wgsl_type()).collect();
    assert_eq!(types, ["f32", "bool", "bool", "i32", "u32"]);
    let constants: Vec<_> = m
        .options
        .iter()
        .map(|o| o.kind.default_constant())
        .collect();
    assert_eq!(constants, [f64::from(1.1f32), 1.0, 1.0, 4.0, 1.0]);
    assert_eq!(m.tags.len(), 1);
    assert_eq!(m.tags[0].id, "glass");
    assert_eq!(m.budget.gpu_ms, Some(1.5));
    let option = m.reflection.as_ref().and_then(|r| r.option.as_deref());
    assert_eq!(option, Some("reflections"));
    let shadow = m.shadow.as_ref().unwrap();
    assert_eq!((shadow.resolution, shadow.distance), (1024, 96.0));
}

#[test]
fn a_shadow_map_stays_in_range_and_needs_a_stage_to_read_it() {
    let refused = |shadow: &str, needle: &str| {
        let mut text = sample_text();
        text = text.replace("resolution = 1024\ndistance = 96.0", shadow);
        text = text.replace("resolution = 1024\r\ndistance = 96.0", shadow);
        let e = Manifest::parse(&text).unwrap_err().to_string();
        assert!(e.contains(needle), "{e}");
    };
    refused("resolution = 64", "shadow.resolution");
    refused("distance = 8.0", "shadow.distance");
    let m = Manifest::parse(
        "schema = 1\nid = \"a\"\nname = \"A\"\nversion = \"1\"\napi = \"0.1\"\n\
         [sky]\nfile = \"s.wgsl\"\n[shadow]\n",
    );
    assert!(m.unwrap_err().to_string().contains("[shadow] needs"));
}

#[test]
fn material_ids_go_by_the_first_entry_and_may_cut_shadows_whole() {
    let manifest = |rest: &str| {
        Manifest::parse(&format!(
            "schema = 1\nid = \"a\"\nname = \"A\"\nversion = \"1\"\napi = \"0.1\"\n\
             [material]\nfile = \"m.wgsl\"\n{rest}"
        ))
    };
    let m = manifest(
        "[shadow]\nsolid_materials = [7]\n\
         [[materials]]\nid = 7\nsprites = [\"block/oak_leaves\", \"block/*_*_leaves\"]\n\
         [[materials]]\nid = 8\nsprites = [\"block/*_leaves\", \"mod:*\"]\n",
    )
    .unwrap();
    assert_eq!(m.shadow.as_ref().unwrap().solid_materials, [7]);
    let sprites = [
        ("minecraft:block/dark_oak_leaves", 1),
        ("minecraft:block/birch_leaves", 2),
        // One layer under two names, in either order: the earlier entry decides.
        ("minecraft:block/birch_leaves", 3),
        ("minecraft:block/oak_leaves", 3),
        ("mod:anything", 4),
        ("minecraft:block/stone", 5),
    ];
    assert_eq!(
        m.sprite_materials(sprites),
        [(1, 7), (2, 8), (3, 7), (4, 8)]
    );

    let refused = |rest: &str, needle: &str| {
        let e = manifest(rest).unwrap_err().to_string();
        assert!(e.contains(needle), "{e}");
    };
    refused("[[materials]]\nid = 0\nsprites = [\"a\"]\n", "1..=65535");
    refused(
        "[[materials]]\nid = 65536\nsprites = [\"a\"]\n",
        "1..=65535",
    );
    refused("[[materials]]\nid = 1\nsprites = []\n", "needs sprites");
    refused(
        "[shadow]\nsolid_materials = [3]\n",
        "no [[materials]] entry",
    );

    // Without a material stage, only the shadow pass can read the ids.
    let post_only = |rest: &str| {
        Manifest::parse(&format!(
            "schema = 1\nid = \"a\"\nname = \"A\"\nversion = \"1\"\napi = \"0.1\"\n\
             [[post]]\nfile = \"p.wgsl\"\n[[materials]]\nid = 7\nsprites = [\"a\"]\n{rest}"
        ))
    };
    assert!(post_only("[shadow]\nsolid_materials = [7]\n").is_ok());
    assert!(
        post_only("")
            .unwrap_err()
            .to_string()
            .contains("materials are read")
    );
}

#[test]
fn sprite_tags_name_sprites_with_one_wildcard() {
    let mut m = Manifest::parse(&sample_text()).unwrap();
    m.tags.push(horizon_shader_api::SpriteTag {
        id: "ice".into(),
        sprites: vec!["minecraft:block/*ice".into()],
    });
    let sprites = [
        ("minecraft:block/stone", 0),
        ("minecraft:block/glass", 1),
        ("minecraft:block/red_stained_glass", 2),
        ("minecraft:block/red_stained_glass_pane_top", 3),
        ("minecraft:block/packed_ice", 4),
        ("minecraft:block/ice", 5),
    ];
    assert_eq!(
        m.sprite_tags(sprites.iter().rev().copied()),
        [(1, 1), (2, 1), (4, 2), (5, 2)]
    );
    // Two names on one layer (identical stills share it) give one entry, tags merged.
    let shared = [
        ("minecraft:block/glass", 7),
        ("minecraft:block/packed_ice", 7),
    ];
    assert_eq!(m.sprite_tags(shared), [(7, 3)]);
}

#[test]
fn the_smallest_manifest() {
    let m = Manifest::parse(
        "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.1\"\n[[post]]\nfile = \"a.wgsl\"\n",
    )
    .unwrap();
    assert!(m.sky.is_none() && m.options.is_empty() && m.budget.gpu_ms.is_none());
}

#[test]
fn refuses_what_this_build_does_not_read() {
    let text = sample_text();
    let cases = [
        (text.replace("schema = 1", "schema = 2"), "schema 2"),
        (
            text.replace("api = \"0.1\"", "api = \"0.2\""),
            "shader API 0.2",
        ),
        (
            text.replace("\"sample_pack\"", "\"Sample\""),
            "is not [a-z0-9_]",
        ),
        (
            text.replace("license = \"MIT\"", "licence = \"MIT\""),
            "unknown field",
        ),
        (
            text.replace(
                "[\"minecraft:block/glass\", \"block/*_stained_glass\"]",
                "[]",
            ),
            "needs sprites",
        ),
        (
            text.replace("\"block/*_stained_glass\"", "\"block/*_*\""),
            "at most one *",
        ),
        (
            text.replace("gpu_ms = 1.5", "gpu_ms = 40.0"),
            "budget.gpu_ms",
        ),
        (
            text.replace("\"common.wgsl\"", "\"../common.wgsl\""),
            "not a plain path",
        ),
        (
            text.replace("\"sky.wgsl\"", "\"sky.glsl\""),
            "not a plain path to a .wgsl",
        ),
        (
            text.replace("id = \"samples\"", "id = \"vignette\""),
            "declared twice",
        ),
        (
            text.replace("id = \"samples\"", "id = \"horizon_samples\""),
            "WGSL identifier",
        ),
        (
            text.replace("default = \"Soft\"", "default = \"Hard\""),
            "one of the values",
        ),
        (
            text.replace("default = 1.1", "default = 3.0"),
            "default in [0, 2]",
        ),
        (
            text.replace("default = 4", "default = 4.5"),
            "not a 32-bit integer",
        ),
        (
            text.replace("type = \"bool\"", "type = \"color\""),
            "is not bool",
        ),
        (
            text.replace("default = true", "default = true\nmin = 0"),
            "doesn't apply",
        ),
        (
            text.replace("option = \"reflections\"", "option = \"mirror\""),
            "reflection.option: `mirror` is no option",
        ),
    ];
    for (text, needle) in cases {
        let e = refused(&text);
        assert!(e.contains(needle), "{needle:?} not in {e:?}");
        assert!(e.starts_with(MANIFEST), "{e}");
    }
    let no_stage = "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1\"\napi = \"0.1\"\n";
    assert!(refused(no_stage).contains("no stage"));
    let mirror = format!("{no_stage}[sky]\nfile = \"sky.wgsl\"\n[reflection]\n");
    assert!(refused(&mirror).contains("[reflection] needs [material]"));
    let many = (0..9).map(|i| format!("[[post]]\nfile = \"p{i}.wgsl\"\n"));
    assert!(refused(&format!("{no_stage}{}", many.collect::<String>())).contains("9 post passes"));
}

#[test]
fn reads_the_sample_folder_and_its_package_alike() {
    let folder = Pack::read(sample_dir()).unwrap();
    assert_eq!(folder.files.len(), 7);
    assert!(
        folder
            .source("post/grade.wgsl")
            .unwrap()
            .contains("fn post")
    );
    let bytes = folder.package().unwrap();
    assert_eq!(
        bytes,
        folder.package().unwrap(),
        "packaging is reproducible"
    );
    let dir = std::env::temp_dir().join(format!("horizon-shader-api-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("sample_pack.hshader");
    std::fs::write(&path, &bytes).unwrap();
    let packaged = Pack::read(&path).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(packaged.files, folder.files);
    assert_eq!(packaged.manifest, folder.manifest);
}

#[test]
fn refuses_files_a_pack_does_not_use() {
    let base = || -> BTreeMap<String, Vec<u8>> { Pack::read(sample_dir()).unwrap().files };
    let mut files = base();
    files.insert("old.wgsl".into(), b"fn f() {}".to_vec());
    assert!(
        Pack::from_files(files)
            .unwrap_err()
            .contains("old.wgsl is not used")
    );
    let mut files = base();
    files.insert("run.sh".into(), Vec::new());
    assert!(
        Pack::from_files(files)
            .unwrap_err()
            .contains("run.sh: a pack holds only")
    );
    let mut files = base();
    files.remove("sky.wgsl");
    assert!(
        Pack::from_files(files)
            .unwrap_err()
            .contains("sky.wgsl: named in")
    );
    let mut files = base();
    files.insert("sky.wgsl".into(), vec![0xff, 0xfe]);
    assert!(Pack::from_files(files).unwrap_err().contains("not UTF-8"));
    let mut files = base();
    files.insert("post/grade.wgsl".into(), vec![b' '; 300 << 10]);
    assert!(
        Pack::from_files(files)
            .unwrap_err()
            .contains("over 256 KiB")
    );
    let mut files = base();
    files.remove(MANIFEST);
    assert!(
        Pack::from_files(files)
            .unwrap_err()
            .contains("missing at the pack's root")
    );
}

#[test]
fn fog_is_a_key_of_the_material_stage() {
    let with_fog = |stage: &str| {
        format!(
            "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1.0.0\"\napi = \"0.1\"\n\n{stage}\nfile = \"a.wgsl\"\nfog = false\n"
        )
    };
    let material = horizon_shader_api::Manifest::parse(&with_fog("[material]")).unwrap();
    assert_eq!(material.material.unwrap().fog, Some(false));
    for stage in ["[sky]", "[[post]]"] {
        let e = horizon_shader_api::Manifest::parse(&with_fog(stage)).unwrap_err();
        assert!(e.contains("`fog`"), "{e}");
    }
}

#[test]
fn a_sky_can_keep_or_replace_minecrafts_clouds_sun_and_stars() {
    let manifest = |stages: &str| {
        Manifest::parse(&format!(
            "schema = 1\nid = \"a\"\nname = \"A\"\nversion = \"1\"\napi = \"0.1\"\n{stages}"
        ))
    };
    let m =
        manifest("[sky]\nfile = \"s.wgsl\"\nclouds = false\nsun = false\nstars = true\n").unwrap();
    let sky = m.sky.unwrap();
    assert_eq!(
        (sky.clouds, sky.sun, sky.stars),
        (Some(false), Some(false), Some(true))
    );
    let m = manifest("[sky]\nfile = \"s.wgsl\"\n").unwrap();
    assert_eq!(m.sky.unwrap().sun, None);
    for key in ["clouds", "sun", "stars"] {
        let e = manifest(&format!("[material]\nfile = \"m.wgsl\"\n{key} = false\n"));
        assert!(e.unwrap_err().to_string().contains("keys of [sky] only"));
    }
}

#[test]
fn reads_settings_pages_and_refuses_broken_links() {
    let pages = "[[pages]]\nid = \"main\"\nname = \"Sample\"\nitems = [\"saturation\", \"\", \"page:more\"]\n\
                 [[pages]]\nid = \"more\"\nname = \"More\"\ndescription = \"The rest.\"\ncolumns = 1\n\
                 items = [\"tonemap\", \"samples\", \"vignette\", \"reflections\"]\n";
    let m = Manifest::parse(&format!("{}{pages}", sample_text())).unwrap();
    assert_eq!(m.pages.len(), 2);
    assert_eq!(
        m.pages[0].items,
        [PageItem::Option(0), PageItem::Empty, PageItem::Page(1)]
    );
    assert_eq!(
        (m.pages[1].columns, m.pages[1].description.as_str()),
        (1, "The rest.")
    );
    assert_eq!(
        m.pages[1].items,
        [
            PageItem::Option(4),
            PageItem::Option(3),
            PageItem::Option(2),
            PageItem::Option(1)
        ]
    );
    // Without pages, the settings show every option on one.
    assert!(Manifest::parse(&sample_text()).unwrap().pages.is_empty());
    let cases = [
        (
            pages.replace("\"page:more\"", "\"page:less\""),
            "no page \"less\"",
        ),
        (
            pages.replace("\"page:more\"", "\"page:main\""),
            "links to itself",
        ),
        (
            pages.replace("\"tonemap\"", "\"tone\""),
            "no option \"tone\"",
        ),
        (
            pages.replace("\"samples\",", "\"tonemap\","),
            "on the page twice",
        ),
        (pages.replace("columns = 1", "columns = 3"), "1 or 2"),
        (
            pages.replace("id = \"more\"", "id = \"main\""),
            "declared twice",
        ),
        (
            pages.replace(", \"reflections\"]", "]"),
            "option \"reflections\" is on no page",
        ),
        (
            pages.replace("\"page:more\"", "\"\""),
            "page \"more\" can't be reached",
        ),
    ];
    for (pages, needle) in cases {
        let e = refused(&format!("{}{pages}", sample_text()));
        assert!(e.contains(needle), "{needle:?} not in {e:?}");
    }
}

#[test]
fn a_weather_stage_is_a_stage_of_its_own() {
    let m = Manifest::parse(
        "schema = 1\nid = \"a\"\nname = \"A\"\nversion = \"1\"\napi = \"0.1\"\n\
         [weather]\nfile = \"w.wgsl\"\n",
    )
    .unwrap();
    assert_eq!(m.weather.as_ref().unwrap().file, "w.wgsl");
    assert_eq!(m.files(), ["w.wgsl"]);
}

fn png_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&[255, 0, 0, 0, 255, 0]).unwrap();
    writer.finish().unwrap();
    bytes
}

#[test]
fn a_pack_texture_is_declared_for_every_stage_and_decoded() {
    let head = "schema = 1\nid = \"a\"\nname = \"A\"\nversion = \"1\"\napi = \"0.1\"\n\
                [sky]\nfile = \"s.wgsl\"\n";
    let texture = "[[textures]]\nid = \"noise\"\nfile = \"noise.png\"\nwrap = \"clamp\"\n";
    let dir = std::env::temp_dir().join(format!("horizon-pack-texture-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(MANIFEST), format!("{head}{texture}")).unwrap();
    std::fs::write(
        dir.join("s.wgsl"),
        "fn sky(input: SkyInput) -> vec4<f32> {\n    \
         return textureSampleLevel(noise, noise_sampler, input.direction.xy, 0.0);\n}\n",
    )
    .unwrap();
    // Named but missing, then present.
    assert!(Pack::read(&dir).unwrap_err().contains("noise.png"));
    std::fs::write(dir.join("noise.png"), png_bytes()).unwrap();
    let pack = Pack::read(&dir).unwrap();
    let t = &pack.manifest.textures[0];
    assert!(t.linear && !t.repeat);
    let report = horizon_shader_api::check(&pack).unwrap();
    let image = &report.textures[0].1;
    assert_eq!((image.width, image.height), (2, 1));
    assert_eq!(image.rgba, [255, 0, 0, 255, 0, 255, 0, 255]);
    // A file that is no PNG is the check's refusal, not a panic.
    std::fs::write(dir.join("noise.png"), b"nope").unwrap();
    let pack = Pack::read(&dir).unwrap();
    assert!(horizon_shader_api::check(&pack).is_err());
    std::fs::remove_dir_all(&dir).unwrap();

    let refused = |rest: &str, needle: &str| {
        let e = Manifest::parse(&format!("{head}{rest}"))
            .unwrap_err()
            .to_string();
        assert!(e.contains(needle), "{e}");
    };
    refused(&texture.replace("noise.png", "noise.jpg"), ".png");
    refused(&format!("{texture}{texture}"), "declared twice");
    refused(&texture.replace("clamp", "mirror"), "repeat or clamp");
    refused(
        &format!("{texture}filter = \"cubic\"\n"),
        "linear or nearest",
    );
}

#[test]
fn a_block_takes_the_colour_of_the_first_light_that_names_it() {
    let manifest = |rest: &str| {
        Manifest::parse(&format!(
            "schema = 1\nid = \"a\"\nname = \"A\"\nversion = \"1\"\napi = \"0.1\"\n\
             [material]\nfile = \"m.wgsl\"\n{rest}"
        ))
    };
    let m = manifest(
        "[[lights]]\ncolor = [0.2, 0.8, 1.0]\nblocks = [\"soul_*\"]\n\
         [[lights]]\ncolor = [1.0, 0.5, 0.1]\nblocks = [\"*torch\", \"mod:*_*\"]\n",
    )
    .unwrap();
    assert_eq!(m.light_of("minecraft:soul_torch"), Some([0.2, 0.8, 1.0]));
    assert_eq!(m.light_of("minecraft:wall_torch"), Some([1.0, 0.5, 0.1]));
    assert_eq!(m.light_of("mod:glow_stick"), Some([1.0, 0.5, 0.1]));
    assert_eq!(m.light_of("mod:glow"), None);
    assert_eq!(m.light_of("minecraft:stone"), None);
    let refused = |rest: &str, needle: &str| {
        let e = manifest(rest).unwrap_err().to_string();
        assert!(e.contains(needle), "{e}");
    };
    refused(
        "[[lights]]\ncolor = [5.0, 0.0, 0.0]\nblocks = [\"a\"]\n",
        "0..=4",
    );
    refused(
        "[[lights]]\ncolor = [1.0, 0.0, 0.0]\nblocks = []\n",
        "needs blocks",
    );
    let e = Manifest::parse(
        "schema = 1\nid = \"a\"\nname = \"A\"\nversion = \"1\"\napi = \"0.1\"\n\
         [[post]]\nfile = \"p.wgsl\"\n[[lights]]\ncolor = [1.0, 0.0, 0.0]\nblocks = [\"a\"]\n",
    );
    assert!(e.unwrap_err().to_string().contains("give it a [material]"));
}

#[test]
fn post_outputs_are_named_scaled_and_never_the_last_pass() {
    let head = "schema = 1\nid = \"p\"\nname = \"P\"\nversion = \"1.0.0\"\napi = \"0.1\"\n";
    let pack = |passes: &str| horizon_shader_api::Manifest::parse(&format!("{head}{passes}"));
    let good = pack(
        "[[post]]\nfile = \"a.wgsl\"\noutput = \"bloom\"\nscale = 0.5\n\
         [[post]]\nfile = \"b.wgsl\"\n",
    )
    .unwrap();
    assert_eq!(good.post_targets(), [("bloom", 0.5)]);
    for (passes, why) in [
        ("[[post]]\nfile = \"a.wgsl\"\noutput = \"bloom\"\n", "last"),
        (
            "[[post]]\nfile = \"a.wgsl\"\nscale = 0.5\n[[post]]\nfile = \"b.wgsl\"\n",
            "output",
        ),
        (
            "[[post]]\nfile = \"a.wgsl\"\noutput = \"b\"\nscale = 0.3\n[[post]]\nfile = \"b.wgsl\"\n",
            "scale",
        ),
        (
            "[[post]]\nfile = \"a.wgsl\"\noutput = \"scene_color\"\n[[post]]\nfile = \"b.wgsl\"\n",
            "free",
        ),
        (
            "[material]\nfile = \"a.wgsl\"\noutput = \"b\"\n",
            "[[post]]",
        ),
    ] {
        let e = pack(passes).unwrap_err();
        assert!(e.contains(why), "{passes}: {e}");
    }
}
