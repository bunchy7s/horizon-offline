//! A still picture of a player wearing items, drawn on the CPU (the launcher's Wardrobe): the
//! player model at rest with its skin, each item on its bone, seen from the front at an angle.

use crate::item::cube_faces;
use crate::{Item, Part, Texture, pose_matrix};
use glam::{Mat4, Vec3, Vec4Swizzles};

/// RGBA pixels, row by row, straight alpha.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The player model's parts at rest, as vanilla's `PlayerModel` lays them out on a 64×64 skin:
/// each body part with its outer layer (`hat`, `jacket`, sleeves, pants) as a child. `slim` is the
/// three-pixel-wide arms of a slim skin.
pub fn player_model(slim: bool) -> Part {
    let tex = [64.0, 64.0];
    let part = |name: &str, pivot: [f32; 3], origin, size, uv, outer: (&str, [f32; 2], f32)| Part {
        name: name.into(),
        pose: [pivot[0], pivot[1], pivot[2], 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        faces: cube_faces(origin, size, uv, 0.0, false, tex),
        children: vec![Part {
            name: outer.0.into(),
            pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            faces: cube_faces(origin, size, outer.1, outer.2, false, tex),
            children: Vec::new(),
        }],
    };
    let (arm, arm_y, right_x) = if slim {
        ([3.0, 12.0, 4.0], 2.5, -2.0)
    } else {
        ([4.0, 12.0, 4.0], 2.0, -3.0)
    };
    let leg = [4.0, 12.0, 4.0];
    Part {
        name: "root".into(),
        pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        faces: Vec::new(),
        children: vec![
            part(
                "head",
                [0.0; 3],
                [-4.0, -8.0, -4.0],
                [8.0; 3],
                [0.0, 0.0],
                ("hat", [32.0, 0.0], 0.5),
            ),
            part(
                "body",
                [0.0; 3],
                [-4.0, 0.0, -2.0],
                [8.0, 12.0, 4.0],
                [16.0, 16.0],
                ("jacket", [16.0, 32.0], 0.25),
            ),
            part(
                "right_arm",
                [-5.0, arm_y, 0.0],
                [right_x, -2.0, -2.0],
                arm,
                [40.0, 16.0],
                ("right_sleeve", [40.0, 32.0], 0.25),
            ),
            part(
                "left_arm",
                [5.0, arm_y, 0.0],
                [-1.0, -2.0, -2.0],
                arm,
                [32.0, 48.0],
                ("left_sleeve", [48.0, 48.0], 0.25),
            ),
            part(
                "right_leg",
                [-1.9, 12.0, 0.0],
                [-2.0, 0.0, -2.0],
                leg,
                [0.0, 16.0],
                ("right_pants", [0.0, 32.0], 0.25),
            ),
            part(
                "left_leg",
                [1.9, 12.0, 0.0],
                [-2.0, 0.0, -2.0],
                leg,
                [16.0, 48.0],
                ("left_pants", [0.0, 48.0], 0.25),
            ),
        ],
    }
}

/// The matrix of the part named `name` under `parent`, at rest.
pub fn rest_matrix(part: &Part, parent: Mat4, name: &str) -> Option<Mat4> {
    let m = parent * pose_matrix(part.pose);
    if part.name == name {
        return Some(m);
    }
    part.children.iter().find_map(|c| rest_matrix(c, m, name))
}

/// One model to draw: its root under `base` (model space, blocks), its parts posed by
/// `poses` (name → pose; the rest at rest).
pub struct Draw<'a> {
    pub root: &'a Part,
    pub base: Mat4,
    pub texture: &'a Texture,
    pub poses: Vec<(String, crate::Pose)>,
}

impl<'a> Draw<'a> {
    /// `item` on the player model `body`, at `ticks` of its animations.
    pub fn item(item: &'a Item, body: &Part, ticks: f32) -> Option<Self> {
        let bone = rest_matrix(body, Mat4::IDENTITY, item.bone.part())?;
        let mut poses = Vec::new();
        item.animate(ticks, |name, pose| poses.push((name.to_owned(), pose)));
        Some(Self {
            root: &item.root,
            base: bone,
            texture: &item.texture,
            poses,
        })
    }
}

/// Draws `draws` into a `width`×`height` picture: turned `yaw` degrees (0 shows the front),
/// tilted `pitch` degrees down: 2.5 blocks to the height, the head's top a tenth of the way down
/// (room for a hat).
pub fn render(draws: &[Draw], width: u32, height: u32, yaw: f32, pitch: f32) -> Image {
    let (w, h) = (width as usize, height as usize);
    let mut rgba = vec![0u8; w * h * 4];
    let mut depth = vec![f32::INFINITY; w * h];
    // Model space has y down and the front at -z: the camera looks along +z from the front.
    let scale = height as f32 / 2.5;
    let view = Mat4::from_translation(Vec3::new(width as f32 / 2.0, height as f32 * 0.1, 0.0))
        * Mat4::from_scale(Vec3::new(scale, scale, scale))
        * Mat4::from_rotation_x(-pitch.to_radians())
        * Mat4::from_rotation_y(yaw.to_radians())
        // Up the body's pivot: the head's top is 0.5 blocks above it.
        * Mat4::from_translation(Vec3::new(0.0, 0.5, 0.0));
    for d in draws {
        let mut stack = vec![(d.root, d.base)];
        while let Some((part, parent)) = stack.pop() {
            let pose = d
                .poses
                .iter()
                .find(|(n, _)| *n == part.name)
                .map_or(part.pose, |(_, p)| *p);
            let m = parent * pose_matrix(pose);
            let to_screen = view * m;
            let normal = glam::Mat3::from_mat4(view * m);
            for face in &part.faces {
                let n = (normal * Vec3::from_array(face.normal)).normalize_or_zero();
                // Faces turned away are hidden by the depth test; the light is vanilla-like:
                // brighter facing up and toward the viewer.
                let light = (0.55 + 0.25 * (-n.y).max(0.0) + 0.2 * (-n.z).max(0.0)).min(1.0);
                let v: Vec<(Vec3, [f32; 2])> = face
                    .vertices
                    .iter()
                    .map(|v| {
                        let p =
                            to_screen * glam::Vec4::new(v[0] / 16.0, v[1] / 16.0, v[2] / 16.0, 1.0);
                        (p.xyz(), [v[3], v[4]])
                    })
                    .collect();
                for tri in [[0, 1, 2], [0, 2, 3]] {
                    raster(
                        [v[tri[0]], v[tri[1]], v[tri[2]]],
                        d.texture,
                        light,
                        (w, h),
                        &mut rgba,
                        &mut depth,
                    );
                }
            }
            for child in &part.children {
                stack.push((child, m));
            }
        }
    }
    Image {
        width,
        height,
        rgba,
    }
}

fn raster(
    v: [(Vec3, [f32; 2]); 3],
    tex: &Texture,
    light: f32,
    (w, h): (usize, usize),
    rgba: &mut [u8],
    depth: &mut [f32],
) {
    let (a, b, c) = (v[0].0, v[1].0, v[2].0);
    let area = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    if area.abs() < 1e-6 {
        return;
    }
    let x0 = a.x.min(b.x).min(c.x).floor().max(0.0) as usize;
    let x1 = (a.x.max(b.x).max(c.x).ceil() as usize).min(w);
    let y0 = a.y.min(b.y).min(c.y).floor().max(0.0) as usize;
    let y1 = (a.y.max(b.y).max(c.y).ceil() as usize).min(h);
    for y in y0..y1 {
        for x in x0..x1 {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let w0 = ((b.x - px) * (c.y - py) - (b.y - py) * (c.x - px)) / area;
            let w1 = ((c.x - px) * (a.y - py) - (c.y - py) * (a.x - px)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let z = w0 * a.z + w1 * b.z + w2 * c.z;
            let i = y * w + x;
            if z >= depth[i] {
                continue;
            }
            let u = w0 * v[0].1[0] + w1 * v[1].1[0] + w2 * v[2].1[0];
            let t = w0 * v[0].1[1] + w1 * v[1].1[1] + w2 * v[2].1[1];
            let tx = ((u * tex.width as f32) as usize).min(tex.width as usize - 1);
            let ty = ((t * tex.height as f32) as usize).min(tex.height as usize - 1);
            let texel = &tex.rgba[(ty * tex.width as usize + tx) * 4..][..4];
            // Cutout, as entity textures draw.
            if texel[3] < 128 {
                continue;
            }
            depth[i] = z;
            for k in 0..3 {
                rgba[i * 4 + k] = (f32::from(texel[k]) * light).round() as u8;
            }
            rgba[i * 4 + 3] = 255;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 64×64 skin in one colour per region: what lands where shows which face was drawn.
    fn skin() -> Texture {
        let mut rgba = vec![0u8; 64 * 64 * 4];
        for y in 0..64 {
            for x in 0..64 {
                let c: [u8; 4] = match (x, y) {
                    // The head's front (box UV: u+d, v+d, 8×8 from (0, 0)).
                    (8..16, 8..16) => [200, 0, 0, 255],
                    (0..32, 0..16) => [0, 200, 0, 255],
                    // The hat layer: transparent.
                    (32..64, 0..16) => [0, 0, 0, 0],
                    _ => [0, 0, 200, 255],
                };
                rgba[(y * 64 + x) * 4..][..4].copy_from_slice(&c);
            }
        }
        Texture {
            width: 64,
            height: 64,
            rgba,
        }
    }

    fn at(img: &Image, x: u32, y: u32) -> [u8; 4] {
        img.rgba[((y * img.width + x) * 4) as usize..][..4]
            .try_into()
            .unwrap()
    }

    #[test]
    fn the_front_of_the_head_faces_the_viewer() {
        let body = player_model(false);
        let tex = skin();
        let draw = Draw {
            root: &body,
            base: Mat4::IDENTITY,
            texture: &tex,
            poses: Vec::new(),
        };
        let img = render(&[draw], 100, 100, 0.0, 0.0);
        // 40 px a block: the head's top at row 10, its bottom (the pivot) 20 px lower; its
        // middle at row 20, column 50.
        let face = at(&img, 50, 20);
        assert_eq!(face[3], 255);
        assert!(face[0] > 100 && face[1] == 0 && face[2] == 0, "{face:?}");
        // Above the head, nothing (the transparent hat layer doesn't hide it).
        assert_eq!(at(&img, 50, 5)[3], 0);
        // Turned round: the back of the head (green region), not the face.
        let back = render(
            &[Draw {
                root: &body,
                base: Mat4::IDENTITY,
                texture: &tex,
                poses: Vec::new(),
            }],
            100,
            100,
            180.0,
            0.0,
        );
        let c = at(&back, 50, 20);
        assert!(c[1] > 100 && c[0] == 0, "{c:?}");
    }

    #[test]
    fn an_item_sits_on_its_bone() {
        let file = format!(
            r#"{{"format": 1, "id": "cap", "slot": "head", "bone": "head",
              "attach": {{"offset": [0, -8, 0]}}, "texture": {{"png": "{}"}},
              "parts": [{{"name": "c", "cubes": [{{"origin": [-4, -4, -4], "size": [8, 4, 8], "uv": [0, 0]}}]}}]}}"#,
            crate::pose::tests::png(32, 32)
        );
        let item = Item::parse(file.as_bytes()).unwrap();
        let body = player_model(false);
        let tex = skin();
        let mut draws = vec![Draw {
            root: &body,
            base: Mat4::IDENTITY,
            texture: &tex,
            poses: Vec::new(),
        }];
        let bare = render(&draws, 100, 100, 0.0, 0.0);
        draws.push(Draw::item(&item, &body, 0.0).unwrap());
        let worn = render(&draws, 100, 100, 0.0, 0.0);
        // The cap spans 8 to 12 model pixels above the head's pivot: rows 0 to 10, over the
        // head's top at row 10.
        assert_eq!(at(&bare, 50, 5)[3], 0);
        let cap = at(&worn, 50, 5);
        assert_eq!(cap[3], 255);
        // Grey (200 lit), not the skin's colours.
        assert!(cap[0] == cap[1] && cap[1] == cap[2], "{cap:?}");
        // The face is still the face.
        assert_eq!(at(&worn, 50, 20), at(&bare, 50, 20));
    }
}
