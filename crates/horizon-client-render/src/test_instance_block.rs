//! `TestInstanceBlockEntity`: the test instance block's test, size, rotation
//! and status, its error markers, and the boxes derived from them.
use horizon_client_core::nbt_value::{number, string, text};
use horizon_world::{
    nbt::Tag,
    worldgen::structure::{BoundingBox, Rotation},
};

/// `STRUCTURE_OFFSET`: where the structure starts from the block.
pub const STRUCTURE_OFFSET: [i32; 3] = [0, 1, 1];

/// `TestInstanceBlockEntity.Status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Cleared,
    Running,
    Finished,
}

impl Status {
    pub fn serialized_name(self) -> &'static str {
        match self {
            Self::Cleared => "cleared",
            Self::Running => "running",
            Self::Finished => "finished",
        }
    }
    pub fn of_serialized_name(name: &str) -> Option<Self> {
        [Self::Cleared, Self::Running, Self::Finished]
            .into_iter()
            .find(|s| s.serialized_name() == name)
    }
    pub fn index(self) -> i32 {
        self as i32
    }
    /// `Status.byIndex`: out of range is CLEARED.
    pub fn by_index(index: i32) -> Self {
        match index {
            1 => Self::Running,
            2 => Self::Finished,
            _ => Self::Cleared,
        }
    }
}

/// `TestInstanceBlockEntity.Data`. Components are kept as the NBT they are
/// sent and saved as.
#[derive(Clone, Debug, PartialEq)]
pub struct Data {
    /// The test's registry id.
    pub test: Option<String>,
    pub size: [i32; 3],
    pub rotation: Rotation,
    pub ignore_entities: bool,
    pub status: Status,
    pub error_message: Option<Tag>,
}

impl Default for Data {
    fn default() -> Self {
        Self {
            test: None,
            size: [0; 3],
            rotation: Rotation::None,
            ignore_entities: false,
            status: Status::Cleared,
            error_message: None,
        }
    }
}

impl Data {
    /// `Data.CODEC` encoded.
    pub fn save(&self) -> Tag {
        let mut entries = Vec::new();
        if let Some(test) = &self.test {
            entries.push(("test", string(test)));
        }
        entries.push(("size", Tag::IntArray(self.size.to_vec())));
        entries.push(("rotation", string(self.rotation.serialized_name())));
        entries.push(("ignore_entities", Tag::Byte(i8::from(self.ignore_entities))));
        entries.push(("status", string(self.status.serialized_name())));
        if let Some(error) = &self.error_message {
            entries.push(("error_message", error.clone()));
        }
        Tag::compound(entries)
    }

    /// `Data.CODEC` decoded: size, rotation, ignore_entities and status are
    /// required, or the whole value is dropped.
    pub fn load(tag: &Tag) -> Option<Self> {
        let size = match tag.get("size")? {
            Tag::IntArray(v) if v.len() == 3 => [v[0], v[1], v[2]],
            _ => return None,
        };
        Some(Self {
            test: text(tag.get("test")),
            size,
            rotation: Rotation::from_name(&text(tag.get("rotation"))?)?,
            ignore_entities: number(tag.get("ignore_entities")).map(|v| v != 0)?,
            status: Status::of_serialized_name(&text(tag.get("status"))?)?,
            error_message: tag.get("error_message").cloned(),
        })
    }
}

/// `TestInstanceBlockEntity.ErrorMarker`.
#[derive(Clone, Debug, PartialEq)]
pub struct ErrorMarker {
    pub pos: [i32; 3],
    pub text: Tag,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TestInstanceBlock {
    pub data: Data,
    pub error_markers: Vec<ErrorMarker>,
}

impl TestInstanceBlock {
    /// `saveAdditional` (also `getUpdateTag`, which is `saveCustomOnly`).
    pub fn save(&self) -> Vec<(&'static str, Tag)> {
        let mut entries = vec![("data", self.data.save())];
        if !self.error_markers.is_empty() {
            let markers = self
                .error_markers
                .iter()
                .map(|m| {
                    Tag::compound([
                        ("pos", Tag::IntArray(m.pos.to_vec())),
                        ("text", m.text.clone()),
                    ])
                })
                .collect();
            entries.push(("errors", Tag::List(markers)));
        }
        entries
    }

    /// `loadAdditional`: the data is replaced only when it decodes, and the
    /// markers are always replaced.
    pub fn load(&mut self, tag: &Tag) {
        if let Some(data) = tag.get("data").and_then(Data::load) {
            self.data = data;
        }
        self.error_markers.clear();
        if let Some(Tag::List(markers)) = tag.get("errors") {
            for m in markers {
                if let (Some(Tag::IntArray(p)), Some(text)) = (m.get("pos"), m.get("text"))
                    && p.len() == 3
                {
                    self.error_markers.push(ErrorMarker {
                        pos: [p[0], p[1], p[2]],
                        text: text.clone(),
                    });
                }
            }
        }
    }

    /// `getStructurePos(blockPos)`.
    pub fn structure_pos_of(block: [i32; 3]) -> [i32; 3] {
        [0, 1, 2].map(|i| block[i] + STRUCTURE_OFFSET[i])
    }

    /// `getTransformedSize`: the size with the axes swapped for a quarter turn.
    pub fn transformed_size(size: [i32; 3], rotation: Rotation) -> [i32; 3] {
        if matches!(
            rotation,
            Rotation::Clockwise90 | Rotation::Counterclockwise90
        ) {
            [size[2], size[1], size[0]]
        } else {
            size
        }
    }

    /// `getRenderableBox`: the offset from the block and the size to draw,
    /// given the test's `padding` and rotation.
    pub fn renderable_box(&self, padding: i32, rotation: Rotation) -> ([i32; 3], [i32; 3]) {
        let offset = STRUCTURE_OFFSET.map(|o| o + padding);
        (offset, Self::transformed_size(self.data.size, rotation))
    }

    /// `getStructureBoundingBox`.
    pub fn structure_box(&self, block: [i32; 3], padding: i32, rotation: Rotation) -> BoundingBox {
        let corner = Self::structure_pos_of([0, 1, 2].map(|i| block[i] + padding));
        let size = Self::transformed_size(self.data.size, rotation);
        BoundingBox::from_corners(corner, [0, 1, 2].map(|i| corner[i] + size[i] - 1))
    }

    /// `getStartCorner`: the corner the template is placed from.
    pub fn start_corner(&self, block: [i32; 3], padding: i32, rotation: Rotation) -> [i32; 3] {
        let [x, _, z] = self.data.size;
        let c = Self::structure_pos_of([0, 1, 2].map(|i| block[i] + padding));
        match rotation {
            Rotation::None => c,
            Rotation::Clockwise90 => [c[0] + z - 1, c[1], c[2]],
            Rotation::Clockwise180 => [c[0] + x - 1, c[1], c[2] + z - 1],
            Rotation::Counterclockwise90 => [c[0], c[1], c[2] + x - 1],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> TestInstanceBlock {
        TestInstanceBlock {
            data: Data {
                test: Some("minecraft:always_pass".into()),
                size: [3, 4, 5],
                rotation: Rotation::Clockwise180,
                ignore_entities: true,
                status: Status::Finished,
                error_message: Some(string("boom")),
            },
            error_markers: vec![ErrorMarker {
                pos: [1, 2, 3],
                text: string("here"),
            }],
        }
    }

    #[test]
    fn saves_and_loads_like_test_instance_block_entity() {
        let block = sample();
        let tag = Tag::compound(block.save());
        assert_eq!(
            text(tag.get("data").unwrap().get("rotation")).as_deref(),
            Some("180")
        );
        let mut loaded = TestInstanceBlock::default();
        loaded.load(&tag);
        assert_eq!(loaded, block);
        // No markers, no `errors` key; a bad `data` leaves the old one.
        let mut plain = block.clone();
        plain.error_markers.clear();
        assert!(Tag::compound(plain.save()).get("errors").is_none());
        let mut kept = block;
        kept.load(&Tag::compound([(
            "data",
            Tag::compound(Vec::<(&str, Tag)>::new()),
        )]));
        assert_eq!(kept.data.test.as_deref(), Some("minecraft:always_pass"));
        assert!(kept.error_markers.is_empty());
    }

    #[test]
    fn boxes_follow_padding_and_rotation() {
        let block = sample();
        // A quarter turn swaps the horizontal sizes.
        let (offset, size) = block.renderable_box(2, Rotation::Clockwise90);
        assert_eq!((offset, size), ([2, 3, 3], [5, 4, 3]));
        let bounds = block.structure_box([10, 64, 10], 2, Rotation::None);
        assert_eq!(bounds.min, [12, 67, 13]);
        assert_eq!(bounds.max, [14, 70, 17]);
        assert_eq!(
            block.start_corner([10, 64, 10], 2, Rotation::Counterclockwise90),
            [12, 67, 15]
        );
        assert_eq!(Status::by_index(7), Status::Cleared);
    }
}
