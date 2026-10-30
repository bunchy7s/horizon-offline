//! Enchanting-table offers for differential tests against vanilla
//! (tools/test_enchanting_parity.py): reads "seed bookcases item" lines and
//! prints costs, clues and applied enchantments in the oracle's format.
//!
//! usage: enchanting_fixture RESOURCES INPUT OUTPUT
use horizon_client::{
    enchant::Enchantments,
    items::{ItemStack, Items},
    stations,
};
use horizon_world::loot::PackOverlay;
use std::{fmt::Write as _, fs, path::Path};

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let root = Path::new(
        args.get(1)
            .ok_or("usage: enchanting_fixture RESOURCES INPUT OUTPUT")?,
    );
    let input = fs::read_to_string(args.get(2).ok_or("missing INPUT")?)?;
    let items = Items::tags_only(root, &PackOverlay::NONE)?;
    let e = Enchantments::load(root);
    let mut out = String::new();
    for line in input.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        let (seed, bookcases, item): (i32, i32, &str) = (f[0].parse()?, f[1].parse()?, f[2]);
        let stack = ItemStack::new(item, 1);
        let offers = stations::enchant_offers(&e, &items, seed, bookcases, Some(&stack));
        for i in 0..3 {
            let cost = offers.costs[i];
            let clue = offers.clues[i]
                .as_ref()
                .map_or("-".to_owned(), |(id, l)| format!("{id}:{l}"));
            let applied = if cost > 0 {
                let list = stations::enchantment_list(&e, &items, seed, i, cost, item);
                if list.is_empty() {
                    "-".to_owned()
                } else {
                    list.iter()
                        .map(|(id, l)| format!("{id}:{l}"))
                        .collect::<Vec<_>>()
                        .join(",")
                }
            } else {
                "-".to_owned()
            };
            writeln!(out, "{line} {i} {cost} {clue} {applied}")?;
        }
    }
    fs::write(args.get(3).ok_or("missing OUTPUT")?, out)?;
    Ok(())
}
