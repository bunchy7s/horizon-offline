//! Poses as the game's model parts have them.

/// x, y, z (pixels), x, y, z rotations (radians), x, y, z scales: the
/// game's `Part::pose`.
pub type Pose = [f32; 9];

/// `ModelPart.translateAndRotate`: the matrix a part's pose applies under
/// its parent's (the game's `mobs::render::posed`). An item's root pose is
/// its attachment, so the item sits at `bone * pose_matrix(root.pose)`.
pub fn pose_matrix(p: Pose) -> glam::Mat4 {
    glam::Mat4::from_translation(glam::Vec3::new(p[0], p[1], p[2]) / 16.0)
        * glam::Mat4::from_quat(glam::Quat::from_euler(
            glam::EulerRot::ZYX,
            p[5],
            p[4],
            p[3],
        ))
        * glam::Mat4::from_scale(glam::Vec3::new(p[6], p[7], p[8]))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{Bone, Error, Item, MAX_VERTICES, Motion, Slot, Tag};
    use glam::{Mat4, Vec3};

    /// A `width`×`height` PNG, base64.
    pub(crate) fn png(width: u32, height: u32) -> String {
        use base64::Engine;
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&vec![200; (width * height * 4) as usize])
            .unwrap();
        writer.finish().unwrap();
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    /// A hat: an 8×2×8 brim 8 pixels above the head's pivot, and a crown
    /// on it that swings 10° about z over 40 ticks.
    fn hat(extra: &str) -> String {
        format!(
            r#"{{"format": 1, "id": "hat", "slot": "head", "bone": "head",
              "attach": {{"offset": [0, -8, 0]}},
              "texture": {{"png": "{}"}},
              "parts": [{{"name": "brim", "cubes": [{{"origin": [-4, -2, -4], "size": [8, 2, 8], "uv": [0, 0]}}],
                "children": [{{"name": "crown", "pivot": [0, -2, 0],
                  "cubes": [{{"origin": [-2, -4, -2], "size": [4, 4, 4], "uv": [0, 10]}}]}}]}}],
              "animations": [{{"part": "crown", "motion": "swing", "axis": "z", "amplitude": 10, "period": 40}}]
              {extra}}}"#,
            png(32, 32)
        )
    }

    /// The posed position of a part's first face's vertex `i`.
    fn corner(item: &Item, bone: Mat4, path: &[&str], face: usize, i: usize, ticks: f32) -> Vec3 {
        let mut poses = std::collections::HashMap::new();
        item.animate(ticks, |n, p| {
            poses.insert(n.to_owned(), p);
        });
        let mut m = bone * pose_matrix(item.root.pose);
        let mut part = &item.root;
        for name in path {
            part = part.children.iter().find(|c| c.name == *name).unwrap();
            m *= pose_matrix(*poses.get(*name).unwrap_or(&part.pose));
        }
        let v = part.faces[face].vertices[i];
        m.transform_point3(Vec3::new(v[0], v[1], v[2]) / 16.0)
    }

    fn close(a: Vec3, b: Vec3) {
        assert!((a - b).abs().max_element() < 1e-5, "{a} != {b}");
    }

    #[test]
    fn reads_the_item_and_bakes_box_uv() {
        let item = Item::parse(hat("").as_bytes()).unwrap();
        assert_eq!((item.slot, item.bone), (Slot::Head, Bone::Head));
        assert_eq!(item.vertices, 2 * 6 * 4);
        assert_eq!(
            (item.texture.width, item.texture.rgba.len()),
            (32, 32 * 32 * 4)
        );
        // The brim's down face: corners l1, l0, t0, t1 with UV (u+d, v) to
        // (u+d+w, v+d) remapped as (u1,v0) (u0,v0) (u0,v1) (u1,v1).
        let down = &item.root.children[0].faces[0];
        assert_eq!(down.normal, [0.0, -1.0, 0.0]);
        assert_eq!(
            down.vertices,
            [
                [4.0, -2.0, 4.0, 16.0 / 32.0, 0.0],
                [-4.0, -2.0, 4.0, 8.0 / 32.0, 0.0],
                [-4.0, -2.0, -4.0, 8.0 / 32.0, 8.0 / 32.0],
                [4.0, -2.0, -4.0, 16.0 / 32.0, 8.0 / 32.0],
            ]
        );
    }

    #[test]
    fn the_item_follows_its_bone() {
        let item = Item::parse(hat("").as_bytes()).unwrap();
        // Brim face 0, vertex 2 is (-4, -2, -4) px: 8 px up from the
        // attachment, (-0.25, -0.625, -0.25) blocks from the bone's pivot.
        close(
            corner(&item, Mat4::IDENTITY, &["brim"], 0, 2, 0.0),
            Vec3::new(-0.25, -0.625, -0.25),
        );
        // The head turned 90° about y: (x, y, z) -> (z, y, -x).
        let turned = Mat4::from_rotation_y(90f32.to_radians());
        close(
            corner(&item, turned, &["brim"], 0, 2, 0.0),
            Vec3::new(-0.25, -0.625, 0.25),
        );
        // The head 1.5 blocks up and nodding 90° about x: (x, y, z) -> (x, -z, y).
        let nod = Mat4::from_translation(Vec3::new(0.0, 1.5, 0.0))
            * Mat4::from_rotation_x(90f32.to_radians());
        close(
            corner(&item, nod, &["brim"], 0, 2, 0.0),
            Vec3::new(-0.25, 1.5 + 0.25, -0.625),
        );
    }

    #[test]
    fn swings_on_its_period() {
        let item = Item::parse(hat("").as_bytes()).unwrap();
        // The crown's down face, vertex 2: (-2, -4, -2) px from its pivot,
        // which is 10 px above the bone's.
        let rest = Vec3::new(-2.0, -14.0, -2.0) / 16.0;
        close(
            corner(&item, Mat4::IDENTITY, &["brim", "crown"], 0, 2, 0.0),
            rest,
        );
        close(
            corner(&item, Mat4::IDENTITY, &["brim", "crown"], 0, 2, 20.0),
            rest,
        );
        // A quarter period: 10° about z at the crown's pivot (0, -10, 0) px:
        // (x, y) -> (x cos - y sin, x sin + y cos) about it.
        let (s, c) = 10f32.to_radians().sin_cos();
        let (x, y) = (-2.0, -4.0);
        let swung = Vec3::new(x * c - y * s, -10.0 + x * s + y * c, -2.0) / 16.0;
        close(
            corner(&item, Mat4::IDENTITY, &["brim", "crown"], 0, 2, 10.0),
            swung,
        );
        // The same pose a period later.
        close(
            corner(&item, Mat4::IDENTITY, &["brim", "crown"], 0, 2, 50.0),
            swung,
        );
    }

    #[test]
    fn spins_once_per_period_and_bobs() {
        let spin = hat("").replace(
            r#""motion": "swing", "axis": "z", "amplitude": 10"#,
            r#""motion": "spin", "axis": "y""#,
        );
        let item = Item::parse(spin.as_bytes()).unwrap();
        assert_eq!(item.animations[0].motion, Motion::Spin);
        let mut got = None;
        item.animate(10.0, |_, p| got = Some(p));
        assert!((got.unwrap()[4] - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        let bob = hat("").replace(
            r#""motion": "swing", "axis": "z", "amplitude": 10"#,
            r#""motion": "bob", "axis": "y", "amplitude": 1.5"#,
        );
        let item = Item::parse(bob.as_bytes()).unwrap();
        item.animate(10.0, |_, p| got = Some(p));
        // The crown's pivot y is -2; a quarter period adds the amplitude.
        assert!((got.unwrap()[1] - (-2.0 + 1.5)).abs() < 1e-6);
    }

    #[test]
    fn refuses_items_past_the_limits() {
        let big = hat("").replace(r#""size": [4, 4, 4]"#, r#""size": [4, 4, 4], "inflate": 0"#);
        assert!(Item::parse(big.as_bytes()).is_ok());
        // 22 boxes of 24 vertices are past the budget.
        let cube = r#"{"origin": [0, 0, 0], "size": [1, 1, 1], "uv": [0, 0]}"#;
        let many = vec![cube; MAX_VERTICES / 24 + 1].join(",");
        let file = hat("").replace(
            r#""cubes": [{"origin": [-4, -2, -4], "size": [8, 2, 8], "uv": [0, 0]}]"#,
            &format!(r#""cubes": [{many}]"#),
        );
        assert_eq!(
            Item::parse(file.as_bytes()),
            Err(Error::TooManyVertices(23 * 24))
        );
        // Far from the head.
        let far = hat("").replace(r#""offset": [0, -8, 0]"#, r#""offset": [0, -30, 0]"#);
        assert!(matches!(Item::parse(far.as_bytes()), Err(Error::TooFar(_))));
        // A texture past 64x64.
        let wide = hat("").replace(&png(32, 32), &png(128, 32));
        assert!(matches!(
            Item::parse(wide.as_bytes()),
            Err(Error::Texture(_))
        ));
        // UV off the texture, an animation of no part, a wild swing.
        for (from, to) in [
            (r#""uv": [0, 10]"#, r#""uv": [20, 10]"#),
            (r#""part": "crown""#, r#""part": "tail""#),
            (r#""amplitude": 10"#, r#""amplitude": 90"#),
            (r#""id": "hat""#, r#""id": "Hat!""#),
        ] {
            assert!(
                Item::parse(hat("").replace(from, to).as_bytes()).is_err(),
                "{to}"
            );
        }
        assert!(Item::parse(hat(r#", "glow": true"#).as_bytes()).is_err());
    }

    /// A name tag: an 8×8 mark and a violet line, no parts.
    fn name_tag(extra: &str) -> String {
        format!(
            r##"{{"format": 1, "id": "tag", "slot": "name_tag", "texture": {{"png": "{}"}},
              "parts": [], "tag": {{"line": "#a98fFF"}}{extra}}}"##,
            png(8, 8)
        )
    }

    #[test]
    fn a_name_tag_is_a_mark_and_a_line_without_a_model() {
        let item = Item::parse(name_tag("").as_bytes()).unwrap();
        assert_eq!(item.slot, Slot::NameTag);
        assert_eq!(
            item.tag,
            Some(Tag {
                line: [0xa9, 0x8f, 0xff]
            })
        );
        assert_eq!(item.vertices, 0);
        assert_eq!((item.texture.width, item.texture.height), (8, 8));
        // A model item never has a line.
        assert_eq!(Item::parse(hat("").as_bytes()).unwrap().tag, None);
    }

    #[test]
    fn refuses_name_tags_with_a_model_or_a_big_mark() {
        let cube =
            r#"{"name": "box", "cubes": [{"origin": [0, 0, 0], "size": [1, 1, 1], "uv": [0, 0]}]}"#;
        let boxed = name_tag("").replace(r#""parts": []"#, &format!(r#""parts": [{cube}]"#));
        let wide = name_tag("").replace(&png(8, 8), &png(32, 32));
        let oblong = name_tag("").replace(&png(8, 8), &png(16, 8));
        let no_line = name_tag("").replace(r##", "tag": {"line": "#a98fFF"}"##, "");
        let bad_line = name_tag("").replace("#a98fFF", "violet");
        for file in [boxed, wide, oblong, no_line, bad_line] {
            assert!(Item::parse(file.as_bytes()).is_err(), "{file}");
        }
        let hat_with_tag = hat(r##", "tag": {"line": "#ffffff"}"##);
        assert!(Item::parse(hat_with_tag.as_bytes()).is_err());
    }
}
