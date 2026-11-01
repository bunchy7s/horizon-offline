use super::{ShapeError, VoxelShape, volume};
use crate::geometry::{Aabb, Axis, COLLISION_EPSILON, Direction, Vec3};

impl VoxelShape {
    /// Native face selection samples just inside the unit-block boundary. The
    /// result preserves identity on empty/cube-like axes and caches other faces.
    pub fn face_shape(&self, direction: Direction) -> Result<Self, ShapeError> {
        let (axis, _) = face_axis(direction);
        if self.is_empty() || self.cube_like_along(axis) {
            return Ok(self.clone());
        }
        self.identity
            .faces
            .get_or_init(|| Box::new(std::array::from_fn(|_| std::sync::OnceLock::new())))
            [direction as usize]
            .get_or_init(|| self.calculate_face(direction))
            .clone()
    }
    /// [`Self::face_shape`] that keeps no face it had to calculate on this
    /// shape: for a one-off pass over a whole table at load, whose faces
    /// nothing asks for again (kept, they held tens of MB).
    pub fn face_shape_transient(&self, direction: Direction) -> Result<Self, ShapeError> {
        let (axis, _) = face_axis(direction);
        if self.is_empty() || self.cube_like_along(axis) {
            return Ok(self.clone());
        }
        match self
            .identity
            .faces
            .get()
            .and_then(|faces| faces[direction as usize].get())
        {
            Some(face) => face.clone(),
            None => self.calculate_face(direction),
        }
    }
    fn cube_like_along(&self, axis: Axis) -> bool {
        let coords = self.coordinates(axis);
        coords.len() == 2
            && coords[0].abs() <= COLLISION_EPSILON
            && (coords[1] - 1.0).abs() <= COLLISION_EPSILON
    }
    fn calculate_face(&self, direction: Direction) -> Result<Self, ShapeError> {
        let (axis, positive) = face_axis(direction);
        let a = axis as usize;
        let index = self.find_index(
            axis,
            if positive {
                0.9999999
            } else {
                COLLISION_EPSILON
            },
        );
        let mut starts = [0; 3];
        starts[a] = index;
        let mut ends = self.sizes.map(|v| v as i32);
        ends[a] = index + 1;
        // SubShape extrema clamp the parent bounds; they are not recomputed
        // from the selected occupancy. Sparse slices can therefore be nonempty
        // by native metadata even when their decomposition contains no boxes.
        let min =
            std::array::from_fn(|i| self.discrete_min[i].clamp(starts[i], ends[i]) - starts[i]);
        let max =
            std::array::from_fn(|i| self.discrete_max[i].clamp(starts[i], ends[i]) - starts[i]);
        if (0..3).any(|i| min[i] >= max[i]) {
            return Ok(Self::empty());
        }
        let mut coords = (*self.coords).clone();
        coords[a] = vec![0.0, 1.0];
        if (0..3).all(|i| {
            coords[i].len() == 2
                && coords[i][0].abs() <= COLLISION_EPSILON
                && (coords[i][1] - 1.0).abs() <= COLLISION_EPSILON
        }) {
            return Ok(Self::block());
        }
        let sizes = std::array::from_fn(|i| coords[i].len() - 1);
        let mut occupied = Vec::with_capacity(volume(sizes)?);
        for x in 0..sizes[0] {
            for y in 0..sizes[1] {
                for z in 0..sizes[2] {
                    let mut p = [x as i32, y as i32, z as i32];
                    p[a] = index;
                    occupied.push(self.is_full(p[0], p[1], p[2]));
                }
            }
        }
        let mut cubes = self.coordinate_cubes;
        cubes[a] = true;
        let mut result = Self::build(coords, &occupied, false, cubes)?;
        result.discrete_min = min;
        result.discrete_max = max;
        result.bounds = Some(Aabb::new(
            Vec3::new(
                result.min(Axis::X),
                result.min(Axis::Y),
                result.min(Axis::Z),
            ),
            Vec3::new(
                result.max(Axis::X),
                result.max(Axis::Y),
                result.max(Axis::Z),
            ),
        ));
        Ok(result)
    }
}
fn face_axis(direction: Direction) -> (Axis, bool) {
    match direction {
        Direction::Down => (Axis::Y, false),
        Direction::Up => (Axis::Y, true),
        Direction::North => (Axis::Z, false),
        Direction::South => (Axis::Z, true),
        Direction::West => (Axis::X, false),
        Direction::East => (Axis::X, true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    #[test]
    fn face_cache_is_lazy_shared_and_does_not_retain_parent() {
        let shape = VoxelShape::from_cube(
            [2, 2, 2],
            &[true, false, false, false, false, false, false, false],
        )
        .unwrap();
        assert!(shape.identity.faces.get().is_none());
        let weak = Arc::downgrade(&shape.identity);
        let face = shape.face_shape(Direction::Down).unwrap();
        assert!(!face.same_identity(&shape));
        assert!(face.same_identity(&shape.clone().face_shape(Direction::Down).unwrap()));
        assert!(shape.fresh_identity().identity.faces.get().is_none());
        assert!(shape.moved(Vec3::ZERO).identity.faces.get().is_none());
        drop(shape);
        assert!(weak.upgrade().is_none());
        assert!(!face.is_empty());
    }

    /// A transient face is the cached one's geometry, keeps nothing, and
    /// returns the cached face once there is one.
    #[test]
    fn transient_faces_are_not_kept() {
        let shape = VoxelShape::from_cube(
            [2, 2, 2],
            &[true, false, false, false, false, false, false, false],
        )
        .unwrap();
        for direction in Direction::ALL {
            let kept = |shape: &VoxelShape| {
                shape
                    .identity
                    .faces
                    .get()
                    .is_some_and(|faces| faces[direction as usize].get().is_some())
            };
            let transient = shape.face_shape_transient(direction).unwrap();
            assert!(!kept(&shape), "{direction:?}");
            let cached = shape.face_shape(direction).unwrap();
            assert!(kept(&shape));
            assert_eq!(transient.boxes(), cached.boxes(), "{direction:?}");
            let again = shape.face_shape_transient(direction).unwrap();
            assert!(again.same_identity(&cached));
        }
    }
}
