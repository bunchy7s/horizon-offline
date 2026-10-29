//! Double chests for tools/test_chest_pair_parity.py: reads one JSON case per
//! line (see VanillaChestPairOracle), a chest's block state at the origin and
//! another block state at the offset "at", and prints what
//! `horizon_client::containers::ChestHalf` pairs them into, as the game's
//! menus and hoppers do: "double" with the right half's x,z then the left
//! half's, or "single".
//!
//! usage: chest_pair_fixture INPUT OUTPUT
use horizon_client::containers::{ChestHalf, is_chest_block};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;

/// `minecraft:chest[facing=north,type=left]` as its name and properties.
fn parse(state: &str) -> Option<(&str, BTreeMap<&str, &str>)> {
    let (name, props) = match state.split_once('[') {
        Some((name, rest)) => (name, rest.strip_suffix(']')?),
        None => (state, ""),
    };
    let props = props
        .split(',')
        .filter(|p| !p.is_empty())
        .map(|p| p.split_once('='))
        .collect::<Option<_>>()?;
    Some((name, props))
}

fn half<'a>(name: &'a str, props: &BTreeMap<&str, &'a str>) -> Option<ChestHalf<'a>> {
    Some(ChestHalf {
        block: name,
        kind: props.get("type")?,
        facing: props.get("facing")?,
    })
}

fn case(line: &str) -> Option<String> {
    let case: Value = serde_json::from_str(line).ok()?;
    let (name, props) = parse(case["state"].as_str()?)?;
    let (other_name, other_props) = parse(case["other"].as_str()?)?;
    let at = (
        i32::try_from(case["at"][0].as_i64()?).ok()?,
        i32::try_from(case["at"][1].as_i64()?).ok()?,
    );
    if !is_chest_block(name) {
        return None;
    }
    let own = half(name, &props)?;
    let paired = own.connected_step() == Some(at)
        && half(other_name, &other_props).is_some_and(|other| own.pairs_with(&other));
    Some(if !paired {
        "single".to_owned()
    } else if own.kind == "right" {
        format!("double 0,0 {},{}", at.0, at.1)
    } else {
        format!("double {},{} 0,0", at.0, at.1)
    })
}

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let input = fs::read_to_string(
        args.get(1)
            .ok_or("usage: chest_pair_fixture INPUT OUTPUT")?,
    )?;
    let lines: Vec<String> = input
        .lines()
        .map(|line| case(line).unwrap_or_else(|| "error".to_owned()))
        .collect();
    fs::write(
        args.get(2).ok_or("missing OUTPUT")?,
        lines.join("\n") + "\n",
    )?;
    Ok(())
}
