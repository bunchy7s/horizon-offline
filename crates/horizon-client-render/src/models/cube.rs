//! Exact unit-cube faces eligible for repeated-texture greedy rectangles.
use super::catalog::BakedFace;
use horizon_assets::model::Direction;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CubeStyle {
    pub layer: u32,
    /// UV at spatial (u,v)=(0,0), then the two signed unit-axis derivatives.
    pub uv: [i8; 6],
    pub shade_bits: u32,
}
pub fn shade(face: &BakedFace) -> f32 {
    match face
        .quad
        .shade_direction_override
        .unwrap_or(face.quad.facing)
    {
        Direction::Down => 0.5,
        Direction::Up => 1.0,
        Direction::North | Direction::South => 0.8,
        Direction::West | Direction::East => 0.6,
    }
}
pub fn cube_styles(faces: &[BakedFace]) -> Option<[CubeStyle; 6]> {
    if faces.len() != 6 {
        return None;
    }
    let mut styles = [None; 6];
    for face in faces {
        if !face.opaque
            || face.translucent
            || face.quad.tint_index != -1
            || face.quad.light_emission != 0
        {
            return None;
        }
        let q = &face.quad;
        let direction = q.cull?;
        if q.facing != direction || styles[direction as usize].is_some() {
            return None;
        }
        let (axis, u, v) = match direction {
            Direction::Down | Direction::Up => (1, 0, 2),
            Direction::North | Direction::South => (2, 0, 1),
            Direction::West | Direction::East => (0, 2, 1),
        };
        let plane = if (direction as usize).is_multiple_of(2) {
            0.0
        } else {
            1.0
        };
        let mut uv = [None; 4];
        for (p, texture) in q.positions.iter().zip(q.uv) {
            if p[axis] != plane
                || ![p[u], p[v], texture[0], texture[1]]
                    .iter()
                    .all(|&x| x == 0.0 || x == 1.0)
            {
                return None;
            }
            let index = p[u] as usize + 2 * p[v] as usize;
            if uv[index].replace(texture.map(|x| x as i8)).is_some() {
                return None;
            }
        }
        let [a, b, c, d] = uv.map(|v| v.unwrap());
        let du = [b[0] - a[0], b[1] - a[1]];
        let dv = [c[0] - a[0], c[1] - a[1]];
        if d != [a[0] + du[0] + dv[0], a[1] + du[1] + dv[1]]
            || (du[0] * dv[1] - du[1] * dv[0]).abs() != 1
        {
            return None;
        }
        styles[direction as usize] = Some(CubeStyle {
            layer: face.layer,
            uv: [a[0], a[1], du[0], du[1], dv[0], dv[1]],
            shade_bits: shade(face).to_bits(),
        });
    }
    Some(styles.map(|v| v.unwrap()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_assets::{
        bake::{BlockModelTransform, bake_model_transformed},
        model::CuboidModel,
        resolve::ModelLibrary,
    };
    use serde_json::json;
    #[test]
    fn projected_cube_uvs_match_baked_quarter_turns() {
        let faces: serde_json::Map<String, serde_json::Value> = Direction::ALL
            .into_iter()
            .map(|d| {
                (
                    d.name().to_owned(),
                    json!({"texture":"#all","cullface":d.name()}),
                )
            })
            .collect();
        let model = CuboidModel::from_json(&json!({"textures":{"all":"horizon:probe"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":faces}]}).to_string()).unwrap();
        let library = ModelLibrary::new([("horizon:cube".to_owned(), model)].into()).unwrap();
        let model = library.resolve("horizon:cube").unwrap();
        let mut eligible = 0;
        for x in 0..4 {
            for y in 0..4 {
                for z in 0..4 {
                    for lock in [false, true] {
                        let faces: Vec<_> = bake_model_transformed(
                            &model,
                            BlockModelTransform::new([x, y, z], lock),
                        )
                        .unwrap()
                        .into_iter()
                        .map(|quad| BakedFace {
                            quad,
                            layer: 7,
                            opaque: true,
                            translucent: false,
                        })
                        .collect();
                        let Some(styles) = cube_styles(&faces) else {
                            assert!(lock);
                            continue;
                        };
                        eligible += 1;
                        for face in &faces {
                            let direction = face.quad.cull.unwrap();
                            let (u, v) = match direction {
                                Direction::Down | Direction::Up => (0, 2),
                                Direction::North | Direction::South => (0, 1),
                                _ => (2, 1),
                            };
                            let style = styles[direction as usize];
                            let [a, b, c, d, e, f] = style.uv.map(f32::from);
                            for (p, uv) in face.quad.positions.iter().zip(face.quad.uv) {
                                assert_eq!([a + c * p[u] + e * p[v], b + d * p[u] + f * p[v]], uv);
                            }
                        }
                    }
                }
            }
        }
        assert!(eligible >= 64);
        let mut faces: Vec<_> =
            bake_model_transformed(&model, BlockModelTransform::new([0; 3], false))
                .unwrap()
                .into_iter()
                .map(|quad| BakedFace {
                    quad,
                    layer: 0,
                    opaque: true,
                    translucent: false,
                })
                .collect();
        faces[0].opaque = false;
        assert!(cube_styles(&faces).is_none());
        faces[0].opaque = true;
        faces[0].quad.positions[0][0] = 0.25;
        assert!(cube_styles(&faces).is_none());
    }
}
