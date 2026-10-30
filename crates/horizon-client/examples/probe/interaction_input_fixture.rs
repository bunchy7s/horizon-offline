use horizon_client::interaction_input::{
    Button, InteractionInput, InteractionWorld, TargetUseWorld, use_on_target,
};
use serde_json::Value;
use std::io::{BufRead, Write};

struct World {
    using: bool,
    destroying: bool,
    entity: bool,
    dig_on_attack: bool,
    begin_using: bool,
    delay: u32,
    events: Vec<String>,
    passing: bool,
}
impl InteractionWorld for World {
    // The fixture's targets don't change within a tick.
    fn pick_hit_result(&mut self) {}
    fn release_hit_result(&mut self) {}
    fn using_item(&self) -> bool {
        self.using
    }
    fn release_using(&mut self) {
        self.events.push("release".into());
        self.using = false;
    }
    fn start_attack(&mut self) -> bool {
        self.events.push("attack".into());
        if !self.entity {
            self.destroying = self.dig_on_attack;
        }
        false
    }
    fn start_use(&mut self) {
        if self.destroying {
            return;
        }
        self.delay = 4;
        let entity = self.entity;
        use_on_target(self, entity);
    }
    fn pick(&mut self) {
        self.events.push("pick".into());
    }
    fn use_delay(&self) -> u32 {
        self.delay
    }
    fn continue_attack(&mut self, down: bool) {
        if self.using {
            return;
        }
        let down = down && !self.entity;
        self.events.push(format!("continue {down}"));
        if !down {
            self.destroying = false;
        }
    }
}

impl TargetUseWorld for World {
    fn use_entity(&mut self) -> bool {
        self.events
            .push(if self.passing { "entity" } else { "use" }.into());
        if self.passing {
            false
        } else {
            self.using = self.begin_using;
            true
        }
    }
    fn use_block(&mut self) -> bool {
        self.events
            .push(if self.passing { "block" } else { "use" }.into());
        if self.passing {
            false
        } else {
            self.using = self.begin_using;
            true
        }
    }
    fn use_item(&mut self) {
        self.events.push("item".into());
        self.using = self.begin_using;
    }
}

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = crate::args().collect();
    let input = std::io::BufReader::new(std::fs::File::open(&args[1])?);
    let mut out = std::io::BufWriter::new(std::fs::File::create(&args[2])?);
    for (i, line) in input.lines().enumerate() {
        let row: Value = serde_json::from_str(&line?)?;
        let flag = |key: &str| row[key].as_bool().unwrap();
        let mut world = World {
            using: flag("using"),
            destroying: flag("destroying"),
            entity: flag("entity"),
            dig_on_attack: flag("dig_on_attack"),
            begin_using: flag("begin_using"),
            delay: row["delay"].as_u64().unwrap() as u32,
            events: vec![],
            passing: row["passing"].as_bool().unwrap_or(false),
        };
        let mut input = InteractionInput::default();
        for button in row["clicks"].as_array().unwrap() {
            input.press(match button.as_u64().unwrap() {
                0 => Button::Attack,
                1 => Button::Use,
                _ => Button::Pick,
            });
        }
        input.tick(&mut world, flag("attack_down"), flag("use_down"));
        writeln!(out, "== {i}")?;
        for event in world.events {
            writeln!(out, "{event}")?;
        }
        writeln!(
            out,
            "state {} {} {}",
            world.using, world.destroying, world.delay
        )?;
    }
    Ok(())
}
