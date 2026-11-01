//! `TestBlockEditScreen`: the test block's mode and message.
use super::{
    Game,
    block_editor::{Editor, Item, Outcome, Pos, cycle},
};
use horizon_client::{
    containers::BlockEntity,
    test_block::{TestBlock, TestBlockMode},
    text::Text,
    ui::EditBox,
};
use horizon_protocol::packets::play::{self, Serverbound as S};

const DONE: u8 = 0;
const CANCEL: u8 = 1;
const MODE: u8 = 2;

pub(super) struct TestBlockEditor {
    edits: Vec<EditBox>,
    mode: TestBlockMode,
}

impl TestBlockEditor {
    pub(super) fn new(b: &TestBlock) -> Self {
        let mut message = EditBox::new(0.0, 0.0, 240.0, 20.0);
        message.max_length = 128;
        message.set_value(&b.message);
        message.focused = false;
        Self {
            edits: vec![message],
            mode: b.mode,
        }
    }
}

impl Editor for TestBlockEditor {
    fn edits(&mut self) -> &mut [EditBox] {
        &mut self.edits
    }

    fn items(&self, _entity: &BlockEntity, w: f32, _h: f32) -> Vec<Item> {
        let mid = (w / 2.0).floor();
        let tr = |key: &str| Text::translate(key, &[]);
        let button = |id: u8, x: f32, bw: f32, y: f32, text: Text| Item::Button {
            id,
            x,
            y,
            w: bw,
            label: text,
            active: true,
        };
        let mut items = Vec::new();
        // The message box shows unless the block starts tests.
        if self.mode != TestBlockMode::Start {
            items.push(Item::Edit {
                index: 0,
                x: mid - 152.0,
                y: 80.0,
                w: 240.0,
            });
        }
        items.push(button(
            MODE,
            mid - 4.0 - 150.0,
            50.0,
            185.0,
            tr(&self.mode.label_key()),
        ));
        items.push(button(
            DONE,
            mid - 4.0 - 150.0,
            150.0,
            210.0,
            tr("gui.done"),
        ));
        items.push(button(CANCEL, mid + 4.0, 150.0, 210.0, tr("gui.cancel")));
        items.push(Item::Title {
            text: tr("block.minecraft.test_block"),
            y: 10.0,
        });
        if self.mode != TestBlockMode::Start {
            items.push(Item::Label {
                text: tr("test_block.message"),
                x: mid - 153.0,
                y: 70.0,
                right: false,
            });
        }
        items.push(Item::Label {
            text: tr(&self.mode.info_key()),
            x: mid - 153.0,
            y: 174.0,
            right: false,
        });
        items
    }

    fn press(
        &mut self,
        entity: &mut BlockEntity,
        pos: Pos,
        id: u8,
        shift: bool,
        _alt: bool,
    ) -> Outcome {
        match id {
            DONE => return self.done(entity, pos),
            CANCEL => return Outcome::Cancel,
            MODE => {
                self.mode = cycle(&TestBlockMode::ALL, self.mode, if shift { -1 } else { 1 });
            }
            _ => {}
        }
        Outcome::Stay
    }

    /// `onDone`: the mode and message, as the screen has them.
    fn done(&mut self, _entity: &mut BlockEntity, pos: Pos) -> Outcome {
        Outcome::Send(S::SetTestBlock(play::SetTestBlock {
            pos: [pos.0, pos.1, pos.2],
            mode: self.mode.id(),
            message: self.edits[0].value.clone(),
        }))
    }

    fn cancel(&mut self, _entity: &mut BlockEntity) {}

    /// The screen has no confirm key of its own.
    fn enter_confirms(&self) -> bool {
        false
    }
}

impl Game<'_> {
    /// `LocalPlayer.openTestBlock`.
    pub(super) fn open_test_block(&mut self, pos: Pos) {
        let Some(BlockEntity::TestBlock(b)) = self.world.block_entities.get(&pos) else {
            return;
        };
        let editor = TestBlockEditor::new(b);
        self.open_block_editor(pos, Box::new(editor));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn done_sends_the_mode_and_message() {
        let block = TestBlock {
            mode: TestBlockMode::Log,
            message: "hello".into(),
            powered: false,
            triggered: false,
        };
        let mut e = TestBlockEditor::new(&block);
        let mut entity = BlockEntity::TestBlock(Box::new(block));
        e.press(&mut entity, (0, 0, 0), MODE, false, false);
        let Outcome::Send(S::SetTestBlock(p)) = e.done(&mut entity, (4, 5, 6)) else {
            panic!("done sends the block");
        };
        assert_eq!((p.pos, p.mode, p.message.as_str()), ([4, 5, 6], 2, "hello"));
        // Shift steps back, wrapping from the first mode to the last.
        e.mode = TestBlockMode::Start;
        e.press(&mut entity, (0, 0, 0), MODE, true, false);
        assert_eq!(e.mode, TestBlockMode::Accept);
        assert!(!e.enter_confirms());
    }

    #[test]
    fn the_message_box_hides_for_start() {
        let mut e = TestBlockEditor::new(&TestBlock::new(TestBlockMode::Start));
        let entity = BlockEntity::TestBlock(Box::new(TestBlock::new(TestBlockMode::Start)));
        let boxes = |e: &TestBlockEditor| {
            e.items(&entity, 320.0, 240.0)
                .iter()
                .filter(|i| matches!(i, Item::Edit { .. }))
                .count()
        };
        assert_eq!(boxes(&e), 0);
        e.mode = TestBlockMode::Log;
        assert_eq!(boxes(&e), 1);
    }
}
