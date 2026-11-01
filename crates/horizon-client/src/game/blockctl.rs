//! Control-socket block commands for scripted tests: `setblock`, `fill`,
//! `getblock` and `container` (list or put items in a block entity).
use super::Game;
use horizon_client::{Result, items::ItemStack};
use horizon_world::{chunk::BlockState, registry::BlockStateId};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(super) const HELP: &[&str] = &[
    "setblock X Y Z BLOCK[props]    set a block with neighbour updates",
    "fill X1 Y1 Z1 X2 Y2 Z2 BLOCK   set a box of blocks",
    "getblock X Y Z                 block state at a position",
    "container X Y Z [ITEM COUNT]   list a container, or add items to it",
];

fn parse_state(text: &str) -> Result<BlockState> {
    let (name, props) = match text.split_once('[') {
        Some((name, rest)) => (name, rest.trim_end_matches(']')),
        None => (text, ""),
    };
    let name = if name.contains(':') {
        name.to_owned()
    } else {
        format!("minecraft:{name}")
    };
    let mut properties = BTreeMap::new();
    for pair in props.split(',').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').ok_or("properties are key=value")?;
        properties.insert(k.to_owned(), v.to_owned());
    }
    Ok(BlockState { name, properties })
}

impl Game<'_> {
    fn state_text(&self, state: BlockStateId) -> String {
        let Some(s) = self.pack.assets.registry.state(state) else {
            return "?".into();
        };
        if s.properties.is_empty() {
            return s.name.clone();
        }
        let props: Vec<String> = s
            .properties
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        format!("{}[{}]", s.name, props.join(","))
    }
    fn put_block(&mut self, pos: [i32; 3], state: BlockStateId) -> Result<()> {
        let name = self.res.facts().get(state).name.clone();
        self.world.block_entities.remove(&(pos[0], pos[1], pos[2]));
        self.level_set(pos, state)?;
        if let Some(entity) = self.new_block_entity(&name) {
            self.world
                .block_entities
                .insert((pos[0], pos[1], pos[2]), entity);
        }
        Ok(())
    }
    /// Block commands; None when `words[0]` is not one of them.
    pub(super) fn block_command(&mut self, words: &[&str]) -> Result<Option<Value>> {
        let ours = matches!(
            words.first(),
            Some(&("setblock" | "fill" | "getblock" | "container"))
        );
        // Single player on its server thread: the world is the server's, so
        // the command runs there, in the owner's level, and the client hears
        // of it like any change.
        if ours && let Some(server) = &self.integrated {
            let owned: Vec<String> = words.iter().map(|w| (*w).to_owned()).collect();
            return server
                .call(move |g| {
                    let words: Vec<&str> = owned.iter().map(String::as_str).collect();
                    g.in_owner_level(|g| g.block_command(&words).map_err(|e| e.to_string()))
                })
                .ok_or("the integrated server stopped")?
                .map_err(Into::into);
        }
        // Another server's world changes only through its own commands, which
        // it checks as the player's; `getblock` and `container` read what this
        // client has of it.
        if self.remote.is_some() && matches!(words.first(), Some(&("setblock" | "fill"))) {
            self.send_chat(&format!("/{}", words.join(" ")));
            return Ok(Some(json!({ "ok": true, "sent": true })));
        }
        let int = |i: usize| -> Result<i32> {
            Ok(words
                .get(i)
                .ok_or_else(|| format!("missing argument {i}"))?
                .parse()?)
        };
        match words[0] {
            "setblock" => {
                let pos = [int(1)?, int(2)?, int(3)?];
                let state = self
                    .pack
                    .assets
                    .registry
                    .resolve(&parse_state(words.get(4).ok_or("missing block")?)?)?;
                self.put_block(pos, state)?;
                Ok(Some(
                    json!({ "ok": true, "block": self.state_text(self.world.block(pos[0], pos[1], pos[2])) }),
                ))
            }
            "fill" => {
                let (a, b) = ([int(1)?, int(2)?, int(3)?], [int(4)?, int(5)?, int(6)?]);
                let state = self
                    .pack
                    .assets
                    .registry
                    .resolve(&parse_state(words.get(7).ok_or("missing block")?)?)?;
                let mut count = 0;
                for x in a[0].min(b[0])..=a[0].max(b[0]) {
                    for y in a[1].min(b[1])..=a[1].max(b[1]) {
                        for z in a[2].min(b[2])..=a[2].max(b[2]) {
                            self.put_block([x, y, z], state)?;
                            count += 1;
                        }
                    }
                }
                Ok(Some(json!({ "ok": true, "count": count })))
            }
            "getblock" => {
                let pos = [int(1)?, int(2)?, int(3)?];
                Ok(Some(
                    json!({ "block": self.state_text(self.world.block(pos[0], pos[1], pos[2])) }),
                ))
            }
            "container" => {
                let key = (int(1)?, int(2)?, int(3)?);
                if let Some(item) = words.get(4) {
                    let item = if item.contains(':') {
                        (*item).to_owned()
                    } else {
                        format!("minecraft:{item}")
                    };
                    let count: u32 = words.get(5).map_or(Ok(1), |c| c.parse())?;
                    let entity = self
                        .world
                        .block_entities
                        .get_mut(&key)
                        .ok_or("no container there")?;
                    let slot = entity
                        .slots_mut()
                        .iter_mut()
                        .find(|s| s.is_none())
                        .ok_or("container is full")?;
                    *slot = Some(ItemStack {
                        item: item.as_str().into(),
                        count,
                        damage: 0,
                        extra: None,
                    });
                }
                let entity = self
                    .world
                    .block_entities
                    .get(&key)
                    .ok_or("no container there")?;
                let slots: Vec<Value> = entity
                    .clone()
                    .slots_mut()
                    .iter()
                    .map(|s| {
                        s.as_ref().map_or(
                            Value::Null,
                            |s| json!({ "item": &*s.item, "count": s.count }),
                        )
                    })
                    .collect();
                Ok(Some(json!({ "slots": slots })))
            }
            _ => Ok(None),
        }
    }
}
