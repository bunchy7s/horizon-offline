//! `TestBlockEntity`: the test block's mode, message and powered flag, with
//! its save format (`saveAdditional` / `loadAdditional`).
use horizon_client_core::nbt_value::{number, string, text};
use horizon_world::nbt::Tag;

/// `TestBlockMode`, in id order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestBlockMode {
    Start,
    Log,
    Fail,
    Accept,
}

impl TestBlockMode {
    pub const ALL: [Self; 4] = [Self::Start, Self::Log, Self::Fail, Self::Accept];
    /// `getSerializedName`: the block state's `mode` value.
    pub fn serialized_name(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Log => "log",
            Self::Fail => "fail",
            Self::Accept => "accept",
        }
    }
    pub fn of_serialized_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.serialized_name() == name)
    }
    pub fn id(self) -> i32 {
        self as i32
    }
    /// `ByIdMap.continuous(.., OutOfBoundsStrategy.ZERO)`.
    pub fn by_id(id: i32) -> Self {
        usize::try_from(id)
            .ok()
            .and_then(|i| Self::ALL.get(i).copied())
            .unwrap_or(Self::Start)
    }
    /// `getDisplayName`'s lang key.
    pub fn label_key(self) -> String {
        format!("test_block.mode.{}", self.serialized_name())
    }
    /// `getDetailedMessage`'s lang key.
    pub fn info_key(self) -> String {
        format!("test_block.mode_info.{}", self.serialized_name())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestBlock {
    pub mode: TestBlockMode,
    pub message: String,
    pub powered: bool,
    /// `triggered`: not saved; the game test reads and clears it.
    pub triggered: bool,
}

/// What a `TestBlockEntity` call asks of the level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Effects {
    /// `level.updateNeighborsAt(pos, block)`.
    pub update_neighbors: bool,
    /// `log()`.
    pub log: bool,
}

impl TestBlock {
    /// `new TestBlockEntity(pos, state)`: the mode is the state's.
    pub fn new(mode: TestBlockMode) -> Self {
        Self {
            mode,
            message: String::new(),
            powered: false,
            triggered: false,
        }
    }

    /// `reset`.
    pub fn reset(&mut self) -> Effects {
        self.triggered = false;
        let start = self.mode == TestBlockMode::Start;
        if start {
            self.powered = false;
        }
        Effects {
            update_neighbors: start,
            log: false,
        }
    }

    /// `trigger`: a start block powers up and logs; the others log (LOG
    /// only) and remember they were triggered.
    pub fn trigger(&mut self) -> Effects {
        if self.mode == TestBlockMode::Start {
            self.powered = true;
            return Effects {
                update_neighbors: true,
                log: true,
            };
        }
        self.triggered = true;
        Effects {
            update_neighbors: false,
            log: self.mode == TestBlockMode::Log,
        }
    }

    /// `TestBlock.neighborChanged`: `signal` is `hasNeighborSignal`; a start
    /// block ignores it, the others trigger on a rising signal.
    pub fn neighbor_changed(&mut self, signal: bool) -> Option<Effects> {
        if self.mode == TestBlockMode::Start {
            return None;
        }
        if signal && !self.powered {
            self.powered = true;
            Some(self.trigger())
        } else {
            if !signal && self.powered {
                self.powered = false;
            }
            None
        }
    }

    /// `TestBlock.ownSignal`: a powered start block gives 15.
    pub fn own_signal(&self) -> i32 {
        if self.mode == TestBlockMode::Start && self.powered {
            15
        } else {
            0
        }
    }

    /// `saveAdditional` (also `getUpdateTag`, which is `saveCustomOnly`).
    pub fn save(&self) -> Vec<(&'static str, Tag)> {
        vec![
            ("mode", string(self.mode.serialized_name())),
            ("message", string(&self.message)),
            ("powered", Tag::Byte(i8::from(self.powered))),
        ]
    }

    /// `loadAdditional`: a missing or unknown mode is FAIL.
    pub fn load(tag: &Tag) -> Self {
        Self {
            mode: text(tag.get("mode"))
                .and_then(|m| TestBlockMode::of_serialized_name(&m))
                .unwrap_or(TestBlockMode::Fail),
            message: text(tag.get("message")).unwrap_or_default(),
            powered: number(tag.get("powered")).is_some_and(|v| v != 0),
            triggered: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_loads_like_test_block_entity() {
        let block = TestBlock {
            mode: TestBlockMode::Accept,
            message: "all done".into(),
            powered: true,
            triggered: false,
        };
        let tag = Tag::compound(block.save());
        assert_eq!(text(tag.get("mode")).as_deref(), Some("accept"));
        assert_eq!(TestBlock::load(&tag), block);
    }

    #[test]
    fn start_blocks_power_up_and_the_others_trigger_on_a_rising_signal() {
        let mut start = TestBlock::new(TestBlockMode::Start);
        assert_eq!(start.neighbor_changed(true), None);
        assert_eq!(start.own_signal(), 0);
        let effects = start.trigger();
        assert!(effects.update_neighbors && effects.log && start.own_signal() == 15);
        assert!(start.reset().update_neighbors && start.own_signal() == 0);

        let mut log = TestBlock::new(TestBlockMode::Log);
        let effects = log.neighbor_changed(true).unwrap();
        assert!(effects.log && !effects.update_neighbors && log.triggered && log.powered);
        // Still powered: no second trigger; a fall clears the flag.
        assert_eq!(log.neighbor_changed(true), None);
        assert_eq!(log.neighbor_changed(false), None);
        assert!(!log.powered && log.triggered);
        assert!(!log.reset().update_neighbors && !log.triggered);
        let mut accept = TestBlock::new(TestBlockMode::Accept);
        assert!(!accept.neighbor_changed(true).unwrap().log);
    }

    #[test]
    fn load_defaults_to_fail_and_ids_wrap_to_start() {
        let block = TestBlock::load(&Tag::compound(Vec::<(&str, Tag)>::new()));
        assert_eq!(block, TestBlock::new(TestBlockMode::Fail));
        assert_eq!(TestBlockMode::by_id(3), TestBlockMode::Accept);
        assert_eq!(TestBlockMode::by_id(9), TestBlockMode::Start);
        assert_eq!(TestBlockMode::Log.id(), 1);
    }
}
