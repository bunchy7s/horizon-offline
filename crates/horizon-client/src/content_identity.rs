//! Identity of the ordered block-state registry used by chunk packets.
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

pub const CHANNEL: &str = "horizon:block_registry";
pub const MARKER: &str = "horizon-content-blocks";
pub const MISMATCH: &str =
    "Content block registry mismatch: install the same content blocks as the server.";

/// Hash the network order and each state's name and properties, not its gameplay facts.
pub fn identity(root: &Path) -> Result<[u8; 32], String> {
    static IDENTITIES: OnceLock<Mutex<HashMap<PathBuf, [u8; 32]>>> = OnceLock::new();
    let mut identities = IDENTITIES.get_or_init(Default::default).lock().unwrap();
    if let Some(hash) = identities.get(root) {
        return Ok(*hash);
    }
    let bytes = std::fs::read(root.join("facts.json")).map_err(|e| e.to_string())?;
    let hash = hash_facts(&bytes)?;
    identities.insert(root.to_owned(), hash);
    Ok(hash)
}

fn hash_facts(bytes: &[u8]) -> Result<[u8; 32], String> {
    let facts: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let states = facts
        .get("states")
        .and_then(|s| s.as_array())
        .ok_or("facts.json has no block states")?;
    let mut hash = Sha256::new();
    for row in states {
        let state = row
            .as_array()
            .and_then(|r| r.get(4))
            .ok_or("facts.json has an invalid block state")?;
        let encoded = serde_json::to_vec(state).map_err(|e| e.to_string())?;
        hash.update((encoded.len() as u64).to_le_bytes());
        hash.update(encoded);
    }
    Ok(hash.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::hash_facts;

    #[test]
    fn identity_tracks_order_and_properties_not_gameplay_data() {
        let facts = |a: &str, b: &str| format!(r#"{{"states":[["x",0,0,0,{a}],["y",0,0,0,{b}]]}}"#);
        let a = r#"{"id":"demo:a","properties":{"facing":"north"}}"#;
        let b = r#"{"id":"demo:b"}"#;
        let first = hash_facts(facts(a, b).as_bytes()).unwrap();
        assert_eq!(first, hash_facts(facts(a, b).as_bytes()).unwrap());
        assert_ne!(first, hash_facts(facts(b, a).as_bytes()).unwrap());
        assert_ne!(first, hash_facts(facts(a, a).as_bytes()).unwrap());
        assert_ne!(
            first,
            hash_facts(facts(&a.replace("north", "south"), b).as_bytes()).unwrap()
        );
        assert_ne!(
            first,
            hash_facts(format!(r#"{{"states":[["x",0,0,0,{a}]]}}"#).as_bytes()).unwrap()
        );
        assert!(hash_facts(br#"{"states":[[0,0,0,0,"minecraft:stone"]]}"#).is_ok());
        assert!(hash_facts(br#"{"states":[[]]}"#).is_err());
    }
}
