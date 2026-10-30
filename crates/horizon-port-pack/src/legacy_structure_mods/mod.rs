//! The conversion of three families of structure mods' data, kept apart from
//! the general conversion code (`worldgen`, `processors`, `carry`):
//!
//! - [`yung`]: YUNG's API and the mods built on it (Better Dungeons, Desert
//!   Temples, Jungle Temples, Witch Huts, Ocean Monuments, Strongholds,
//!   Fortresses, Mineshafts), written under `horizon:yungsapi` and the other
//!   keys `horizon_world::worldgen::structure::yung` reads.
//! - [`lithostitched`]: Lithostitched's jigsaw, worldgen modifiers and
//!   processors.
//! - [`ctov`]: Choice Theorem's Overhauled Village's code-registered set
//!   entries.
//!
//! The general code reaches them only through the `pub(crate)` entry points
//! of these three modules. Nothing in them is used by another mod's
//! conversion.
pub(crate) mod ctov;
