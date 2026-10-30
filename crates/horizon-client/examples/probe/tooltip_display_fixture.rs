//! Tooltip display visibility oracle companion for tools/test_tooltip_display_parity.py.
use horizon_client::{item_tooltip::TooltipDisplay, items::ItemStack};
use horizon_world::nbt::Tag;
use std::{error::Error, fmt::Write};

pub fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = crate::args().collect();
    let mut output = String::new();
    for line in std::fs::read_to_string(&args[1])?.lines() {
        if let Some(name) = line.strip_prefix("name\t") {
            let components = horizon_client::item_data::command_components(&format!(
                "[minecraft:custom_name={name}]"
            ))?;
            let (_, extra) = horizon_client::item_data::decode_components(Some(&components));
            let stack = ItemStack {
                extra,
                ..ItemStack::new("minecraft:stick", 1)
            };
            let text = horizon_client::item_tooltip::styled_name(&stack, || "unused".into());
            output.push_str("name\t");
            for span in text.spans {
                for c in span.text.chars() {
                    let style = &span.style;
                    write!(
                        output,
                        "{},{},{},{},{},{},{}|",
                        c as u32,
                        style.color.unwrap_or(horizon_client::text::WHITE),
                        style.is_bold(),
                        style.is_italic(),
                        style.underlined == Some(true),
                        style.strikethrough == Some(true),
                        style.obfuscated == Some(true)
                    )?;
                }
            }
            output.push('\n');
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        let hide_tooltip: bool = fields[0].parse()?;
        let creative: bool = fields[1].parse()?;
        let hide_attributes: bool = fields[2].parse()?;
        let mut component = Vec::new();
        if hide_tooltip {
            component.push(("hide_tooltip", Tag::Byte(1)));
        }
        if hide_attributes {
            component.push((
                "hidden_components",
                Tag::List(vec![Tag::String("minecraft:attribute_modifiers".into())]),
            ));
        }
        let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
        if !component.is_empty() {
            stack
                .data_mut()
                .set_other("minecraft:tooltip_display", Some(Tag::compound(component)));
        }
        let display = TooltipDisplay::for_stack(&stack);
        writeln!(
            output,
            "{}\t{}",
            display.name_visible(creative),
            display.shows("minecraft:attribute_modifiers")
        )?;
    }
    std::fs::write(&args[2], output)?;
    Ok(())
}
