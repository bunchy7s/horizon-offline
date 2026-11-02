use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap},
};

pub(crate) type Node = [i32; 3];

#[derive(Clone, Copy, Default)]
pub(crate) struct Cell {
    pub solid: bool,
    pub fluid: bool,
    pub lava: bool,
}

#[derive(Debug)]
pub(crate) struct Plan {
    pub nodes: Vec<Node>,
    pub complete: bool,
}

fn solid(query: &impl Fn(Node) -> Option<Cell>, n: Node) -> bool {
    query(n).is_none_or(|b| b.solid)
}

fn walkable(query: &impl Fn(Node) -> Option<Cell>, n: Node) -> bool {
    let Some(feet) = query(n) else {
        return false;
    };
    let Some(below) = query([n[0], n[1] - 1, n[2]]) else {
        return false;
    };
    let Some(head) = query([n[0], n[1] + 1, n[2]]) else {
        return false;
    };
    (below.solid || below.fluid) && !feet.solid && !head.solid && !feet.lava && !below.lava
}

pub(crate) fn find_path(
    from: Node,
    goal: [f64; 3],
    near: f64,
    blocked: &[(Node, Node)],
    query: impl Fn(Node) -> Option<Cell>,
) -> Option<Plan> {
    let reached = |n: Node| {
        let c = [
            f64::from(n[0]) + 0.5,
            f64::from(n[1]),
            f64::from(n[2]) + 0.5,
        ];
        ((c[0] - goal[0]).powi(2) + (c[2] - goal[2]).powi(2)).sqrt() <= near.max(0.5)
            && (c[1] - goal[1]).abs() <= near.max(1.0) + 0.5
    };
    let h = |n: Node| {
        ((f64::from(n[0]) + 0.5 - goal[0]).abs()
            + (f64::from(n[1]) - goal[1]).abs()
            + (f64::from(n[2]) + 0.5 - goal[2]).abs())
            * 10.0
    };
    let mut open = BinaryHeap::new();
    let mut cost: HashMap<Node, u32> = HashMap::new();
    let mut came: HashMap<Node, Node> = HashMap::new();
    cost.insert(from, 0);
    open.push(Reverse((h(from) as u32, from)));
    let distance = |n: Node| {
        (f64::from(n[0]) + 0.5 - goal[0]).powi(2)
            + (f64::from(n[1]) - goal[1]).powi(2)
            + (f64::from(n[2]) + 0.5 - goal[2]).powi(2)
    };
    let mut best = from;
    let mut expanded = 0;
    while let Some(Reverse((_, n))) = open.pop() {
        if reached(n) {
            return Some(reconstruct(n, &came, true));
        }
        if distance(n) < distance(best) {
            best = n;
        }
        expanded += 1;
        if expanded >= 30_000 {
            break;
        }
        let base = cost[&n];
        for d in [[1, 0], [-1, 0], [0, 1], [0, -1]] {
            for dy in [0, 1, -1, -2, -3] {
                let m = [n[0] + d[0], n[1] + dy, n[2] + d[1]];
                // A step up needs head room above the start; a drop needs
                // the column in front clear down to the landing.
                let clear = match dy {
                    1 => !solid(&query, [n[0], n[1] + 2, n[2]]),
                    d if d < 0 => (m[1]..=n[1] + 1).all(|y| !solid(&query, [m[0], y, m[2]])),
                    _ => true,
                };
                if !clear || !walkable(&query, m) {
                    continue;
                }
                if blocked
                    .iter()
                    .any(|&(a, b)| (a == n && b == m) || (a == m && b == n))
                {
                    break;
                }
                let water = query(m).is_some_and(|b| b.fluid);
                let step = 10 + if dy != 0 { 5 } else { 0 } + if water { 30 } else { 0 };
                let c = base + step;
                if cost.get(&m).is_none_or(|&old| c < old) {
                    cost.insert(m, c);
                    came.insert(m, n);
                    open.push(Reverse((c + h(m) as u32, m)));
                }
                break;
            }
        }
    }
    (best != from && distance(best) + 1e-6 < distance(from))
        .then(|| reconstruct(best, &came, false))
}

fn reconstruct(end: Node, came: &HashMap<Node, Node>, complete: bool) -> Plan {
    let mut nodes = vec![end];
    let mut at = end;
    while let Some(&previous) = came.get(&at) {
        nodes.push(previous);
        at = previous;
    }
    nodes.reverse();
    Plan { nodes, complete }
}

impl Plan {
    pub(crate) fn aim(&self, p: [f64; 3], next: &mut usize, target: [f64; 3]) -> Option<[f64; 3]> {
        while *next < self.nodes.len() {
            let n = self.nodes[*next];
            let c = [
                f64::from(n[0]) + 0.5,
                f64::from(n[1]),
                f64::from(n[2]) + 0.5,
            ];
            let last = *next + 1 == self.nodes.len();
            if ((p[0] - c[0]).powi(2) + (p[2] - c[2]).powi(2)).sqrt()
                < if last { 0.3 } else { 0.45 }
                && (p[1] - c[1]).abs() < 1.1
            {
                *next += 1;
            } else {
                return Some(c);
            }
        }
        self.complete.then_some(target)
    }
}

pub(crate) fn arrived(p: [f64; 3], target: [f64; 3], near: f64) -> bool {
    ((p[0] - target[0]).powi(2) + (p[2] - target[2]).powi(2)).sqrt() <= near
        && (p[1] - target[1]).abs() <= near.max(1.0) + 1.0
}

fn horizontal_distance(p: [f64; 3], aim: [f64; 3]) -> f64 {
    ((p[0] - aim[0]).powi(2) + (p[2] - aim[2]).powi(2)).sqrt()
}

pub(crate) struct Progress {
    best_distance: f64,
}
impl Progress {
    pub(crate) fn new(position: [f64; 3], aim: [f64; 3]) -> Self {
        Self {
            best_distance: horizontal_distance(position, aim),
        }
    }
    pub(crate) fn advance(&mut self, p: [f64; 3], aim: [f64; 3]) -> bool {
        let distance = horizontal_distance(p, aim);
        if self.best_distance - distance >= 0.25 {
            self.best_distance = distance;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Terrain {
        floors: HashMap<(i32, i32), i32>,
        cells: HashMap<Node, Cell>,
    }
    impl Terrain {
        fn flat(x: std::ops::RangeInclusive<i32>, z: std::ops::RangeInclusive<i32>) -> Self {
            let mut t = Self::default();
            for x in x {
                for z in z.clone() {
                    t.floors.insert((x, z), 63);
                }
            }
            t
        }
        fn query(&self, n: Node) -> Option<Cell> {
            let floor = self.floors.get(&(n[0], n[2]))?;
            Some(self.cells.get(&n).copied().unwrap_or(Cell {
                solid: n[1] <= *floor,
                ..Cell::default()
            }))
        }
        fn path(&self, from: Node, goal: [f64; 3], blocked: &[(Node, Node)]) -> Option<Plan> {
            find_path(from, goal, 0.6, blocked, |n| self.query(n))
        }
    }

    #[test]
    fn short_path_is_complete() {
        let p = Terrain::flat(0..=5, 0..=0)
            .path([0, 64, 0], [5.5, 64.0, 0.5], &[])
            .unwrap();
        assert!(p.complete);
        assert_eq!(p.nodes.last(), Some(&[5, 64, 0]));
    }

    #[test]
    fn loaded_frontier_continues_after_chunks_extend() {
        let mut t = Terrain::flat(0..=5, 0..=0);
        let goal = [10.5, 64.0, 0.5];
        let partial = t
            .path([0, 64, 0], goal, &[])
            .expect("loaded frontier should be usable");
        assert!(!partial.complete);
        assert_eq!(partial.nodes.last(), Some(&[5, 64, 0]));
        let endpoint = *partial.nodes.last().unwrap();
        assert!(
            t.path(endpoint, goal, &[]).is_none(),
            "unchanged frontier must stop"
        );
        for x in 6..=10 {
            t.floors.insert((x, 0), 63);
        }
        let complete = t.path(endpoint, goal, &[]).unwrap();
        assert!(complete.complete);
        assert_eq!(complete.nodes.last(), Some(&[10, 64, 0]));
    }

    #[test]
    fn no_closer_endpoint_fails() {
        assert!(
            Terrain::flat(-4..=0, 0..=0)
                .path([0, 64, 0], [10.5, 64.0, 0.5], &[])
                .is_none()
        );
    }

    #[test]
    fn complete_path_can_initially_move_away() {
        let mut t = Terrain::flat(-2..=0, 0..=0);
        for z in 0..=2 {
            t.floors.insert((-2, z), 63);
        }
        for x in -2..=5 {
            t.floors.insert((x, 2), 63);
        }
        for z in 0..=2 {
            t.floors.insert((5, z), 63);
        }
        let p = t.path([0, 64, 0], [5.5, 64.0, 0.5], &[]).unwrap();
        assert!(p.complete);
        assert_eq!(p.nodes[1], [-1, 64, 0]);
    }

    #[test]
    fn failed_edge_uses_bypass() {
        let blocked = [([0, 64, 0], [1, 64, 0])];
        let p = Terrain::flat(0..=3, 0..=1)
            .path([0, 64, 0], [3.5, 64.0, 0.5], &blocked)
            .unwrap();
        assert!(p.complete);
        assert!(
            !p.nodes
                .windows(2)
                .any(|w| w == [blocked[0].0, blocked[0].1])
        );
        assert_eq!(p.nodes[1], [0, 64, 1]);
    }

    #[test]
    fn failed_edge_without_bypass_stops() {
        assert!(
            Terrain::flat(0..=1, 0..=0)
                .path([0, 64, 0], [1.5, 64.0, 0.5], &[([0, 64, 0], [1, 64, 0])])
                .is_none()
        );
    }

    #[test]
    fn lava_and_unknown_terrain_are_not_walked() {
        let mut t = Terrain::flat(0..=1, 0..=0);
        t.cells.insert(
            [1, 63, 0],
            Cell {
                lava: true,
                fluid: true,
                solid: false,
            },
        );
        assert!(t.path([0, 64, 0], [1.5, 64.0, 0.5], &[]).is_none());
        t.cells.remove(&[1, 63, 0]);
        t.cells.insert(
            [1, 64, 0],
            Cell {
                lava: true,
                fluid: true,
                solid: false,
            },
        );
        assert!(t.path([0, 64, 0], [1.5, 64.0, 0.5], &[]).is_none());
        assert!(
            Terrain::flat(0..=0, 0..=0)
                .path([0, 64, 0], [5.5, 64.0, 0.5], &[])
                .is_none()
        );
    }

    #[test]
    fn one_step_up_and_three_step_drop_only_with_clearance() {
        let mut t = Terrain::flat(0..=1, 0..=0);
        t.floors.insert((1, 0), 64);
        assert!(t.path([0, 64, 0], [1.5, 65.0, 0.5], &[]).unwrap().complete);
        t.cells.insert(
            [0, 66, 0],
            Cell {
                solid: true,
                ..Cell::default()
            },
        );
        assert!(t.path([0, 64, 0], [1.5, 65.0, 0.5], &[]).is_none());
        t.cells.clear();
        t.floors.insert((1, 0), 65);
        assert!(t.path([0, 64, 0], [1.5, 66.0, 0.5], &[]).is_none());
        t.floors.insert((1, 0), 60);
        assert!(t.path([0, 64, 0], [1.5, 61.0, 0.5], &[]).unwrap().complete);
        t.cells.insert(
            [1, 64, 0],
            Cell {
                solid: true,
                ..Cell::default()
            },
        );
        assert!(t.path([0, 64, 0], [1.5, 61.0, 0.5], &[]).is_none());
        t.cells.clear();
        t.floors.insert((1, 0), 59);
        assert!(t.path([0, 64, 0], [1.5, 60.0, 0.5], &[]).is_none());
    }

    #[test]
    fn dry_bypass_costs_less_than_water() {
        let mut t = Terrain::flat(0..=2, 0..=1);
        t.cells.insert(
            [1, 64, 0],
            Cell {
                fluid: true,
                ..Cell::default()
            },
        );
        let p = t.path([0, 64, 0], [2.5, 64.0, 0.5], &[]).unwrap();
        assert!(p.complete);
        assert!(!p.nodes.contains(&[1, 64, 0]));
    }

    #[test]
    fn search_budget_returns_progress_instead_of_discarding_it() {
        let p = Terrain::flat(0..=30_010, 0..=0)
            .path([0, 64, 0], [40_000.5, 64.0, 0.5], &[])
            .unwrap();
        assert!(!p.complete);
        assert!(p.nodes.len() > 2 && p.nodes.len() <= 30_001);
    }
    #[test]
    fn consumed_partial_never_aims_at_unknown_goal_or_arrives() {
        let plan = Plan {
            nodes: vec![[0, 64, 0], [1, 64, 0]],
            complete: false,
        };
        let p = [1.5, 64.0, 0.5];
        let goal = [10.5, 64.0, 0.5];
        let mut next = 1;
        assert_eq!(plan.aim(p, &mut next, goal), None);
        assert!(!arrived(p, goal, 0.6));
    }

    #[test]
    fn replacement_path_uses_its_new_waypoint() {
        let plan = Plan {
            nodes: vec![[0, 64, 0], [0, 64, 1], [1, 64, 1]],
            complete: false,
        };
        assert_eq!(
            plan.aim([0.5, 64.0, 0.5], &mut 1, [5.5, 64.0, 0.5]),
            Some([0.5, 64.0, 1.5])
        );
        assert!(arrived([5.5, 64.0, 0.5], [5.5, 64.0, 0.5], 0.6));
    }
    #[test]
    fn jumping_in_place_does_not_reset_stuck_timer() {
        let start = [0.5, 64.0, 0.5];
        let aim = [4.5, 64.0, 0.5];
        let mut progress = Progress::new(start, aim);
        assert!(!progress.advance([0.5, 65.0, 0.5], aim));
        assert!(!progress.advance([0.6, 64.0, 0.6], aim));
        assert!(progress.advance([0.8, 64.0, 0.5], aim));
    }
    #[test]
    fn oscillation_and_vertical_drift_do_not_count_as_progress() {
        let aim = [4.5, 64.0, 0.5];
        let mut progress = Progress::new([0.5, 64.0, 0.5], aim);
        for tick in 0..400 {
            let x = if tick % 2 == 0 { 0.2 } else { 0.5 };
            assert!(
                !progress.advance([x, 64.0, 0.5], aim),
                "oscillation reset at {tick}"
            );
        }
        assert!(!progress.advance([0.5, 66.0, 0.5], aim));
    }

    #[test]
    fn only_new_best_distance_counts_and_waypoint_switch_resets_baseline() {
        let aim = [4.5, 64.0, 0.5];
        let start = [0.5, 64.0, 0.5];
        let mut progress = Progress::new(start, aim);
        assert!(!progress.advance([0.6, 64.0, 0.5], aim));
        assert!(progress.advance([0.8, 64.0, 0.5], aim));
        assert!(!progress.advance(start, aim));
        assert!(!progress.advance([0.8, 64.0, 0.5], aim));
        assert!(progress.advance([1.1, 64.0, 0.5], aim));
        let detour_aim = [1.1, 64.0, 2.5];
        let mut progress = Progress::new([1.1, 64.0, 0.5], detour_aim);
        assert!(progress.advance([1.1, 64.0, 0.8], detour_aim));
    }
}
