//! Container loot in the vanilla loot-fill oracle's line format.
//!
//! loot_fill <client resource dir> <table[,table...]> <seed>...: prints
//! `table TABLE SEED` then `fill TABLE SEED SLOT ITEM COUNT DAMAGE [ENCH=LVL...]`.
use horizon_world::loot::{LootData, PackOverlay};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().ok_or("client resource directory")?);
    let data = LootData::load(&root, &PackOverlay::NONE)?;
    let tables: Vec<String> = args
        .next()
        .ok_or("table")?
        .split(',')
        .map(str::to_owned)
        .collect();
    let seeds: Vec<i64> = args.map(|s| s.parse()).collect::<Result<_, _>>()?;
    for table in &tables {
        for &seed in &seeds {
            println!("table {table} {seed}");
            for (slot, stack) in data.fill(table, seed, 27, None).into_iter().enumerate() {
                let Some(s) = stack else { continue };
                let mut line = format!(
                    "fill {table} {seed} {slot} {} {} {}",
                    s.item, s.count, s.damage
                );
                for (name, level) in &s.enchantments {
                    line.push_str(&format!(" {name}={level}"));
                }
                println!("{line}");
            }
        }
    }
    Ok(())
}
