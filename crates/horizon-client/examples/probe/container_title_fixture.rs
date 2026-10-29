//! Container menu titles for tools/test_container_title_parity.py: reads one
//! JSON case per line (see VanillaContainerTitleOracle) and prints the title
//! the server opens the container's menu with, as canonical NBT. A single
//! block entity is placed as the game places it (`block_entity_item::place`,
//! whose `CustomName` the game keeps in `World::block_names`), then named by
//! `containers::display_name`; a double chest's halves by
//! `containers::double_chest_name`.
//!
//! usage: container_title_fixture INPUT OUTPUT
use horizon_client::block_entity_item::place;
use horizon_client::block_entity_nbt::canonical;
use horizon_client::containers::{default_name_key, display_name, double_chest_name};
use horizon_client::save::text;
use horizon_world::nbt::{Tag, parse_snbt};
use serde_json::Value;
use std::fs;

/// ParityData.canon of a text component: sorted keys.
fn canon(tag: &Tag, out: &mut String) {
    match tag {
        Tag::Compound(map) => {
            let mut keys: Vec<_> = map.iter().map(|(k, v)| (k.to_string_lossy(), v)).collect();
            keys.sort_by(|a, b| a.0.cmp(&b.0));
            out.push('{');
            for (i, (k, v)) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&k);
                out.push(':');
                canon(v, out);
            }
            out.push('}');
        }
        Tag::List(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canon(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&horizon_client::item_data::canon_tag(other)),
    }
}

fn snbt(value: &Value) -> Option<Tag> {
    parse_snbt(value.as_str()?).ok().map(|(tag, _)| tag)
}

/// The custom name the game keeps for a block entity saved as `value`.
fn custom_name(value: &Value) -> Option<Option<String>> {
    let saved = canonical(&snbt(value)?)?;
    Some(text(saved.get("CustomName")))
}

fn case(line: &str) -> Option<String> {
    let case: Value = serde_json::from_str(line).ok()?;
    let title = if let Some(first) = case.get("first") {
        let first = custom_name(first)?;
        let second = custom_name(&case["second"])?;
        double_chest_name(first.as_deref(), second.as_deref())
    } else {
        let saved = canonical(&snbt(&case["nbt"])?)?;
        // BlockItem.updateBlockEntityComponents, as Game::apply_placed_item runs it.
        let placed = match case.get("patch") {
            Some(p) => match snbt(p)? {
                Tag::Compound(patch) => place(&saved, &patch, true),
                _ => return None,
            },
            None => saved,
        };
        let key = default_name_key(case["block"].as_str()?)?;
        display_name(text(placed.get("CustomName")).as_deref(), key)
    };
    let mut out = String::new();
    canon(&title, &mut out);
    Some(out)
}

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let input = fs::read_to_string(
        args.get(1)
            .ok_or("usage: container_title_fixture INPUT OUTPUT")?,
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
