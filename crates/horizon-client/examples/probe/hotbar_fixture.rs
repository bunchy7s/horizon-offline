//! Creative toolbars read through the data fixers, for
//! tools/test_hotbar_parity.py: reads one JSON case per line (the SNBT of a
//! `hotbar.nbt`, or null for a missing file) and prints the stacks of the nine
//! rows after `horizon_client::hotbar::from_tag`.
//!
//! usage: hotbar_fixture RESOURCES CASES OUTPUT
use horizon_client::{hotbar, item_data::canon_stack, items::Items};
use horizon_world::loot::PackOverlay;
use horizon_world::nbt::{Tag, parse_snbt};
use serde_json::Value;
use std::path::Path;
use std::{fmt::Write as _, fs};

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let usage = "usage: hotbar_fixture RESOURCES CASES OUTPUT";
    let items = Items::catalog(
        Path::new(args.first().and(args.get(1)).ok_or(usage)?),
        &PackOverlay::NONE,
    )?;
    let known = |id: &str| items.get(id).is_some();
    let cases = fs::read_to_string(args.get(2).ok_or(usage)?)?;
    let mut out = String::new();
    for line in cases.lines() {
        let case: Value = serde_json::from_str(line)?;
        let rows = match case["snbt"].as_str() {
            Some(text) => match parse_snbt(text) {
                Ok((tag @ Tag::Compound(_), _)) => hotbar::from_tag(tag, &known),
                _ => return Err("case snbt".into()),
            },
            None => (0..hotbar::GROUPS)
                .map(|_| vec![None; hotbar::SIZE])
                .collect(),
        };
        for (i, row) in rows.iter().enumerate() {
            let _ = write!(out, "{i}:");
            for stack in row {
                let _ = write!(out, " {}", canon_stack(stack.as_ref()));
            }
            out.push_str(" | ");
        }
        out.push('\n');
    }
    fs::write(args.get(3).ok_or(usage)?, out)?;
    Ok(())
}
