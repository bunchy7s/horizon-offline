//! Nodes and properties of other namespaces (a mod's, NeoForge's) decode to the closest vanilla behaviour.
use super::{
    ItemModelNode, ModelLeaf, Result, TintSource, get, ident, identifier, list, object,
    optional_transformation,
};
use crate::{json::error as err, math::Mat4};
use serde_json::{Map, Value};

/// Keys a mod's node usually keeps its real model under, in order of preference.
const MODEL_KEYS: [&str; 4] = ["model", "base_model", "base", "fallback"];

pub fn is_foreign(id: &str) -> bool {
    !id.starts_with("minecraft:")
}

/// A composite carries the transformation a collapsed node had.
pub fn wrap(node: ItemModelNode, transformation: Option<Mat4>) -> ItemModelNode {
    match transformation {
        None => node,
        Some(_) => ItemModelNode::Composite {
            models: vec![node],
            transformation,
        },
    }
}

pub fn property_is_foreign(m: &Map<String, Value>) -> Result<bool> {
    let property = m
        .get("property")
        .ok_or_else(|| err("missing property"))
        .and_then(ident)?;
    Ok(is_foreign(&property))
}

fn leaf(model: String, m: &Map<String, Value>) -> ItemModelNode {
    let tints = get(m, "tints")
        .and_then(|t| list(t).ok())
        .map(|t| {
            t.iter()
                .filter_map(|t| TintSource::decode(t).ok())
                .collect()
        })
        .unwrap_or_default();
    ItemModelNode::Model(ModelLeaf {
        model,
        transformation: optional_transformation(m).ok().flatten(),
        tints,
    })
}

/// A node of an unknown non-vanilla type: its nested node, else its model id, else nothing.
pub fn decode_node(kind: &str, m: &Map<String, Value>) -> ItemModelNode {
    if kind == "neoforge:fluid_container" {
        return fluid_container(m);
    }
    for key in MODEL_KEYS {
        match m.get(key) {
            Some(nested @ Value::Object(_)) => {
                if let Ok(node) = ItemModelNode::decode(nested) {
                    return wrap(node, optional_transformation(m).ok().flatten());
                }
            }
            Some(Value::String(id)) => {
                if let Ok(id) = identifier(id) {
                    return leaf(id, m);
                }
            }
            _ => {}
        }
    }
    let by_suffix = m
        .iter()
        .filter(|(k, _)| k.ends_with("_model"))
        .find_map(|(_, v)| v.as_str().and_then(|id| identifier(id).ok()));
    match by_suffix {
        Some(id) => leaf(id, m),
        None => ItemModelNode::Empty,
    }
}

// The fluid is not drawn: only the base (and an unmasked cover) texture, taken as a model id (a bucket's).
fn fluid_container(m: &Map<String, Value>) -> ItemModelNode {
    let textures = m.get("textures").and_then(|t| object(t).ok());
    let texture = |key: &str| {
        textures
            .and_then(|t| t.get(key))
            .and_then(Value::as_str)
            .and_then(|id| identifier(id).ok())
    };
    let mut layers = vec![texture("base")];
    if m.get("cover_is_mask") == Some(&Value::Bool(false)) {
        layers.push(texture("cover"));
    }
    let mut models: Vec<ItemModelNode> = layers
        .into_iter()
        .flatten()
        .map(|model| {
            ItemModelNode::Model(ModelLeaf {
                model,
                transformation: None,
                tints: Vec::new(),
            })
        })
        .collect();
    let transformation = optional_transformation(m).ok().flatten();
    match models.len() {
        0 => ItemModelNode::Empty,
        1 => wrap(models.remove(0), transformation),
        _ => ItemModelNode::Composite {
            models,
            transformation,
        },
    }
}
