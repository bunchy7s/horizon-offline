//! Original regression guard: each HDR post draw fits, so its frame clears must also fit.
use super::*;

fn hdr_pack(sized: bool, format: &str) -> Post {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut add = |name: &str, source: &str| {
        zip.start_file(
            format!("shaders/{name}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(source.as_bytes()).unwrap();
    };
    let mut properties = String::new();
    for buffer in 2..7 {
        let program = if buffer == 2 {
            "composite".to_owned()
        } else {
            format!("composite{}", buffer - 2)
        };
        add(&format!("{program}.vsh"), VERTEX);
        add(
            &format!("{program}.fsh"),
            &format!(
                "#version 120\n/* RENDERTARGETS:{buffer} */\n/* const int colortex{buffer}Format={format}; */\nvoid main() {{ gl_FragData[0]=vec4(0,0.5,0,1); }}\n"
            ),
        );
        if sized {
            properties.push_str(&format!("size.buffer.colortex{buffer}=0.5 0.5\n"));
        }
    }
    add("final.vsh", VERTEX);
    add(
        "final.fsh",
        "#version 120\nuniform sampler2D colortex2;\nvarying vec2 uv;\nvoid main() { gl_FragColor=texture2D(colortex2,uv); }\n",
    );
    add("shaders.properties", &properties);
    let pack = horizon_shaders::Pack::from_zip_bytes(
        "original-hdr-clear-budget",
        zip.finish().unwrap().into_inner(),
    )
    .unwrap();
    let mut macros = horizon_shaders::Macros::standard();
    let (properties, _) = horizon_shaders::read_properties(&pack, &mut macros);
    let post = horizon_shaders::load_post(&pack, "", &properties, &macros, &Default::default());
    assert_eq!(post.programs.len(), 6, "{:?}", post.notes);
    let expected = horizon_shaders::Format {
        channels: 4,
        bits: if format == "RGBA16F" { 16 } else { 8 },
        kind: if format == "RGBA16F" {
            horizon_shaders::Kind::Float
        } else {
            horizon_shaders::Kind::Unorm
        },
    };
    for buffer in 2..7 {
        assert_eq!(
            post.config.buffers[buffer].format, expected,
            "fixture buffer {buffer} must have {format}"
        );
    }
    post
}

fn hdr_clear_case(sized: bool, attachment_limit: u32, byte_budget: u32, format: &str) {
    let instance = crate::gfx::test_gpu::instance();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        eprintln!("HDR clear-budget test skipped: no GPU adapter");
        return;
    };
    let mut limits = adapter.limits();
    limits.max_color_attachment_bytes_per_sample = limits
        .max_color_attachment_bytes_per_sample
        .min(byte_budget);
    limits.max_color_attachments = limits.max_color_attachments.min(attachment_limit);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: limits,
        ..Default::default()
    }))
    .expect("GPU device for HDR clear-budget contract");
    eprintln!(
        "HDR clear-budget adapter {:?}, requested attachments {} / bytes {}",
        adapter.get_info(),
        device.limits().max_color_attachments,
        device.limits().max_color_attachment_bytes_per_sample
    );
    let gpu = Fixture {
        adapter,
        device,
        queue,
    };
    // Five sequential single-output programs are legal. An implementation may group
    // their clears, but it cannot turn those legal draws into an over-budget MRT.
    let mut post = gpu.build(hdr_pack(sized, format));
    let actual = gpu.render(&mut post, [16, 16]);
    assert!(
        actual.iter().all(|p| p
            .iter()
            .zip([0, 128, 0, 255])
            .all(|(a, b)| a.abs_diff(b) <= 1)),
        "sized={sized}: {:?}",
        &actual[..16]
    );
}

#[test]
fn fullsize_hdr_clear_respects_byte_budget() {
    hdr_clear_case(false, 8, 32, "RGBA16F");
}
#[test]
fn sized_hdr_clear_respects_byte_budget() {
    hdr_clear_case(true, 8, 32, "RGBA16F");
}
#[test]
fn fullsize_hdr_clear_respects_attachment_count() {
    hdr_clear_case(false, 2, 64, "RGBA16F");
}
#[test]
fn sized_hdr_clear_respects_attachment_count() {
    hdr_clear_case(true, 2, 64, "RGBA16F");
}

#[test]
fn fullsize_unorm_clear_respects_byte_budget() {
    hdr_clear_case(false, 8, 32, "RGBA8");
}
#[test]
fn sized_unorm_clear_respects_byte_budget() {
    hdr_clear_case(true, 8, 32, "RGBA8");
}
