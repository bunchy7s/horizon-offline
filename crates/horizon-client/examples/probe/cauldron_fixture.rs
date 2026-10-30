//! Cauldron interactions for tools/test_cauldron_parity.py: reads one JSON
//! case per line and prints "result TAB events TAB inventory".
//!
//! usage: cauldron_fixture RESOURCES INPUT OUTPUT
use horizon_client::{
    cauldron::{Outcome, Player, interact, removes_dye},
    crafting::Recipes,
    item_data::{canon_stack, parse_stack},
    items::{ItemStack, Items},
    menu::{Env, Kind, Menu},
};
use horizon_world::chunk::BlockState;
use horizon_world::loot::PackOverlay;
use serde_json::Value;
use std::{fs, path::Path};

struct Stand<'a> {
    menu: Menu,
    env: &'a Env<'a>,
    events: Vec<String>,
    hand: Option<ItemStack>,
}

impl Player for Stand<'_> {
    fn creative(&self) -> bool {
        self.menu.creative
    }
    fn hand(&mut self) -> &mut Option<ItemStack> {
        &mut self.hand
    }
    fn add(&mut self, stack: &mut ItemStack) -> bool {
        // The held stack lives in inventory slot 0 for Inventory.add.
        self.menu.set_player_item(0, self.hand.take());
        let added = self.menu.give(self.env, stack);
        self.hand = self.menu.player_item(0).cloned();
        self.menu.set_player_item(0, None);
        added
    }
    fn contains(&self, stack: &ItemStack) -> bool {
        self.menu.player_contains(stack)
            || self
                .hand
                .as_ref()
                .is_some_and(|h| h.same_item_same_components(stack))
    }
    fn drop_stack(&mut self, stack: ItemStack) {
        self.events
            .push(format!("drop {}", canon_stack(Some(&stack))));
    }
}

fn parse(text: &str) -> BlockState {
    let (name, props) = text.split_once('|').unwrap_or((text, ""));
    BlockState {
        name: name.to_owned(),
        properties: props
            .split(',')
            .filter(|p| !p.is_empty())
            .filter_map(|p| p.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
    }
}

fn describe(state: &BlockState) -> String {
    let props: Vec<String> = state
        .properties
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    format!("{}|{}", state.name, props.join(","))
}

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let root = Path::new(
        args.get(1)
            .ok_or("usage: cauldron_fixture RESOURCES INPUT OUTPUT")?,
    );
    let input = fs::read_to_string(args.get(2).ok_or("missing INPUT")?)?;
    let items = Items::catalog(root, &PackOverlay::NONE)?;
    let recipes = Recipes::load(root, &items, &PackOverlay::NONE)?;
    let brewing = horizon_client::brewing::Brewing::load(root, &items, &PackOverlay::NONE);
    // The inventory menu never reads banner patterns.
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
    for line in input.lines() {
        let case: Value = serde_json::from_str(line)?;
        let mut menu = Menu::new(Kind::Inventory);
        menu.creative = case["creative"].as_bool().unwrap_or(false);
        for pair in case["inventory"].as_array().ok_or("inventory")? {
            menu.set_player_item(
                pair[0].as_u64().unwrap_or(0) as usize,
                parse_stack(pair[1].as_str().unwrap_or("-"))?,
            );
        }
        let held = parse_stack(case["held"].as_str().unwrap_or("-"))?;
        let mut stand = Stand {
            menu,
            env: &env,
            events: Vec::new(),
            hand: held,
        };
        let cauldron = parse(case["state"].as_str().unwrap_or_default());
        let dye = stand
            .hand
            .as_ref()
            .is_some_and(|h| removes_dye(&items, &h.item));
        let result = interact(
            &cauldron,
            case["under_water"].as_bool().unwrap_or(false),
            dye,
            &mut stand,
        );
        if let Some(block) = &result.block {
            stand.events.push(format!("set {}", describe(block)));
        }
        if let Some(sound) = result.sound {
            stand.events.push(format!("sound {sound}"));
        }
        let kind = match result.outcome {
            Outcome::Success => "success",
            Outcome::Consume => "consume",
            Outcome::Pass => "pass",
        };
        stand.menu.set_player_item(0, stand.hand.take());
        let inventory: Vec<String> = (0..41)
            .filter_map(|i| {
                stand
                    .menu
                    .player_item(i)
                    .map(|s| format!("{i}={}", canon_stack(Some(s))))
            })
            .collect();
        out += &format!(
            "{kind}\t{}\t{}\n",
            stand.events.join(";"),
            inventory.join(" ")
        );
    }
    fs::write(args.get(3).ok_or("missing OUTPUT")?, out)?;
    Ok(())
}
