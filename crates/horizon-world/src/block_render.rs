//! Neighbor face visibility over immutable block definitions and occlusion grids.
//! This decides whether a culled model face is emitted; it does not shade it or
//! implement translucent sorting, fluid meshing, or block updates.
use crate::{
    collision::CachedCollisionRegistry,
    compression::invalid,
    fluid::{FluidKind, FluidState},
    registry::BlockStateId,
};
use horizon_core::geometry::{Direction, ShapeOp, VoxelShape};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipRendering {
    Never,
    SameBlock,
    Bars,
    Leaves,
    Liquid,
    MangroveRoots,
}
#[derive(Clone, Copy, Debug)]
pub struct RenderDefinition {
    pub skip: SkipRendering,
    pub can_occlude: bool,
    pub bars_tag: bool,
}
#[derive(Clone, Copy)]
struct State {
    block: BlockStateId,
    faces: [u32; 6],
    definition: RenderDefinition,
    connections: u8,
    connection_properties: u8,
    fluid: FluidKind,
}
/// Prepared face geometry and a bounded cache shared across state pairs. Geometry
/// is deduplicated by the imported immutable shape, not copied for every state.
/// A clone (each meshing worker keeps one, with its own cache) shares the states
/// and faces, which never change once made.
#[derive(Clone)]
pub struct FaceVisibility {
    states: Arc<[State]>,
    faces: Arc<[VoxelShape]>,
    cache: HashMap<(u32, u32), bool>,
}
impl FaceVisibility {
    /// No state's faces, for a server, which draws nothing: every lookup
    /// answers "unknown render state".
    pub fn empty() -> Self {
        Self {
            states: Arc::new([]),
            faces: Arc::new([VoxelShape::empty(), VoxelShape::block()]),
            cache: HashMap::new(),
        }
    }
    /// [`Self::empty`]: no state is known.
    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }
    pub fn new(
        occlusion: &CachedCollisionRegistry<'_>,
        definitions: &BTreeMap<String, RenderDefinition>,
    ) -> io::Result<Self> {
        let registry = occlusion.registry();
        let mut states = Vec::with_capacity(registry.len());
        // IDs 0 and 1 represent the canonical empty/full face singletons used
        // by BlockState.initCache; other empty-like shapes retain their identity.
        let mut faces = vec![VoxelShape::empty(), VoxelShape::block()];
        let mut shape_faces = HashMap::new();
        for id in 0..registry.len() {
            let id = BlockStateId(id as u32);
            let state = registry
                .state(id)
                .ok_or_else(|| invalid("unknown render state"))?;
            let definition = *definitions
                .get(&state.name)
                .ok_or_else(|| invalid("missing render definition"))?;
            let shape = occlusion
                .cached_shape(id)?
                .ok_or_else(|| invalid("missing occlusion geometry"))?;
            let face_ids = if !definition.can_occlude || shape.is_empty() {
                [0; 6]
            } else if let Some(ids) = shape_faces.get(&std::ptr::from_ref(shape)) {
                *ids
            } else {
                let mut ids = [1; 6];
                if shape
                    .join_is_not_empty(&faces[1], ShapeOp::XOR)
                    .map_err(|e| invalid(e.to_string()))?
                {
                    for direction in Direction::ALL {
                        let face = shape
                            .face_shape(direction)
                            .map_err(|e| invalid(e.to_string()))?;
                        ids[direction as usize] = if face.same_identity(&faces[0]) {
                            0
                        } else if face.same_identity(&faces[1]) {
                            1
                        } else {
                            let id = faces.len() as u32;
                            faces.push(face);
                            id
                        };
                    }
                }
                shape_faces.insert(std::ptr::from_ref(shape), ids);
                ids
            };
            let mut connections = 0;
            let mut connection_properties = 0;
            for (name, direction) in [
                ("north", Direction::North),
                ("south", Direction::South),
                ("west", Direction::West),
                ("east", Direction::East),
            ] {
                if let Some(value @ ("false" | "true")) =
                    state.properties.get(name).map(String::as_str)
                {
                    connection_properties |= 1 << direction as u8;
                    if value == "true" {
                        connections |= 1 << direction as u8;
                    }
                }
            }
            if definition.skip == SkipRendering::Bars && connection_properties != 0b111100 {
                return Err(invalid("bars rendering requires four boolean connections"));
            }
            states.push(State {
                block: registry
                    .default_state(&state.name)
                    .ok_or_else(|| invalid("missing render block identity"))?,
                faces: face_ids,
                definition,
                connections,
                connection_properties,
                fluid: FluidState::from_block(state)?.kind(),
            });
        }
        Ok(Self {
            states: states.into(),
            faces: faces.into(),
            cache: HashMap::new(),
        })
    }
    fn state(&self, id: BlockStateId) -> io::Result<State> {
        self.states
            .get(id.0 as usize)
            .copied()
            .ok_or_else(|| invalid("unknown render state ID"))
    }
    fn skips(a: State, b: State, direction: Direction, cutout_leaves: bool) -> bool {
        let vertical = matches!(direction, Direction::Down | Direction::Up);
        let bit = 1 << direction as u8;
        let opposite = 1 << direction.opposite() as u8;
        match a.definition.skip {
            SkipRendering::Never => false,
            SkipRendering::SameBlock => a.block == b.block,
            SkipRendering::Leaves => !cutout_leaves && b.definition.skip == SkipRendering::Leaves,
            SkipRendering::Liquid => a.fluid.same_family(b.fluid),
            SkipRendering::MangroveRoots => a.block == b.block && vertical,
            SkipRendering::Bars => {
                (a.block == b.block
                    || (a.definition.bars_tag
                        && b.definition.bars_tag
                        && b.connection_properties & opposite != 0))
                    && (vertical || (a.connections & bit != 0 && b.connections & opposite != 0))
            }
        }
    }
    /// The state's occlusion face toward `direction` (`getFaceOcclusionShape`) and
    /// its shared ID: 0 is the empty singleton, 1 the full-block singleton.
    pub fn occlusion_face(
        &self,
        state: BlockStateId,
        direction: Direction,
    ) -> Option<(u32, &VoxelShape)> {
        let id = self.states.get(state.0 as usize)?.faces[direction as usize];
        Some((id, &self.faces[id as usize]))
    }
    pub fn skip_rendering(
        &self,
        state: BlockStateId,
        neighbor: BlockStateId,
        direction: Direction,
        cutout_leaves: bool,
    ) -> io::Result<bool> {
        Ok(Self::skips(
            self.state(state)?,
            self.state(neighbor)?,
            direction,
            cutout_leaves,
        ))
    }
    pub fn should_render_face(
        &mut self,
        state: BlockStateId,
        neighbor: BlockStateId,
        direction: Direction,
        cutout_leaves: bool,
    ) -> io::Result<bool> {
        let a = self.state(state)?;
        let b = self.state(neighbor)?;
        let neighbor_face = b.faces[direction.opposite() as usize];
        if neighbor_face == 1 || Self::skips(a, b, direction, cutout_leaves) {
            return Ok(false);
        }
        let own_face = a.faces[direction as usize];
        if neighbor_face == 0 || own_face == 0 {
            return Ok(true);
        }
        let key = (own_face, neighbor_face);
        if let Some(value) = self.cache.get(&key) {
            return Ok(*value);
        }
        let visible = self.faces[own_face as usize]
            .join_is_not_empty(&self.faces[neighbor_face as usize], ShapeOp::ONLY_FIRST)
            .map_err(|e| invalid(e.to_string()))?;
        // Cache policy affects speed only. Keep bounded memory while retaining
        // more shared face pairs than the native 256-entry per-thread cache.
        if self.cache.len() == 4096 {
            self.cache.clear();
        }
        self.cache.insert(key, visible);
        Ok(visible)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A server's empty visibility knows no state: lookups fail instead of
    /// panicking.
    #[test]
    fn empty_visibility_knows_no_state() {
        let mut visibility = FaceVisibility::empty();
        assert!(visibility.is_empty());
        let stone = BlockStateId(1);
        assert!(visibility.occlusion_face(stone, Direction::Up).is_none());
        assert!(
            visibility
                .skip_rendering(stone, stone, Direction::Up, true)
                .is_err()
        );
        assert!(
            visibility
                .should_render_face(stone, stone, Direction::Up, true)
                .is_err()
        );
    }
}
