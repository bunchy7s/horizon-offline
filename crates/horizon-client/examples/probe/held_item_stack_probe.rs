//! The stacks passed to item models for an entity's two arms.
use horizon_client::{
    Result,
    items::ItemStack,
    mobs::{Extra, render},
};
use serde_json::Value;
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
};

pub fn main() -> Result<()> {
    let args: Vec<String> = crate::args().collect();
    let mut out = BufWriter::new(File::create(&args[2])?);
    for line in BufReader::new(File::open(&args[1])?).lines() {
        let case: Value = serde_json::from_str(&line?)?;
        let mut extra = Extra::default();
        let main = case["main"].as_str().ok_or("main")?;
        if main != "air" {
            extra.set_main_hand_stack(Some(ItemStack::new(&format!("minecraft:{main}"), 1)));
        }
        let off = case["off"].as_str().ok_or("off")?;
        if off != "air" {
            extra.off_hand = Some(format!("minecraft:{off}").into());
        }
        let left = case["left"].as_bool().ok_or("left")?;
        for physical_left in [false, true] {
            let stack = render::held_item_stack(&extra, physical_left != left);
            writeln!(
                out,
                "{}",
                stack.as_ref().map_or("minecraft:air", |s| &s.item)
            )?;
        }
    }
    Ok(())
}
