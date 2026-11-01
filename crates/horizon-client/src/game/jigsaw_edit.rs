//! `JigsawBlockEditScreen`: the jigsaw block's pool, name, target, final
//! state, priorities and joint, with the Generate button and its levels.
use super::{
    Game,
    block_editor::{Editor, Item, Outcome, Pos, cycle, on_off},
};
use horizon_client::{
    containers::BlockEntity,
    jigsaw_block::{JigsawBlock, JointType, is_vertical_front},
    structure_block::parse_identifier,
    text::Text,
    ui::EditBox,
};
use horizon_protocol::packets::play::{self, Serverbound as S};

const POOL: usize = 0;
const NAME: usize = 1;
const TARGET: usize = 2;
const FINAL_STATE: usize = 3;
const SELECTION_PRIORITY: usize = 4;
const PLACEMENT_PRIORITY: usize = 5;

const DONE: u8 = 0;
const CANCEL: u8 = 1;
const JOINT: u8 = 2;
const LEVELS: u8 = 3;
const KEEP_JIGSAWS: u8 = 4;
const GENERATE: u8 = 5;

/// `JigsawBlockEditScreen.levels`' range: `clampedLerp(value, 0, 20)`.
const MAX_LEVELS: f32 = 20.0;

pub(super) struct JigsawEditor {
    edits: Vec<EditBox>,
    joint: JointType,
    /// The block faces up or down: only then is the joint chosen.
    vertical: bool,
    levels: i32,
    /// The slider's position (0..1), as the player left it.
    slider: f32,
    keep_jigsaws: bool,
}

impl JigsawEditor {
    pub(super) fn new(b: &JigsawBlock, vertical: bool) -> Self {
        let edit = |max: usize, value: String| {
            let mut e = EditBox::new(0.0, 0.0, 98.0, 20.0);
            e.max_length = max;
            e.set_value(&value);
            e.focused = false;
            e
        };
        Self {
            edits: vec![
                edit(128, b.pool.clone()),
                edit(128, b.name.clone()),
                edit(128, b.target.clone()),
                edit(256, b.final_state.clone()),
                edit(3, b.selection_priority.to_string()),
                edit(3, b.placement_priority.to_string()),
            ],
            joint: b.joint,
            vertical,
            levels: 0,
            slider: 0.0,
            keep_jigsaws: false,
        }
    }

    fn value(&self, i: usize) -> &str {
        &self.edits[i].value
    }

    /// `updateValidity`: Done and Generate need three legal identifiers.
    fn valid(&self) -> bool {
        [NAME, TARGET, POOL]
            .into_iter()
            .all(|i| parse_identifier(self.value(i)).is_some())
    }

    /// `parseAsInt`.
    fn int(&self, i: usize) -> i32 {
        self.value(i).parse().unwrap_or(0)
    }

    /// `sendToServer`.
    fn set_packet(&self, pos: Pos) -> Option<S> {
        let id = |i: usize| parse_identifier(self.value(i));
        Some(S::SetJigsawBlock(play::SetJigsawBlock {
            pos: [pos.0, pos.1, pos.2],
            name: id(NAME)?,
            target: id(TARGET)?,
            pool: id(POOL)?,
            final_state: self.value(FINAL_STATE).to_owned(),
            joint: self.joint.serialized_name().to_owned(),
            selection_priority: self.int(SELECTION_PRIORITY),
            placement_priority: self.int(PLACEMENT_PRIORITY),
        }))
    }

    fn generate_packet(&self, pos: Pos) -> S {
        S::JigsawGenerate(play::JigsawGenerate {
            pos: [pos.0, pos.1, pos.2],
            levels: self.levels,
            keep_jigsaws: self.keep_jigsaws,
        })
    }
}

impl Editor for JigsawEditor {
    fn edits(&mut self) -> &mut [EditBox] {
        &mut self.edits
    }

    fn items(&self, _entity: &BlockEntity, w: f32, _h: f32) -> Vec<Item> {
        let mid = (w / 2.0).floor();
        let tr = |key: &str| Text::translate(key, &[]);
        let valid = self.valid();
        let button = |id: u8, x: f32, bw: f32, y: f32, text: Text, active: bool| Item::Button {
            id,
            x,
            y,
            w: bw,
            label: text,
            active,
        };
        let label = |key: &str, x: f32, y: f32| Item::Label {
            text: tr(key),
            x,
            y,
            right: false,
        };
        let edit = |index: usize, x: f32, y: f32, ew: f32| Item::Edit { index, x, y, w: ew };
        let mut items = Vec::new();
        if self.vertical {
            items.push(button(
                JOINT,
                mid + 54.0,
                100.0,
                160.0,
                tr(&self.joint.label_key()),
                true,
            ));
        }
        items.push(Item::Slider {
            id: LEVELS,
            x: mid - 154.0,
            y: 185.0,
            w: 100.0,
            label: Text::translate(
                "jigsaw_block.levels",
                &[Text::literal(self.levels.to_string())],
            ),
            value: self.slider,
        });
        items.push(button(
            KEEP_JIGSAWS,
            mid - 50.0,
            100.0,
            185.0,
            Text::translate(
                "options.generic_value",
                &[tr("jigsaw_block.keep_jigsaws"), on_off(self.keep_jigsaws)],
            ),
            true,
        ));
        items.push(button(
            GENERATE,
            mid + 54.0,
            100.0,
            185.0,
            tr("jigsaw_block.generate"),
            valid,
        ));
        items.push(button(
            DONE,
            mid - 4.0 - 150.0,
            150.0,
            210.0,
            tr("gui.done"),
            valid,
        ));
        items.push(button(
            CANCEL,
            mid + 4.0,
            150.0,
            210.0,
            tr("gui.cancel"),
            true,
        ));
        // The boxes in the order they are added; the labels are drawn with them.
        items.push(label("jigsaw_block.pool", mid - 153.0, 10.0));
        items.push(edit(POOL, mid - 153.0, 20.0, 300.0));
        items.push(label("jigsaw_block.name", mid - 153.0, 45.0));
        items.push(edit(NAME, mid - 153.0, 55.0, 300.0));
        items.push(label("jigsaw_block.target", mid - 153.0, 80.0));
        items.push(edit(TARGET, mid - 153.0, 90.0, 300.0));
        items.push(label("jigsaw_block.final_state", mid - 153.0, 115.0));
        items.push(edit(FINAL_STATE, mid - 153.0, 125.0, 300.0));
        items.push(label("jigsaw_block.selection_priority", mid - 153.0, 150.0));
        items.push(edit(PLACEMENT_PRIORITY, mid - 50.0, 160.0, 98.0));
        items.push(label("jigsaw_block.placement_priority", mid - 50.0, 150.0));
        items.push(edit(SELECTION_PRIORITY, mid - 153.0, 160.0, 98.0));
        if self.vertical {
            items.push(label("jigsaw_block.joint_label", mid + 53.0, 150.0));
        }
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
        let delta = if shift { -1 } else { 1 };
        match id {
            DONE => return self.done(entity, pos),
            CANCEL => return Outcome::Cancel,
            JOINT if self.vertical => self.joint = cycle(&JointType::ALL, self.joint, delta),
            KEEP_JIGSAWS => self.keep_jigsaws = !self.keep_jigsaws,
            GENERATE if self.valid() => {
                // onDone(); sendGenerate().
                if let Some(set) = self.set_packet(pos) {
                    return Outcome::SendAll(vec![set, self.generate_packet(pos)]);
                }
            }
            _ => {}
        }
        Outcome::Stay
    }

    fn slide(&mut self, id: u8, value: f32) {
        if id == LEVELS {
            // Mth.floor(Mth.clampedLerp(value, 0.0, 20.0)).
            self.slider = value.clamp(0.0, 1.0);
            self.levels = (self.slider * MAX_LEVELS).floor() as i32;
        }
    }

    fn done(&mut self, _entity: &mut BlockEntity, pos: Pos) -> Outcome {
        // Enter confirms only while the Done button is active.
        match self.valid().then(|| self.set_packet(pos)).flatten() {
            Some(set) => Outcome::Send(set),
            None => Outcome::Stay,
        }
    }

    fn cancel(&mut self, _entity: &mut BlockEntity) {}
}

impl Game<'_> {
    /// `LocalPlayer.openJigsawBlock`.
    pub(super) fn open_jigsaw_block(&mut self, pos: Pos) {
        let Some(BlockEntity::JigsawBlock(b)) = self.world.block_entities.get(&pos) else {
            return;
        };
        let vertical = self
            .jigsaw_front(pos)
            .is_some_and(|f| is_vertical_front(&f));
        let editor = JigsawEditor::new(b, vertical);
        self.open_block_editor(pos, Box::new(editor));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor() -> JigsawEditor {
        let b = JigsawBlock {
            name: "minecraft:bottom".into(),
            target: "minecraft:top".into(),
            pool: "minecraft:village/plains/houses".into(),
            ..JigsawBlock::default()
        };
        JigsawEditor::new(&b, true)
    }

    #[test]
    fn done_sends_the_screens_values_with_parsed_priorities() {
        let mut e = editor();
        e.edits[SELECTION_PRIORITY].set_value("7");
        e.edits[PLACEMENT_PRIORITY].set_value("junk");
        e.edits[TARGET].set_value("door");
        let mut entity = BlockEntity::JigsawBlock(Box::default());
        let Outcome::Send(S::SetJigsawBlock(p)) = e.done(&mut entity, (1, 2, 3)) else {
            panic!("done sends the block");
        };
        assert_eq!(p.pos, [1, 2, 3]);
        assert_eq!(p.target, "minecraft:door");
        assert_eq!((p.selection_priority, p.placement_priority), (7, 0));
        assert_eq!(p.joint, "rollable");
    }

    #[test]
    fn bad_identifiers_deactivate_done_and_generate() {
        let mut e = editor();
        e.edits[NAME].set_value("Bad Name");
        let mut entity = BlockEntity::JigsawBlock(Box::default());
        assert!(matches!(e.done(&mut entity, (0, 0, 0)), Outcome::Stay));
        assert!(matches!(
            e.press(&mut entity, (0, 0, 0), GENERATE, false, false),
            Outcome::Stay
        ));
    }

    #[test]
    fn generate_sends_the_block_then_the_levels() {
        let mut e = editor();
        e.slide(LEVELS, 0.5);
        let mut entity = BlockEntity::JigsawBlock(Box::default());
        e.press(&mut entity, (0, 0, 0), KEEP_JIGSAWS, false, false);
        let Outcome::SendAll(packets) = e.press(&mut entity, (5, 6, 7), GENERATE, false, false)
        else {
            panic!("generate sends two packets");
        };
        assert!(matches!(packets[0], S::SetJigsawBlock(_)));
        let S::JigsawGenerate(g) = &packets[1] else {
            panic!("then the generate packet");
        };
        assert_eq!((g.pos, g.levels, g.keep_jigsaws), ([5, 6, 7], 10, true));
        // The slider's top is level 20; the joint cycles only when vertical.
        e.slide(LEVELS, 1.0);
        assert_eq!(e.levels, 20);
        e.press(&mut entity, (0, 0, 0), JOINT, false, false);
        assert_eq!(e.joint, JointType::Aligned);
    }
}
