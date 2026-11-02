//! The 48 update contexts of vanilla `Orientation` and `ExperimentalRedstoneUtils`.
use super::{LevelAccess, Sim, query::axis_of};
use horizon_core::{
    geometry::{Axis, Direction},
    random::RandomSource,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SideBias {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Orientation {
    up: Direction,
    front: Direction,
    side_bias: SideBias,
}

impl Orientation {
    pub fn new(up: Direction, front: Direction, side_bias: SideBias) -> Self {
        assert_ne!(axis_of(up), axis_of(front));
        Self {
            up,
            front,
            side_bias,
        }
    }
    pub fn index(self) -> usize {
        let front_axis = if axis_of(self.up) == Axis::Y {
            usize::from(axis_of(self.front) == Axis::X)
        } else {
            usize::from(axis_of(self.front) == Axis::Y)
        };
        // Direction.AxisDirection declares POSITIVE before NEGATIVE.
        let negative = usize::from(matches!(
            self.front,
            Direction::Down | Direction::North | Direction::West
        ));
        ((self.up as usize * 4 + ((front_axis * 2) | negative)) * 2)
            + usize::from(self.side_bias == SideBias::Right)
    }
    pub fn from_index(index: usize) -> Self {
        assert!(index < 48);
        let up = Direction::ALL[index / 8];
        let axis = match (axis_of(up), (index / 4) % 2) {
            (Axis::Y, 1) => Axis::X,
            (_, 1) => Axis::Y,
            (Axis::Z, _) => Axis::X,
            _ => Axis::Z,
        };
        let negative = index & 2 != 0;
        let front = match (axis, negative) {
            (Axis::X, false) => Direction::East,
            (Axis::X, true) => Direction::West,
            (Axis::Y, false) => Direction::Up,
            (Axis::Y, true) => Direction::Down,
            (Axis::Z, false) => Direction::South,
            (Axis::Z, true) => Direction::North,
        };
        Self::new(
            up,
            front,
            if index & 1 == 0 {
                SideBias::Left
            } else {
                SideBias::Right
            },
        )
    }
    pub fn up(self) -> Direction {
        self.up
    }
    pub fn front(self) -> Direction {
        self.front
    }
    pub fn side_bias(self) -> SideBias {
        self.side_bias
    }
    pub fn side(self) -> Direction {
        let [x, y, z] = self.front.normal();
        let [u, v, w] = self.up.normal();
        let cross = [y * w - z * v, z * u - x * w, x * v - y * u];
        let right = Direction::ALL
            .into_iter()
            .find(|d| d.normal() == cross)
            .unwrap();
        if self.side_bias == SideBias::Right {
            right
        } else {
            right.opposite()
        }
    }
    pub fn with_front(self, front: Direction) -> Self {
        let up = if front == self.up {
            self.front.opposite()
        } else if front == self.up.opposite() {
            self.front
        } else {
            self.up
        };
        Self::new(up, front, self.side_bias)
    }
    pub fn with_up(self, up: Direction) -> Self {
        let front = if up == self.front {
            self.up.opposite()
        } else if up == self.front.opposite() {
            self.up
        } else {
            self.front
        };
        Self::new(up, front, self.side_bias)
    }
    pub fn with_front_preserve_up(self, front: Direction) -> Self {
        if axis_of(front) == axis_of(self.up) {
            self
        } else {
            self.with_front(front)
        }
    }
    pub fn with_front_adjust_side_bias(self, front: Direction) -> Self {
        let next = self.with_front(front);
        if self.front == next.side() {
            next.with_mirror()
        } else {
            next
        }
    }
    pub fn with_side_bias(self, side_bias: SideBias) -> Self {
        Self { side_bias, ..self }
    }
    pub fn with_mirror(self) -> Self {
        self.with_side_bias(if self.side_bias == SideBias::Left {
            SideBias::Right
        } else {
            SideBias::Left
        })
    }
    pub fn directions(self) -> [Direction; 6] {
        [
            self.front.opposite(),
            self.front,
            self.side(),
            self.side().opposite(),
            self.up.opposite(),
            self.up,
        ]
    }
    pub fn horizontal_directions(self) -> [Direction; 4] {
        [
            self.front.opposite(),
            self.front,
            self.side(),
            self.side().opposite(),
        ]
    }
    pub fn vertical_directions(self) -> [Direction; 2] {
        [self.up.opposite(), self.up]
    }
    pub fn random(random: &mut impl RandomSource) -> Self {
        Self::from_index(random.next_int_bound(48).unwrap() as usize)
    }
}

impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    pub fn initial_orientation(
        &mut self,
        front: Option<Direction>,
        up: Option<Direction>,
    ) -> Option<Orientation> {
        if !self.level.redstone_experiments() {
            return None;
        }
        let mut orientation = Orientation::random(self.random()).with_side_bias(SideBias::Left);
        if let Some(up) = up {
            orientation = orientation.with_up(up);
        }
        if let Some(front) = front {
            orientation = orientation.with_front(front);
        }
        Some(orientation)
    }
}
