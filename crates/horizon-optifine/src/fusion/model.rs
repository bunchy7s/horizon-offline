//! Fusion block models (`"loader": "fusion:model"`): the model type, and the `connections` of a
//! connecting model, which name the predicates each face's connecting texture joins by.

use super::{Connection, kind};
use crate::cem::json::Json;

/// A model's Fusion type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelType {
    Base,
    Connecting,
    Composite,
}

impl ModelType {
    /// `None` for a model Fusion leaves to vanilla.
    pub fn of(model: &Json) -> Option<Self> {
        if model.get("loader").and_then(Json::as_str) != Some("fusion:model") {
            return None;
        }
        Some(match kind(model.get("type")?.as_str()?) {
            "base" => Self::Base,
            "connecting" => Self::Connecting,
            "composite" => Self::Composite,
            _ => return None,
        })
    }
}

/// A connecting model's `connections`.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Connections {
    /// None given: each texture joins by its own.
    #[default]
    Textures,
    /// One set of predicates for every face.
    All(Connection),
    /// Predicates by key, as `textures` names textures: each a predicate or `#another`.
    Keys(Vec<(String, Entry)>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Predicate(Connection),
    Ref(String),
}

/// `#key` chains longer than this are a loop.
const DEPTH: usize = 32;

impl Connections {
    /// A model's `connections` value: a predicate (an object with a `type`, or an array), or an
    /// object of keys.
    pub fn parse(json: &Json) -> Result<Self, String> {
        let keys = match json {
            Json::Obj(keys) if json.get("type").is_none() => keys,
            _ => return Ok(Self::All(Connection::parse(json)?)),
        };
        keys.iter()
            .map(|(key, value)| {
                let entry = match value.as_str() {
                    Some(r) => Entry::Ref(r.strip_prefix('#').unwrap_or(r).to_owned()),
                    None => Entry::Predicate(
                        Connection::parse(value)
                            .map_err(|e| format!("connections `{key}`: {e}"))?,
                    ),
                };
                Ok((key.clone(), entry))
            })
            .collect::<Result<_, String>>()
            .map(Self::Keys)
    }

    /// The model's `connections`: [`Connections::Textures`] without one.
    pub fn of(model: &Json) -> Result<Self, String> {
        model
            .get("connections")
            .map_or(Ok(Self::Textures), Self::parse)
    }

    /// A child model's over its parent's: a child's keys add to and replace the parent's, and a
    /// parent's predicates for every face stand for the keys the child doesn't define.
    pub fn inherit(self, parent: &Self) -> Self {
        match (self, parent) {
            (Self::Textures, parent) => parent.clone(),
            (Self::Keys(mut child), Self::Keys(parent)) => {
                for (key, entry) in parent {
                    if !child.iter().any(|(k, _)| k == key) {
                        child.push((key.clone(), entry.clone()));
                    }
                }
                Self::Keys(child)
            }
            (Self::Keys(mut child), Self::All(all)) => {
                if !child.iter().any(|(k, _)| k == "default") {
                    child.push(("default".to_owned(), Entry::Predicate(all.clone())));
                }
                Self::Keys(child)
            }
            (child, _) => child,
        }
    }

    /// The predicates of a face: its `connections` reference, else its `texture` one (each
    /// `#key` or `key`). A key the model doesn't define takes `default`'s; `None` leaves the
    /// texture's own.
    pub fn for_face(&self, connections: Option<&str>, texture: &str) -> Option<&Connection> {
        let keys = match self {
            Self::Textures => return None,
            Self::All(c) => return Some(c),
            Self::Keys(keys) => keys,
        };
        let get = |key: &str| keys.iter().rev().find(|(k, _)| k == key).map(|(_, e)| e);
        let key = connections.unwrap_or(texture);
        let mut key = key.strip_prefix('#').unwrap_or(key);
        for _ in 0..DEPTH {
            match get(key).or_else(|| get("default"))? {
                Entry::Predicate(c) => return Some(c),
                Entry::Ref(next) => key = next,
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BlockMatch;

    fn json(text: &str) -> Json {
        Json::parse(text).unwrap()
    }

    fn block(id: &str) -> Connection {
        Connection::Neighbour(vec![BlockMatch {
            id: id.into(),
            states: vec![],
        }])
    }

    #[test]
    fn model_types() {
        let ty = |text| ModelType::of(&json(text));
        assert_eq!(
            ty(r#"{"type": "fusion:connecting", "loader": "fusion:model"}"#),
            Some(ModelType::Connecting)
        );
        assert_eq!(
            ty(r#"{"loader": "fusion:model", "type": "base"}"#),
            Some(ModelType::Base)
        );
        // Without the loader Fusion leaves a model to vanilla.
        assert_eq!(ty(r#"{"type": "connecting"}"#), None);
    }

    #[test]
    fn keys_resolve_like_textures() {
        // The wiki's example: ice and lapis join by `blue`, dirt by its own, stone by `default`.
        let c = Connections::of(&json(
            r##"{"connections": {
                "blue": [{"type": "match_block", "block": "ice"}, {"type": "match_block", "block": "lapis_block"}],
                "lapis": "#blue",
                "dirt": {"type": "is_same_block"},
                "default": {"type": "is_same_state"}
            }}"##,
        ))
        .unwrap();
        let blue = Connection::Or(vec![block("minecraft:ice"), block("minecraft:lapis_block")]);
        assert_eq!(c.for_face(Some("#blue"), "#ice"), Some(&blue));
        assert_eq!(c.for_face(Some("#lapis"), "#lapis"), Some(&blue));
        assert_eq!(c.for_face(None, "#dirt"), Some(&Connection::SameBlock));
        assert_eq!(c.for_face(None, "#stone"), Some(&Connection::SameState));
    }

    #[test]
    fn one_predicate_for_every_face() {
        let c = Connections::of(&json(
            r#"{"connections": {"type": "fusion:or", "predicates": [{"type": "fusion:is_same_block"}]}}"#,
        ))
        .unwrap();
        assert_eq!(
            c.for_face(None, "#pane"),
            Some(&Connection::Or(vec![Connection::SameBlock]))
        );
        // No connections: the textures' own; no `default`: also the texture's.
        assert_eq!(
            Connections::of(&json("{}")).unwrap().for_face(None, "#all"),
            None
        );
        let keyed =
            Connections::of(&json(r#"{"connections": {"top": {"type": "true"}}}"#)).unwrap();
        assert_eq!(keyed.for_face(None, "#side"), None);
        assert_eq!(keyed.for_face(None, "top"), Some(&Connection::Const(true)));
    }

    #[test]
    fn loops_and_inheritance() {
        let looped =
            Connections::of(&json(r##"{"connections": {"a": "#b", "b": "#a"}}"##)).unwrap();
        assert_eq!(looped.for_face(None, "#a"), None);
        // A reference to a key nobody defines takes `default`'s.
        let dangling = Connections::of(&json(
            r##"{"connections": {"a": "#missing", "default": {"type": "true"}}}"##,
        ))
        .unwrap();
        assert_eq!(
            dangling.for_face(None, "#a"),
            Some(&Connection::Const(true))
        );
        // A parent's predicates for every face fill the keys a child leaves out, unless the
        // child has its own `default`.
        let all = Connections::All(Connection::SameBlock);
        let child = Connections::of(&json(r#"{"connections": {"top": {"type": "true"}}}"#))
            .unwrap()
            .inherit(&all);
        assert_eq!(child.for_face(None, "#top"), Some(&Connection::Const(true)));
        assert_eq!(child.for_face(None, "#side"), Some(&Connection::SameBlock));
        let child = Connections::of(&json(r#"{"connections": {"default": {"type": "false"}}}"#))
            .unwrap()
            .inherit(&all);
        assert_eq!(
            child.for_face(None, "#side"),
            Some(&Connection::Const(false))
        );
        let parent = Connections::of(&json(
            r#"{"connections": {"top": {"type": "true"}, "side": {"type": "false"}}}"#,
        ))
        .unwrap();
        let child = Connections::of(&json(
            r#"{"connections": {"side": {"type": "is_same_block"}}}"#,
        ))
        .unwrap()
        .inherit(&parent);
        assert_eq!(child.for_face(None, "#top"), Some(&Connection::Const(true)));
        assert_eq!(child.for_face(None, "#side"), Some(&Connection::SameBlock));
        assert_eq!(
            Connections::Textures
                .inherit(&parent)
                .for_face(None, "#top"),
            Some(&Connection::Const(true))
        );
    }
}
