//! Wavefront OBJ and MTL text, as obj-loader models use them.
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObjVertex {
    /// Block units.
    pub position: [f32; 3],
    /// Texture coordinates as written, 0..1 across the sprite.
    pub uv: [f32; 2],
}
/// One quad; a triangle repeats its last vertex.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjFace {
    /// Empty before any `usemtl`.
    pub material: String,
    /// The `o` (else `g`) it belongs to, which a model's `visibility` names.
    pub object: String,
    pub vertices: [ObjVertex; 4],
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParsedObj {
    pub mtllib: Option<String>,
    pub faces: Vec<ObjFace>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MtlTexture {
    /// `map_Kd #name`: a texture slot of the model.
    Slot(String),
    /// `map_Kd namespace:path`: a sprite.
    Sprite(String),
}
/// A parsed OBJ with its materials.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObjModel {
    pub faces: Vec<ObjFace>,
    pub materials: BTreeMap<String, MtlTexture>,
}

pub fn parse_obj(text: &str) -> Result<ParsedObj, String> {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut parsed = ParsedObj::default();
    let mut material = String::new();
    let mut object = String::new();
    for (number, line) in text.lines().enumerate() {
        let mut words = line.split_whitespace();
        let Some(keyword) = words.next() else {
            continue;
        };
        let at = |message: &str| format!("line {}: {message}", number + 1);
        match keyword {
            _ if keyword.starts_with('#') => {}
            "v" => positions.push(floats::<3>(words).ok_or_else(|| at("bad vertex"))?),
            "vt" => {
                let mut words = words;
                let u = words.next().and_then(|w| w.parse().ok());
                let v = words.next().map_or(Some(0.0), |w| w.parse().ok());
                let (Some(u), Some(v)) = (u, v) else {
                    return Err(at("bad texture coordinate"));
                };
                uvs.push([u, v]);
            }
            "mtllib" => parsed.mtllib = words.next().map(str::to_owned),
            "usemtl" => material = words.next().unwrap_or("").to_owned(),
            "o" | "g" => object = words.next().unwrap_or("").to_owned(),
            "f" => {
                let mut corners = Vec::new();
                for word in words {
                    let mut parts = word.split('/');
                    let position = index(parts.next(), positions.len())
                        .ok_or_else(|| at("bad vertex index"))?;
                    let uv = match parts.next() {
                        None | Some("") => [0.0; 2],
                        some => {
                            uvs[index(some, uvs.len()).ok_or_else(|| at("bad texture index"))?]
                        }
                    };
                    corners.push(ObjVertex {
                        position: positions[position],
                        uv,
                    });
                }
                if corners.len() < 3 {
                    return Err(at("face needs three vertices"));
                }
                let mut push = |vertices: [ObjVertex; 4]| {
                    parsed.faces.push(ObjFace {
                        material: material.clone(),
                        object: object.clone(),
                        vertices,
                    });
                };
                if corners.len() <= 4 {
                    let last = corners[corners.len() - 1];
                    push(std::array::from_fn(|i| {
                        corners.get(i).copied().unwrap_or(last)
                    }));
                } else {
                    for i in 1..corners.len() - 1 {
                        push([corners[0], corners[i], corners[i + 1], corners[i + 1]]);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(parsed)
}
fn floats<'a, const N: usize>(mut words: impl Iterator<Item = &'a str>) -> Option<[f32; N]> {
    let mut result = [0.0; N];
    for value in &mut result {
        *value = words.next()?.parse().ok()?;
    }
    Some(result)
}
/// A 1-based index, or a negative one counted from the end.
fn index(word: Option<&str>, len: usize) -> Option<usize> {
    let value: i64 = word?.parse().ok()?;
    let index = if value < 0 {
        len as i64 + value
    } else {
        value - 1
    };
    (0..len as i64).contains(&index).then_some(index as usize)
}

/// Material name to its `map_Kd`; other statements are ignored.
pub fn parse_mtl(text: &str) -> BTreeMap<String, MtlTexture> {
    let mut materials = BTreeMap::new();
    let mut current = None;
    for line in text.lines() {
        let mut words = line.split_whitespace();
        match words.next() {
            Some("newmtl") => current = words.next().map(str::to_owned),
            Some("map_Kd") => {
                let (Some(name), Some(value)) = (&current, words.last()) else {
                    continue;
                };
                let texture = match value.strip_prefix('#') {
                    Some(slot) => MtlTexture::Slot(slot.to_owned()),
                    None => MtlTexture::Sprite(value.to_owned()),
                };
                materials.insert(name.clone(), texture);
            }
            _ => {}
        }
    }
    materials
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quads_triangles_and_negative_indices() {
        let text = "mtllib a.mtl\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\n\
                    usemtl first\nf 1/1/1 2/2/1 3/3/1 4/4/1\nusemtl second\nf -4//1 -3//1 -2//1\n";
        let parsed = parse_obj(text).unwrap();
        assert_eq!(parsed.mtllib.as_deref(), Some("a.mtl"));
        assert_eq!(parsed.faces.len(), 2);
        assert_eq!(parsed.faces[0].material, "first");
        assert_eq!(parsed.faces[0].vertices[2].uv, [1.0, 1.0]);
        let triangle = &parsed.faces[1];
        assert_eq!(triangle.material, "second");
        assert_eq!(triangle.vertices[0].position, [0.0, 0.0, 0.0]);
        assert_eq!(triangle.vertices[3], triangle.vertices[2]);
        assert_eq!(triangle.vertices[2].position, [1.0, 1.0, 0.0]);
    }
    #[test]
    fn bad_indices_are_errors() {
        assert!(parse_obj("v 0 0 0\nf 1 2 3\n").is_err());
        assert!(parse_obj("v 0 0 0\nv 1 0 0\nf 1 2\n").is_err());
    }
    #[test]
    fn mtl_maps_slots_and_sprites() {
        let materials =
            parse_mtl("newmtl a\nKd 1 1 1\nmap_Kd #top\nnewmtl b\nmap_Kd mod:block/x\nnewmtl c\n");
        assert_eq!(materials["a"], MtlTexture::Slot("top".into()));
        assert_eq!(materials["b"], MtlTexture::Sprite("mod:block/x".into()));
        assert!(!materials.contains_key("c"));
    }
}
