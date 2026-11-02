//! Connection predicates: whether a connecting texture joins the neighbour in one of the eight
//! directions of its face. Each direction is tested on its own, and the true ones make the
//! face's [`crate::ctm::connect`] mask.
//!
//! The predicate's block lists are `B`: [`BlockMatch`]es as parsed, which the game turns into
//! whatever it tests block states with ([`Connection::map_blocks`]). What a test reads of the
//! world, the game answers ([`Connection::joins`]).

use super::kind;
use crate::BlockMatch;
use crate::cem::json::Json;
use crate::ctm::connect::{D, DL, DR, L, R, U, UL, UR};

#[derive(Clone, Debug, PartialEq)]
pub enum Connection<B = Vec<BlockMatch>> {
    /// `is_same_block`: the neighbour is the same block.
    SameBlock,
    /// `is_same_state`: the same block in the same state.
    SameState,
    /// `match_block`, `match_state`: the neighbour is one of these.
    Neighbour(B),
    /// `match_block_in_front`, `match_state_in_front`: the block in front of the neighbour (on
    /// the side the face looks out of) is one of these.
    InFront(B),
    /// `is_direction`: the direction is one of these mask bits.
    Direction(u8),
    /// `is_face_visible`: the neighbour's face on this side isn't covered.
    FaceVisible,
    And(Vec<Connection<B>>),
    Or(Vec<Connection<B>>),
    Not(Box<Connection<B>>),
    Const(bool),
}

/// A test only the world answers, for one direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Query<'a, B> {
    SameBlock,
    SameState,
    Neighbour(&'a B),
    InFront(&'a B),
    FaceVisible,
}

/// Directions in the texture's terms, as mask bits.
fn direction(name: &str) -> Option<u8> {
    Some(match name {
        "top" => U,
        "top_right" => UR,
        "right" => R,
        "bottom_right" => DR,
        "bottom" => D,
        "bottom_left" => DL,
        "left" => L,
        "top_left" => UL,
        _ => return None,
    })
}

/// A value, or each value of an array.
fn one_or_many(value: &Json) -> &[Json] {
    match value {
        Json::Arr(items) => items,
        single => std::slice::from_ref(single),
    }
}

/// `one` or `many` of a field: a string or an array of them.
fn names<'a>(json: &'a Json, one: &str, many: &str) -> Result<Vec<&'a str>, String> {
    let (key, value) = [one, many]
        .into_iter()
        .find_map(|key| Some((key, json.get(key)?)))
        .ok_or_else(|| format!("no `{one}` or `{many}`"))?;
    one_or_many(value)
        .iter()
        .map(|v| {
            v.as_str()
                .ok_or_else(|| format!("`{key}` holds a non-string"))
        })
        .collect()
}

/// `block`/`blocks`, each with `properties` (a value or an array of values per property) when
/// `states` is set.
fn blocks(json: &Json, states: bool) -> Result<Vec<BlockMatch>, String> {
    let mut properties = Vec::new();
    if states && let Some(map) = json.get("properties") {
        for (key, values) in map.as_object().ok_or("`properties` is not an object")? {
            let values = one_or_many(values)
                .iter()
                .map(|v| match v {
                    Json::Str(s) => Ok(s.clone()),
                    Json::Bool(b) => Ok(b.to_string()),
                    Json::Num(n) => Ok(n.to_string()),
                    _ => Err(format!("property `{key}` has a value that isn't one")),
                })
                .collect::<Result<_, String>>()?;
            properties.push((key.clone(), values));
        }
    }
    Ok(names(json, "block", "blocks")?
        .into_iter()
        .map(|name| BlockMatch {
            id: crate::matchers::namespaced(name),
            states: properties.clone(),
        })
        .collect())
}

impl Connection {
    /// A predicate, or an array of them that joins when any does (a texture's or a model's
    /// `connections`).
    pub fn parse(json: &Json) -> Result<Self, String> {
        if let Some(items) = json.as_array() {
            return Ok(Self::Or(Self::parse_all(items)?));
        }
        let ty = super::str_field(json, "type")?.ok_or("a predicate without a `type`")?;
        let list = |key: &str| match json.get(key).and_then(Json::as_array) {
            Some(items) => Self::parse_all(items),
            None => Err(format!("`{ty}` without a `{key}` array")),
        };
        Ok(match kind(ty) {
            "is_same_block" => Self::SameBlock,
            "is_same_state" => Self::SameState,
            "match_block" => Self::Neighbour(blocks(json, false)?),
            "match_state" => Self::Neighbour(blocks(json, true)?),
            "match_block_in_front" => Self::InFront(blocks(json, false)?),
            "match_state_in_front" => Self::InFront(blocks(json, true)?),
            "is_direction" => Self::Direction(
                names(json, "direction", "directions")?
                    .into_iter()
                    .map(|d| direction(d).ok_or_else(|| format!("unknown direction `{d}`")))
                    .try_fold(0, |mask, bit| bit.map(|b| mask | b))?,
            ),
            "is_face_visible" => Self::FaceVisible,
            "and" => Self::And(list("predicates")?),
            "or" => Self::Or(list("predicates")?),
            "not" => Self::Not(Box::new(Self::parse(
                json.get("predicate").ok_or("`not` without a `predicate`")?,
            )?)),
            "true" => Self::Const(true),
            "false" => Self::Const(false),
            other => return Err(format!("unknown connection predicate `{other}`")),
        })
    }

    fn parse_all(items: &[Json]) -> Result<Vec<Self>, String> {
        items.iter().map(Self::parse).collect()
    }
}

impl<B> Connection<B> {
    /// The same predicate with its block lists turned into `C`s.
    pub fn map_blocks<C>(self, f: &mut impl FnMut(B) -> C) -> Connection<C> {
        let all =
            |items: Vec<Self>, f: &mut _| items.into_iter().map(|p| p.map_blocks(f)).collect();
        match self {
            Self::SameBlock => Connection::SameBlock,
            Self::SameState => Connection::SameState,
            Self::Neighbour(b) => Connection::Neighbour(f(b)),
            Self::InFront(b) => Connection::InFront(f(b)),
            Self::Direction(d) => Connection::Direction(d),
            Self::FaceVisible => Connection::FaceVisible,
            Self::And(items) => Connection::And(all(items, f)),
            Self::Or(items) => Connection::Or(all(items, f)),
            Self::Not(p) => Connection::Not(Box::new(p.map_blocks(f))),
            Self::Const(c) => Connection::Const(c),
        }
    }

    /// Whether the face joins toward `direction` (one mask bit), `world` answering the tests
    /// about the neighbour there. `and` and `or` stop at their first deciding predicate.
    pub fn joins(&self, direction: u8, world: &mut impl FnMut(Query<'_, B>) -> bool) -> bool {
        match self {
            Self::SameBlock => world(Query::SameBlock),
            Self::SameState => world(Query::SameState),
            Self::Neighbour(b) => world(Query::Neighbour(b)),
            Self::InFront(b) => world(Query::InFront(b)),
            Self::Direction(d) => d & direction != 0,
            Self::FaceVisible => world(Query::FaceVisible),
            Self::And(items) => items.iter().all(|p| p.joins(direction, world)),
            Self::Or(items) => items.iter().any(|p| p.joins(direction, world)),
            Self::Not(p) => !p.joins(direction, world),
            Self::Const(c) => *c,
        }
    }

    /// The mask of the eight directions that join, `world` answering for the direction it is
    /// given.
    pub fn mask(&self, mut world: impl FnMut(u8, Query<'_, B>) -> bool) -> u8 {
        [U, UR, R, DR, D, DL, L, UL]
            .into_iter()
            .filter(|&bit| self.joins(bit, &mut |q| world(bit, q)))
            .fold(0, |mask, bit| mask | bit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Connection {
        Connection::parse(&Json::parse(text).unwrap()).unwrap()
    }

    #[test]
    fn parses_every_predicate() {
        let p = parse(
            r#"[
                {"type": "fusion:is_same_block"},
                {"type": "match_state", "blocks": ["redstone_torch", "create:lamp"],
                 "properties": {"lit": "false", "facing": ["north", "south"]}},
                {"type": "and", "predicates": [
                    {"type": "match_block_in_front", "block": "Sand"},
                    {"type": "not", "predicate": {"type": "is_direction", "directions": ["top", "bottom_left"]}},
                    {"type": "is_face_visible"}, {"type": "true"}
                ]}
            ]"#,
        );
        let states = vec![
            ("lit".to_owned(), vec!["false".to_owned()]),
            (
                "facing".to_owned(),
                vec!["north".to_owned(), "south".to_owned()],
            ),
        ];
        assert_eq!(
            p,
            Connection::Or(vec![
                Connection::SameBlock,
                Connection::Neighbour(vec![
                    BlockMatch {
                        id: "minecraft:redstone_torch".into(),
                        states: states.clone(),
                    },
                    BlockMatch {
                        id: "create:lamp".into(),
                        states,
                    },
                ]),
                Connection::And(vec![
                    Connection::InFront(vec![BlockMatch {
                        id: "minecraft:sand".into(),
                        states: vec![],
                    }]),
                    Connection::Not(Box::new(Connection::Direction(U | DL))),
                    Connection::FaceVisible,
                    Connection::Const(true),
                ]),
            ])
        );
        // `match_block` has no properties to test.
        assert_eq!(
            parse(r#"{"type": "match_block", "block": "stone", "properties": {"a": "b"}}"#),
            Connection::Neighbour(vec![BlockMatch {
                id: "minecraft:stone".into(),
                states: vec![],
            }])
        );
    }

    #[test]
    fn directions_and_property_values() {
        for (name, bit) in [
            ("top", U),
            ("top_right", UR),
            ("right", R),
            ("bottom_right", DR),
            ("bottom", D),
            ("bottom_left", DL),
            ("left", L),
            ("top_left", UL),
        ] {
            let p = parse(&format!(
                r#"{{"type": "is_direction", "direction": "{name}"}}"#
            ));
            assert_eq!(p, Connection::Direction(bit), "{name}");
        }
        // Booleans and numbers as block states write them.
        assert_eq!(
            parse(
                r#"{"type": "match_state", "block": "light", "properties": {"waterlogged": false, "level": 15}}"#
            ),
            Connection::Neighbour(vec![BlockMatch {
                id: "minecraft:light".into(),
                states: vec![
                    ("waterlogged".into(), vec!["false".into()]),
                    ("level".into(), vec!["15".into()]),
                ],
            }])
        );
    }

    #[test]
    fn refuses_what_it_cannot_read() {
        for bad in [
            r#"{"type": "is_same_thing"}"#,
            r#"{"block": "stone"}"#,
            r#"{"type": "match_block"}"#,
            r#"{"type": "is_direction", "direction": "up"}"#,
            r#"{"type": "and", "predicates": {"type": "true"}}"#,
            r#"{"type": "not"}"#,
            r#"{"type": "match_state", "block": "stone", "properties": ["lit"]}"#,
        ] {
            assert!(
                Connection::parse(&Json::parse(bad).unwrap()).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn masks_from_the_world() {
        // Block transitions' sand edge: sand beside the face, or sand in front of the
        // neighbour, but only where the neighbour's face shows.
        let p = parse(
            r#"{"type": "or", "predicates": [
                {"type": "match_block_in_front", "block": "sand"},
                {"type": "and", "predicates": [
                    {"type": "match_block", "block": "sand"}, {"type": "is_face_visible"}]}
            ]}"#,
        );
        // Sand to the right, its face covered; sand in front of the neighbour above.
        let mask = p.mask(|dir, q| match q {
            Query::Neighbour(_) => dir == R,
            Query::InFront(_) => dir == U,
            Query::FaceVisible => false,
            _ => unreachable!("not in the predicate"),
        });
        assert_eq!(mask, U);
        // `is_direction` needs no world; `not` and `and` combine it.
        let p = parse(
            r#"{"type": "and", "predicates": [{"type": "is_same_state"},
                {"type": "not", "predicate": {"type": "is_direction", "direction": "left"}}]}"#,
        );
        assert_eq!(
            p.mask(|_, q| q == Query::SameState),
            !L,
            "joins everywhere but left"
        );
    }

    #[test]
    fn map_blocks_keeps_the_shape() {
        let p = parse(r#"{"type": "not", "predicate": {"type": "match_block", "block": "dirt"}}"#);
        let mapped = p.map_blocks(&mut |b: Vec<BlockMatch>| b.len());
        assert_eq!(mapped, Connection::Not(Box::new(Connection::Neighbour(1))));
    }
}
