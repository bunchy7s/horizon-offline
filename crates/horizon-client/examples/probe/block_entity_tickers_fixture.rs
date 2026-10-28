//! The Rust side of tools/test_block_entity_tickers_parity.py: the ops of
//! `VanillaBlockEntityTickerOracle` on `BlockEntities`, as `World` drives it
//! (a state write calls `state_changed`), printing the tick order.
use horizon_client::{
    block_entities::{BlockEntities, Pos, Ticking},
    containers::BlockEntity,
};
use std::collections::HashMap;

/// State ids of the fixture: 0 air, 1 a block whose block entity ticks, 2 a
/// jukebox without a record, 3 a block entity that never ticks (a chest).
fn state(name: &str) -> u32 {
    match name {
        "chest" | "barrel" => 3,
        "jukebox" => 2,
        _ => 1,
    }
}

fn entity(name: &str) -> BlockEntity {
    if name.starts_with("jukebox") {
        // The game makes a jukebox's block entity when a record goes in.
        return BlockEntity::Jukebox(Default::default());
    }
    BlockEntity::for_block(&format!("minecraft:{name}"))
        .unwrap_or_else(|| panic!("no block entity for {name}"))
}

pub fn main() {
    let args: Vec<String> = crate::args().collect();
    let input = std::fs::read_to_string(&args[1]).expect("input");
    let mut out = String::new();
    let mut entities = BlockEntities::default();
    let mut states: HashMap<Pos, u32> = HashMap::new();
    let mut kinds: HashMap<Pos, String> = HashMap::new();
    let table = vec![Ticking::No, Ticking::Yes, Ticking::Idle, Ticking::No];
    for line in input.lines() {
        let w: Vec<&str> = line.split_whitespace().collect();
        let pos = |i: usize| -> Pos {
            let n = |j: usize| w[i + j].parse::<i32>().expect("coordinate");
            (n(0), n(1), n(2))
        };
        match w[0] {
            "reset" => {
                entities = BlockEntities::default();
                entities.set_states(table.clone());
                states.clear();
                kinds.clear();
            }
            "load" => {
                let mut chunk = Vec::new();
                for i in (3..w.len()).step_by(4) {
                    let p = pos(i + 1);
                    states.insert(p, state(w[i]));
                    kinds.insert(p, w[i].to_owned());
                    chunk.push((p, entity(w[i])));
                }
                entities.load_chunk(chunk);
            }
            "place" => {
                let p = pos(2);
                // World::set, then the game adds the block entity.
                let old = states.insert(p, state(w[1])).unwrap_or(0);
                kinds.insert(p, w[1].to_owned());
                entities.state_changed(p, old, state(w[1]));
                entities.insert(p, entity(w[1]));
            }
            "replace" => {
                let p = pos(1);
                entities.insert(p, entity(&kinds[&p]));
            }
            "remove" => {
                let p = pos(1);
                entities.remove(&p);
                let old = states.insert(p, 0).unwrap_or(0);
                entities.state_changed(p, old, 0);
            }
            "record" => {
                let p = pos(1);
                let s = if w[4] == "1" { 1 } else { 2 };
                let old = states.insert(p, s).unwrap_or(0);
                entities.state_changed(p, old, s);
            }
            "unload" => {
                let c = (w[1].parse::<i32>().unwrap(), w[2].parse::<i32>().unwrap());
                entities.retain(|p, _| (p.0 >> 4, p.2 >> 4) != c);
            }
            "tick" => {
                out.push_str("tick");
                for (p, _) in entities
                    .tickers(|p| states.get(&p).copied().unwrap_or(0))
                    .iter()
                    .copied()
                {
                    out.push_str(&format!(" {},{},{}", p.0, p.1, p.2));
                }
                out.push('\n');
            }
            other => panic!("unknown op {other}"),
        }
    }
    std::fs::write(&args[2], out).expect("output");
}
