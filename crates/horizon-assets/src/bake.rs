//! Cuboid geometry in block units, including blockstate rotation and UV locking.
use crate::{
    json::*,
    model::*,
    resolve::{ResolvedCuboidModel, ResolvedLoader},
};
mod transform;
pub use transform::BlockModelTransform;

#[derive(Clone, Debug, PartialEq)]
pub struct ModelQuad {
    pub positions: [[f32; 3]; 4],
    /// Normalized unit-atlas coordinates, before placement in a larger atlas.
    pub uv: [[f32; 2]; 4],
    pub facing: Direction,
    pub cull: Option<Direction>,
    pub material: Material,
    pub tint_index: i32,
    pub shade_direction_override: Option<Direction>,
    /// The face's or element's `light_emission`.
    pub light_emission: u8,
    /// Least (block, sky) light the quad is drawn with: its emission, raised by `neoforge_data`.
    pub light: [u8; 2],
    /// `neoforge_data` colour (ARGB) multiplied into the quad's tint.
    pub color: Option<i32>,
    /// `neoforge_data` override of the model's ambient occlusion for this quad.
    pub ambient_occlusion: Option<bool>,
    pub render_type: Option<RenderType>,
}

/// NeoForge's `render_type` names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderType {
    Solid,
    Cutout,
    CutoutMipped,
    Translucent,
    Tripwire,
}
impl RenderType {
    /// Accepts a bare or `minecraft:` name; others (mods' own types) are `None`.
    pub fn by_name(name: &str) -> Option<Self> {
        match name.strip_prefix("minecraft:").unwrap_or(name) {
            "solid" => Some(Self::Solid),
            "cutout" => Some(Self::Cutout),
            "cutout_mipped" => Some(Self::CutoutMipped),
            "translucent" => Some(Self::Translucent),
            "tripwire" => Some(Self::Tripwire),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ElementMatrix {
    pub rows: [[f32; 3]; 3],
    identity: bool,
}
impl ElementMatrix {
    pub fn from_rotation(rotation: &ElementRotation) -> Self {
        const DEG: f32 = std::f32::consts::PI / 180.0;
        let mut matrix = match rotation.value {
            RotationValue::SingleAxis { angle: 0.0, .. } => Self {
                rows: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                identity: true,
            },
            RotationValue::SingleAxis { axis, angle } => {
                let (s, c) = sin_cos(angle * DEG);
                let rows = match axis {
                    Axis::X => [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]],
                    Axis::Y => [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]],
                    Axis::Z => [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]],
                };
                Self {
                    rows,
                    identity: false,
                }
            }
            RotationValue::EulerXyz([x, y, z]) => {
                let (sx, cx) = sin_cos(x * DEG);
                let (sy, cy) = sin_cos(y * DEG);
                let (sz, cz) = sin_cos(z * DEG);
                let czsy = cz * sy;
                let szsy = sz * sy;
                Self {
                    rows: [
                        [cz * cy, -sz * cx + czsy * sx, -sz * -sx + czsy * cx],
                        [sz * cy, cz * cx + szsy * sx, cz * -sx + szsy * cx],
                        [-sy, cy * sx, cy * cx],
                    ],
                    identity: false,
                }
            }
        };
        if rotation.rescale && !matrix.identity {
            let scales: [f32; 3] = std::array::from_fn(|axis| {
                let basis = std::array::from_fn(|i| if i == axis { 1.0 } else { 0.0 });
                let column = matrix.direction(basis);
                let maximum = java_max(java_max(column[0].abs(), column[1].abs()), column[2].abs());
                1.0 / maximum
            });
            for (axis, scale) in scales.into_iter().enumerate() {
                for row in &mut matrix.rows {
                    row[axis] *= scale;
                }
            }
        }
        matrix
    }
    fn direction(self, value: [f32; 3]) -> [f32; 3] {
        self.rows
            .map(|r| r[0] * value[0] + (r[1] * value[1] + r[2] * value[2]))
    }
    fn position(self, value: [f32; 3]) -> [f32; 3] {
        if self.identity {
            value
        } else {
            self.rows
                .map(|r| r[0] * value[0] + (r[1] * value[1] + (r[2] * value[2] + 0.0)))
        }
    }
}
fn java_max(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else {
        a.max(b)
    }
}
/// JOML's default float path uses a double sine rounded to float, and obtains
/// cosine's magnitude from that rounded sine. Keep all subsequent steps f32.
fn sin_cos(angle: f32) -> (f32, f32) {
    let sin = f64::from(angle).sin() as f32;
    let cos = (1.0 - sin * sin).sqrt();
    let shifted = angle + std::f32::consts::FRAC_PI_2;
    let mut period =
        shifted - (shifted / std::f32::consts::TAU) as i32 as f32 * std::f32::consts::TAU;
    if period < 0.0 {
        period += std::f32::consts::TAU;
    }
    (
        sin,
        if period >= std::f32::consts::PI {
            -cos
        } else {
            cos
        },
    )
}

pub fn default_uv(from: [f32; 3], to: [f32; 3], direction: Direction) -> [f32; 4] {
    match direction {
        Direction::Down => [from[0], 16.0 - to[2], to[0], 16.0 - from[2]],
        Direction::Up => [from[0], from[2], to[0], to[2]],
        Direction::North => [16.0 - to[0], 16.0 - to[1], 16.0 - from[0], 16.0 - from[1]],
        Direction::South => [from[0], 16.0 - to[1], to[0], 16.0 - from[1]],
        Direction::West => [from[2], 16.0 - to[1], to[2], 16.0 - from[1]],
        Direction::East => [16.0 - to[2], 16.0 - to[1], 16.0 - from[2], 16.0 - from[1]],
    }
}
fn corners(from: [f32; 3], to: [f32; 3], direction: Direction) -> [[f32; 3]; 4] {
    let selectors = match direction {
        Direction::Down => [[0, 0, 1], [0, 0, 0], [1, 0, 0], [1, 0, 1]],
        Direction::Up => [[0, 1, 0], [0, 1, 1], [1, 1, 1], [1, 1, 0]],
        Direction::North => [[1, 1, 0], [1, 0, 0], [0, 0, 0], [0, 1, 0]],
        Direction::South => [[0, 1, 1], [0, 0, 1], [1, 0, 1], [1, 1, 1]],
        Direction::West => [[0, 1, 0], [0, 0, 0], [0, 0, 1], [0, 1, 1]],
        Direction::East => [[1, 1, 1], [1, 0, 1], [1, 0, 0], [1, 1, 0]],
    };
    selectors
        .map(|selector| std::array::from_fn(|i| if selector[i] == 0 { from[i] } else { to[i] }))
}
fn facing(vertices: &[[f32; 3]; 4]) -> Option<Direction> {
    let a: [f32; 3] = std::array::from_fn(|i| vertices[1][i] - vertices[0][i]);
    let b: [f32; 3] = std::array::from_fn(|i| vertices[2][i] - vertices[0][i]);
    let n = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    let inverse_length = 1.0 / (n[0] * n[0] + (n[1] * n[1] + n[2] * n[2])).sqrt();
    let n = n.map(|v| v * inverse_length);
    if n.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let mut best = 0.0;
    let mut result = None;
    for (direction, dot) in Direction::ALL
        .into_iter()
        .zip([-n[1], n[1], -n[2], n[2], -n[0], n[0]])
    {
        if dot >= 0.0 && dot > best {
            best = dot;
            result = Some(direction);
        }
    }
    result
}

pub fn bake_model(model: &ResolvedCuboidModel) -> Result<Vec<ModelQuad>> {
    bake_model_transformed(model, BlockModelTransform::new([0; 3], false))
}
pub fn bake_model_transformed(
    model: &ResolvedCuboidModel,
    transform: BlockModelTransform,
) -> Result<Vec<ModelQuad>> {
    bake_elements(model, transform, false)
}
/// Block-model baking as `UnbakedCuboidGeometry.bake` does it: an element with
/// zero extent along an axis draws no faces across that extent (a flat X
/// element keeps only its X faces, and so on).
pub fn bake_block_model(
    model: &ResolvedCuboidModel,
    transform: BlockModelTransform,
) -> Result<Vec<ModelQuad>> {
    bake_elements(model, transform, true)
}
fn bake_elements(
    model: &ResolvedCuboidModel,
    transform: BlockModelTransform,
    skip_flat: bool,
) -> Result<Vec<ModelQuad>> {
    let mut output = Vec::new();
    bake_into(model, transform, skip_flat, &mut output)?;
    Ok(output)
}
/// The elements, then what the model's loader adds.
fn bake_into(
    model: &ResolvedCuboidModel,
    transform: BlockModelTransform,
    skip_flat: bool,
    output: &mut Vec<ModelQuad>,
) -> Result<()> {
    let render_type = model.render_type.as_deref().and_then(RenderType::by_name);
    for element in model.elements() {
        let element = element
            .as_ref()
            .ok_or_else(|| error("null model element"))?;
        let matrix = element.rotation.as_ref().map(ElementMatrix::from_rotation);
        let flat = |axis: usize| skip_flat && element.from[axis] == element.to[axis];
        for (direction, face) in &element.faces {
            let drawn = match direction {
                Direction::West | Direction::East => !flat(1) && !flat(2),
                Direction::Down | Direction::Up => !flat(0) && !flat(2),
                Direction::North | Direction::South => !flat(0) && !flat(1),
            };
            if !drawn {
                continue;
            }
            let face = face.as_ref().ok_or_else(|| error("null model face"))?;
            let material = model.material(&face.texture)?.cloned().unwrap_or(Material {
                sprite: "minecraft:missingno".to_owned(),
                force_translucent: false,
            });
            output.push(bake_face(
                element,
                face,
                *direction,
                matrix,
                material,
                transform,
                render_type,
            )?);
        }
    }
    if let Some(ResolvedLoader::Composite(children)) = model.loader() {
        for child in children {
            bake_into(child, transform, skip_flat, output)?;
        }
    }
    Ok(())
}
/// `FaceBakery.bakeQuad` for a quad with explicit corners and unit-atlas UVs
/// (`uv` in 0..16 texels), as `ItemModelGenerator` builds its layers.
pub fn bake_flat_face(
    from: [f32; 3],
    to: [f32; 3],
    uv: [f32; 4],
    direction: Direction,
    material: Material,
    tint_index: i32,
) -> Result<ModelQuad> {
    let element = Element {
        from,
        to,
        faces: Default::default(),
        rotation: None,
        shade_direction_override: None,
        light_emission: 0,
        neoforge_data: None,
    };
    let face = Face {
        cull: None,
        tint_index,
        texture: String::new(),
        uv: Some(uv),
        rotation: 0,
        light_emission: None,
        neoforge_data: None,
    };
    bake_face(
        &element,
        &face,
        direction,
        None,
        material,
        BlockModelTransform::new([0; 3], false),
        None,
    )
}
fn bake_face(
    element: &Element,
    face: &Face,
    direction: Direction,
    matrix: Option<ElementMatrix>,
    material: Material,
    transform: BlockModelTransform,
    render_type: Option<RenderType>,
) -> Result<ModelQuad> {
    let mut positions = corners(element.from, element.to, direction).map(|v| v.map(|c| c / 16.0));
    if let (Some(rotation), Some(matrix)) = (&element.rotation, matrix) {
        for vertex in &mut positions {
            let relative = std::array::from_fn(|i| vertex[i] - rotation.origin[i]);
            let rotated = matrix.position(relative);
            *vertex = std::array::from_fn(|i| rotated[i] + rotation.origin[i]);
        }
    }
    positions = positions.map(|v| transform.position(v));
    let bounds = face
        .uv
        .unwrap_or_else(|| default_uv(element.from, element.to, direction));
    let mut uv = std::array::from_fn(|i| {
        let corner = (i + usize::from(face.rotation)) % 4;
        transform
            .uv(
                direction,
                [
                    bounds[if corner < 2 { 0 } else { 2 }] / 16.0,
                    bounds[if corner == 0 || corner == 3 { 1 } else { 3 }] / 16.0,
                ],
            )
            // Native TextureAtlasSprite applies origin + extent * coordinate;
            // even the unit atlas changes negative zero to positive zero.
            .map(|value| 0.0 + value)
    });
    let normal = facing(&positions);
    if let (None, Some(facing)) = (&element.rotation, normal) {
        let mut min = [999.0; 3];
        let mut max = [-999.0; 3];
        for vertex in &positions {
            for axis in 0..3 {
                if vertex[axis] < min[axis] {
                    min[axis] = vertex[axis];
                }
                if vertex[axis] > max[axis] {
                    max[axis] = vertex[axis];
                }
            }
        }
        for (index, expected) in corners(min, max, facing).into_iter().enumerate() {
            let found = (index..4)
                .find(|i| positions[*i] == expected)
                .ok_or_else(|| error("cannot reorder model face"))?;
            positions.swap(index, found);
            uv.swap(index, found);
        }
    }
    let data = face
        .neoforge_data
        .unwrap_or_default()
        .over(element.neoforge_data.unwrap_or_default());
    let light_emission = face.light_emission.unwrap_or(element.light_emission);
    Ok(ModelQuad {
        positions,
        uv,
        facing: normal.unwrap_or(Direction::Up),
        cull: face.cull.map(|d| transform.direction(d)),
        material,
        tint_index: face.tint_index,
        shade_direction_override: element.shade_direction_override,
        light_emission,
        light: [data.block_light, data.sky_light].map(|l| l.unwrap_or(0).max(light_emission)),
        color: data.color,
        ambient_occlusion: data.ambient_occlusion,
        render_type,
    })
}

#[cfg(test)]
mod loader_tests {
    use super::*;
    use crate::resolve::ModelLibrary;

    fn library(models: &[(&str, &str)]) -> ModelLibrary {
        let models = models
            .iter()
            .map(|(name, json)| ((*name).to_owned(), CuboidModel::from_json(json).unwrap()))
            .collect();
        ModelLibrary::new(models).unwrap()
    }
    fn bake(library: &ModelLibrary, name: &str) -> Vec<ModelQuad> {
        bake_block_model(
            &library.resolve(name).unwrap(),
            BlockModelTransform::new([0; 3], false),
        )
        .unwrap()
    }
    fn sprites(quads: &[ModelQuad]) -> Vec<&str> {
        quads.iter().map(|q| q.material.sprite.as_str()).collect()
    }
    const CUBE: &str =
        r##"{"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#all"}}}]}"##;
    const COMPOSITE: &str = r#"{"loader":"neoforge:composite","textures":{"all":"block/shared"},
        "children":{"zeta":{"parent":"t:cube","textures":{"all":"block/own"}},"alpha":{"parent":"t:cube"}}}"#;

    #[test]
    fn composite_bakes_children_in_document_order_with_slot_fallback() {
        let library = library(&[("t:cube", CUBE), ("t:comp", COMPOSITE)]);
        let quads = bake(&library, "t:comp");
        assert_eq!(
            sprites(&quads),
            ["minecraft:block/own", "minecraft:block/shared"]
        );
    }
    #[test]
    fn composite_visibility_hides_a_child() {
        let hidden =
            COMPOSITE.replace(r#""children""#, r#""visibility":{"zeta":false},"children""#);
        let library = library(&[("t:cube", CUBE), ("t:comp", &hidden)]);
        assert_eq!(
            sprites(&bake(&library, "t:comp")),
            ["minecraft:block/shared"]
        );
    }
    #[test]
    fn an_inheriting_model_overrides_which_children_are_visible() {
        let hidden =
            COMPOSITE.replace(r#""children""#, r#""visibility":{"zeta":false},"children""#);
        let shown = r#"{"parent":"t:hidden","visibility":{"zeta":true,"alpha":false}}"#;
        let library = library(&[("t:cube", CUBE), ("t:hidden", &hidden), ("t:shown", shown)]);
        assert_eq!(sprites(&bake(&library, "t:shown")), ["minecraft:block/own"]);
    }
    #[test]
    fn loader_is_inherited_by_parent_after_own_elements() {
        let derived = r##"{"parent":"t:comp","textures":{"all":"block/derived"},
            "elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"down":{"texture":"#all"}}}]}"##;
        let library = library(&[
            ("t:cube", CUBE),
            ("t:comp", COMPOSITE),
            ("t:derived", derived),
        ]);
        let quads = bake(&library, "t:derived");
        assert_eq!(quads[0].facing, Direction::Down);
        assert_eq!(
            sprites(&quads),
            [
                "minecraft:block/derived",
                "minecraft:block/own",
                "minecraft:block/derived"
            ]
        );
    }
    #[test]
    fn own_loader_wins_and_unknown_loader_adds_nothing() {
        let other = r#"{"parent":"t:comp","loader":"somemod:thing"}"#;
        let library = library(&[("t:cube", CUBE), ("t:comp", COMPOSITE), ("t:other", other)]);
        assert!(bake(&library, "t:other").is_empty());
    }
    #[test]
    fn composites_that_nest_through_themselves_are_an_error() {
        let looping = r#"{"loader":"neoforge:composite","children":{"a":{"parent":"t:loop"}}}"#;
        let library = library(&[("t:loop", looping)]);
        assert!(library.resolve("t:loop").is_err());
    }
}
