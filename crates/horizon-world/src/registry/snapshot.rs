//! A `BlockRegistry`'s snapshot wire format: the built tables, exact and
//! small — names and property strings once in a table, states and defaults as
//! indexes into it. Restore rebuilds through `SharedStrings`, so the process
//! still holds one string per distinct name and value.
use super::{BlockRegistry, BlockStateId};
use crate::{
    block_state_properties::{BlockProperties, SharedStrings},
    chunk::BlockState,
    compression::invalid,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io,
};

/// A state: its name and property strings as `strings` indexes.
#[derive(Serialize, Deserialize)]
struct WireState {
    name: u32,
    properties: Vec<(u32, u32)>,
}

/// Everything `BlockRegistry::from_states` builds, in id order.
#[derive(Serialize, Deserialize)]
pub(crate) struct Snapshot {
    /// Every block name and property name and value, in first-use order.
    strings: Vec<String>,
    states: Vec<WireState>,
    /// `facts.json`'s `(air, fluid)` columns per state, bits 0 and 1.
    flags: Vec<u8>,
    /// `state_key` → id; the same build recomputes the same keys.
    ids: Vec<(u64, u32)>,
    collided: Vec<(WireState, u32)>,
    /// Block name → default state id.
    defaults: Vec<(u32, u32)>,
}

/// The snapshot's string table while it is written: indexes each new string.
struct Strings<'a> {
    strings: Vec<String>,
    index: HashMap<&'a str, u32>,
}
impl<'a> Strings<'a> {
    fn get(&mut self, text: &'a str) -> u32 {
        if let Some(&index) = self.index.get(text) {
            return index;
        }
        let index = self.strings.len() as u32;
        self.strings.push(text.to_owned());
        self.index.insert(text, index);
        index
    }
    fn state(&mut self, state: &'a BlockState) -> WireState {
        WireState {
            name: self.get(&state.name),
            properties: state
                .properties
                .iter()
                .map(|(key, value)| (self.get(key), self.get(value)))
                .collect(),
        }
    }
}

pub(crate) fn capture(registry: &BlockRegistry, flags: &[(bool, bool)]) -> Snapshot {
    let mut strings = Strings {
        strings: Vec::new(),
        index: HashMap::new(),
    };
    let states = registry
        .states
        .iter()
        .map(|state| strings.state(state))
        .collect();
    let flags = flags
        .iter()
        .map(|&(air, fluid)| air as u8 | ((fluid as u8) << 1))
        .collect();
    let ids = registry.ids.iter().map(|(&key, &id)| (key, id)).collect();
    let collided = registry
        .collided
        .iter()
        .map(|(state, id)| (strings.state(state), id.0))
        .collect();
    let defaults = registry
        .defaults
        .iter()
        .map(|(name, id)| (strings.get(name.as_str()), id.0))
        .collect();
    Snapshot {
        strings: strings.strings,
        states,
        flags,
        ids,
        collided,
        defaults,
    }
}

pub(crate) fn restore(snapshot: Snapshot) -> io::Result<(BlockRegistry, Vec<(bool, bool)>)> {
    let Snapshot {
        strings,
        states,
        flags,
        ids,
        collided,
        defaults,
    } = snapshot;
    if states.is_empty()
        || states.len() > i32::MAX as usize
        || flags.len() != states.len()
        || flags.iter().any(|f| *f > 3)
        || ids.len() > states.len()
        || collided.len() > states.len()
    {
        return Err(invalid("cached registry size mismatch"));
    }
    let mut shared = SharedStrings::default();
    let text: Vec<&'static String> = strings.iter().map(|s| shared.get(s)).collect();
    let at = |index: u32| {
        text.get(index as usize)
            .copied()
            .ok_or_else(|| invalid("cached registry string out of range"))
    };
    let state_of = |wire: WireState| -> io::Result<BlockState> {
        let properties = wire
            .properties
            .iter()
            .map(|&(key, value)| Ok((at(key)?, at(value)?)))
            .collect::<io::Result<Vec<_>>>()?;
        Ok(BlockState {
            name: (*at(wire.name)?).clone(),
            properties: BlockProperties::from_shared(properties),
        })
    };
    let states = states
        .into_iter()
        .map(state_of)
        .collect::<io::Result<Vec<_>>>()?;
    // `from_states` puts every state in `ids` or `collided`: require that.
    let mut seen = vec![false; states.len()];
    let mut mark = |id: u32| match seen.get_mut(id as usize) {
        Some(slot) if !*slot => {
            *slot = true;
            Ok(())
        }
        _ => Err(invalid("cached registry state id out of range")),
    };
    let mut id_map = HashMap::with_capacity(ids.len());
    for (key, id) in ids {
        mark(id)?;
        if id_map.insert(key, id).is_some() {
            return Err(invalid("cached registry ids duplicate a key"));
        }
    }
    let mut collided_map = BTreeMap::new();
    for (wire, id) in collided {
        mark(id)?;
        if collided_map
            .insert(state_of(wire)?, BlockStateId(id))
            .is_some()
        {
            return Err(invalid("cached registry collided states duplicate"));
        }
    }
    if seen.iter().any(|covered| !*covered) {
        return Err(invalid("cached registry is missing state ids"));
    }
    let mut default_map = BTreeMap::new();
    for (name, id) in defaults {
        let name = at(name)?;
        let state = states
            .get(id as usize)
            .ok_or_else(|| invalid("cached registry default out of range"))?;
        if state.name != *name {
            return Err(invalid("cached registry default names another state"));
        }
        if default_map
            .insert((*name).clone(), BlockStateId(id))
            .is_some()
        {
            return Err(invalid("cached registry defaults duplicate a name"));
        }
    }
    let flags = flags.iter().map(|f| (f & 1 != 0, f & 2 != 0)).collect();
    Ok((
        BlockRegistry {
            states,
            ids: id_map,
            collided: collided_map,
            defaults: default_map,
        },
        flags,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small() -> (BlockRegistry, Vec<(bool, bool)>) {
        let json = br#"{"defaults": {"minecraft:air": 0, "minecraft:oak_slab": 1},
            "states": [
              ["minecraft:air", true, false, "x", "minecraft:air"],
              ["minecraft:oak_slab", false, false, "x", {"id": "minecraft:oak_slab", "properties": {"type": "top"}}],
              ["minecraft:oak_slab", false, true, "x", {"id": "minecraft:oak_slab", "properties": {"type": "bottom"}}]
            ]}"#;
        BlockRegistry::from_facts_json_with_flags(&json[..]).unwrap()
    }

    #[test]
    fn snapshot_roundtrips_the_built_tables() {
        let (registry, flags) = small();
        let (restored, restored_flags) = restore(capture(&registry, &flags)).unwrap();
        assert_eq!(restored_flags, flags);
        assert_eq!(restored.len(), registry.len());
        for i in 0..registry.len() as u32 {
            let id = BlockStateId(i);
            assert_eq!(restored.state(id), registry.state(id));
            assert_eq!(
                restored.id(registry.state(id).unwrap()),
                registry.id(registry.state(id).unwrap())
            );
        }
        for name in ["minecraft:air", "minecraft:oak_slab"] {
            assert_eq!(restored.default_state(name), registry.default_state(name));
        }
        // The restored properties keep the shared strings, not copies.
        let (a, b) = (
            restored.state(BlockStateId(1)).unwrap(),
            restored.state(BlockStateId(2)).unwrap(),
        );
        let key = a.properties.keys().next().unwrap();
        assert!(std::ptr::eq(key, b.properties.keys().next().unwrap()));
    }

    #[test]
    fn restore_rejects_beyond_the_tables() {
        let (registry, flags) = small();
        let mut snapshot = capture(&registry, &flags);
        snapshot.flags.pop();
        assert!(restore(snapshot).is_err(), "short flags");
        let mut snapshot = capture(&registry, &flags);
        snapshot.ids.push((0, 99));
        assert!(restore(snapshot).is_err(), "id out of range");
        let mut snapshot = capture(&registry, &flags);
        snapshot.ids.pop();
        assert!(restore(snapshot).is_err(), "a state unindexed");
        let mut snapshot = capture(&registry, &flags);
        snapshot.strings.push("minecraft:cobblestone".into());
        let cobble = snapshot.strings.len() as u32 - 1;
        snapshot.defaults[0].0 = cobble;
        assert!(restore(snapshot).is_err(), "default names another state");
        let mut snapshot = capture(&registry, &flags);
        snapshot.defaults[0].0 = 99;
        assert!(restore(snapshot).is_err(), "default string out of range");
        let mut snapshot = capture(&registry, &flags);
        snapshot.states[0].name = 99;
        assert!(restore(snapshot).is_err(), "name out of range");
        let mut snapshot = capture(&registry, &flags);
        snapshot.flags[0] = 4;
        assert!(restore(snapshot).is_err(), "unused flag bits");
    }
}
