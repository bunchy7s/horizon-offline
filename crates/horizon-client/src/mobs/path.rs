//! Vanilla's A* pathfinder (`PathFinder`, `BinaryHeap`, `Node`, `Path`) with
//! the walking, swimming, flying and amphibious node evaluators
//! (`WalkNodeEvaluator`, `SwimNodeEvaluator`, `FlyNodeEvaluator`,
//! `AmphibiousNodeEvaluator`), ported rule for rule.
use super::{MobWorld, data};
use crate::blocks::Fluid;
use crate::fast_hash::FxHashMap;
use horizon_core::geometry::{Aabb, Vec3};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PathType {
    Blocked,
    Open,
    Walkable,
    WalkableDoor,
    Trapdoor,
    PowderSnow,
    OnTopOfPowderSnow,
    Fence,
    Lava,
    Water,
    WaterBorder,
    Rail,
    UnpassableRail,
    FireInNeighbor,
    Fire,
    DamagingInNeighbor,
    Damaging,
    DoorOpen,
    DoorWoodClosed,
    DoorIronClosed,
    Breach,
    Leaves,
    StickyHoney,
    Cocoa,
    DamageCautious,
    OnTopOfTrapdoor,
    BigMobsCloseToDanger,
}
pub const PATH_TYPES: usize = 27;
impl PathType {
    pub fn from_name(name: &str) -> Self {
        match name {
            "OPEN" => Self::Open,
            "WALKABLE" => Self::Walkable,
            "WALKABLE_DOOR" => Self::WalkableDoor,
            "TRAPDOOR" => Self::Trapdoor,
            "POWDER_SNOW" => Self::PowderSnow,
            "ON_TOP_OF_POWDER_SNOW" => Self::OnTopOfPowderSnow,
            "FENCE" => Self::Fence,
            "LAVA" => Self::Lava,
            "WATER" => Self::Water,
            "WATER_BORDER" => Self::WaterBorder,
            "RAIL" => Self::Rail,
            "UNPASSABLE_RAIL" => Self::UnpassableRail,
            "FIRE_IN_NEIGHBOR" => Self::FireInNeighbor,
            "FIRE" => Self::Fire,
            "DAMAGING_IN_NEIGHBOR" => Self::DamagingInNeighbor,
            "DAMAGING" => Self::Damaging,
            "DOOR_OPEN" => Self::DoorOpen,
            "DOOR_WOOD_CLOSED" => Self::DoorWoodClosed,
            "DOOR_IRON_CLOSED" => Self::DoorIronClosed,
            "BREACH" => Self::Breach,
            "LEAVES" => Self::Leaves,
            "STICKY_HONEY" => Self::StickyHoney,
            "COCOA" => Self::Cocoa,
            "DAMAGE_CAUTIOUS" => Self::DamageCautious,
            "ON_TOP_OF_TRAPDOOR" => Self::OnTopOfTrapdoor,
            "BIG_MOBS_CLOSE_TO_DANGER" => Self::BigMobsCloseToDanger,
            _ => Self::Blocked,
        }
    }
    /// PathType's default malus.
    pub fn malus(self) -> f32 {
        match self {
            Self::Blocked | Self::PowderSnow | Self::Fence | Self::Lava | Self::UnpassableRail => {
                -1.0
            }
            Self::Damaging | Self::DoorWoodClosed | Self::DoorIronClosed | Self::Leaves => -1.0,
            Self::Water
            | Self::WaterBorder
            | Self::FireInNeighbor
            | Self::DamagingInNeighbor
            | Self::StickyHoney => 8.0,
            Self::Fire => 16.0,
            Self::Breach | Self::BigMobsCloseToDanger => 4.0,
            _ => 0.0,
        }
    }
}

impl MobWorld<'_> {
    pub fn path_type_from_state(&self, x: i32, y: i32, z: i32) -> PathType {
        self.data.state(self.world.block(x, y, z)).path
    }
    /// WalkNodeEvaluator.getPathTypeStatic.
    pub fn path_type_static(&self, x: i32, y: i32, z: i32) -> PathType {
        let t = self.path_type_from_state(x, y, z);
        if t == PathType::Open && y > self.world.bottom_y() {
            match self.path_type_from_state(x, y - 1, z) {
                PathType::Open | PathType::Water | PathType::Lava | PathType::Walkable => {
                    PathType::Open
                }
                PathType::Fire => PathType::Fire,
                PathType::Damaging => PathType::Damaging,
                PathType::StickyHoney => PathType::StickyHoney,
                PathType::PowderSnow => PathType::OnTopOfPowderSnow,
                PathType::DamageCautious => PathType::DamageCautious,
                PathType::Trapdoor => PathType::OnTopOfTrapdoor,
                _ => self.check_neighbours(x, y, z, PathType::Walkable),
            }
        } else {
            t
        }
    }
    /// WalkNodeEvaluator.checkNeighbourBlocks.
    pub fn check_neighbours(&self, x: i32, y: i32, z: i32, t: PathType) -> PathType {
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if dx != 0 || dz != 0 {
                        match self.path_type_from_state(x + dx, y + dy, z + dz) {
                            PathType::Damaging => return PathType::DamagingInNeighbor,
                            PathType::Fire | PathType::Lava => return PathType::FireInNeighbor,
                            PathType::Water => return PathType::WaterBorder,
                            PathType::DamageCautious => return PathType::DamageCautious,
                            _ => {}
                        }
                    }
                }
            }
        }
        t
    }
    /// WalkNodeEvaluator.getFloorLevel(level, pos).
    pub fn floor_level(&self, x: i32, y: i32, z: i32) -> f64 {
        f64::from(y - 1) + f64::from(self.data.state(self.world.block(x, y - 1, z)).top)
    }
}

/// The parts of a mob the node evaluators read (`Mob` accessors at prepare time).
#[derive(Clone, Debug)]
pub struct PathMob {
    pub position: Vec3,
    pub width: f32,
    pub height: f32,
    pub on_ground: bool,
    pub in_water: bool,
    pub max_up_step: f32,
    pub max_fall_distance: i32,
    pub malus: [Option<f32>; PATH_TYPES],
    pub can_float: bool,
    pub can_open_doors: bool,
    pub can_pass_doors: bool,
    pub can_walk_over_fences: bool,
    /// Striders stand on lava (`canStandOnFluid`).
    pub stands_on_lava: bool,
}
impl PathMob {
    pub fn malus(&self, t: PathType) -> f32 {
        self.malus[t as usize].unwrap_or_else(|| t.malus())
    }
    pub fn bounds(&self) -> Aabb {
        let w = f64::from(self.width) / 2.0;
        Aabb::new(
            Vec3::new(self.position.x - w, self.position.y, self.position.z - w),
            Vec3::new(
                self.position.x + w,
                self.position.y + f64::from(self.height),
                self.position.z + w,
            ),
        )
    }
    fn block_position(&self) -> [i32; 3] {
        [
            self.position.x.floor() as i32,
            self.position.y.floor() as i32,
            self.position.z.floor() as i32,
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Evaluator {
    Walk,
    Swim { breach: bool },
    Fly,
    Amphibious { prefers_shallow: bool },
}

#[derive(Clone, Debug)]
pub struct Node {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    heap: i32,
    g: f32,
    h: f32,
    f: f32,
    came_from: Option<u32>,
    pub closed: bool,
    walked: f32,
    pub cost_malus: f32,
    pub kind: PathType,
}
impl Node {
    fn new(x: i32, y: i32, z: i32) -> Self {
        Self {
            x,
            y,
            z,
            heap: -1,
            g: 0.0,
            h: 0.0,
            f: 0.0,
            came_from: None,
            closed: false,
            walked: 0.0,
            cost_malus: 0.0,
            kind: PathType::Blocked,
        }
    }
    fn distance(&self, x: i32, y: i32, z: i32) -> f32 {
        let (dx, dy, dz) = (
            (x - self.x) as f32,
            (y - self.y) as f32,
            (z - self.z) as f32,
        );
        (dx * dx + dy * dy + dz * dz).sqrt()
    }
    fn manhattan(&self, x: i32, y: i32, z: i32) -> f32 {
        ((x - self.x).abs() + (y - self.y).abs() + (z - self.z).abs()) as f32
    }
}

/// A computed path: node positions, the requested target and whether it was reached.
#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    pub nodes: Vec<PathNode>,
    pub next: usize,
    pub target: [i32; 3],
    pub dist_to_target: f32,
    pub reached: bool,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathNode {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub kind: PathType,
}
impl Path {
    fn new(nodes: Vec<PathNode>, target: [i32; 3], reached: bool) -> Self {
        let dist_to_target = nodes.last().map_or(f32::MAX, |n| {
            ((n.x - target[0]).abs() + (n.y - target[1]).abs() + (n.z - target[2]).abs()) as f32
        });
        Self {
            nodes,
            next: 0,
            target,
            dist_to_target,
            reached,
        }
    }
    pub fn is_done(&self) -> bool {
        self.next >= self.nodes.len()
    }
    pub fn advance(&mut self) {
        self.next += 1;
    }
    pub fn end(&self) -> Option<PathNode> {
        self.nodes.last().copied()
    }
    pub fn next_node(&self) -> PathNode {
        self.nodes[self.next]
    }
    /// Path.getEntityPosAtNode: node corner plus half the mob's block width.
    pub fn entity_pos_at(&self, index: usize, width: f32) -> Vec3 {
        let n = self.nodes[index];
        let half = f64::from((width + 1.0) as i32) * 0.5;
        Vec3::new(f64::from(n.x) + half, f64::from(n.y), f64::from(n.z) + half)
    }
    pub fn same_as(&self, other: Option<&Path>) -> bool {
        other.is_some_and(|o| o.nodes == self.nodes)
    }
}

struct Target {
    x: i32,
    y: i32,
    z: i32,
    best_h: f32,
    best: Option<u32>,
    reached: bool,
    pos: [i32; 3],
}

/// One pathfinding query: the node arena, open set and per-query caches.
pub struct Finder<'a, 'w> {
    w: &'a MobWorld<'w>,
    mob: &'a PathMob,
    eval: Evaluator,
    nodes: Vec<Node>,
    index: FxHashMap<(i32, i32, i32), u32>,
    heap: Vec<u32>,
    types: FxHashMap<(i32, i32, i32), PathType>,
    /// `path_type` of each position asked during this query: it reads up to
    /// 26 blocks, and overlapping mob boxes ask for the same positions.
    statics: std::cell::RefCell<FxHashMap<(i32, i32, i32), PathType>>,
    collisions: FxHashMap<[u64; 6], bool>,
    entity_width: i32,
    entity_height: i32,
    entity_depth: i32,
}

impl<'a, 'w> Finder<'a, 'w> {
    pub fn new(w: &'a MobWorld<'w>, mob: &'a PathMob, eval: Evaluator) -> Self {
        Self {
            w,
            mob,
            eval,
            nodes: Vec::new(),
            index: FxHashMap::default(),
            heap: Vec::new(),
            types: FxHashMap::default(),
            statics: Default::default(),
            collisions: FxHashMap::default(),
            entity_width: (mob.width + 1.0).floor() as i32,
            entity_height: (mob.height + 1.0).floor() as i32,
            entity_depth: (mob.width + 1.0).floor() as i32,
        }
    }
    /// Mob.getPathfindingMalus; AmphibiousNodeEvaluator.prepare overrides three
    /// maluses for the duration of the query.
    fn malus(&self, t: PathType) -> f32 {
        if let Evaluator::Amphibious { .. } = self.eval {
            match t {
                PathType::Water => return 0.0,
                PathType::Walkable => return 6.0,
                PathType::WaterBorder => return 4.0,
                _ => {}
            }
        }
        self.mob.malus(t)
    }
    // --- world access -------------------------------------------------
    fn state(&self, x: i32, y: i32, z: i32) -> data::StateData {
        self.w.data.state(self.w.world.block(x, y, z))
    }
    fn type_from_state(&self, x: i32, y: i32, z: i32) -> PathType {
        self.w.path_type_from_state(x, y, z)
    }
    fn fluid(&self, x: i32, y: i32, z: i32) -> Fluid {
        self.w.facts.get(self.w.world.block(x, y, z)).fluid
    }
    fn is_air(&self, x: i32, y: i32, z: i32) -> bool {
        self.w.facts.is_air(self.w.world.block(x, y, z))
    }
    /// WalkNodeEvaluator.getFloorLevel(level, pos).
    pub fn static_floor(&self, x: i32, y: i32, z: i32) -> f64 {
        f64::from(y - 1) + f64::from(self.state(x, y - 1, z).top)
    }
    fn floor_level(&self, x: i32, y: i32, z: i32) -> f64 {
        let amphibious = matches!(self.eval, Evaluator::Amphibious { .. });
        if (self.mob.can_float || amphibious) && self.fluid(x, y, z) == Fluid::Water {
            f64::from(y) + 0.5
        } else {
            self.static_floor(x, y, z)
        }
    }
    fn has_collisions(&mut self, b: Aabb) -> bool {
        let (lo, hi) = (b.min_position(), b.max_position());
        let key = [
            lo.x.to_bits(),
            lo.y.to_bits(),
            lo.z.to_bits(),
            hi.x.to_bits(),
            hi.y.to_bits(),
            hi.z.to_bits(),
        ];
        if let Some(v) = self.collisions.get(&key) {
            return *v;
        }
        let v = !self.w.no_collision(b);
        self.collisions.insert(key, v);
        v
    }
    // --- path types -----------------------------------------------------
    /// WalkNodeEvaluator.getPathTypeStatic (+ the fly/amphibious overrides).
    /// The world doesn't change during a query, so each position's type is
    /// worked out once.
    fn path_type(&self, x: i32, y: i32, z: i32) -> PathType {
        if let Some(t) = self.statics.borrow().get(&(x, y, z)) {
            return *t;
        }
        let t = self.compute_path_type(x, y, z);
        self.statics.borrow_mut().insert((x, y, z), t);
        t
    }
    fn compute_path_type(&self, x: i32, y: i32, z: i32) -> PathType {
        match self.eval {
            Evaluator::Fly => self.fly_path_type(x, y, z),
            Evaluator::Amphibious { .. } => {
                let t = self.type_from_state(x, y, z);
                if t == PathType::Water {
                    for (dx, dy, dz) in [
                        (0, -1, 0),
                        (0, 1, 0),
                        (0, 0, -1),
                        (0, 0, 1),
                        (-1, 0, 0),
                        (1, 0, 0),
                    ] {
                        if self.type_from_state(x + dx, y + dy, z + dz) == PathType::Blocked {
                            return PathType::WaterBorder;
                        }
                    }
                    PathType::Water
                } else {
                    self.walk_path_type(x, y, z)
                }
            }
            _ => self.walk_path_type(x, y, z),
        }
    }
    pub fn walk_path_type(&self, x: i32, y: i32, z: i32) -> PathType {
        self.w.path_type_static(x, y, z)
    }
    fn check_neighbours(&self, x: i32, y: i32, z: i32, t: PathType) -> PathType {
        self.w.check_neighbours(x, y, z, t)
    }
    fn fly_path_type(&self, x: i32, y: i32, z: i32) -> PathType {
        let mut t = self.type_from_state(x, y, z);
        if t == PathType::Open && y > self.w.world.bottom_y() {
            let below = self.type_from_state(x, y - 1, z);
            t = match below {
                PathType::Fire | PathType::Lava => PathType::Fire,
                PathType::Damaging => PathType::Damaging,
                PathType::Cocoa => PathType::Cocoa,
                PathType::Fence => {
                    if [x, y - 1, z] != self.mob.block_position() {
                        PathType::Fence
                    } else {
                        t
                    }
                }
                PathType::Walkable | PathType::Open | PathType::Water => PathType::Open,
                _ => PathType::Walkable,
            };
        }
        if t == PathType::Walkable || t == PathType::Open {
            t = self.check_neighbours(x, y, z, t);
        }
        t
    }
    fn within_bb(&self, x: i32, y: i32, z: i32) -> Vec<PathType> {
        let mut out = Vec::with_capacity(4);
        let mp = self.mob.block_position();
        for dx in 0..self.entity_width {
            for dy in 0..self.entity_height {
                for dz in 0..self.entity_depth {
                    let mut t = self.path_type(x + dx, y + dy, z + dz);
                    if t == PathType::DoorWoodClosed
                        && self.mob.can_open_doors
                        && self.mob.can_pass_doors
                    {
                        t = PathType::WalkableDoor;
                    }
                    if t == PathType::DoorOpen && !self.mob.can_pass_doors {
                        t = PathType::Blocked;
                    }
                    if t == PathType::Rail
                        && self.path_type(mp[0], mp[1], mp[2]) != PathType::Rail
                        && self.path_type(mp[0], mp[1] - 1, mp[2]) != PathType::Rail
                    {
                        t = PathType::UnpassableRail;
                    }
                    if !out.contains(&t) {
                        out.push(t);
                    }
                }
            }
        }
        out
    }
    /// WalkNodeEvaluator.getPathTypeOfMob.
    fn path_type_of_mob(&self, x: i32, y: i32, z: i32) -> PathType {
        if let Evaluator::Swim { .. } = self.eval {
            return self.swim_path_type(x, y, z);
        }
        let mut types = self.within_bb(x, y, z);
        // EnumSet iteration follows declaration order.
        types.sort_by_key(|t| *t as u8);
        if types.len() == 1 {
            return types[0];
        }
        if types.contains(&PathType::Fence) {
            return PathType::Fence;
        }
        if types.contains(&PathType::UnpassableRail) {
            return PathType::UnpassableRail;
        }
        let mut highest = PathType::Blocked;
        let mut highest_malus = self.malus(highest);
        for t in &types {
            let m = self.malus(*t);
            if m < 0.0 {
                return *t;
            }
            if m >= highest_malus {
                highest_malus = m;
                highest = *t;
            }
        }
        let current = self.path_type(x, y, z);
        if self.entity_width > 1 {
            let cheaper = self.malus(current) < highest_malus;
            if cheaper && self.malus(PathType::BigMobsCloseToDanger) < highest_malus {
                PathType::BigMobsCloseToDanger
            } else {
                highest
            }
        } else if current == PathType::Open && highest != PathType::Open && highest_malus == 0.0 {
            PathType::Open
        } else {
            highest
        }
    }
    fn swim_path_type(&self, x: i32, y: i32, z: i32) -> PathType {
        let mut last = (x, y, z);
        for xx in x..x + self.entity_width {
            for yy in y..y + self.entity_height {
                for zz in z..z + self.entity_depth {
                    let fluid = self.fluid(xx, yy, zz);
                    let below = self.state(xx, yy - 1, zz);
                    if fluid == Fluid::None
                        && below.flags & data::WATER != 0
                        && self.is_air(xx, yy, zz)
                    {
                        return PathType::Breach;
                    }
                    if fluid != Fluid::Water {
                        return PathType::Blocked;
                    }
                    last = (xx, yy, zz);
                }
            }
        }
        if self.state(last.0, last.1, last.2).flags & data::WATER != 0 {
            PathType::Water
        } else {
            PathType::Blocked
        }
    }
    fn cached(&mut self, x: i32, y: i32, z: i32) -> PathType {
        if let Some(t) = self.types.get(&(x, y, z)) {
            return *t;
        }
        let t = self.path_type_of_mob(x, y, z);
        self.types.insert((x, y, z), t);
        t
    }
    // --- node arena -------------------------------------------------------
    fn node(&mut self, x: i32, y: i32, z: i32) -> u32 {
        if let Some(i) = self.index.get(&(x, y, z)) {
            return *i;
        }
        let i = self.nodes.len() as u32;
        self.nodes.push(Node::new(x, y, z));
        self.index.insert((x, y, z), i);
        i
    }
    fn start_node(&mut self, x: i32, y: i32, z: i32) -> u32 {
        let n = self.node(x, y, z);
        let t = self.cached(x, y, z);
        let m = self.malus(t);
        let node = &mut self.nodes[n as usize];
        node.kind = t;
        node.cost_malus = m;
        n
    }
    fn can_start_at(&mut self, x: i32, y: i32, z: i32) -> bool {
        let t = self.cached(x, y, z);
        if self.eval == Evaluator::Fly {
            return self.malus(t) >= 0.0;
        }
        t != PathType::Open && self.malus(t) >= 0.0
    }
    fn start(&mut self) -> Option<u32> {
        let m = self.mob.clone();
        let p = m.position;
        let b = m.bounds();
        let (lo, hi) = (b.min_position(), b.max_position());
        match self.eval {
            Evaluator::Swim { .. } => {
                return Some(self.node(
                    lo.x.floor() as i32,
                    (lo.y + 0.5).floor() as i32,
                    lo.z.floor() as i32,
                ));
            }
            Evaluator::Amphibious { .. } if m.in_water => {
                return Some(self.start_node(
                    lo.x.floor() as i32,
                    (lo.y + 0.5).floor() as i32,
                    lo.z.floor() as i32,
                ));
            }
            Evaluator::Fly => {
                let (bx, bz) = (p.x.floor() as i32, p.z.floor() as i32);
                let mut start_y;
                if m.can_float && m.in_water {
                    start_y = p.y.floor() as i32;
                    while &*self.w.facts.get(self.w.world.block(bx, start_y, bz)).name
                        == "minecraft:water"
                    {
                        start_y += 1;
                    }
                } else {
                    start_y = (p.y + 0.5).floor() as i32;
                }
                if !self.can_start_at(bx, start_y, bz) {
                    let by = p.y.floor() as i32;
                    for (cx, cz) in [(lo.x, lo.z), (lo.x, hi.z), (hi.x, lo.z), (hi.x, hi.z)] {
                        let (cx, cz) = (cx.floor() as i32, cz.floor() as i32);
                        if self.can_start_at(cx, by, cz) {
                            return Some(self.start_node(cx, by, cz));
                        }
                    }
                }
                return Some(self.start_node(bx, start_y, bz));
            }
            _ => {}
        }
        // WalkNodeEvaluator.getStart
        let (bx, bz) = (p.x.floor() as i32, p.z.floor() as i32);
        let mut start_y = p.y.floor() as i32;
        let fluid_at = |s: &Self, y: i32| s.fluid(bx, y, bz);
        if !(m.stands_on_lava && fluid_at(self, start_y) == Fluid::Lava) {
            if m.can_float && fluid_at(self, start_y) == Fluid::Water {
                while fluid_at(self, start_y) == Fluid::Water {
                    start_y += 1;
                }
                start_y -= 1;
            } else if m.on_ground {
                start_y = (p.y + 0.5).floor() as i32;
            } else {
                let mut y = (p.y + 1.0).floor() as i32;
                while y > self.w.world.bottom_y() {
                    start_y = y;
                    y -= 1;
                    let s = self.w.world.block(bx, y, bz);
                    if !self.w.facts.is_air(s) && self.w.data.state(s).flags & data::LAND == 0 {
                        break;
                    }
                }
            }
        } else {
            while m.stands_on_lava && fluid_at(self, start_y) == Fluid::Lava {
                start_y += 1;
            }
            start_y -= 1;
        }
        if !self.can_start_at(bx, start_y, bz) {
            for (cx, cz) in [(lo.x, lo.z), (lo.x, hi.z), (hi.x, lo.z), (hi.x, hi.z)] {
                let (cx, cz) = (cx.floor() as i32, cz.floor() as i32);
                if self.can_start_at(cx, start_y, cz) {
                    return Some(self.start_node(cx, start_y, cz));
                }
            }
        }
        Some(self.start_node(bx, start_y, bz))
    }
    // --- walking neighbours ----------------------------------------------
    fn node_update_max(&mut self, x: i32, y: i32, z: i32, t: PathType, cost: f32) -> u32 {
        let n = self.node(x, y, z);
        let node = &mut self.nodes[n as usize];
        node.kind = t;
        node.cost_malus = node.cost_malus.max(cost);
        n
    }
    fn blocked_node(&mut self, x: i32, y: i32, z: i32) -> u32 {
        let n = self.node(x, y, z);
        let node = &mut self.nodes[n as usize];
        node.kind = PathType::Blocked;
        node.cost_malus = -1.0;
        n
    }
    fn closed_node(&mut self, x: i32, y: i32, z: i32, t: PathType) -> u32 {
        let n = self.node(x, y, z);
        let node = &mut self.nodes[n as usize];
        node.closed = true;
        node.kind = t;
        node.cost_malus = t.malus();
        n
    }
    fn jump_height(&self) -> f64 {
        f64::from(self.mob.max_up_step).max(1.125)
    }
    fn partial_collision(t: PathType) -> bool {
        matches!(
            t,
            PathType::Fence | PathType::DoorWoodClosed | PathType::DoorIronClosed
        )
    }
    fn can_reach_without_collision(&mut self, n: u32) -> bool {
        let (nx, ny, nz) = {
            let node = &self.nodes[n as usize];
            (node.x, node.y, node.z)
        };
        let mut bb = self.mob.bounds();
        let size = {
            let (lo, hi) = (bb.min_position(), bb.max_position());
            ((hi.x - lo.x) + (hi.y - lo.y) + (hi.z - lo.z)) / 3.0
        };
        let (lo, hi) = (bb.min_position(), bb.max_position());
        let p = self.mob.position;
        let mut delta = Vec3::new(
            f64::from(nx) - p.x + (hi.x - lo.x) / 2.0,
            f64::from(ny) - p.y + (hi.y - lo.y) / 2.0,
            f64::from(nz) - p.z + (hi.z - lo.z) / 2.0,
        );
        let len = (delta.x * delta.x + delta.y * delta.y + delta.z * delta.z).sqrt();
        let steps = (len / size).ceil().max(1.0) as i32;
        delta = Vec3::new(
            delta.x / f64::from(steps),
            delta.y / f64::from(steps),
            delta.z / f64::from(steps),
        );
        for _ in 1..=steps {
            bb = bb.move_by(delta);
            if self.has_collisions(bb) {
                return false;
            }
        }
        true
    }
    #[allow(clippy::too_many_arguments)]
    fn find_accepted(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        jump: i32,
        node_height: f64,
        dir: (i32, i32),
        current: PathType,
    ) -> Option<u32> {
        let amphibious = matches!(self.eval, Evaluator::Amphibious { .. });
        let max_y_target = self.floor_level(x, y, z);
        if max_y_target - node_height > self.jump_height() {
            return None;
        }
        let t = self.cached(x, y, z);
        let cost = self.malus(t);
        let mut best = None;
        if cost >= 0.0 {
            best = Some(self.node_update_max(x, y, z, t, cost));
        }
        if Self::partial_collision(current)
            && let Some(b) = best
            && self.nodes[b as usize].cost_malus >= 0.0
            && !self.can_reach_without_collision(b)
        {
            best = None;
        }
        if t == PathType::Walkable || (amphibious && t == PathType::Water) {
            return best;
        }
        let best_cost_negative = best.is_none_or(|b| self.nodes[b as usize].cost_malus < 0.0);
        if best_cost_negative
            && jump > 0
            && (t != PathType::Fence || self.mob.can_walk_over_fences)
            && t != PathType::UnpassableRail
            && t != PathType::Trapdoor
            && t != PathType::PowderSnow
        {
            best = self.try_jump_on(x, y, z, jump, node_height, dir, current);
        } else if !amphibious && t == PathType::Water && !self.mob.can_float {
            best = self.first_non_water_below(x, y, z, best);
        } else if t == PathType::Open {
            best = Some(self.first_ground_below(x, y, z));
        } else if Self::partial_collision(t) && best.is_none() {
            best = Some(self.closed_node(x, y, z, t));
        }
        best
    }
    #[allow(clippy::too_many_arguments)]
    fn try_jump_on(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        jump: i32,
        node_height: f64,
        dir: (i32, i32),
        current: PathType,
    ) -> Option<u32> {
        let above = self.find_accepted(x, y + 1, z, jump - 1, node_height, dir, current)?;
        if self.mob.width >= 1.0 {
            return Some(above);
        }
        let (ak, ax, ay, az) = {
            let n = &self.nodes[above as usize];
            (n.kind, n.x, n.y, n.z)
        };
        if ak != PathType::Open && ak != PathType::Walkable {
            return Some(above);
        }
        let cx = f64::from(x - dir.0) + 0.5;
        let cz = f64::from(z - dir.1) + 0.5;
        let half = f64::from(self.mob.width) / 2.0;
        let grow = Aabb::new(
            Vec3::new(
                cx - half,
                self.floor_level(cx.floor() as i32, y + 1, cz.floor() as i32) + 0.001,
                cz - half,
            ),
            Vec3::new(
                cx + half,
                f64::from(self.mob.height) + self.floor_level(ax, ay, az) - 0.002,
                cz + half,
            ),
        );
        if self.has_collisions(grow) {
            None
        } else {
            Some(above)
        }
    }
    fn first_non_water_below(
        &mut self,
        x: i32,
        mut y: i32,
        z: i32,
        mut best: Option<u32>,
    ) -> Option<u32> {
        y -= 1;
        while y > self.w.world.bottom_y() {
            let t = self.cached(x, y, z);
            if t != PathType::Water {
                return best;
            }
            let m = self.malus(t);
            best = Some(self.node_update_max(x, y, z, t, m));
            y -= 1;
        }
        best
    }
    fn first_ground_below(&mut self, x: i32, y: i32, z: i32) -> u32 {
        let mut cy = y - 1;
        while cy >= self.w.world.bottom_y() {
            if y - cy > self.mob.max_fall_distance {
                return self.blocked_node(x, cy, z);
            }
            let t = self.cached(x, cy, z);
            let cost = self.malus(t);
            if t != PathType::Open {
                if cost >= 0.0 {
                    return self.node_update_max(x, cy, z, t, cost);
                }
                return self.blocked_node(x, cy, z);
            }
            cy -= 1;
        }
        self.blocked_node(x, y, z)
    }
    fn neighbor_valid(&self, n: Option<u32>, current: u32) -> bool {
        n.is_some_and(|n| {
            let node = &self.nodes[n as usize];
            !node.closed
                && (node.cost_malus >= 0.0 || self.nodes[current as usize].cost_malus < 0.0)
        })
    }
    fn diagonal_pair_valid(&self, pos: u32, ew: Option<u32>, ns: Option<u32>) -> bool {
        let (Some(ew), Some(ns)) = (ew, ns) else {
            return false;
        };
        let (p, e, n) = (
            &self.nodes[pos as usize],
            &self.nodes[ew as usize],
            &self.nodes[ns as usize],
        );
        if n.y > p.y || e.y > p.y {
            return false;
        }
        if e.kind == PathType::WalkableDoor || n.kind == PathType::WalkableDoor {
            return false;
        }
        if self.mob.width > 1.0 && (e.cost_malus > 0.0 || n.cost_malus > 0.0) {
            return false;
        }
        let between_posts = n.kind == PathType::Fence
            && e.kind == PathType::Fence
            && f64::from(self.mob.width) < 0.5;
        (n.y < p.y || n.cost_malus >= 0.0 || between_posts)
            && (e.y < p.y || e.cost_malus >= 0.0 || between_posts)
    }
    fn diagonal_valid(&self, d: Option<u32>) -> bool {
        d.is_some_and(|d| {
            let n = &self.nodes[d as usize];
            !n.closed && n.kind != PathType::WalkableDoor && n.cost_malus >= 0.0
        })
    }
    fn walk_neighbors(&mut self, pos: u32, out: &mut Vec<u32>) {
        let (px, py, pz) = {
            let n = &self.nodes[pos as usize];
            (n.x, n.y, n.z)
        };
        let above = self.cached(px, py + 1, pz);
        let current = self.cached(px, py, pz);
        let mut jump = 0;
        if self.malus(above) >= 0.0 && current != PathType::StickyHoney {
            jump = self.mob.max_up_step.max(1.0).floor() as i32;
        }
        let height = self.floor_level(px, py, pz);
        // Direction.Plane.HORIZONTAL: north, east, south, west; 2D index S=0 W=1 N=2 E=3.
        const DIRS: [(i32, i32, usize); 4] = [(0, -1, 2), (1, 0, 3), (0, 1, 0), (-1, 0, 1)];
        let mut by2d: [Option<u32>; 4] = [None; 4];
        for (dx, dz, i2) in DIRS {
            let n = self.find_accepted(px + dx, py, pz + dz, jump, height, (dx, dz), current);
            by2d[i2] = n;
            if self.neighbor_valid(n, pos) {
                out.push(n.unwrap_or_default());
            }
        }
        // Clockwise of north is east, of east south, of south west, of west north.
        let clockwise = |i2: usize| match i2 {
            2 => 3,
            3 => 0,
            0 => 1,
            _ => 2,
        };
        for (dx, dz, i2) in DIRS {
            let c = clockwise(i2);
            let (cdx, cdz) = match c {
                0 => (0, 1),
                1 => (-1, 0),
                2 => (0, -1),
                _ => (1, 0),
            };
            if self.diagonal_pair_valid(pos, by2d[i2], by2d[c]) {
                let d = self.find_accepted(
                    px + dx + cdx,
                    py,
                    pz + dz + cdz,
                    jump,
                    height,
                    (dx, dz),
                    current,
                );
                if self.diagonal_valid(d) {
                    out.push(d.unwrap_or_default());
                }
            }
        }
        if let Evaluator::Amphibious { prefers_shallow } = self.eval {
            let up = self.find_accepted(px, py + 1, pz, (jump - 1).max(0), height, (0, 0), current);
            let down = self.find_accepted(px, py - 1, pz, jump, height, (0, 0), current);
            if self.neighbor_valid(up, pos)
                && up.is_some_and(|u| self.nodes[u as usize].kind == PathType::Water)
            {
                out.push(up.unwrap_or_default());
            }
            if self.neighbor_valid(down, pos)
                && down.is_some_and(|d| self.nodes[d as usize].kind == PathType::Water)
                && current != PathType::Trapdoor
            {
                out.push(down.unwrap_or_default());
            }
            if prefers_shallow {
                for &n in out.iter() {
                    let node = &mut self.nodes[n as usize];
                    if node.kind == PathType::Water && node.y < 63 - 10 {
                        node.cost_malus += 1.0;
                    }
                }
            }
        }
    }
    // --- swimming / flying neighbours ------------------------------------
    fn swim_accepted(&mut self, x: i32, y: i32, z: i32) -> Option<u32> {
        let breach = matches!(self.eval, Evaluator::Swim { breach: true });
        let t = self.cached(x, y, z);
        if (breach && t == PathType::Breach) || t == PathType::Water {
            let cost = self.malus(t);
            if cost >= 0.0 {
                let n = self.node_update_max(x, y, z, t, cost);
                if self.fluid(x, y, z) == Fluid::None {
                    self.nodes[n as usize].cost_malus += 8.0;
                }
                return Some(n);
            }
        }
        None
    }
    fn swim_neighbors(&mut self, pos: u32, out: &mut Vec<u32>) {
        let (px, py, pz) = {
            let n = &self.nodes[pos as usize];
            (n.x, n.y, n.z)
        };
        let open = |s: &Self, n: Option<u32>| n.is_some_and(|n| !s.nodes[n as usize].closed);
        let malus =
            |s: &Self, n: Option<u32>| n.is_some_and(|n| s.nodes[n as usize].cost_malus >= 0.0);
        // Direction.values(): down, up, north, south, west, east.
        let mut by_dir: FxHashMap<(i32, i32, i32), Option<u32>> = FxHashMap::default();
        for (dx, dy, dz) in [
            (0, -1, 0),
            (0, 1, 0),
            (0, 0, -1),
            (0, 0, 1),
            (-1, 0, 0),
            (1, 0, 0),
        ] {
            let n = self.swim_accepted(px + dx, py + dy, pz + dz);
            by_dir.insert((dx, dy, dz), n);
            if open(self, n) {
                out.push(n.unwrap_or_default());
            }
        }
        for ((dx, dz), (cx, cz)) in [
            ((0, -1), (1, 0)),
            ((1, 0), (0, 1)),
            ((0, 1), (-1, 0)),
            ((-1, 0), (0, -1)),
        ] {
            if malus(self, by_dir[&(dx, 0, dz)]) && malus(self, by_dir[&(cx, 0, cz)]) {
                let d = self.swim_accepted(px + dx + cx, py, pz + dz + cz);
                if open(self, d) {
                    out.push(d.unwrap_or_default());
                }
            }
        }
    }
    fn fly_accepted(&mut self, x: i32, y: i32, z: i32) -> Option<u32> {
        let t = self.cached(x, y, z);
        let cost = self.malus(t);
        if cost >= 0.0 {
            let n = self.node_update_max(x, y, z, t, cost);
            if t == PathType::Walkable {
                self.nodes[n as usize].cost_malus += 1.0;
            }
            return Some(n);
        }
        None
    }
    fn fly_neighbors(&mut self, pos: u32, out: &mut Vec<u32>) {
        let (px, py, pz) = {
            let n = &self.nodes[pos as usize];
            (n.x, n.y, n.z)
        };
        let f = |s: &mut Self, dx: i32, dy: i32, dz: i32| s.fly_accepted(px + dx, py + dy, pz + dz);
        let open = |s: &Self, n: Option<u32>| n.is_some_and(|n| !s.nodes[n as usize].closed);
        let m = |s: &Self, n: Option<u32>| n.is_some_and(|n| s.nodes[n as usize].cost_malus >= 0.0);
        let south = f(self, 0, 0, 1);
        let west = f(self, -1, 0, 0);
        let east = f(self, 1, 0, 0);
        let north = f(self, 0, 0, -1);
        let up = f(self, 0, 1, 0);
        let down = f(self, 0, -1, 0);
        for n in [south, west, east, north, up, down] {
            if open(self, n) {
                out.push(n.unwrap_or_default());
            }
        }
        let push = |s: &mut Self, n: Option<u32>, ok: bool, out: &mut Vec<u32>| {
            if open(s, n) && ok {
                out.push(n.unwrap_or_default());
            }
        };
        let south_up = f(self, 0, 1, 1);
        let ok = m(self, south) && m(self, up);
        push(self, south_up, ok, out);
        let west_up = f(self, -1, 1, 0);
        let ok = m(self, west) && m(self, up);
        push(self, west_up, ok, out);
        let east_up = f(self, 1, 1, 0);
        let ok = m(self, east) && m(self, up);
        push(self, east_up, ok, out);
        let north_up = f(self, 0, 1, -1);
        let ok = m(self, north) && m(self, up);
        push(self, north_up, ok, out);
        let south_down = f(self, 0, -1, 1);
        let ok = m(self, south) && m(self, down);
        push(self, south_down, ok, out);
        let west_down = f(self, -1, -1, 0);
        let ok = m(self, west) && m(self, down);
        push(self, west_down, ok, out);
        let east_down = f(self, 1, -1, 0);
        let ok = m(self, east) && m(self, down);
        push(self, east_down, ok, out);
        let north_down = f(self, 0, -1, -1);
        let ok = m(self, north) && m(self, down);
        push(self, north_down, ok, out);
        let north_east = f(self, 1, 0, -1);
        let ok = m(self, north) && m(self, east);
        push(self, north_east, ok, out);
        let south_east = f(self, 1, 0, 1);
        let ok = m(self, south) && m(self, east);
        push(self, south_east, ok, out);
        let north_west = f(self, -1, 0, -1);
        let ok = m(self, north) && m(self, west);
        push(self, north_west, ok, out);
        let south_west = f(self, -1, 0, 1);
        let ok = m(self, south) && m(self, west);
        push(self, south_west, ok, out);
        let neu = f(self, 1, 1, -1);
        let ok = m(self, north_east)
            && m(self, north)
            && m(self, east)
            && m(self, up)
            && m(self, north_up)
            && m(self, east_up);
        push(self, neu, ok, out);
        let seu = f(self, 1, 1, 1);
        let ok = m(self, south_east)
            && m(self, south)
            && m(self, east)
            && m(self, up)
            && m(self, south_up)
            && m(self, east_up);
        push(self, seu, ok, out);
        let nwu = f(self, -1, 1, -1);
        let ok = m(self, north_west)
            && m(self, north)
            && m(self, west)
            && m(self, up)
            && m(self, north_up)
            && m(self, west_up);
        push(self, nwu, ok, out);
        let swu = f(self, -1, 1, 1);
        let ok = m(self, south_west)
            && m(self, south)
            && m(self, west)
            && m(self, up)
            && m(self, south_up)
            && m(self, west_up);
        push(self, swu, ok, out);
        let ned = f(self, 1, -1, -1);
        let ok = m(self, north_east)
            && m(self, north)
            && m(self, east)
            && m(self, down)
            && m(self, north_down)
            && m(self, east_down);
        push(self, ned, ok, out);
        let sed = f(self, 1, -1, 1);
        let ok = m(self, south_east)
            && m(self, south)
            && m(self, east)
            && m(self, down)
            && m(self, south_down)
            && m(self, east_down);
        push(self, sed, ok, out);
        let nwd = f(self, -1, -1, -1);
        let ok = m(self, north_west)
            && m(self, north)
            && m(self, west)
            && m(self, down)
            && m(self, north_down)
            && m(self, west_down);
        push(self, nwd, ok, out);
        let swd = f(self, -1, -1, 1);
        let ok = m(self, south_west)
            && m(self, south)
            && m(self, west)
            && m(self, down)
            && m(self, south_down)
            && m(self, west_down);
        push(self, swd, ok, out);
    }
    // --- binary heap (vanilla BinaryHeap) --------------------------------
    fn heap_insert(&mut self, n: u32) {
        let i = self.heap.len();
        self.heap.push(n);
        self.nodes[n as usize].heap = i as i32;
        self.up_heap(i);
    }
    fn heap_pop(&mut self) -> u32 {
        let popped = self.heap[0];
        let last = self.heap.pop().unwrap_or(popped);
        if !self.heap.is_empty() {
            self.heap[0] = last;
            self.nodes[last as usize].heap = 0;
            self.down_heap(0);
        }
        self.nodes[popped as usize].heap = -1;
        popped
    }
    fn change_cost(&mut self, n: u32, cost: f32) {
        let old = self.nodes[n as usize].f;
        self.nodes[n as usize].f = cost;
        let i = self.nodes[n as usize].heap as usize;
        if cost < old {
            self.up_heap(i);
        } else {
            self.down_heap(i);
        }
    }
    fn up_heap(&mut self, mut i: usize) {
        let node = self.heap[i];
        let cost = self.nodes[node as usize].f;
        while i > 0 {
            let parent_i = (i - 1) >> 1;
            let parent = self.heap[parent_i];
            if cost >= self.nodes[parent as usize].f {
                break;
            }
            self.heap[i] = parent;
            self.nodes[parent as usize].heap = i as i32;
            i = parent_i;
        }
        self.heap[i] = node;
        self.nodes[node as usize].heap = i as i32;
    }
    fn down_heap(&mut self, mut i: usize) {
        let node = self.heap[i];
        let cost = self.nodes[node as usize].f;
        let size = self.heap.len();
        loop {
            let left_i = 1 + (i << 1);
            let right_i = left_i + 1;
            if left_i >= size {
                break;
            }
            let left = self.heap[left_i];
            let left_cost = self.nodes[left as usize].f;
            let (right, right_cost) = if right_i >= size {
                (u32::MAX, f32::INFINITY)
            } else {
                (
                    self.heap[right_i],
                    self.nodes[self.heap[right_i] as usize].f,
                )
            };
            if left_cost < right_cost {
                if left_cost >= cost {
                    break;
                }
                self.heap[i] = left;
                self.nodes[left as usize].heap = i as i32;
                i = left_i;
            } else {
                if right_cost >= cost {
                    break;
                }
                self.heap[i] = right;
                self.nodes[right as usize].heap = i as i32;
                i = right_i;
            }
        }
        self.heap[i] = node;
        self.nodes[node as usize].heap = i as i32;
    }
    // --- PathFinder.findPath ------------------------------------------------
    fn best_h(&self, n: u32, targets: &mut [Target]) -> f32 {
        let mut best = f32::MAX;
        let node = &self.nodes[n as usize];
        for t in targets.iter_mut() {
            let h = node.distance(t.x, t.y, t.z);
            if h < t.best_h {
                t.best_h = h;
                t.best = Some(n);
            }
            best = best.min(h);
        }
        best
    }
    /// PathFinder.findPath over several target positions.
    pub fn find(
        mut self,
        positions: &[[i32; 3]],
        max_path_length: f32,
        reach_range: i32,
        max_visited: i32,
    ) -> Option<Path> {
        let from = self.start()?;
        let mut targets: Vec<Target> = positions
            .iter()
            .map(|p| {
                // AmphibiousNodeEvaluator.getTarget floors y + 0.5 (the same block).
                let n = self.node(p[0], p[1], p[2]);
                let node = &self.nodes[n as usize];
                Target {
                    x: node.x,
                    y: node.y,
                    z: node.z,
                    best_h: f32::MAX,
                    best: None,
                    reached: false,
                    pos: *p,
                }
            })
            .collect();
        self.nodes[from as usize].g = 0.0;
        let h = self.best_h(from, &mut targets);
        self.nodes[from as usize].h = h;
        self.nodes[from as usize].f = h;
        self.heap.clear();
        self.heap_insert(from);
        let mut count = 0;
        let mut neighbors = Vec::with_capacity(32);
        let (fx, fy, fz) = {
            let n = &self.nodes[from as usize];
            (n.x, n.y, n.z)
        };
        while !self.heap.is_empty() {
            count += 1;
            if count >= max_visited {
                break;
            }
            let current = self.heap_pop();
            self.nodes[current as usize].closed = true;
            let mut any = false;
            for t in targets.iter_mut() {
                if self.nodes[current as usize].manhattan(t.x, t.y, t.z) <= reach_range as f32 {
                    t.reached = true;
                    any = true;
                }
            }
            if any {
                break;
            }
            if self.nodes[current as usize].distance(fx, fy, fz) >= max_path_length {
                continue;
            }
            neighbors.clear();
            match self.eval {
                Evaluator::Walk | Evaluator::Amphibious { .. } => {
                    self.walk_neighbors(current, &mut neighbors)
                }
                Evaluator::Swim { .. } => self.swim_neighbors(current, &mut neighbors),
                Evaluator::Fly => self.fly_neighbors(current, &mut neighbors),
            }
            for &neighbor in &neighbors {
                let (cx, cy, cz, cg, cw) = {
                    let c = &self.nodes[current as usize];
                    (c.x, c.y, c.z, c.g, c.walked)
                };
                let distance = self.nodes[neighbor as usize].distance(cx, cy, cz);
                let walked = cw + distance;
                self.nodes[neighbor as usize].walked = walked;
                let tentative = cg + distance + self.nodes[neighbor as usize].cost_malus;
                let in_open = self.nodes[neighbor as usize].heap >= 0;
                if walked < max_path_length
                    && (!in_open || tentative < self.nodes[neighbor as usize].g)
                {
                    self.nodes[neighbor as usize].came_from = Some(current);
                    self.nodes[neighbor as usize].g = tentative;
                    let h = self.best_h(neighbor, &mut targets) * 1.5;
                    self.nodes[neighbor as usize].h = h;
                    if in_open {
                        self.change_cost(neighbor, tentative + h);
                    } else {
                        self.nodes[neighbor as usize].f = tentative + h;
                        self.heap_insert(neighbor);
                    }
                }
            }
        }
        let reached: Vec<&Target> = targets.iter().filter(|t| t.reached).collect();
        let build = |s: &Self, t: &Target, reached: bool| -> Option<Path> {
            let mut n = t.best?;
            let mut nodes = vec![];
            loop {
                let node = &s.nodes[n as usize];
                nodes.push(PathNode {
                    x: node.x,
                    y: node.y,
                    z: node.z,
                    kind: node.kind,
                });
                match node.came_from {
                    Some(c) => n = c,
                    None => break,
                }
            }
            nodes.reverse();
            Some(Path::new(nodes, t.pos, reached))
        };
        if !reached.is_empty() {
            reached
                .iter()
                .filter_map(|t| build(&self, t, true))
                .min_by_key(|p| p.nodes.len())
        } else {
            targets
                .iter()
                .filter_map(|t| build(&self, t, false))
                .min_by(|a, b| {
                    a.dist_to_target
                        .total_cmp(&b.dist_to_target)
                        .then(a.nodes.len().cmp(&b.nodes.len()))
                })
        }
    }
}
