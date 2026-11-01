//! The menu a container vehicle opens for tools/test_vehicle_container_parity.py:
//! reads one entity type id per line and prints the menu type its `createMenu`
//! builds (`ChestMenu.threeRows` for a chest boat or chest minecart,
//! `HopperMenu` for a hopper minecart - `horizon_client::containers::vehicle_menu`,
//! the menu `lan_open_vehicle_container` sends), its container size, and the
//! title's name key (`MenuProvider.getDisplayName` = `Entity.getDisplayName`).
//!
//! usage: vehicle_container_fixture INPUT OUTPUT
use horizon_client::containers::{vehicle_menu, vehicle_menu_name};
use std::fs;

fn case(entity_type: &str) -> Option<String> {
    let entity_type = entity_type.trim();
    let (menu, size) = vehicle_menu(entity_type)?;
    Some(format!("{menu} {size} {}", vehicle_menu_name(entity_type)))
}

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let input = fs::read_to_string(
        args.get(1)
            .ok_or("usage: vehicle_container_fixture INPUT OUTPUT")?,
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
