//! What the recipe book lists, for tools/test_recipe_book_view_parity.py:
//! reads one JSON case per line (the history of displays the client was
//! sent, the book type and grid, the inventory and menu slots, the filter)
//! and prints every tab's visibility, page count and buttons from
//! `horizon_client::recipe_book::ClientBook`.
//!
//! usage: recipe_book_view_fixture RESOURCES CASES OUTPUT
use horizon_client::{
    crafting::Recipes,
    item_data::parse_stack,
    items::Items,
    menu::place::Stacked,
    recipe_book::{Book, BookType, ClientBook, Display, RecipeIndex, Tab},
};
use horizon_world::loot::PackOverlay;
use serde_json::Value;
use std::{fmt::Write as _, fs, path::Path};

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let usage = "usage: recipe_book_view_fixture RESOURCES CASES OUTPUT";
    let root = Path::new(args.get(1).ok_or(usage)?);
    let cases = fs::read_to_string(args.get(2).ok_or(usage)?)?;
    let items = Items::catalog(root, &PackOverlay::NONE)?;
    let recipes = Recipes::load(root, &items, &PackOverlay::NONE)?;
    let index = RecipeIndex::load(root, &PackOverlay::NONE);
    let mut book = Book::build(&recipes, &items);
    book.set_display_ids(&index);
    let max = |s: &horizon_client::items::ItemStack| items.get(&s.item).map_or(64, |i| i.max_stack);
    let names = horizon_client::creative::Names::load(root);
    let translate = |k: &str| names.get(k).map(str::to_owned);
    let tooltip = |s: &horizon_client::items::ItemStack| {
        horizon_client::item_tooltip::lines(s, items.get(&s.item), &translate)
    };
    let mut out = String::new();
    for line in cases.lines() {
        let case: Value = serde_json::from_str(line)?;
        let mut client = ClientBook::default();
        for batch in case["history"].as_array().ok_or("history")? {
            let op = batch["op"].as_str().unwrap_or_default();
            if op == "replace" {
                client.clear();
            }
            for id in batch["ids"].as_array().into_iter().flatten() {
                for d in index.display_ids(id.as_str().unwrap_or_default()) {
                    if op == "remove" {
                        client.remove(d);
                    } else {
                        client.add(d);
                    }
                }
            }
        }
        let kind = case["type"].as_str().unwrap_or_default();
        let grid = case["grid"].as_u64().unwrap_or(3) as usize;
        let mut stock = Stacked::default();
        for s in case["inventory"].as_array().into_iter().flatten() {
            if let Some(stack) = parse_stack(s.as_str().unwrap_or("-"))? {
                stock.account_simple(&stack, max(&stack));
            }
        }
        for s in case["slots"].as_array().into_iter().flatten() {
            if let Some(stack) = parse_stack(s.as_str().unwrap_or("-"))? {
                if kind == "crafting" {
                    stock.account_simple(&stack, max(&stack));
                } else {
                    stock.account_stack(&stack, max(&stack));
                }
            }
        }
        let book_type = match kind {
            "crafting" => BookType::Crafting,
            "furnace" => BookType::Furnace,
            "blast_furnace" => BookType::BlastFurnace,
            _ => BookType::Smoker,
        };
        let filtering = case["filtering"].as_bool().unwrap_or(false);
        let query = case["search"].as_str().unwrap_or_default();
        for tab in book_type.tabs() {
            let collections = client.collections(&book, book_type, tab);
            let selected = |i: usize| match (&book.entries[i].display, book_type) {
                (Display::Furnace(_), BookType::Crafting) => false,
                (Display::Furnace(_), _) => true,
                (_, BookType::Crafting) => book.entries[i].fits(grid, grid),
                _ => false,
            };
            let craftable = |i: usize| selected(i) && book.can_craft(&book.entries[i], &stock);
            let visible = tab == Tab::Search
                || collections
                    .iter()
                    .any(|c| c.entries.iter().any(|&i| selected(i)));
            let shown: Vec<_> = collections
                .iter()
                .filter(|c| c.entries.iter().any(|&i| selected(i)))
                .filter(|c| {
                    query.is_empty() || book.collection_matches(&c.entries, query, &tooltip)
                })
                .filter(|c| !filtering || c.entries.iter().any(|&i| craftable(i)))
                .collect();
            let name = match tab {
                Tab::Search => "search".to_owned(),
                Tab::Category(c) => c.name().to_owned(),
            };
            let _ = write!(
                out,
                "{name} {} pages={}",
                if visible { "shown" } else { "hidden" },
                shown.len().div_ceil(20)
            );
            for c in shown {
                let parts: Vec<String> = c
                    .entries
                    .iter()
                    .map(|&i| {
                        format!(
                            "{}{}{}",
                            book.entries[i].display_id,
                            if selected(i) { "s" } else { "" },
                            if craftable(i) { "c" } else { "" }
                        )
                    })
                    .collect();
                let _ = write!(out, " [{}]", parts.join(","));
            }
            out.push_str(" | ");
        }
        out.push('\n');
    }
    fs::write(args.get(3).ok_or(usage)?, out)?;
    Ok(())
}
