use super::*;

/// A skin whose every texel says where it is: red = x, green = y.
fn mapped(height: u32) -> image::RgbaImage {
    image::RgbaImage::from_fn(64, height, |x, y| image::Rgba([x as u8, y as u8, 0, 255]))
}

fn png(img: &image::RgbaImage) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

#[test]
fn the_front_view_takes_each_part_from_its_place() {
    let mut skin = mapped(64);
    // No outer layer, except one hat texel.
    for (x, y) in (0..64).flat_map(|x| (0..64).map(move |y| (x, y))) {
        let outer = (40..48).contains(&x) && y < 16
            || (16..56).contains(&x) && (32..48).contains(&y)
            || (0..16).contains(&x) && (48..64).contains(&y)
            || (48..64).contains(&x) && (48..64).contains(&y);
        if outer {
            skin.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
        }
    }
    skin.put_pixel(41, 9, image::Rgba([200, 0, 9, 255]));
    let f = front(&skin, Model::Classic);
    let at = |x: usize, y: usize| f[y * 16 + x];
    assert_eq!(at(4, 0), [8, 8, 0, 255], "head front starts at (8, 8)");
    assert_eq!(at(5, 1), [200, 0, 9, 255], "the hat covers it");
    assert_eq!(at(4, 8), [20, 20, 0, 255], "body");
    assert_eq!(at(0, 8), [44, 20, 0, 255], "right arm");
    assert_eq!(
        at(12, 8),
        [36, 52, 0, 255],
        "left arm, its own on a 64×64 skin"
    );
    assert_eq!(at(8, 20), [20, 52, 0, 255], "left leg");
    let slim = front(&skin, Model::Slim);
    assert_eq!(slim[8 * 16], [0, 0, 0, 0], "slim arms leave a column");
    assert_eq!(slim[8 * 16 + 1], [44, 20, 0, 255]);
}

#[test]
fn an_old_skin_mirrors_its_right_limbs() {
    let f = front(&mapped(32), Model::Classic);
    let at = |x: usize, y: usize| f[y * 16 + x];
    assert_eq!(
        at(12, 8),
        [47, 20, 0, 255],
        "the left arm is the right one mirrored"
    );
    assert_eq!(at(15, 8), [44, 20, 0, 255]);
    assert_eq!(at(8, 20), [7, 20, 0, 255]);
}

#[test]
fn only_skin_and_cape_sizes_are_kept() {
    assert!(read_skin(&png(&mapped(64))).is_ok());
    assert!(read_skin(&png(&mapped(32))).is_ok());
    let big = image::RgbaImage::new(128, 128);
    assert_eq!(
        read_skin(&png(&big)).unwrap_err(),
        "A skin is 64×64 (or the older 64×32) pixels; this one is 128×128."
    );
    assert!(read_skin(b"not a png").unwrap_err().contains("isn't a PNG"));
    assert!(read_cape(&png(&mapped(32))).is_ok());
    assert!(read_cape(&png(&image::RgbaImage::new(22, 17))).is_ok());
    assert!(read_cape(&png(&mapped(64))).is_err());
    assert_eq!(cape_front(&mapped(32))[0], [1, 1, 0, 255]);
}

#[test]
fn a_kept_look_reaches_the_game_only_with_its_files() {
    let root = std::env::temp_dir().join(format!("horizon-skins-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let (good, bad) = (root.join("alex.png"), root.join("big.png"));
    std::fs::write(&good, png(&mapped(64))).unwrap();
    std::fs::write(&bad, png(&image::RgbaImage::new(10, 10))).unwrap();
    let dir = root.join("skins");
    let look = Look {
        model: Model::Slim,
        skin: true,
        cape: true,
    };
    assert!(env(&dir, "Alex", &look).is_empty(), "nothing kept yet");
    keep(&dir, "Alex", &good, false).unwrap();
    assert!(keep(&dir, "Alex", &bad, true).is_err());
    let (skin, _) = files(&dir, "ALEX");
    assert_eq!(
        env(&dir, "alex", &look),
        [
            (SKIN_ENV, skin.to_string_lossy().into_owned()),
            (MODEL_ENV, "slim".to_string())
        ]
    );
    assert_eq!(
        serde_json::to_string(&Look::default()).unwrap(),
        r#"{"model":"classic"}"#
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn forgetting_a_name_drops_its_files() {
    let root = std::env::temp_dir().join(format!("horizon-skins-forget-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let (src, cape_src) = (root.join("alex.png"), root.join("cape.png"));
    std::fs::write(&src, png(&mapped(64))).unwrap();
    std::fs::write(&cape_src, png(&mapped(32))).unwrap();
    let dir = root.join("skins");
    keep(&dir, "Alex", &src, false).unwrap();
    keep(&dir, "Alex", &cape_src, true).unwrap();
    let (skin, cape) = files(&dir, "Alex");
    assert!(skin.is_file() && cape.is_file());
    forget(&dir, "ALEX");
    assert!(!skin.exists() && !cape.exists());
    // A name that never had files is forgotten all the same.
    forget(&dir, "Alex");
    std::fs::remove_dir_all(&root).unwrap();
}
