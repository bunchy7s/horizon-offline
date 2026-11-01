//! Version-pinned cached block collision data, without entity-context behavior.
//!
//! Java's no-context BlockState.getCollisionShape uses its cache when present.
//! The overload taking CollisionContext always calls block behavior instead.
//! This module covers the cached branch only; uncached states remain explicit.
use crate::{
    DATA_VERSION,
    chunk::BlockSection,
    compression::invalid,
    nbt::Tag,
    registry::{BlockRegistry, BlockStateId, BlockTable},
};
use horizon_core::geometry::{Axis, VoxelShape};
use std::{collections::HashMap, io};

/// `PipeBlock`'s shape for `size` pixels across (`makeShapes`): the core
/// cube, with an arm from the centre out to each joined side (`sides` by
/// `Direction`: down, up, north, south, west, east).
pub fn pipe_shape(size: f64, sides: [bool; 6]) -> io::Result<VoxelShape> {
    use horizon_core::geometry::{Aabb, ShapeOp, Vec3};
    let (lo, hi) = ((8.0 - size / 2.0) / 16.0, (8.0 + size / 2.0) / 16.0);
    let shape = |min: [f64; 3], max: [f64; 3]| {
        VoxelShape::from_box(Aabb::new(
            Vec3::new(min[0], min[1], min[2]),
            Vec3::new(max[0], max[1], max[2]),
        ))
        .map_err(|e| invalid(e.to_string()))
    };
    let mut result = shape([lo; 3], [hi; 3])?;
    let arms = [
        ([lo, 0.0, lo], [hi, 0.5, hi]),
        ([lo, 0.5, lo], [hi, 1.0, hi]),
        ([lo, lo, 0.0], [hi, hi, 0.5]),
        ([lo, lo, 0.5], [hi, hi, 1.0]),
        ([0.0, lo, lo], [0.5, hi, hi]),
        ([0.5, lo, lo], [1.0, hi, hi]),
    ];
    for (joined, (min, max)) in sides.into_iter().zip(arms) {
        if joined {
            result = shape(min, max)?
                .join(&result, ShapeOp::OR)
                .map_err(|e| invalid(e.to_string()))?;
        }
    }
    Ok(result)
}

#[derive(PartialEq, Eq, Hash)]
struct GridKey {
    coords: [Vec<u64>; 3],
    occupied: Vec<u8>,
    cube: bool,
    axes: u8,
    min: [i32; 3],
    max: [i32; 3],
}

pub struct CachedCollisionRegistry<'r> {
    registry: &'r BlockRegistry,
    shapes: Vec<VoxelShape>,
    state_shapes: Vec<i32>,
    grid_count: usize,
    canonical_identities: bool,
}
impl<'r> CachedCollisionRegistry<'r> {
    pub fn registry(&self) -> &'r BlockRegistry {
        self.registry
    }
    pub fn from_nbt(tag: &Tag, registry: &'r BlockRegistry) -> io::Result<Self> {
        Self::from_table(BlockTable::check(tag, registry, "collision")?)
    }
    pub fn from_table(table: BlockTable<'_, 'r>) -> io::Result<Self> {
        let (tag, registry) = (table.tag(), table.registry());
        if tag.get("DataVersion") != Some(&Tag::Int(DATA_VERSION)) {
            return Err(invalid("collision DataVersion mismatch"));
        }
        let Some(Tag::List(entries)) = tag.get("shapes") else {
            return Err(invalid("missing collision shapes"));
        };
        if entries.len() > registry.len() {
            return Err(invalid("too many cached collision shapes"));
        }
        let mut shapes = Vec::with_capacity(entries.len());
        let mut canonical_identities = true;
        let mut seen_canonical = [false; 3];
        let mut grids = HashMap::<GridKey, VoxelShape>::new();
        for entry in entries {
            let canonical = match entry.get("canonical") {
                None => {
                    canonical_identities = false;
                    0
                }
                Some(Tag::Byte(value @ 0..=2)) => *value as usize,
                _ => return Err(invalid("invalid canonical collision identity")),
            };
            if canonical != 0 {
                if seen_canonical[canonical] {
                    return Err(invalid("duplicate canonical collision identity"));
                }
                seen_canonical[canonical] = true;
            }
            let mut coords: [Vec<f64>; 3] = std::array::from_fn(|_| Vec::new());
            for (i, axis) in ["x", "y", "z"].into_iter().enumerate() {
                let Some(Tag::LongArray(bits)) = entry.get(axis) else {
                    return Err(invalid("missing collision coordinates"));
                };
                coords[i] = bits.iter().map(|b| f64::from_bits(*b as u64)).collect();
            }
            let Some(Tag::ByteArray(bits)) = entry.get("occupied") else {
                return Err(invalid("missing collision occupancy"));
            };
            if bits.iter().any(|b| *b != 0 && *b != 1) {
                return Err(invalid("invalid collision occupancy"));
            }
            let occupied = bits.iter().map(|b| *b != 0).collect::<Vec<_>>();
            let Some(Tag::Byte(cube_axes)) = entry.get("cube_axes") else {
                return Err(invalid("missing cube-coordinate metadata"));
            };
            if *cube_axes < 0 || *cube_axes > 7 {
                return Err(invalid("invalid cube-coordinate metadata"));
            }
            let coordinate_cubes = [cube_axes & 1 != 0, cube_axes & 2 != 0, cube_axes & 4 != 0];
            let discrete_bounds = |name| -> io::Result<[i32; 3]> {
                let Some(Tag::IntArray(values)) = entry.get(name) else {
                    return Err(invalid("missing collision discrete extrema"));
                };
                values
                    .as_slice()
                    .try_into()
                    .map_err(|_| invalid("invalid collision discrete extrema length"))
            };
            let min = discrete_bounds("discrete_min")?;
            let max = discrete_bounds("discrete_max")?;
            let cube = match entry.get("cube") {
                Some(Tag::Byte(0)) => false,
                Some(Tag::Byte(1)) => true,
                _ => return Err(invalid("invalid collision cube flag")),
            };
            let key = GridKey {
                coords: std::array::from_fn(|i| coords[i].iter().map(|v| v.to_bits()).collect()),
                occupied: bits.clone(),
                cube,
                axes: *cube_axes as u8,
                min,
                max,
            };
            let shape = if let Some(existing) = grids.get(&key) {
                existing.fresh_identity()
            } else {
                match entry.get("cube") {
                    Some(Tag::Byte(0)) => {
                        VoxelShape::from_mixed_grid(coords, &occupied, coordinate_cubes)
                    }
                    Some(Tag::Byte(1)) => {
                        if *cube_axes != 7 {
                            return Err(invalid("cube shape has array-coordinate metadata"));
                        }
                        let sizes = std::array::from_fn(|i| coords[i].len().saturating_sub(1));
                        let shape = VoxelShape::from_cube(sizes, &occupied)
                            .map_err(|e| invalid(e.to_string()))?;
                        for axis in Axis::ALL {
                            if shape
                                .coordinates(axis)
                                .iter()
                                .map(|v| v.to_bits())
                                .ne(coords[axis as usize].iter().map(|v| v.to_bits()))
                            {
                                return Err(invalid("cube collision coordinates differ"));
                            }
                        }
                        Ok(shape)
                    }
                    _ => return Err(invalid("invalid collision cube flag")),
                }
                .map_err(|e| invalid(e.to_string()))?
                .with_discrete_bounds(min, max)
                .map_err(|e| invalid(e.to_string()))?
            };
            let shape = if canonical == 0 {
                shape
            } else {
                let reference = if canonical == 1 {
                    VoxelShape::empty()
                } else {
                    VoxelShape::block()
                };
                let expected_count = usize::from(canonical == 2);
                if cube != reference.is_cube()
                    || coordinate_cubes != reference.cube_coordinate_axes()
                    || occupied != vec![true; expected_count]
                    || min != [0; 3]
                    || max != [expected_count as i32; 3]
                    || Axis::ALL.into_iter().any(|axis| {
                        key.coords[axis as usize]
                            .iter()
                            .copied()
                            .ne(reference.coordinates(axis).iter().map(|v| v.to_bits()))
                    })
                {
                    return Err(invalid(
                        "canonical collision identity has different geometry",
                    ));
                }
                reference
            };
            grids.entry(key).or_insert_with(|| shape.clone());
            shapes.push(shape);
        }
        let Some(Tag::IntArray(state_shapes)) = tag.get("cached_shapes") else {
            return Err(invalid("missing cached shape mapping"));
        };
        if state_shapes.len() != registry.len()
            || state_shapes
                .iter()
                .any(|id| *id < -1 || *id >= shapes.len() as i32)
        {
            return Err(invalid("invalid cached shape mapping"));
        }
        Ok(Self {
            registry,
            shapes,
            state_shapes: state_shapes.clone(),
            grid_count: grids.len(),
            canonical_identities,
        })
    }
    /// Older exports omit native canonical identities. Such imports retain their
    /// local alias graph but cannot support identity-dependent world iteration.
    pub fn has_canonical_identities(&self) -> bool {
        self.canonical_identities
    }
    pub fn shape_count(&self) -> usize {
        self.shapes.len()
    }
    pub fn unique_grid_count(&self) -> usize {
        self.grid_count
    }
    pub fn cached_state_count(&self) -> usize {
        self.state_shapes.iter().filter(|id| **id >= 0).count()
    }
    /// None means vanilla must call block behavior for this state. It does not
    /// mean air, an empty shape, or permission to ignore a collision.
    pub fn cached_shape(&self, state: BlockStateId) -> io::Result<Option<&VoxelShape>> {
        let id = *self
            .state_shapes
            .get(state.0 as usize)
            .ok_or_else(|| invalid("collision state outside registry"))?;
        Ok(if id < 0 {
            None
        } else {
            Some(&self.shapes[id as usize])
        })
    }
    pub fn section_shape(
        &self,
        section: &BlockSection<'_>,
        index: usize,
    ) -> io::Result<Option<&VoxelShape>> {
        if !std::ptr::eq(section.registry(), self.registry) {
            return Err(invalid("section uses a different collision registry"));
        }
        self.cached_shape(section.get(index)?)
    }
}

#[cfg(test)]
mod pipe_tests {
    use super::*;
    use crate::registry::BlockTableFile;
    use horizon_core::geometry::ShapeOp;
    use std::path::PathBuf;

    /// `pipe_shape` at the chorus plant's 10 pixels is vanilla's own shape for
    /// each of its 64 states.
    #[test]
    fn pipe_shapes_match_the_chorus_plant() {
        let root = std::env::var_os("HORIZON_RESOURCES")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/horizon-client")
                    .join(horizon_core::VANILLA_VERSION)
            });
        if !root.join("cached-collision.nbt").exists() {
            eprintln!("skipping: no resources at {}", root.display());
            return;
        }
        let registry = crate::level::data::load_registry(&root).unwrap();
        let file = BlockTableFile::open(&root.join("cached-collision.nbt"), &registry).unwrap();
        let cached = CachedCollisionRegistry::from_table(file.table("collision").unwrap()).unwrap();
        let mut checked = 0;
        for id in 0..registry.len() as u32 {
            let state = BlockStateId(id);
            let s = registry.state(state).unwrap();
            if s.name != "minecraft:chorus_plant" {
                continue;
            }
            let sides = ["down", "up", "north", "south", "west", "east"]
                .map(|k| s.properties.get(k).map(String::as_str) == Some("true"));
            let ours = pipe_shape(10.0, sides).unwrap();
            let theirs = cached.cached_shape(state).unwrap().unwrap();
            assert!(
                !ours.join_is_not_empty(theirs, ShapeOp::XOR).unwrap(),
                "{sides:?}"
            );
            assert_eq!(ours.boxes(), theirs.boxes(), "{sides:?}");
            checked += 1;
        }
        assert_eq!(checked, 64);
    }
}
