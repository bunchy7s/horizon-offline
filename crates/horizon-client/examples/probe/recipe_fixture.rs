//! Recipe lookups for tools/test_recipe_parity.py: reads the oracle's
//! tab-separated commands (craft, cook, stonecut, smith) and prints one line
//! per command in the oracle's format.
//!
//! usage: recipe_fixture RESOURCES INPUT OUTPUT
use horizon_client::{
    crafting::Recipes,
    enchant::Enchantments,
    item_data::{canon_stack, parse_stack},
    items::Items,
    stations::StationRecipes,
};
use horizon_world::loot::PackOverlay;
use std::{fs, path::Path};

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let root = Path::new(
        args.get(1)
            .ok_or("usage: recipe_fixture RESOURCES INPUT OUTPUT")?,
    );
    let input = fs::read_to_string(args.get(2).ok_or("missing INPUT")?)?;
    let items = Items::catalog(root, &PackOverlay::NONE)?;
    let recipes = Recipes::load(root, &items, &PackOverlay::NONE)?;
    let stations = StationRecipes::load(root, &items, &PackOverlay::NONE);
    let enchantments = Enchantments::load(root);
    let mut out = String::new();
    for line in input.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        let stack = |i: usize| parse_stack(f[i]);
        let text = match f[0] {
            "craft" => {
                let n: usize = f[1].parse()?;
                let mut grid = Vec::new();
                for i in 0..n * n {
                    grid.push(stack(2 + i)?);
                }
                match recipes.find(&grid, n) {
                    None => "none".to_owned(),
                    Some((id, result)) => {
                        let rest = recipes.remainders(id, &grid, n);
                        let rest: Vec<String> = rest
                            .iter()
                            .enumerate()
                            .filter_map(|(i, s)| {
                                Some(format!("{i}={}", canon_stack(Some(s.as_ref()?))))
                            })
                            .collect();
                        format!("{id}\t{}\t{}", canon_stack(Some(&result)), rest.join(";"))
                    }
                }
            }
            "cook" => match stack(2)?.and_then(|s| {
                recipes
                    .cook(&format!("minecraft:{}", f[1]), &s)
                    .map(|c| (c.id.clone(), c.result.clone(), c.experience, c.time))
            }) {
                None => "none".to_owned(),
                Some((id, result, xp, time)) => {
                    format!("{id}\t{}\t{xp:?}\t{time}", canon_stack(Some(&result)))
                }
            },
            "stonecut" => match stack(1)? {
                None => String::new(),
                Some(s) => stations
                    .stonecutter(&s)
                    .iter()
                    .map(|r| {
                        format!(
                            "{}={}",
                            r.id.trim_start_matches("minecraft:"),
                            canon_stack(Some(&r.result))
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(";"),
            },
            "smith" => {
                let (t, b, a) = (stack(1)?, stack(2)?, stack(3)?);
                match stations.smithing_result(&enchantments, t.as_ref(), b.as_ref(), a.as_ref()) {
                    None => "none".to_owned(),
                    Some(r) => canon_stack(Some(&r)),
                }
            }
            other => return Err(format!("unknown command {other}").into()),
        };
        out.push_str(&text);
        out.push('\n');
    }
    fs::write(args.get(3).ok_or("missing OUTPUT")?, out)?;
    Ok(())
}
