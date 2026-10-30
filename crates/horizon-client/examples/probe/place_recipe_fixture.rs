//! Recipe book placements for tools/test_place_recipe_parity.py: reads one
//! JSON case per line (menu, inventory, grid, recipe, max, creative,
//! selected) and the oracle's order of the counted items, runs
//! `Menu::place_recipe` and prints the oracle's line.
//!
//! usage: place_recipe_fixture RESOURCES CASES ORDERS OUTPUT
use horizon_client::{
    crafting::Recipes,
    item_data::{canon_stack, parse_stack},
    items::Items,
    menu::{Env, Kind, Menu, place::PostPlace},
};
use horizon_world::loot::PackOverlay;
use serde_json::Value;
use std::{fs, path::Path};

fn state(menu: &Menu) -> String {
    let slots: Vec<String> = (0..menu.slot_count())
        .map(|i| canon_stack(menu.slot(i)))
        .collect();
    let data: String = menu.data.iter().map(|d| format!(" {d}")).collect();
    let inv: Vec<String> = (0..41).map(|i| canon_stack(menu.player_item(i))).collect();
    format!(
        "{} | {} |{data} | {} | inv {}",
        slots.join(" "),
        canon_stack(menu.carried.as_ref()),
        menu.drops.join(";"),
        inv.join(" ")
    )
}

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let usage = "usage: place_recipe_fixture RESOURCES CASES ORDERS OUTPUT";
    let root = Path::new(args.get(1).ok_or(usage)?);
    let cases = fs::read_to_string(args.get(2).ok_or(usage)?)?;
    let orders = fs::read_to_string(args.get(3).ok_or(usage)?)?;
    let items = Items::catalog(root, &PackOverlay::NONE)?;
    let recipes = Recipes::load(root, &items, &PackOverlay::NONE)?;
    let brewing = horizon_client::brewing::Brewing::load(root, &items, &PackOverlay::NONE);
    let banners = horizon_client::banner::Banners::default();
    let station_recipes = horizon_client::stations::StationRecipes::default();
    let enchantments = horizon_client::enchant::Enchantments::default();
    let names = horizon_client::creative::Names::default();
    let env = Env {
        items: &items,
        recipes: &recipes,
        brewing: &brewing,
        banners: &banners,
        station_recipes: &station_recipes,
        enchantments: &enchantments,
        names: &names,
        trade: None,
        maps: None,
    };
    let mut out = String::new();
    for (line, order) in cases.lines().zip(orders.lines()) {
        let case: Value = serde_json::from_str(line)?;
        let kind = Kind::parse(case["menu"].as_str().unwrap_or_default()).ok_or("menu kind")?;
        let mut menu = Menu::new(kind);
        menu.creative = case["creative"].as_bool().unwrap_or(false);
        menu.selected = case["selected"].as_u64().unwrap_or(0) as usize;
        for pair in case["inventory"].as_array().ok_or("inventory")? {
            menu.set_player_item(
                pair[0].as_u64().unwrap_or(0) as usize,
                parse_stack(pair[1].as_str().unwrap_or("-"))?,
            );
        }
        for pair in case["grid"].as_array().ok_or("grid")? {
            menu.set_slot(
                &env,
                pair[0].as_u64().unwrap_or(0) as usize,
                parse_stack(pair[1].as_str().unwrap_or("-"))?,
            );
        }
        menu.drops.clear();
        let order: Vec<String> = order.split_whitespace().map(str::to_owned).collect();
        let recipe = case["recipe"].as_str().unwrap_or_default();
        let impossible = recipes.placement(recipe).is_none_or(|p| p.impossible());
        let post = if impossible {
            "-"
        } else {
            match menu.place_recipe(
                &env,
                recipe,
                case["max"].as_bool().unwrap_or(false),
                menu.creative,
                Some(&order),
            ) {
                PostPlace::Nothing => "NOTHING",
                PostPlace::Ghost => "PLACE_GHOST_RECIPE",
            }
        };
        out += &format!("post {post} | {}\n", state(&menu));
    }
    fs::write(args.get(4).ok_or(usage)?, out)?;
    Ok(())
}
