//! `StructureBlockEditScreen`: the structure block's name, position, size,
//! mode and options, with its buttons laid out as vanilla's.
use super::{
    Game,
    block_editor::{Editor, Item, Outcome, Pos, cycle, on_off, valid_character_for_name},
};
use horizon_client::{
    containers::BlockEntity,
    structure_block::{
        self, StructureBlock, StructureMode, UpdateType, mirror_ordinal, rotation_ordinal,
    },
    text::Text,
    ui::EditBox,
};
use horizon_protocol::packets::play::{self, Serverbound as S};
use horizon_world::worldgen::structure::{Mirror, Rotation};

const NAME: usize = 0;
const POS: usize = 1;
const SIZE: usize = 4;
const INTEGRITY: usize = 7;
const SEED: usize = 8;
const DATA: usize = 9;

const DONE: u8 = 0;
const CANCEL: u8 = 1;
const SAVE: u8 = 2;
const LOAD: u8 = 3;
const MODE: u8 = 4;
const DETECT: u8 = 5;
const ENTITIES: u8 = 6;
const STRICT: u8 = 7;
const MIRROR: u8 = 8;
const SHOW_AIR: u8 = 9;
const SHOW_BOX: u8 = 10;
/// The four rotation buttons, 11 to 14, in `Rotation` order.
const ROTATION: u8 = 11;

/// `StructureBlockEditScreen.DEFAULT_MODES`: every mode but DATA.
const DEFAULT_MODES: [StructureMode; 3] = [
    StructureMode::Save,
    StructureMode::Load,
    StructureMode::Corner,
];

/// The values the screen restores on Cancel.
struct Initial {
    mirror: Mirror,
    rotation: Rotation,
    mode: StructureMode,
    ignore_entities: bool,
    strict: bool,
    show_air: bool,
    show_bounding_box: bool,
}

pub(super) struct StructureEditor {
    edits: Vec<EditBox>,
    initial: Initial,
}

/// `DecimalFormat("0.0###")` of the integrity.
fn format_integrity(value: f32) -> String {
    let text = format!("{:.4}", f64::from(value));
    let trimmed = text.trim_end_matches('0');
    if trimmed.ends_with('.') {
        format!("{trimmed}0")
    } else {
        trimmed.to_owned()
    }
}

impl StructureEditor {
    pub(super) fn new(b: &StructureBlock) -> Self {
        let edit = |max: usize, value: String| {
            let mut e = EditBox::new(0.0, 0.0, 80.0, 20.0);
            e.max_length = max;
            e.set_value(&value);
            e.focused = false;
            e
        };
        let mut edits = vec![edit(128, b.structure_name().to_owned())];
        edits.extend(b.pos.map(|v| edit(15, v.to_string())));
        edits.extend(b.size.map(|v| edit(15, v.to_string())));
        edits.push(edit(15, format_integrity(b.integrity)));
        edits.push(edit(31, b.seed.to_string()));
        edits.push(edit(128, b.metadata.clone()));
        Self {
            edits,
            initial: Initial {
                mirror: b.mirror,
                rotation: b.rotation,
                mode: b.mode,
                ignore_entities: b.ignore_entities,
                strict: b.strict,
                show_air: b.show_air,
                show_bounding_box: b.show_bounding_box,
            },
        }
    }

    fn value(&self, i: usize) -> &str {
        &self.edits[i].value
    }
    fn coordinate(&self, i: usize) -> i32 {
        self.value(i).parse().unwrap_or(0)
    }

    /// `sendToServer`.
    fn packet(&self, b: &StructureBlock, pos: Pos, update: UpdateType) -> S {
        S::SetStructureBlock(play::SetStructureBlock {
            pos: [pos.0, pos.1, pos.2],
            update_type: update.ordinal(),
            mode: b.mode.ordinal(),
            name: self.value(NAME).to_owned(),
            offset: [0, 1, 2].map(|i| self.coordinate(POS + i)),
            size: [0, 1, 2].map(|i| self.coordinate(SIZE + i)),
            mirror: mirror_ordinal(b.mirror),
            rotation: rotation_ordinal(b.rotation),
            data: self.value(DATA).to_owned(),
            ignore_entities: b.ignore_entities,
            strict: b.strict,
            show_air: b.show_air,
            show_bounding_box: b.show_bounding_box,
            integrity: self.value(INTEGRITY).trim().parse().unwrap_or(1.0),
            seed: self.value(SEED).parse().unwrap_or(0),
        })
    }
}

fn mirror_key(mirror: Mirror) -> &'static str {
    match mirror {
        Mirror::None => "mirror.none",
        Mirror::LeftRight => "mirror.left_right",
        Mirror::FrontBack => "mirror.front_back",
    }
}

impl Editor for StructureEditor {
    fn edits(&mut self) -> &mut [EditBox] {
        &mut self.edits
    }

    fn items(&self, entity: &BlockEntity, w: f32, _h: f32) -> Vec<Item> {
        let BlockEntity::StructureBlock(b) = entity else {
            return Vec::new();
        };
        let mid = (w / 2.0).floor();
        let tr = |key: &str| Text::translate(key, &[]);
        let label = |key: &str, x: f32, y: f32, right: bool| Item::Label {
            text: tr(key),
            x,
            y,
            right,
        };
        let button = |id: u8, x: f32, bw: f32, y: f32, text: Text, active: bool| Item::Button {
            id,
            x,
            y,
            w: bw,
            label: text,
            active,
        };
        let edit = |index: usize, x: f32, y: f32, ew: f32| Item::Edit { index, x, y, w: ew };
        let mode = b.mode;
        let mut items = vec![
            button(DONE, mid - 4.0 - 150.0, 150.0, 210.0, tr("gui.done"), true),
            button(CANCEL, mid + 4.0, 150.0, 210.0, tr("gui.cancel"), true),
            button(
                MODE,
                mid - 4.0 - 150.0,
                50.0,
                185.0,
                tr(&mode.label_key()),
                true,
            ),
        ];
        let right = mid + 4.0 + 100.0;
        let saving = mode == StructureMode::Save;
        let loading = mode == StructureMode::Load;
        if saving {
            items.push(button(
                SAVE,
                right,
                50.0,
                185.0,
                tr("structure_block.button.save"),
                true,
            ));
            items.push(button(
                DETECT,
                right,
                50.0,
                120.0,
                tr("structure_block.button.detect_size"),
                true,
            ));
            items.push(button(
                SHOW_AIR,
                right,
                50.0,
                80.0,
                on_off(b.show_air),
                true,
            ));
        }
        if loading {
            items.push(button(
                LOAD,
                right,
                50.0,
                185.0,
                tr("structure_block.button.load"),
                true,
            ));
            items.push(button(STRICT, right, 50.0, 120.0, on_off(b.strict), true));
            items.push(button(
                MIRROR,
                mid - 20.0,
                40.0,
                185.0,
                tr(mirror_key(b.mirror)),
                true,
            ));
            items.push(button(
                SHOW_BOX,
                right,
                50.0,
                80.0,
                on_off(b.show_bounding_box),
                true,
            ));
            for (i, (x, name)) in [
                (mid - 1.0 - 40.0 - 1.0 - 40.0 - 20.0, "0"),
                (mid - 1.0 - 40.0 - 20.0, "90"),
                (mid + 1.0 + 20.0, "180"),
                (mid + 1.0 + 40.0 + 1.0 + 20.0, "270"),
            ]
            .into_iter()
            .enumerate()
            {
                items.push(button(
                    ROTATION + i as u8,
                    x,
                    40.0,
                    185.0,
                    Text::literal(name),
                    rotation_ordinal(b.rotation) != i as i32,
                ));
            }
        }
        if saving || loading {
            items.push(button(
                ENTITIES,
                right,
                50.0,
                160.0,
                on_off(!b.ignore_entities),
                true,
            ));
        }
        items.push(Item::Title {
            text: tr("block.minecraft.structure_block"),
            y: 10.0,
        });
        if mode != StructureMode::Data {
            items.push(label(
                "structure_block.structure_name",
                mid - 153.0,
                30.0,
                false,
            ));
            items.push(edit(NAME, mid - 152.0, 40.0, 300.0));
        }
        if saving || loading {
            items.push(label("structure_block.position", mid - 153.0, 70.0, false));
            for (i, x) in [-152.0, -72.0, 8.0].into_iter().enumerate() {
                items.push(edit(POS + i, mid + x, 80.0, 80.0));
            }
            items.push(label(
                "structure_block.include_entities",
                mid + 154.0,
                150.0,
                true,
            ));
        }
        if saving {
            items.push(label("structure_block.size", mid - 153.0, 110.0, false));
            for (i, x) in [-152.0, -72.0, 8.0].into_iter().enumerate() {
                items.push(edit(SIZE + i, mid + x, 120.0, 80.0));
            }
            items.push(label(
                "structure_block.detect_size",
                mid + 154.0,
                110.0,
                true,
            ));
            items.push(label("structure_block.show_air", mid + 154.0, 70.0, true));
        }
        if loading {
            items.push(label(
                "structure_block.integrity",
                mid - 153.0,
                110.0,
                false,
            ));
            items.push(edit(INTEGRITY, mid - 152.0, 120.0, 80.0));
            items.push(edit(SEED, mid - 72.0, 120.0, 80.0));
            items.push(label("structure_block.strict", mid + 154.0, 110.0, true));
            items.push(label(
                "structure_block.show_boundingbox",
                mid + 154.0,
                70.0,
                true,
            ));
        }
        if mode == StructureMode::Data {
            items.push(label(
                "structure_block.custom_data",
                mid - 153.0,
                110.0,
                false,
            ));
            items.push(edit(DATA, mid - 152.0, 120.0, 240.0));
        }
        items.push(Item::Label {
            text: tr(&mode.info_key()),
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
        alt: bool,
    ) -> Outcome {
        let BlockEntity::StructureBlock(b) = entity else {
            return Outcome::Cancel;
        };
        let delta = if shift { -1 } else { 1 };
        match id {
            DONE => return self.done(entity, pos),
            CANCEL => {
                self.cancel(entity);
                return Outcome::Cancel;
            }
            SAVE if b.mode == StructureMode::Save => {
                return Outcome::Send(self.packet(b, pos, UpdateType::SaveArea));
            }
            LOAD if b.mode == StructureMode::Load => {
                return Outcome::Send(self.packet(b, pos, UpdateType::LoadArea));
            }
            DETECT if b.mode == StructureMode::Save => {
                return Outcome::Send(self.packet(b, pos, UpdateType::ScanArea));
            }
            MODE => {
                // Alt cycles through DATA too.
                b.mode = if alt {
                    cycle(&StructureMode::ALL, b.mode, delta)
                } else {
                    cycle(&DEFAULT_MODES, b.mode, delta)
                };
            }
            ENTITIES => b.ignore_entities = !b.ignore_entities,
            STRICT => b.strict = !b.strict,
            MIRROR => {
                let all = [Mirror::None, Mirror::LeftRight, Mirror::FrontBack];
                b.mirror = cycle(&all, b.mirror, delta);
            }
            SHOW_AIR => b.show_air = !b.show_air,
            SHOW_BOX => b.show_bounding_box = !b.show_bounding_box,
            ROTATION..=14 => {
                if let Some(r) = structure_block::rotation_from_ordinal(i32::from(id - ROTATION)) {
                    b.rotation = r;
                }
            }
            _ => {}
        }
        Outcome::Stay
    }

    fn done(&mut self, entity: &mut BlockEntity, pos: Pos) -> Outcome {
        match entity {
            BlockEntity::StructureBlock(b) => {
                Outcome::Send(self.packet(b, pos, UpdateType::UpdateData))
            }
            _ => Outcome::Cancel,
        }
    }

    fn cancel(&mut self, entity: &mut BlockEntity) {
        if let BlockEntity::StructureBlock(b) = entity {
            let i = &self.initial;
            b.mirror = i.mirror;
            b.rotation = i.rotation;
            b.mode = i.mode;
            b.ignore_entities = i.ignore_entities;
            b.strict = i.strict;
            b.show_air = i.show_air;
            b.show_bounding_box = i.show_bounding_box;
        }
    }

    fn accepts(&self, edit: usize, current: &str, cursor: usize, c: char) -> bool {
        edit != NAME || valid_character_for_name(current, c, cursor)
    }
}

impl Game<'_> {
    /// `LocalPlayer.openStructureBlock`.
    pub(super) fn open_structure_block(&mut self, pos: Pos) {
        let Some(BlockEntity::StructureBlock(b)) = self.world.block_entities.get(&pos) else {
            return;
        };
        let editor = StructureEditor::new(b);
        self.open_block_editor(pos, Box::new(editor));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integrity_formats_like_decimal_format() {
        assert_eq!(format_integrity(1.0), "1.0");
        assert_eq!(format_integrity(0.25), "0.25");
        assert_eq!(format_integrity(0.5), "0.5");
        assert_eq!(format_integrity(0.1), "0.1");
        assert_eq!(format_integrity(0.123_456), "0.1235");
    }

    #[test]
    fn done_sends_the_screens_values() {
        let mut block = StructureBlock::new(StructureMode::Save);
        block.set_structure_name("house");
        block.size = [3, 4, 5];
        let mut editor = StructureEditor::new(&block);
        editor.edits[POS].set_value("-2");
        editor.edits[SIZE].set_value("junk");
        editor.edits[INTEGRITY].set_value("0.5");
        let mut entity = BlockEntity::StructureBlock(Box::new(block));
        let Outcome::Send(S::SetStructureBlock(p)) = editor.done(&mut entity, (1, 2, 3)) else {
            panic!("done sends")
        };
        assert_eq!(p.pos, [1, 2, 3]);
        assert_eq!(p.update_type, UpdateType::UpdateData.ordinal());
        assert_eq!(p.name, "minecraft:house");
        // An unparsable coordinate is 0, as `parseCoordinate`.
        assert_eq!((p.offset, p.size), ([-2, 1, 0], [0, 4, 5]));
        assert_eq!(p.integrity, 0.5);
    }

    #[test]
    fn cancel_puts_back_what_the_buttons_changed() {
        let block = StructureBlock::new(StructureMode::Load);
        let mut editor = StructureEditor::new(&block);
        let mut entity = BlockEntity::StructureBlock(Box::new(block));
        editor.press(&mut entity, (0, 0, 0), ROTATION + 2, false, false);
        editor.press(&mut entity, (0, 0, 0), STRICT, false, false);
        editor.press(&mut entity, (0, 0, 0), MODE, false, false);
        let BlockEntity::StructureBlock(b) = &entity else {
            unreachable!()
        };
        assert_eq!(b.rotation, Rotation::Clockwise180);
        assert!(b.strict);
        assert_eq!(b.mode, StructureMode::Corner);
        editor.cancel(&mut entity);
        let BlockEntity::StructureBlock(b) = &entity else {
            unreachable!()
        };
        assert_eq!(
            (b.rotation, b.strict, b.mode),
            (Rotation::None, false, StructureMode::Load)
        );
    }

    #[test]
    fn the_mode_button_skips_data_unless_alt_is_held() {
        let block = StructureBlock::new(StructureMode::Corner);
        let mut editor = StructureEditor::new(&block);
        let mut entity = BlockEntity::StructureBlock(Box::new(block));
        editor.press(&mut entity, (0, 0, 0), MODE, false, false);
        let BlockEntity::StructureBlock(b) = &entity else {
            unreachable!()
        };
        assert_eq!(b.mode, StructureMode::Save);
        b_mode(&mut entity, StructureMode::Corner);
        editor.press(&mut entity, (0, 0, 0), MODE, false, true);
        let BlockEntity::StructureBlock(b) = &entity else {
            unreachable!()
        };
        assert_eq!(b.mode, StructureMode::Data);
    }

    fn b_mode(entity: &mut BlockEntity, mode: StructureMode) {
        if let BlockEntity::StructureBlock(b) = entity {
            b.mode = mode;
        }
    }
}
