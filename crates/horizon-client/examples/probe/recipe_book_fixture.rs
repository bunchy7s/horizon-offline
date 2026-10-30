//! The player's recipe book for tools/test_recipe_book_parity.py: reads one
//! JSON case per line (a list of give / take / seen / set / save / load ops)
//! and prints the oracle's line after each op, from
//! `horizon_client::recipe_book::{Settings, RecipeIndex}`.
//!
//! usage: recipe_book_fixture RESOURCES CASES OUTPUT
use horizon_client::recipe_book::{BookType, RecipeIndex, Settings};
use horizon_protocol::{
    buf::{Codec, Reader, Writer},
    packets::recipes::RecipeBookChangeSettings,
};
use horizon_world::loot::PackOverlay;
use horizon_world::nbt::Tag;
use serde_json::Value;
use std::{collections::BTreeSet, fs, path::Path};

fn fnv<'a>(ids: impl IntoIterator<Item = &'a String>) -> u64 {
    let mut sorted: Vec<&String> = ids.into_iter().collect();
    sorted.sort();
    let joined = sorted
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in joined.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}
fn ids(set: &BTreeSet<String>) -> String {
    format!("{}:{:x}", set.len(), fnv(set))
}
/// The saved compound as the oracle prints it: keys sorted, lists hashed.
fn compound(tag: &Tag) -> String {
    let Tag::Compound(c) = tag else {
        return String::new();
    };
    let mut keys: Vec<String> = c.keys().map(|k| k.to_string_lossy()).collect();
    keys.sort();
    let mut out = String::new();
    for k in keys {
        out.push(' ');
        out.push_str(&k);
        out.push('=');
        match tag.get(&k) {
            Some(Tag::List(list)) => {
                let v: Vec<String> = list
                    .iter()
                    .map(|t| match t {
                        Tag::String(s) => s.to_string_lossy(),
                        _ => "?".into(),
                    })
                    .collect();
                out.push_str(&format!("{}:{:x}", v.len(), fnv(&v)));
            }
            Some(Tag::Byte(b)) => out.push_str(&format!("{b}b")),
            _ => out.push('?'),
        }
    }
    out
}

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let usage = "usage: recipe_book_fixture RESOURCES CASES OUTPUT";
    let root = Path::new(args.get(1).ok_or(usage)?);
    let cases = fs::read_to_string(args.get(2).ok_or(usage)?)?;
    let index = RecipeIndex::load(root, &PackOverlay::NONE);
    let mut out = String::new();
    for line in cases.lines() {
        let case: Value = serde_json::from_str(line)?;
        let mut book = Settings::default();
        let mut saved: Option<Tag> = None;
        for op in case["ops"].as_array().ok_or("ops")? {
            let kind = op["op"].as_str().unwrap_or_default();
            let list = || -> Vec<String> {
                op["ids"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .filter(|id| index.contains(id))
                    .collect()
            };
            let mut count = 0;
            let mut extra = String::new();
            match kind {
                "give" => count = book.add_recipes(&index, &list()).0,
                "take" => count = book.remove_recipes(&index, &list()).len(),
                "seen" => book.remove_highlight(op["id"].as_str().unwrap_or_default()),
                "set" => {
                    let i = match op["type"].as_str().unwrap_or_default() {
                        "CRAFTING" => BookType::Crafting,
                        "FURNACE" => BookType::Furnace,
                        "BLAST_FURNACE" => BookType::BlastFurnace,
                        _ => BookType::Smoker,
                    }
                    .index();
                    let packet = RecipeBookChangeSettings {
                        book_type: i as i32,
                        open: op["open"].as_bool().unwrap_or(false),
                        filtering: op["filtering"].as_bool().unwrap_or(false),
                    };
                    let mut wire = Writer::new();
                    RecipeBookChangeSettings::write(&packet, &mut wire);
                    extra = format!(
                        " packet={}",
                        wire.data
                            .iter()
                            .map(|b| format!("{b:02x}"))
                            .collect::<String>()
                    );
                    let decoded = RecipeBookChangeSettings::read(&mut Reader::new(&wire.data))?;
                    book.open[decoded.book_type as usize] = decoded.open;
                    book.filtering[decoded.book_type as usize] = decoded.filtering;
                }
                "save" => {
                    let tag = book.write();
                    extra = compound(&tag);
                    saved = Some(tag);
                }
                "load" => {
                    let mut tag = saved
                        .clone()
                        .unwrap_or_else(|| Tag::Compound(Default::default()));
                    if let Tag::Compound(c) = &mut tag {
                        for name in ["recipes", "toBeDisplayed"] {
                            if let Some(Tag::List(l)) = c.get_mut(&name.into()) {
                                l.push(Tag::String("minecraft:no_such_recipe".into()));
                                l.push(Tag::String("stick".into()));
                            }
                        }
                        if let Some(drop) = op["drop"].as_str() {
                            c.remove(&drop.into());
                        }
                    }
                    book = Settings::read_valid(Some(&tag), &|id| index.contains(id));
                }
                other => return Err(format!("op {other}").into()),
            }
            let settings: String = BookType::ALL
                .iter()
                .map(|b| {
                    format!(
                        "{}{}",
                        u8::from(book.open[b.index()]),
                        u8::from(book.filtering[b.index()])
                    )
                })
                .collect();
            out += &format!(
                "{kind} {count} known={} highlight={} settings={settings}{extra}\n",
                ids(&book.recipes),
                ids(&book.to_be_displayed)
            );
        }
        out += "end\n";
    }
    fs::write(args.get(3).ok_or(usage)?, out)?;
    Ok(())
}
