//! crate_demo's own crate: the mod's logic, compiled apart from the game and kept when only the
//! mod's helpers change.

/// The crate's data file, compiled in.
const SALT: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/salt.txt"));

/// CRC-32 of the salt then `bytes`.
pub fn checksum(bytes: &[u8]) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(SALT);
    hasher.update(bytes);
    hasher.finalize()
}
