#![forbid(unsafe_code)]
//! Runs tools/test_container_screen_parity.py's screens and mouse events
//! through `horizon_client::container_screen`.
use horizon_client::{
    Result,
    container_screen::{Book, Button, Clicks, Image, Release, Target, clicked_outside},
    menu::Input,
};
use serde_json::Value;
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
};

fn input_name(input: Input) -> &'static str {
    match input {
        Input::Pickup => "PICKUP",
        Input::QuickMove => "QUICK_MOVE",
        Input::Swap => "SWAP",
        Input::Clone => "CLONE",
        Input::Throw => "THROW",
        Input::QuickCraft => "QUICK_CRAFT",
        Input::PickupAll => "PICKUP_ALL",
    }
}

pub fn main() -> Result<()> {
    let args: Vec<String> = crate::args().collect();
    let input = BufReader::new(File::open(&args[1])?);
    let mut out = BufWriter::new(File::create(&args[2])?);
    for (n, line) in input.lines().enumerate() {
        let c: Value = serde_json::from_str(&line?)?;
        writeln!(out, "== {n}")?;
        let int = |v: &Value| v.as_i64().unwrap() as i32;
        let image = Image {
            left: int(&c["image"][0]),
            top: int(&c["image"][1]),
            width: int(&c["image"][2]),
            height: int(&c["image"][3]),
        };
        let book = c.get("book").map(|b| Book {
            visible: b[0].as_bool().unwrap(),
            selected_tab_hovered: b[1].as_bool().unwrap(),
        });
        let mut clicks = Clicks::default();
        // `keyPickItem.matchesMouse(event)`: the pick-item button is the middle
        // one, and only a player with infinite materials clones with it.
        let creative = c.get("creative").and_then(Value::as_bool).unwrap_or(false);
        for ev in c["events"].as_array().unwrap() {
            let name = ev[0].as_str().unwrap();
            let (x, y) = (ev[1].as_f64().unwrap(), ev[2].as_f64().unwrap());
            let outside = clicked_outside(x, y, image, book);
            let mut line = format!("{name} {outside}");
            if name != "point" {
                // The case says whether the event is over the menu's slot (7).
                let on_slot = ev[6].as_bool().unwrap();
                line += &format!(" {on_slot}");
                let target = Target::of(on_slot.then_some(7), outside);
                let button = match ev[3].as_i64().unwrap() {
                    1 => Button::Left,
                    3 => Button::Right,
                    _ => Button::Middle,
                };
                let shift = ev[4].as_i64().unwrap() & 3 != 0;
                let carried_empty = !ev[5].as_bool().unwrap();
                let pick = button == Button::Middle;
                let click = if name == "press" {
                    clicks.press(&target, button, carried_empty, shift, pick && creative)
                } else {
                    match clicks.release(&target, button, carried_empty, false, shift, pick) {
                        Release::Click(input) => Some(input),
                        Release::QuickCraft => panic!("no drags in these cases"),
                        Release::Nothing => None,
                    }
                };
                if let Some(input) = click {
                    let id = match target {
                        Target::Slot(s) => s,
                        Target::Outside => -999,
                        Target::Nothing => -1,
                    };
                    line += &format!(" click {id} {} {}", button.menu_button(), input_name(input));
                }
            }
            writeln!(out, "{line}")?;
        }
    }
    out.flush()?;
    Ok(())
}
