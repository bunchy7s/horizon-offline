//! Cosmetic items that Horizon clients draw on players: hats, back items,
//! shoulder pets. Only Horizon clients see them; the game protocol carries
//! nothing about them ([docs/cosmetics.md](../../../docs/cosmetics.md)).
//!
//! An item is a small model of boxes (Minecraft's box UV, as Blockbench's
//! "Modded Entity" format), one texture of at most 64×64, the bone of the
//! player model it rides on and a few simple looping animations. This crate
//! reads and checks items and poses them; the game draws them.

mod fetch;
mod item;
mod pose;

pub use fetch::{BATCH, Clock, Cosmetics, Frame, Http, Transport, Worn};
pub use item::{
    Animation, Axis, Bone, Error, FORMAT, Face, Item, MAX_PARTS, MAX_REACH, MAX_TEXTURE,
    MAX_VERTICES, Motion, Part, Slot, Texture,
};
pub use pose::{Pose, pose_matrix};
