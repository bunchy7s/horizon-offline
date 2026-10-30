use horizon_client::{
    Result,
    item_data::StackData,
    items::{ItemStack, Items},
};
use horizon_world::loot::PackOverlay;
use horizon_world::nbt::Tag;
use serde_json::Value;
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
};

pub fn main() -> Result<()> {
    let args: Vec<_> = crate::args().collect();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3");
    let items = Items::catalog(&root, &PackOverlay::NONE)?;
    let mut out = BufWriter::new(File::create(&args[2])?);
    for line in BufReader::new(File::open(&args[1])?).lines() {
        let row: Value = serde_json::from_str(&line?)?;
        let mut stack = ItemStack::new(
            row["item"].as_str().unwrap(),
            row["count"].as_u64().unwrap() as u32,
        );
        match row["tool"].as_str().unwrap() {
            "default" => {}
            mode => {
                let mut data = StackData::default();
                let tool = if mode == "implicit" {
                    Tag::compound([] as [(&str, Tag); 0])
                } else {
                    Tag::compound([(
                        "can_destroy_blocks_in_creative",
                        Tag::Byte(i8::from(mode == "allow")),
                    )])
                };
                data.set_other("minecraft:tool", Some(tool));
                stack.extra = Some(Box::new(data));
            }
        }
        writeln!(
            out,
            "{}",
            items.can_destroy_block(Some(&stack), row["creative"].as_bool().unwrap())
        )?;
    }
    Ok(())
}
