//! Menu clicks for tools/test_menu_click_parity.py: reads one JSON case per
//! line (menu, creative, slots, carried, data, ops) and prints the oracle's
//! state lines after every click. The `*_be` furnace menus run over a furnace
//! block entity: its data slots follow `Furnace::input_clicked`.
//!
//! usage: menu_fixture RESOURCES INPUT OUTPUT
use horizon_client::{
    containers::{Furnace, FurnaceKind},
    crafting::Recipes,
    furnace::FurnaceCooking,
    item_data::{canon_stack, parse_stack},
    items::Items,
    menu::{Env, Input, Kind, Menu},
};
use horizon_protocol::registry::Ids;
use serde_json::Value;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    fs,
    path::Path,
};

/// The case's maps: (locked, scale) by id, and the next free id.
#[derive(Default)]
struct CaseMaps {
    maps: RefCell<HashMap<i32, (bool, u8)>>,
    next: Cell<i32>,
}

impl horizon_client::menu::Maps for CaseMaps {
    fn state(&self, id: i32) -> Option<(bool, u8)> {
        self.maps.borrow().get(&id).copied()
    }

    /// `MapItemSavedData.scaled` (unlocked, one scale up) or `locked`.
    fn derive(&self, id: i32, scale: bool) -> Option<i32> {
        let (_, s) = self.state(id)?;
        let new = self.next.get();
        self.next.set(new + 1);
        let made = if scale {
            (false, (s + 1).min(4))
        } else {
            (true, s)
        };
        self.maps.borrow_mut().insert(new, made);
        Some(new)
    }
}

fn state(menu: &Menu) -> String {
    let slots: Vec<String> = (0..menu.slot_count())
        .map(|i| canon_stack(menu.slot(i)))
        .collect();
    let data: String = menu.data.iter().map(|d| format!(" {d}")).collect();
    let equipped: Vec<String> = menu
        .equipped
        .iter()
        .map(|(slot, stack)| format!("{slot}:{}", canon_stack(stack.as_ref())))
        .collect();
    let line = format!(
        "{} | {} |{data} | {} | {} | {}",
        slots.join(" "),
        canon_stack(menu.carried.as_ref()),
        menu.drops.join(";"),
        equipped.join(";"),
        menu.player_sounds.join(";")
    );
    if menu.kind() != Kind::Lectern {
        return line;
    }
    // No inventory slots: the inventory Take Book fills.
    let items: String = (0..36)
        .map(|i| format!(" {}", canon_stack(menu.player_item(i))))
        .collect();
    format!("{line} | inv{items}")
}

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let root = Path::new(
        args.get(1)
            .ok_or("usage: menu_fixture RESOURCES INPUT OUTPUT")?,
    );
    let input = fs::read_to_string(args.get(2).ok_or("missing INPUT")?)?;
    let items = Items::catalog(root)?;
    let recipes = Recipes::load(root, &items)?;
    let brewing = horizon_client::brewing::Brewing::load(root, &items);
    let banners = horizon_client::banner::Banners::load(root);
    let station_recipes = horizon_client::stations::StationRecipes::load(root, &items);
    let enchantments = horizon_client::enchant::Enchantments::load(root);
    let names = horizon_client::creative::Names::load(root);
    let case_maps = CaseMaps::default();
    let env = Env {
        items: &items,
        recipes: &recipes,
        brewing: &brewing,
        banners: &banners,
        station_recipes: &station_recipes,
        enchantments: &enchantments,
        names: &names,
        trade: None,
        maps: Some(&case_maps),
    };
    // The mob effects' network ids (BeaconMenu.encodeEffect).
    let builtin = horizon_protocol::registry::Builtin::find(root)
        .and_then(|p| horizon_protocol::registry::Builtin::load(&p).ok());
    let effects = builtin
        .as_ref()
        .and_then(|b| b.get("minecraft:mob_effect"))
        .cloned();
    let mut out = String::new();
    for line in input.lines() {
        let case: Value = serde_json::from_str(line)?;
        let name = case["menu"].as_str().unwrap_or_default();
        // FurnaceMenu over the server's AbstractFurnaceBlockEntity.
        let furnace_kind = match name {
            "furnace_be" => Some(FurnaceKind::Furnace),
            "blast_furnace_be" => Some(FurnaceKind::BlastFurnace),
            "smoker_be" => Some(FurnaceKind::Smoker),
            _ => None,
        };
        let kind = Kind::parse(
            name.strip_suffix("_be")
                .filter(|_| furnace_kind.is_some())
                .unwrap_or(name),
        )
        .ok_or("menu kind")?;
        // [id, scale, locked] each; the level's new maps from 1000.
        case_maps.next.set(1000);
        *case_maps.maps.borrow_mut() = case["maps"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|m| {
                let id = m[0].as_i64().unwrap_or(0) as i32;
                (
                    id,
                    (
                        m[2].as_bool().unwrap_or(false),
                        m[1].as_u64().unwrap_or(0) as u8,
                    ),
                )
            })
            .collect();
        let mut menu = Menu::new(kind);
        menu.creative = case["creative"].as_bool().unwrap_or(false);
        menu.levels = case["levels"].as_i64().unwrap_or(0) as i32;
        for (i, slot) in case["slots"].as_array().ok_or("slots")?.iter().enumerate() {
            menu.set_slot(&env, i, parse_stack(slot.as_str().unwrap_or("-"))?);
        }
        menu.carried = parse_stack(case["carried"].as_str().unwrap_or("-"))?;
        for (i, item) in case["inventory"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            menu.set_player_item(i, parse_stack(item.as_str().unwrap_or("-"))?);
        }
        menu.settle(&env);
        for (i, v) in case["data"].as_array().ok_or("data")?.iter().enumerate() {
            menu.set_data(i, v.as_i64().unwrap_or(0) as i32);
        }
        let mut furnace = furnace_kind.map(|kind| Furnace {
            kind,
            items: [0, 1, 2].map(|i| menu.slot(i).cloned()),
            lit_remaining: menu.data[0],
            lit_total: menu.data[1],
            cooking_timer: menu.data[2],
            cooking_total: menu.data[3],
            speed: 1.0,
            recipes_used: Default::default(),
        });
        menu.drops.clear();
        menu.equipped.clear();
        menu.player_sounds.clear();
        out += &format!("init {}\n", state(&menu));
        for op in case["ops"].as_array().ok_or("ops")? {
            let slot = op[0].as_i64().unwrap_or(0) as i32;
            let button = op[1].as_i64().unwrap_or(0) as i32;
            match op[2].as_str().unwrap_or_default() {
                // ServerboundContainerButtonClickPacket: clickMenuButton.
                "BUTTON" => {
                    menu.button(&env, slot);
                    out += &format!("{}\n", state(&menu));
                    continue;
                }
                // ServerboundRenameItemPacket: AnvilMenu.setItemName.
                "RENAME" => {
                    menu.rename(&env, op[3].as_str().unwrap_or_default());
                    out += &format!("{}\n", state(&menu));
                    continue;
                }
                // ServerboundSetBeaconPacket: BeaconMenu.updateEffects.
                "SET_BEACON" => {
                    fn power<'a>(ids: Option<&Ids>, v: &'a Value) -> Option<(i32, &'a str)> {
                        let name = v.as_str()?;
                        Some((ids?.id(name)?, name))
                    }
                    let (primary, secondary) = (
                        power(effects.as_ref(), &op[3]),
                        power(effects.as_ref(), &op[4]),
                    );
                    menu.beacon_update(&env, primary, secondary);
                    out += &format!("{}\n", state(&menu));
                    continue;
                }
                // ServerboundContainerSlotStateChangedPacket.
                "SLOT_STATE" => {
                    if (0..9).contains(&slot) {
                        menu.set_slot_state(slot as usize, button == 1);
                    }
                    out += &format!("{}\n", state(&menu));
                    continue;
                }
                _ => {}
            }
            let clicked = if op[2] == "QUICK_MOVE_ALL" {
                // A Shift double click: lastQuickMoved is the op's fourth field.
                let moved = parse_stack(op[3].as_str().unwrap_or("-"))?;
                match moved {
                    Some(moved) => menu.quick_move_all(&env, slot as usize, &moved),
                    None => Ok(()),
                }
            } else {
                let kind = Input::parse(op[2].as_str().unwrap_or_default()).ok_or("input")?;
                menu.click(&env, slot, button, kind)
            };
            if let Some(f) = furnace.as_mut() {
                let before = f.items[0].clone();
                f.items = [0, 1, 2].map(|i| menu.slot(i).cloned());
                f.input_clicked(&recipes, before);
                menu.data[2] = f.cooking_timer;
                menu.data[3] = f.cooking_total;
            }
            match clicked {
                Ok(()) => out += &format!("{}\n", state(&menu)),
                Err(name) => out += &format!("error {name}\n"),
            }
        }
        out += "end\n";
    }
    fs::write(args.get(3).ok_or("missing OUTPUT")?, out)?;
    Ok(())
}
