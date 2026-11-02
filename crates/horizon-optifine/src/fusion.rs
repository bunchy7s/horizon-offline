//! Fusion's resource pack format (the Fusion Connected Textures mod): a `fusion` section in a
//! texture's `.mcmeta` ([`texture`]: connecting, random, continuous and scrolling textures, and
//! the emissive, render type and tinting options every type takes), the connection predicates
//! that say when a connecting texture joins a neighbour ([`predicate`]), the `connections` of a
//! `fusion:connecting` block model ([`model`]), and which tiles of a connecting texture's image a
//! face draws ([`layout`]).
//!
//! The layouts read the neighbour masks of [`crate::ctm::connect`], which OptiFine's connected
//! textures build too: the two formats share how a face finds its neighbours.
//!
//! Written from Fusion's public wiki (its texture type, model type and model predicate pages)
//! and the MIT-licensed layout templates of SuperMartijn642/FusionExamples, read off the images.
//! Fusion itself is all rights reserved: none of its code was read.

pub mod layout;
pub mod model;
pub mod predicate;
pub mod texture;

pub use layout::{Layout, Tiles};
pub use predicate::Connection;
pub use texture::Meta;

use crate::cem::json::Json;

/// A type name without the `fusion:` namespace packs may give it (`fusion:connecting`).
fn kind(name: &str) -> &str {
    name.strip_prefix("fusion:").unwrap_or(name)
}

fn bool_field(json: &Json, key: &str) -> Result<Option<bool>, String> {
    match json.get(key) {
        None => Ok(None),
        Some(Json::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("`{key}` is not true or false")),
    }
}

fn str_field<'a>(json: &'a Json, key: &str) -> Result<Option<&'a str>, String> {
    match json.get(key) {
        None => Ok(None),
        Some(Json::Str(s)) => Ok(Some(s)),
        Some(_) => Err(format!("`{key}` is not a string")),
    }
}

/// An integer field, which must be in `range`.
fn int_field(
    json: &Json,
    key: &str,
    range: std::ops::RangeInclusive<i64>,
) -> Result<Option<i64>, String> {
    match json.get(key) {
        None => Ok(None),
        Some(Json::Num(n)) if n.fract() == 0.0 && range.contains(&(*n as i64)) => {
            Ok(Some(*n as i64))
        }
        Some(_) => Err(format!(
            "`{key}` is not an integer from {} to {}",
            range.start(),
            range.end()
        )),
    }
}
