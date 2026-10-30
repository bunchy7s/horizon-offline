//! The item wire harness's Rust side (tools/test_item_wire_parity.py): reads
//! the oracle's stacks (JSON lines with `label` and `hex`), decodes each with
//! `item::Slot` and prints, per stack, whether every byte was consumed, the
//! patch's component ids (added, then removed), which of them Horizon
//! decoded into its own lossy types (the rest it reads past keeping their
//! bytes, or decodes into ids it writes back unchanged, like trims, instruments
//! and stew effects) and whether encoding the decoded stack gives the same
//! bytes back.
//!
//!     cargo run -p horizon-protocol --example item_wire_probe -- vanilla.jsonl rust.jsonl
use horizon_protocol::{
    buf::{Codec, Reader, Writer},
    item::{Component, Slot},
};
use serde_json::{Value, json};
use std::io::Write;

fn hex(s: &str) -> Option<Vec<u8>> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let input = std::fs::read_to_string(args.get(1).ok_or("usage: item_wire_probe IN OUT")?)?;
    let mut out = std::io::BufWriter::new(std::fs::File::create(
        args.get(2).ok_or("usage: item_wire_probe IN OUT")?,
    )?);
    for line in input.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line)?;
        let label = v["label"].clone();
        let Some(bytes) = v["hex"].as_str().and_then(hex) else {
            continue;
        };
        let mut r = Reader::new(&bytes);
        let result = match Slot::read(&mut r) {
            Err(e) => json!({"label": label, "error": e.to_string()}),
            Ok(stack) => {
                let mut w = Writer::new();
                Slot::write(&stack, &mut w);
                let (added, removed): (Vec<i32>, Vec<i32>) =
                    stack.as_ref().map_or_else(Default::default, |s| {
                        (s.added.iter().map(|(k, _)| *k).collect(), s.removed.clone())
                    });
                let typed: Vec<i32> = stack
                    .iter()
                    .flat_map(|s| &s.added)
                    .filter(|(_, c)| {
                        !matches!(
                            c,
                            Component::Raw(_)
                                | Component::Trim { .. }
                                | Component::Instrument(_)
                                | Component::StewEffects(_)
                        )
                    })
                    .map(|(k, _)| *k)
                    .collect();
                json!({
                    "label": label,
                    "left": r.remaining(),
                    "added": added,
                    "removed": removed,
                    "typed": typed,
                    "same_bytes": w.data == bytes,
                })
            }
        };
        writeln!(out, "{result}")?;
    }
    Ok(())
}
