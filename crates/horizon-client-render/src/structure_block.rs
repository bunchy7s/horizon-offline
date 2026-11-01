//! `StructureBlockEntity`: the structure block's name, offset, size, mode and
//! options, with its save format (`saveAdditional` / `loadAdditional`), the
//! corner scan (`getEnclosingBoundingBox`) and the box the editor shows
//! (`getRenderableBox`).
use horizon_client_core::nbt_value::{float, number, string, text};
use horizon_world::{
    nbt::Tag,
    worldgen::structure::{Mirror, Rotation},
};

pub const MAX_OFFSET_PER_AXIS: i32 = 48;
pub const MAX_SIZE_PER_AXIS: i32 = 48;
/// `SCAN_CORNER_BLOCKS_RANGE`.
pub const SCAN_CORNER_BLOCKS_RANGE: i32 = 80;

/// `StructureMode`, in declaration order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StructureMode {
    Save,
    Load,
    Corner,
    Data,
}

impl StructureMode {
    pub const ALL: [Self; 4] = [Self::Save, Self::Load, Self::Corner, Self::Data];
    /// `StructureMode.getSerializedName`: the block state's `mode` value.
    pub fn serialized_name(self) -> &'static str {
        match self {
            Self::Save => "save",
            Self::Load => "load",
            Self::Corner => "corner",
            Self::Data => "data",
        }
    }
    pub fn of_serialized_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.serialized_name() == name)
    }
    /// `StructureMode.LEGACY_CODEC`: `Enum.name()`.
    pub fn legacy_name(self) -> &'static str {
        match self {
            Self::Save => "SAVE",
            Self::Load => "LOAD",
            Self::Corner => "CORNER",
            Self::Data => "DATA",
        }
    }
    pub fn from_legacy_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.legacy_name() == name)
    }
    pub fn ordinal(self) -> i32 {
        self as i32
    }
    pub fn from_ordinal(ordinal: i32) -> Option<Self> {
        usize::try_from(ordinal)
            .ok()
            .and_then(|i| Self::ALL.get(i).copied())
    }
    /// `StructureMode.getDisplayName`'s lang key.
    pub fn info_key(self) -> String {
        format!("structure_block.mode_info.{}", self.serialized_name())
    }
    /// The mode button's label (`structure_block.mode.<name>`).
    pub fn label_key(self) -> String {
        format!("structure_block.mode.{}", self.serialized_name())
    }
}

/// `StructureBlockEntity.UpdateType`, in declaration order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateType {
    UpdateData,
    SaveArea,
    LoadArea,
    ScanArea,
}

impl UpdateType {
    pub fn ordinal(self) -> i32 {
        self as i32
    }
    pub fn from_ordinal(ordinal: i32) -> Option<Self> {
        Some(match ordinal {
            0 => Self::UpdateData,
            1 => Self::SaveArea,
            2 => Self::LoadArea,
            3 => Self::ScanArea,
            _ => return None,
        })
    }
}

pub fn rotation_from_ordinal(ordinal: i32) -> Option<Rotation> {
    usize::try_from(ordinal)
        .ok()
        .and_then(|i| Rotation::ALL.get(i).copied())
}
pub fn rotation_ordinal(rotation: Rotation) -> i32 {
    rotation as i32
}
pub fn rotation_from_legacy_name(name: &str) -> Option<Rotation> {
    Rotation::ALL.into_iter().find(|r| r.saved_name() == name)
}
pub fn mirror_from_ordinal(ordinal: i32) -> Option<Mirror> {
    Some(match ordinal {
        0 => Mirror::None,
        1 => Mirror::LeftRight,
        2 => Mirror::FrontBack,
        _ => return None,
    })
}
pub fn mirror_ordinal(mirror: Mirror) -> i32 {
    mirror as i32
}
pub fn mirror_from_legacy_name(name: &str) -> Option<Mirror> {
    [Mirror::None, Mirror::LeftRight, Mirror::FrontBack]
        .into_iter()
        .find(|m| m.saved_name() == name)
}

/// `BoundingBoxRenderable.Mode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderMode {
    None,
    Box,
    BoxAndInvisibleBlocks,
}

/// `BoundingBoxRenderable.RenderableBox`: an offset from the block and a
/// size, in blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderableBox {
    pub offset: [i32; 3],
    pub size: [i32; 3],
}

impl RenderableBox {
    /// `RenderableBox.fromCorners`.
    pub fn from_corners(x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32) -> Self {
        let (min_x, min_y, min_z) = (x0.min(x1), y0.min(y1), z0.min(z1));
        Self {
            offset: [min_x, min_y, min_z],
            size: [x0.max(x1) - min_x, y0.max(y1) - min_y, z0.max(z1) - min_z],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StructureBlock {
    /// The structure's id; an unparsable or empty name is none.
    pub name: Option<String>,
    pub author: String,
    pub metadata: String,
    pub pos: [i32; 3],
    pub size: [i32; 3],
    pub mirror: Mirror,
    pub rotation: Rotation,
    pub mode: StructureMode,
    pub ignore_entities: bool,
    pub strict: bool,
    pub powered: bool,
    pub show_air: bool,
    pub show_bounding_box: bool,
    pub integrity: f32,
    pub seed: i64,
}

impl StructureBlock {
    /// `new StructureBlockEntity(pos, state)`: the mode is the state's.
    pub fn new(mode: StructureMode) -> Self {
        Self {
            name: None,
            author: String::new(),
            metadata: String::new(),
            pos: [0, 1, 0],
            size: [0; 3],
            mirror: Mirror::None,
            rotation: Rotation::None,
            mode,
            ignore_entities: true,
            strict: false,
            powered: false,
            show_air: false,
            show_bounding_box: true,
            integrity: 1.0,
            seed: 0,
        }
    }

    /// `StructureBlockEntity.getStructureName`.
    pub fn structure_name(&self) -> &str {
        self.name.as_deref().unwrap_or("")
    }

    /// `setStructureName(String)`: empty or unparsable is none (`Identifier.tryParse`).
    pub fn set_structure_name(&mut self, name: &str) {
        self.name = if name.is_empty() {
            None
        } else {
            parse_identifier(name)
        };
    }

    /// `StructureBlockEntity.saveAdditional`.
    pub fn save(&self) -> Vec<(&'static str, Tag)> {
        let byte = |b: bool| Tag::Byte(i8::from(b));
        vec![
            ("name", string(self.structure_name())),
            ("author", string(&self.author)),
            ("metadata", string(&self.metadata)),
            ("posX", Tag::Int(self.pos[0])),
            ("posY", Tag::Int(self.pos[1])),
            ("posZ", Tag::Int(self.pos[2])),
            ("sizeX", Tag::Int(self.size[0])),
            ("sizeY", Tag::Int(self.size[1])),
            ("sizeZ", Tag::Int(self.size[2])),
            ("rotation", string(self.rotation.saved_name())),
            ("mirror", string(self.mirror.saved_name())),
            ("mode", string(self.mode.legacy_name())),
            ("ignoreEntities", byte(self.ignore_entities)),
            ("strict", byte(self.strict)),
            ("powered", byte(self.powered)),
            ("showair", byte(self.show_air)),
            ("showboundingbox", byte(self.show_bounding_box)),
            ("integrity", Tag::Float(self.integrity.to_bits())),
            ("seed", Tag::Long(self.seed)),
        ]
    }

    /// `StructureBlockEntity.loadAdditional`.
    pub fn load(tag: &Tag) -> Self {
        let int = |key: &str, default: i32, lo: i32, hi: i32| {
            (number(tag.get(key)).unwrap_or(i64::from(default)) as i32).clamp(lo, hi)
        };
        let flag = |key: &str, default: bool| number(tag.get(key)).map_or(default, |v| v != 0);
        let o = MAX_OFFSET_PER_AXIS;
        let s = MAX_SIZE_PER_AXIS;
        let mut block = Self::new(
            text(tag.get("mode"))
                .and_then(|m| StructureMode::from_legacy_name(&m))
                .unwrap_or(StructureMode::Data),
        );
        block.set_structure_name(&text(tag.get("name")).unwrap_or_default());
        block.author = text(tag.get("author")).unwrap_or_default();
        block.metadata = text(tag.get("metadata")).unwrap_or_default();
        block.pos = [
            int("posX", 0, -o, o),
            int("posY", 1, -o, o),
            int("posZ", 0, -o, o),
        ];
        block.size = [
            int("sizeX", 0, 0, s),
            int("sizeY", 0, 0, s),
            int("sizeZ", 0, 0, s),
        ];
        block.rotation = text(tag.get("rotation"))
            .and_then(|r| rotation_from_legacy_name(&r))
            .unwrap_or(Rotation::None);
        block.mirror = text(tag.get("mirror"))
            .and_then(|m| mirror_from_legacy_name(&m))
            .unwrap_or(Mirror::None);
        block.ignore_entities = flag("ignoreEntities", true);
        block.strict = flag("strict", false);
        block.powered = flag("powered", false);
        block.show_air = flag("showair", false);
        block.show_bounding_box = flag("showboundingbox", true);
        block.integrity = float(tag.get("integrity")).map_or(1.0, |f| f as f32);
        block.seed = number(tag.get("seed")).unwrap_or(0);
        block
    }

    /// `StructureBlockEntity.renderMode`.
    pub fn render_mode(&self) -> RenderMode {
        if self.mode != StructureMode::Save && self.mode != StructureMode::Load {
            RenderMode::None
        } else if self.mode == StructureMode::Save && self.show_air {
            RenderMode::BoxAndInvisibleBlocks
        } else if self.mode != StructureMode::Save && !self.show_bounding_box {
            RenderMode::None
        } else {
            RenderMode::Box
        }
    }

    /// `StructureBlockEntity.getRenderableBox`.
    pub fn renderable_box(&self) -> RenderableBox {
        let [x_origin, y0, z_origin] = self.pos;
        let size = self.size;
        let y1 = y0 + size[1];
        let (x_diff, z_diff) = match self.mirror {
            Mirror::LeftRight => (size[0], -size[2]),
            Mirror::FrontBack => (-size[0], size[2]),
            Mirror::None => (size[0], size[2]),
        };
        let (x0, z0, x1, z1);
        match self.rotation {
            Rotation::Clockwise90 => {
                x0 = if z_diff < 0 { x_origin } else { x_origin + 1 };
                z0 = if x_diff < 0 { z_origin + 1 } else { z_origin };
                x1 = x0 - z_diff;
                z1 = z0 + x_diff;
            }
            Rotation::Clockwise180 => {
                x0 = if x_diff < 0 { x_origin } else { x_origin + 1 };
                z0 = if z_diff < 0 { z_origin } else { z_origin + 1 };
                x1 = x0 - x_diff;
                z1 = z0 - z_diff;
            }
            Rotation::Counterclockwise90 => {
                x0 = if z_diff < 0 { x_origin + 1 } else { x_origin };
                z0 = if x_diff < 0 { z_origin } else { z_origin + 1 };
                x1 = x0 + z_diff;
                z1 = z0 - x_diff;
            }
            Rotation::None => {
                x0 = if x_diff < 0 { x_origin + 1 } else { x_origin };
                z0 = if z_diff < 0 { z_origin + 1 } else { z_origin };
                x1 = x0 + x_diff;
                z1 = z0 + z_diff;
            }
        }
        RenderableBox::from_corners(x0, y0, z0, x1, y1, z1)
    }

    /// `StructureBlockEntity.getEnclosingBoundingBox` over the corner blocks
    /// of this structure's name found in range (`corners`: every position of
    /// a corner-mode structure block with the same name), with `at` the block
    /// asking. Returns the inclusive min and max, none without a corner.
    pub fn enclosing_box(corners: &[[i32; 3]], at: [i32; 3]) -> Option<([i32; 3], [i32; 3])> {
        let mut found: Option<([i32; 3], [i32; 3])> = None;
        for c in corners {
            found = Some(match found {
                None => (*c, *c),
                Some((min, max)) => (
                    [min[0].min(c[0]), min[1].min(c[1]), min[2].min(c[2])],
                    [max[0].max(c[0]), max[1].max(c[1]), max[2].max(c[2])],
                ),
            });
        }
        let (min, max) = found?;
        if corners.len() == 1 {
            return Some((
                [min[0].min(at[0]), min[1].min(at[1]), min[2].min(at[2])],
                [max[0].max(at[0]), max[1].max(at[1]), max[2].max(at[2])],
            ));
        }
        Some((min, max))
    }

    /// `detectSize`'s result for the box: the offset and size the block takes
    /// when every side is wider than the corner blocks themselves.
    pub fn detected(min: [i32; 3], max: [i32; 3], at: [i32; 3]) -> Option<([i32; 3], [i32; 3])> {
        let delta = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
        if delta.iter().all(|d| *d > 1) {
            Some((
                [min[0] - at[0] + 1, min[1] - at[1] + 1, min[2] - at[2] + 1],
                [delta[0] - 1, delta[1] - 1, delta[2] - 1],
            ))
        } else {
            None
        }
    }
}

/// `Identifier.tryParse`: `namespace:path` (the namespace defaults to
/// `minecraft`) with only legal characters, as its string form.
pub fn parse_identifier(text: &str) -> Option<String> {
    let (namespace, path) = text.split_once(':').unwrap_or(("minecraft", text));
    let namespace = if namespace.is_empty() {
        "minecraft"
    } else {
        namespace
    };
    let namespace_ok = namespace
        .chars()
        .all(|c| matches!(c, '_' | '-' | '.' | '0'..='9' | 'a'..='z'));
    let path_ok = !path.is_empty()
        && path
            .chars()
            .all(|c| matches!(c, '_' | '-' | '.' | '/' | '0'..='9' | 'a'..='z'));
    (namespace_ok && path_ok).then(|| format!("{namespace}:{path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_loads_like_structure_block_entity() {
        let mut block = StructureBlock::new(StructureMode::Save);
        block.set_structure_name("house");
        block.author = "levzzz".into();
        block.pos = [1, -2, 3];
        block.size = [4, 5, 6];
        block.rotation = Rotation::Clockwise90;
        block.mirror = Mirror::FrontBack;
        block.ignore_entities = false;
        block.strict = true;
        block.integrity = 0.25;
        block.seed = -9;
        let tag = Tag::compound(block.save());
        assert_eq!(text(tag.get("name")).as_deref(), Some("minecraft:house"));
        assert_eq!(text(tag.get("mode")).as_deref(), Some("SAVE"));
        assert_eq!(text(tag.get("rotation")).as_deref(), Some("CLOCKWISE_90"));
        assert_eq!(StructureBlock::load(&tag), block.clone());
    }

    #[test]
    fn load_clamps_and_defaults() {
        let tag = Tag::compound(vec![
            ("posX", Tag::Int(99)),
            ("posY", Tag::Int(-99)),
            ("sizeX", Tag::Int(-4)),
            ("sizeY", Tag::Int(100)),
            ("name", string("Bad Name")),
        ]);
        let block = StructureBlock::load(&tag);
        assert_eq!(block.pos, [48, -48, 0]);
        assert_eq!(block.size, [0, 48, 0]);
        assert_eq!(block.name, None);
        assert_eq!(block.mode, StructureMode::Data);
        assert!(block.ignore_entities && block.show_bounding_box && !block.strict);
        assert_eq!(block.integrity, 1.0);
        // Pos defaults to (0, 1, 0).
        assert_eq!(
            StructureBlock::load(&Tag::compound(Vec::<(&str, Tag)>::new())).pos,
            [0, 1, 0]
        );
    }

    #[test]
    fn render_mode_and_box_follow_get_renderable_box() {
        let mut block = StructureBlock::new(StructureMode::Save);
        block.pos = [0, 1, 0];
        block.size = [3, 2, 4];
        assert_eq!(block.render_mode(), RenderMode::Box);
        assert_eq!(
            block.renderable_box(),
            RenderableBox {
                offset: [0, 1, 0],
                size: [3, 2, 4]
            }
        );
        block.rotation = Rotation::Clockwise90;
        assert_eq!(
            block.renderable_box(),
            RenderableBox {
                offset: [-3, 1, 0],
                size: [4, 2, 3]
            }
        );
        block.rotation = Rotation::None;
        block.mirror = Mirror::LeftRight;
        assert_eq!(
            block.renderable_box(),
            RenderableBox {
                offset: [0, 1, -3],
                size: [3, 2, 4]
            }
        );
        block.show_air = true;
        assert_eq!(block.render_mode(), RenderMode::BoxAndInvisibleBlocks);
        block.mode = StructureMode::Load;
        block.show_bounding_box = false;
        assert_eq!(block.render_mode(), RenderMode::None);
        block.mode = StructureMode::Corner;
        assert_eq!(block.render_mode(), RenderMode::None);
    }

    #[test]
    fn corner_scan_matches_detect_size() {
        // Two corners: their box; one corner: the box with the asking block.
        let (min, max) = StructureBlock::enclosing_box(&[[0, 0, 0], [5, 4, 6]], [2, 0, 2]).unwrap();
        assert_eq!((min, max), ([0, 0, 0], [5, 4, 6]));
        assert_eq!(
            StructureBlock::detected(min, max, [2, 0, 2]),
            Some(([-1, 1, -1], [4, 3, 5]))
        );
        let one = StructureBlock::enclosing_box(&[[0, 0, 0]], [3, 3, 3]).unwrap();
        assert_eq!(one, ([0, 0, 0], [3, 3, 3]));
        assert_eq!(StructureBlock::enclosing_box(&[], [0, 0, 0]), None);
        // A box of one block thickness is no structure.
        assert_eq!(
            StructureBlock::detected([0, 0, 0], [1, 5, 5], [0, 0, 0]),
            None
        );
    }

    #[test]
    fn identifiers_parse_like_try_parse() {
        assert_eq!(parse_identifier("a/b").as_deref(), Some("minecraft:a/b"));
        assert_eq!(parse_identifier("my:house").as_deref(), Some("my:house"));
        assert_eq!(parse_identifier("Bad"), None);
        assert_eq!(parse_identifier("ns:"), None);
    }
}
