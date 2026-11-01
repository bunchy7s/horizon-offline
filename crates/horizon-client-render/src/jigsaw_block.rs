//! `JigsawBlockEntity`: the jigsaw block's name, target, pool, joint, final
//! state and priorities, with its save format (`saveAdditional` /
//! `loadAdditional`).
use crate::structure_block::parse_identifier;
use horizon_client_core::nbt_value::{number, string, text};
use horizon_world::nbt::Tag;

/// `JigsawBlockEntity.EMPTY_ID` and `Pools.EMPTY`.
pub const EMPTY_ID: &str = "minecraft:empty";

/// `JigsawBlockEntity.JointType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JointType {
    Rollable,
    Aligned,
}

impl JointType {
    pub const ALL: [Self; 2] = [Self::Rollable, Self::Aligned];
    pub fn serialized_name(self) -> &'static str {
        match self {
            Self::Rollable => "rollable",
            Self::Aligned => "aligned",
        }
    }
    pub fn of_serialized_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|j| j.serialized_name() == name)
    }
    /// `JointType.getTranslatedName`'s lang key.
    pub fn label_key(self) -> String {
        format!("jigsaw_block.joint.{}", self.serialized_name())
    }
    /// `StructureTemplate.getDefaultJointType`: the joint of a block whose
    /// front faces `front` (the first word of its `orientation` value).
    pub fn default_for_front(front: &str) -> Self {
        if is_vertical_front(front) {
            Self::Rollable
        } else {
            Self::Aligned
        }
    }
}

/// `JigsawBlock.getFrontFacing(state)` from the `orientation` value
/// (`down_east`, `north_up`...): its front is the part before the `_`.
pub fn front_of_orientation(orientation: &str) -> &str {
    orientation.split('_').next().unwrap_or(orientation)
}

pub fn is_vertical_front(front: &str) -> bool {
    matches!(front, "up" | "down")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JigsawBlock {
    pub name: String,
    pub target: String,
    pub pool: String,
    pub joint: JointType,
    pub final_state: String,
    pub placement_priority: i32,
    pub selection_priority: i32,
}

impl Default for JigsawBlock {
    fn default() -> Self {
        Self {
            name: EMPTY_ID.into(),
            target: EMPTY_ID.into(),
            pool: EMPTY_ID.into(),
            joint: JointType::Rollable,
            final_state: "minecraft:air".into(),
            placement_priority: 0,
            selection_priority: 0,
        }
    }
}

impl JigsawBlock {
    /// `saveAdditional` (also `getUpdateTag`, which is `saveCustomOnly`).
    pub fn save(&self) -> Vec<(&'static str, Tag)> {
        vec![
            ("name", string(&self.name)),
            ("target", string(&self.target)),
            ("pool", string(&self.pool)),
            ("final_state", string(&self.final_state)),
            ("joint", string(self.joint.serialized_name())),
            ("placement_priority", Tag::Int(self.placement_priority)),
            ("selection_priority", Tag::Int(self.selection_priority)),
        ]
    }

    /// `loadAdditional`. A missing joint takes `default_joint`
    /// (`StructureTemplate.getDefaultJointType` of the block's state).
    pub fn load(tag: &Tag, default_joint: JointType) -> Self {
        let id = |key: &str| {
            text(tag.get(key))
                .and_then(|s| parse_identifier(&s))
                .unwrap_or_else(|| EMPTY_ID.into())
        };
        Self {
            name: id("name"),
            target: id("target"),
            pool: id("pool"),
            final_state: text(tag.get("final_state")).unwrap_or_else(|| "minecraft:air".into()),
            joint: text(tag.get("joint"))
                .and_then(|j| JointType::of_serialized_name(&j))
                .unwrap_or(default_joint),
            placement_priority: number(tag.get("placement_priority")).unwrap_or(0) as i32,
            selection_priority: number(tag.get("selection_priority")).unwrap_or(0) as i32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_loads_like_jigsaw_block_entity() {
        let block = JigsawBlock {
            name: "minecraft:bottom".into(),
            target: "test:top".into(),
            pool: "minecraft:village/plains/houses".into(),
            joint: JointType::Aligned,
            final_state: "minecraft:stone[x=1]".into(),
            placement_priority: 4,
            selection_priority: -1,
        };
        let tag = Tag::compound(block.save());
        assert_eq!(text(tag.get("joint")).as_deref(), Some("aligned"));
        assert_eq!(JigsawBlock::load(&tag, JointType::Rollable), block);
    }

    #[test]
    fn load_defaults_like_vanilla() {
        let tag = Tag::compound(vec![
            ("name", string("Bad Name")),
            ("target", string("door")),
        ]);
        let block = JigsawBlock::load(&tag, JointType::Aligned);
        assert_eq!(block.name, EMPTY_ID);
        assert_eq!(block.target, "minecraft:door");
        assert_eq!(block.pool, EMPTY_ID);
        assert_eq!(block.final_state, "minecraft:air");
        assert_eq!(block.joint, JointType::Aligned);
        assert_eq!(JointType::default_for_front("up"), JointType::Rollable);
        assert_eq!(
            JointType::default_for_front(front_of_orientation("north_up")),
            JointType::Aligned
        );
    }
}
