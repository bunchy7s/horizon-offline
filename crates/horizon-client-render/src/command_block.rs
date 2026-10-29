//! `CommandBlockEntity` with its `BaseCommandBlock`: the command, the success
//! count comparators read, the output it keeps, and the powered, automatic
//! and condition flags `CommandBlock.tick` runs on.
pub use crate::chunk::block_entity::CommandBlock;
use horizon_client_core::{
    nbt_value::{number, text},
    scoreboard,
    text::Text,
};
use horizon_world::nbt::Tag;

/// `CommandBlockEntity.Mode`, in declaration order (the edit screen's mode
/// button cycles through it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Sequence,
    Auto,
    Redstone,
}

impl Mode {
    /// `CommandBlockEntity.getMode`: the block the entity sits in.
    pub fn of_block(name: &str) -> Option<Self> {
        match name {
            "minecraft:command_block" => Some(Self::Redstone),
            "minecraft:repeating_command_block" => Some(Self::Auto),
            "minecraft:chain_command_block" => Some(Self::Sequence),
            _ => None,
        }
    }
    /// The block `ServerGamePacketListenerImpl.handleSetCommandBlock` puts
    /// down for a mode.
    pub fn block(self) -> &'static str {
        match self {
            Self::Sequence => "minecraft:chain_command_block",
            Self::Auto => "minecraft:repeating_command_block",
            Self::Redstone => "minecraft:command_block",
        }
    }
    /// `Enum.ordinal`, which `ServerboundSetCommandBlockPacket` writes.
    pub fn ordinal(self) -> i32 {
        match self {
            Self::Sequence => 0,
            Self::Auto => 1,
            Self::Redstone => 2,
        }
    }
    /// The mode of a packet's ordinal; `handleSetCommandBlock` treats any
    /// other value as a plain command block (its `default` branch).
    pub fn from_ordinal(ordinal: i32) -> Self {
        match ordinal {
            0 => Self::Sequence,
            1 => Self::Auto,
            _ => Self::Redstone,
        }
    }
    /// `CycleButton`: the next value, wrapping round.
    pub fn next(self) -> Self {
        match self {
            Self::Sequence => Self::Auto,
            Self::Auto => Self::Redstone,
            Self::Redstone => Self::Sequence,
        }
    }
    /// `CommandBlockEditScreen`'s label for the mode.
    pub fn label(self) -> &'static str {
        match self {
            Self::Sequence => "advMode.mode.sequence",
            Self::Auto => "advMode.mode.auto",
            Self::Redstone => "advMode.mode.redstone",
        }
    }
}

/// The `automatic` flag `CommandBlock` gives its entity: chain command blocks
/// are always active when placed.
pub fn automatic_block(name: &str) -> bool {
    name == "minecraft:chain_command_block"
}

impl Default for CommandBlock {
    fn default() -> Self {
        Self {
            command: String::new(),
            success_count: 0,
            custom_name: None,
            track_output: true,
            last_output: None,
            update_last_execution: true,
            last_execution: -1,
            powered: false,
            auto: false,
            condition_met: false,
        }
    }
}

impl CommandBlock {
    /// `CommandBlockEntity.loadAdditional` with `BaseCommandBlock.load`.
    pub fn load(tag: &Tag) -> Self {
        let flag = |key: &str, default: bool| number(tag.get(key)).map_or(default, |v| v != 0);
        let track_output = flag("TrackOutput", true);
        let update_last_execution = flag("UpdateLastExecution", true);
        Self {
            command: text(tag.get("Command")).unwrap_or_default(),
            success_count: number(tag.get("SuccessCount")).unwrap_or(0) as i32,
            custom_name: tag.get("CustomName").cloned(),
            track_output,
            last_output: track_output
                .then(|| tag.get("LastOutput"))
                .flatten()
                .map(|t| scoreboard::comp_text(&scoreboard::tag_to_json(t))),
            update_last_execution,
            last_execution: if update_last_execution {
                number(tag.get("LastExecution")).unwrap_or(-1)
            } else {
                -1
            },
            powered: flag("powered", false),
            auto: flag("auto", false),
            condition_met: flag("conditionMet", false),
        }
    }
    /// `CommandBlockEntity.saveAdditional`. The last output is written as a
    /// plain string component.
    pub fn save(&self) -> Vec<(&'static str, Tag)> {
        let mut out = vec![
            ("Command", Tag::String(self.command.as_str().into())),
            ("SuccessCount", Tag::Int(self.success_count)),
        ];
        if let Some(name) = &self.custom_name {
            out.push(("CustomName", name.clone()));
        }
        out.push(("TrackOutput", Tag::Byte(i8::from(self.track_output))));
        if self.track_output
            && let Some(last) = &self.last_output
        {
            out.push(("LastOutput", Tag::String(last.plain().as_str().into())));
        }
        out.push((
            "UpdateLastExecution",
            Tag::Byte(i8::from(self.update_last_execution)),
        ));
        if self.update_last_execution && self.last_execution != -1 {
            out.push(("LastExecution", Tag::Long(self.last_execution)));
        }
        out.push(("powered", Tag::Byte(i8::from(self.powered))));
        out.push(("conditionMet", Tag::Byte(i8::from(self.condition_met))));
        out.push(("auto", Tag::Byte(i8::from(self.auto))));
        out
    }
    /// `BaseCommandBlock.setCommand`: a new command starts at no successes.
    pub fn set_command(&mut self, command: &str) {
        self.command = command.to_owned();
        self.success_count = 0;
    }
    /// `getLastOutput().getString()`: what the edit screen shows.
    pub fn last_output_string(&self) -> String {
        self.last_output
            .as_ref()
            .map(Text::plain)
            .unwrap_or_default()
    }
    /// `getName`: the custom name, else `@`.
    pub fn name(&self) -> Text {
        self.custom_name.as_ref().map_or_else(
            || Text::literal("@"),
            |t| scoreboard::comp_text(&scoreboard::tag_to_json(t)),
        )
    }
}

/// What `CommandBlock.tick` does with a block of `mode` whose condition was
/// `met` when last checked: run its command (`Some(true)`), clear the success
/// count of a conditional block (`Some(false)`), or nothing. Chain blocks
/// only run from the block before them (`executeChain`).
pub fn tick_action(mode: Mode, met: bool, conditional: bool) -> Option<bool> {
    match mode {
        Mode::Sequence => None,
        _ if met => Some(true),
        _ if conditional => Some(false),
        _ => None,
    }
}

/// `CommandBlock.tick`: a repeating block ticks again next tick while powered
/// or always active.
pub fn repeats(mode: Mode, powered: bool, auto: bool) -> bool {
    mode == Mode::Auto && (powered || auto)
}

/// `CommandBlock.setPoweredAndUpdate`: a rising signal schedules the impulse
/// or repeating block's tick unless it is always active anyway.
pub fn power_schedules(mode: Mode, was_powered: bool, powered: bool, auto: bool) -> bool {
    powered && !was_powered && !auto && mode != Mode::Sequence
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_cycle_and_map_to_blocks() {
        for mode in [Mode::Sequence, Mode::Auto, Mode::Redstone] {
            assert_eq!(Mode::of_block(mode.block()), Some(mode));
        }
        assert_eq!(Mode::Redstone.next(), Mode::Sequence);
        assert_eq!(Mode::Sequence.next(), Mode::Auto);
        assert!(automatic_block("minecraft:chain_command_block"));
        assert!(!automatic_block("minecraft:repeating_command_block"));
    }

    #[test]
    fn tick_follows_the_condition() {
        assert_eq!(tick_action(Mode::Redstone, true, false), Some(true));
        assert_eq!(tick_action(Mode::Redstone, false, true), Some(false));
        assert_eq!(tick_action(Mode::Auto, false, false), None);
        assert_eq!(tick_action(Mode::Sequence, true, false), None);
        assert!(repeats(Mode::Auto, false, true));
        assert!(!repeats(Mode::Auto, false, false));
        assert!(!repeats(Mode::Redstone, true, true));
    }

    #[test]
    fn only_a_rising_edge_schedules() {
        assert!(power_schedules(Mode::Redstone, false, true, false));
        assert!(!power_schedules(Mode::Redstone, true, true, false));
        assert!(!power_schedules(Mode::Redstone, false, true, true));
        assert!(!power_schedules(Mode::Sequence, false, true, false));
    }

    #[test]
    fn saves_like_base_command_block() {
        let block = CommandBlock {
            command: "say hi".into(),
            success_count: 1,
            last_output: Some(Text::literal("[12:00:00] hi")),
            last_execution: 42,
            powered: true,
            ..CommandBlock::default()
        };
        let tag = Tag::compound(block.save());
        let back = CommandBlock::load(&tag);
        assert_eq!(back.command, "say hi");
        assert_eq!(back.success_count, 1);
        assert_eq!(back.last_output_string(), "[12:00:00] hi");
        assert_eq!(back.last_execution, 42);
        assert!(back.powered && !back.auto && back.track_output);
        // Not tracking output drops it, and no execution time is kept at -1.
        let quiet = CommandBlock {
            track_output: false,
            last_output: Some(Text::literal("x")),
            ..CommandBlock::default()
        };
        let tag = Tag::compound(quiet.save());
        assert!(tag.get("LastOutput").is_none() && tag.get("LastExecution").is_none());
        assert_eq!(CommandBlock::load(&tag).last_output, None);
    }
}
