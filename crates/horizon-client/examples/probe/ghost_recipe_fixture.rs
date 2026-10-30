//! The ghost recipe a recipe book click fades into the menu, for
//! tools/test_ghost_recipe_parity.py: reads one JSON case per line (menu, recipe
//! id, the stack in the furnace's fuel slot) and prints, for every display of
//! the recipe, the slots `Book::ghost` fills and the stacks each cycles through.
//!
//! usage: ghost_recipe_fixture RESOURCES CASES OUTPUT
use horizon_client::{
    crafting::Recipes,
    item_data::{canon_stack, parse_stack},
    items::Items,
    recipe_book::{Book, BookType, GhostTarget, RecipeIndex},
};
use horizon_world::loot::PackOverlay;
use serde_json::Value;
use std::{fmt::Write as _, fs, path::Path};

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let usage = "usage: ghost_recipe_fixture RESOURCES CASES OUTPUT";
    let root = Path::new(args.get(1).ok_or(usage)?);
    let cases = fs::read_to_string(args.get(2).ok_or(usage)?)?;
    let items = Items::catalog(root, &PackOverlay::NONE)?;
    let recipes = Recipes::load(root, &items, &PackOverlay::NONE)?;
    let index = RecipeIndex::load(root, &PackOverlay::NONE);
    let mut book = Book::build(&recipes, &items);
    book.set_display_ids(&index);
    let mut out = String::new();
    for line in cases.lines() {
        let case: Value = serde_json::from_str(line)?;
        let menu = case["menu"].as_str().unwrap_or_default();
        let id = case["recipe"].as_str().unwrap_or_default();
        let (book_type, grid) = match menu {
            "inventory" => (BookType::Crafting, 2),
            "crafting" => (BookType::Crafting, 3),
            "furnace" => (BookType::Furnace, 0),
            "blast_furnace" => (BookType::BlastFurnace, 0),
            _ => (BookType::Smoker, 0),
        };
        let fuel = match case["fuel"].as_str() {
            Some(s) => parse_stack(s)?,
            None => None,
        };
        for entry in book.entries.iter().filter(|e| e.id == id) {
            let crafting = book_type == BookType::Crafting;
            let is_furnace = matches!(
                entry.display,
                horizon_client::recipe_book::Display::Furnace(_)
            );
            if is_furnace == crafting || (crafting && !entry.fits(grid, grid)) {
                out.push_str("skip | ");
                continue;
            }
            for slot in book.ghost(entry, book_type, grid, fuel.is_none(), &items) {
                let (index, result) = match slot.target {
                    GhostTarget::Result => (if crafting { 0 } else { 2 }, true),
                    GhostTarget::Grid(i) => (1 + i, false),
                    GhostTarget::Input => (0, false),
                    GhostTarget::Fuel => (1, false),
                };
                let stacks: Vec<String> = slot.items.iter().map(|s| canon_stack(Some(s))).collect();
                let _ = write!(
                    out,
                    "{index}{}={} ",
                    if result { "R" } else { "I" },
                    stacks.join(",")
                );
            }
            out.push_str("| ");
        }
        out.push('\n');
    }
    fs::write(args.get(3).ok_or(usage)?, out)?;
    Ok(())
}
