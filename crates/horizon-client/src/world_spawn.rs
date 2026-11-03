//! Where a new world's spawn is and where a player first appears around it:
//! MinecraftServer.setInitialSpawn's chunk spiral and PlayerSpawnFinder.
use horizon_world::heightmap::HeightmapType;

/// The block queries the spawn searches make on full (decorated, lit) chunks.
pub trait SpawnLevel {
    fn min_y(&self) -> i32;
    /// `chunk.getHeight(kind, x, z)`: one above the column's highest block
    /// matching the heightmap, or the minimum height.
    fn height(&self, kind: HeightmapType, x: i32, z: i32) -> i32;
    /// `!getBlockState(pos).getFluidState().isEmpty()`.
    fn has_fluid(&self, x: i32, y: i32, z: i32) -> bool;
    /// `Block.isFaceFull(state.getCollisionShape(level, pos), Direction.UP)`.
    fn top_face_full(&self, x: i32, y: i32, z: i32) -> bool;
    /// `DimensionType.hasCeiling()`: the search starts at the generator's
    /// spawn height instead of each column's motion-blocking height.
    fn has_ceiling(&self) -> bool;
    /// `ChunkGenerator.getSpawnHeight(level)`.
    fn spawn_height(&self) -> i32;
}

/// PlayerSpawnFinder.getLevelRespawnPos: the block above the column's top
/// full-topped collision, unless water, lava or only non-colliding blocks
/// are on top. A level with a ceiling (`caveWorld`) searches from the
/// generator's spawn height, under the roof, not from the heightmap that
/// sees the roof.
pub fn level_respawn_pos(level: &impl SpawnLevel, x: i32, z: i32) -> Option<[i32; 3]> {
    let top = if level.has_ceiling() {
        level.spawn_height()
    } else {
        level.height(HeightmapType::MotionBlocking, x, z)
    };
    if top < level.min_y() {
        return None;
    }
    let surface = level.height(HeightmapType::WorldSurface, x, z);
    if surface <= top && surface > level.height(HeightmapType::OceanFloor, x, z) {
        return None;
    }
    for y in (level.min_y()..=top + 1).rev() {
        if level.has_fluid(x, y, z) {
            break;
        }
        if level.top_face_full(x, y, z) {
            return Some([x, y + 1, z]);
        }
    }
    None
}

/// PlayerSpawnFinder.getSpawnPosInChunk: the first valid column, x-major.
pub fn spawn_pos_in_chunk(level: &impl SpawnLevel, cx: i32, cz: i32) -> Option<[i32; 3]> {
    for x in cx * 16..cx * 16 + 16 {
        for z in cz * 16..cz * 16 + 16 {
            if let Some(pos) = level_respawn_pos(level, x, z) {
                return Some(pos);
            }
        }
    }
    None
}

/// setInitialSpawn's walk: Mth.square(11) steps of a square spiral around
/// the origin chunk, testing the steps within 5 chunks of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkSpiral {
    step: i32,
    x: i32,
    z: i32,
    dx: i32,
    dz: i32,
}
impl Default for ChunkSpiral {
    fn default() -> Self {
        Self::new()
    }
}
impl ChunkSpiral {
    pub const fn new() -> Self {
        Self {
            step: 0,
            x: 0,
            z: 0,
            dx: 0,
            dz: -1,
        }
    }
    /// Steps taken so far.
    pub fn steps(&self) -> i32 {
        self.step
    }
    /// The chunk offset to test now, or None once the walk is over.
    pub fn current(&self) -> Option<(i32, i32)> {
        (self.step < 11 * 11).then_some((self.x, self.z))
    }
    /// Whether `current` lies within the tested 11 by 11 chunks.
    pub fn tested(&self) -> bool {
        (-5..=5).contains(&self.x) && (-5..=5).contains(&self.z)
    }
    pub fn advance(&mut self) {
        if self.x == self.z
            || (self.x < 0 && self.x == -self.z)
            || (self.x > 0 && self.x == 1 - self.z)
        {
            (self.dx, self.dz) = (-self.dz, self.dx);
        }
        self.x += self.dx;
        self.z += self.dz;
        self.step += 1;
    }
}

/// What PlayerSpawnFinder tries next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnCandidate {
    /// A column to test with `level_respawn_pos` and a free player box.
    Column(i32, i32),
    /// Every column failed: the suggestion itself, height fixed up.
    Suggestion,
}

/// PlayerSpawnFinder: the columns of the square of `radius` around the
/// suggestion in a random coprime-stride order, then the suggestion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerSpawnFinder {
    pub suggestion: [i32; 3],
    radius: i32,
    count: i32,
    coprime: i32,
    offset: i32,
    next: i32,
}
impl PlayerSpawnFinder {
    /// `radius` is the respawn radius already clamped to the world border
    /// (findSpawn); `random(n)` draws the start offset in `0..n`.
    pub fn new(suggestion: [i32; 3], radius: i32, random: impl FnOnce(i32) -> i32) -> Self {
        let side = i64::from(radius) * 2 + 1;
        let count = (side * side).min(1024) as i32;
        Self {
            suggestion,
            radius,
            count,
            coprime: if count <= 16 { count - 1 } else { 17 },
            offset: random(count),
            next: 0,
        }
    }
    /// The next candidate, None once the suggestion was handed out.
    pub fn next_candidate(&mut self) -> Option<SpawnCandidate> {
        let index = self.next;
        if index > self.count {
            return None;
        }
        self.next += 1;
        if index == self.count {
            return Some(SpawnCandidate::Suggestion);
        }
        let value = (self.offset + self.coprime * index) % self.count;
        let side = self.radius * 2 + 1;
        Some(SpawnCandidate::Column(
            self.suggestion[0] + value % side - self.radius,
            self.suggestion[2] + value / side - self.radius,
        ))
    }
    /// Hand the last candidate out again (its chunk was not loaded yet).
    pub fn retry(&mut self) {
        self.next -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spiral_walks_out_from_the_origin_like_set_initial_spawn() {
        let mut spiral = ChunkSpiral::default();
        let mut seen = Vec::new();
        while let Some(at) = spiral.current() {
            if spiral.tested() {
                seen.push(at);
            }
            spiral.advance();
        }
        assert_eq!(
            &seen[..9],
            &[
                (0, 0),
                (1, 0),
                (1, 1),
                (0, 1),
                (-1, 1),
                (-1, 0),
                (-1, -1),
                (0, -1),
                (1, -1)
            ]
        );
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 121);
    }

    #[test]
    fn finder_visits_every_column_once_then_the_suggestion() {
        let mut finder = PlayerSpawnFinder::new([100, 70, -40], 10, |n| n / 3);
        let mut columns = Vec::new();
        while let Some(c) = finder.next_candidate() {
            columns.push(c);
        }
        assert_eq!(columns.len(), 442);
        assert_eq!(columns.last(), Some(&SpawnCandidate::Suggestion));
        columns.sort_by_key(|c| format!("{c:?}"));
        columns.dedup();
        assert_eq!(columns.len(), 442);
        let mut zero = PlayerSpawnFinder::new([5, 64, 5], 0, |_| 0);
        assert_eq!(zero.next_candidate(), Some(SpawnCandidate::Column(5, 5)));
        assert_eq!(zero.next_candidate(), Some(SpawnCandidate::Suggestion));
        assert_eq!(zero.next_candidate(), None);
    }

    struct Flat {
        top: i32,
        water: bool,
    }
    impl SpawnLevel for Flat {
        fn min_y(&self) -> i32 {
            -64
        }
        fn height(&self, kind: HeightmapType, _: i32, _: i32) -> i32 {
            match kind {
                HeightmapType::OceanFloor if self.water => self.top - 3,
                _ => self.top + 1,
            }
        }
        fn has_fluid(&self, _: i32, y: i32, _: i32) -> bool {
            self.water && y <= self.top && y > self.top - 3
        }
        fn top_face_full(&self, _: i32, y: i32, _: i32) -> bool {
            y <= self.top
        }
        fn has_ceiling(&self) -> bool {
            false
        }
        fn spawn_height(&self) -> i32 {
            64
        }
    }

    /// A level with a roof: the heightmap sees the roof, the search should not.
    struct Cave {
        floor: i32,
        roof: i32,
        spawn_height: i32,
        ceiling: bool,
    }
    impl SpawnLevel for Cave {
        fn min_y(&self) -> i32 {
            -64
        }
        fn height(&self, _: HeightmapType, _: i32, _: i32) -> i32 {
            self.roof + 1
        }
        fn has_fluid(&self, _: i32, _: i32, _: i32) -> bool {
            false
        }
        fn top_face_full(&self, _: i32, y: i32, _: i32) -> bool {
            y == self.roof || y <= self.floor
        }
        fn has_ceiling(&self) -> bool {
            self.ceiling
        }
        fn spawn_height(&self) -> i32 {
            self.spawn_height
        }
    }

    #[test]
    fn respawn_column_stands_on_land_not_water() {
        assert_eq!(
            level_respawn_pos(
                &Flat {
                    top: 70,
                    water: false
                },
                3,
                4
            ),
            Some([3, 71, 4])
        );
        assert_eq!(
            level_respawn_pos(
                &Flat {
                    top: 62,
                    water: true
                },
                3,
                4
            ),
            None
        );
        assert_eq!(
            spawn_pos_in_chunk(
                &Flat {
                    top: 70,
                    water: false
                },
                -2,
                1
            ),
            Some([-32, 71, 16])
        );
    }

    #[test]
    fn a_ceiling_starts_the_respawn_search_at_the_generator_spawn_height() {
        let cave = Cave {
            floor: 50,
            roof: 120,
            spawn_height: 60,
            ceiling: true,
        };
        // From under the roof the floor is 51; from the heightmap it would be
        // the roof itself, 121.
        assert_eq!(level_respawn_pos(&cave, 1, 2), Some([1, 51, 2]));
        let open = Cave {
            ceiling: false,
            ..cave
        };
        assert_eq!(level_respawn_pos(&open, 1, 2), Some([1, 121, 2]));
    }
}
