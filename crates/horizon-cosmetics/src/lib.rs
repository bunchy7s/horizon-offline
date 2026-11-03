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
pub mod preview;

pub use fetch::{BATCH, Clock, Cosmetics, Frame, Http, Local, Transport, Worn};
pub use item::{
    Animation, Axis, Bone, Error, FORMAT, Face, Item, MAX_PARTS, MAX_REACH, MAX_TEXTURE,
    MAX_VERTICES, Motion, Part, Slot, Texture,
};
pub use pose::{Pose, pose_matrix};

/// Cosmetics stay off until this is true: no request, no drawing, no settings.
pub const ON_BY_DEFAULT: bool = false;

/// The cosmetics service (`HORIZON_COSMETICS_API` overrides it).
pub const API: &str = "https://cosmetics.playhorizon.dev";

/// Whether cosmetics run: `ON_BY_DEFAULT`, or `HORIZON_COSMETICS=1` for one run (development,
/// the bench). `HORIZON_COSMETICS=0` keeps them off.
pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| match std::env::var("HORIZON_COSMETICS").as_deref() {
        Ok("1") => true,
        Ok("0") => false,
        _ => ON_BY_DEFAULT,
    })
}
