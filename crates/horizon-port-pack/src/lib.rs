//! Port a Java modpack to Horizon (`docs/modpacks.md`): the steps behind the
//! `horizon-port-pack` command, and the launcher's "Import modpack".
//!
//! Every step reads and writes files in a work folder (`pipeline`), so it can
//! be rerun, resumed and inspected. The work folder belongs to the user: the
//! jars they downloaded and what is carried over from them stay there and
//! are never uploaded or committed.
pub mod carry;
pub mod check;
pub mod classes;
pub mod classify;
pub mod fetch;
pub mod import;
pub mod inventory;
pub mod jar;
pub(crate) mod legacy_structure_mods;
mod lithostitched;
pub mod modrinth;
pub mod net;
pub mod oracle;
pub mod pipeline;
pub mod processors;
pub mod report;
pub mod upgrade;
pub mod worldgen;
mod yung;
