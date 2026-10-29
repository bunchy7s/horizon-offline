//! `PathNavigation` and its ground, wall-climbing, water-bound, flying and
//! amphibious variants: path creation, waypoint following, corner cutting,
//! stuck detection and path timeouts.
use super::{
    Mob, MobWorld,
    path::{Evaluator, Finder, Path, PathMob, PathType},
};
use horizon_core::geometry::Vec3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NavKind {
    #[default]
    Ground,
    WallClimber,
    Water {
        breach: bool,
    },
    Flying,
    Amphibious,
}
#[derive(Clone, Debug)]
pub struct Navigation {
    pub kind: NavKind,
    pub path: Option<Path>,
    pub speed_modifier: f64,
    tick: i64,
    last_stuck_check: i64,
    last_stuck_pos: Vec3,
    timeout_node: [i32; 3],
    timeout_timer: i64,
    last_timeout_check: i64,
    timeout_limit: f64,
    pub max_distance_to_waypoint: f32,
    delayed_recompute: bool,
    time_last_recompute: i64,
    pub target_pos: Option<[i32; 3]>,
    reach_range: i32,
    pub max_visited_multiplier: f32,
    pub stuck: bool,
    /// Set through [`Mob::set_required_path_length`].
    pub required_path_length: f32,
    /// `PathFinder.maxVisitedNodes`: the base follow range times 16 when the
    /// navigation is made, then [`Mob::update_max_visited_nodes`].
    pub max_visited_nodes: i32,
    pub can_float: bool,
    pub can_open_doors: bool,
    pub can_pass_doors: bool,
    pub can_walk_over_fences: bool,
    pub avoid_sun: bool,
    pub below_surface: bool,
    /// WallClimberNavigation.pathToPosition
    climb_to: Option<[i32; 3]>,
}
impl Default for Navigation {
    fn default() -> Self {
        Self {
            kind: NavKind::Ground,
            path: None,
            speed_modifier: 0.0,
            tick: 0,
            last_stuck_check: 0,
            last_stuck_pos: Vec3::ZERO,
            timeout_node: [0; 3],
            timeout_timer: 0,
            last_timeout_check: 0,
            timeout_limit: 0.0,
            max_distance_to_waypoint: 0.5,
            delayed_recompute: false,
            time_last_recompute: 0,
            target_pos: None,
            reach_range: 0,
            max_visited_multiplier: 1.0,
            stuck: false,
            required_path_length: 16.0,
            max_visited_nodes: 256,
            can_float: false,
            can_open_doors: false,
            can_pass_doors: true,
            can_walk_over_fences: false,
            avoid_sun: false,
            below_surface: false,
            climb_to: None,
        }
    }
}
impl Navigation {
    pub fn is_done(&self) -> bool {
        self.path.as_ref().is_none_or(Path::is_done)
    }
    pub fn is_in_progress(&self) -> bool {
        !self.is_done()
    }
    pub fn stop(&mut self) {
        self.path = None;
    }
    fn reset_stuck_timeout(&mut self) {
        self.timeout_node = [0; 3];
        self.timeout_timer = 0;
        self.timeout_limit = 0.0;
        self.stuck = false;
    }
    fn evaluator(&self) -> Evaluator {
        match self.kind {
            NavKind::Ground | NavKind::WallClimber => Evaluator::Walk,
            NavKind::Water { breach } => Evaluator::Swim { breach },
            NavKind::Flying => Evaluator::Fly,
            NavKind::Amphibious => Evaluator::Amphibious {
                prefers_shallow: false,
            },
        }
    }
    pub fn can_cut_corner(t: PathType) -> bool {
        !matches!(
            t,
            PathType::FireInNeighbor | PathType::DamagingInNeighbor | PathType::WalkableDoor
        )
    }
}
impl Mob {
    pub fn path_mob(&self) -> PathMob {
        PathMob {
            position: self.position,
            width: self.width(),
            height: self.height(),
            on_ground: self.on_ground,
            in_water: self.fluids.in_water,
            max_up_step: self.kind.step_height as f32,
            max_fall_distance: self.max_fall_distance(),
            malus: self.ai.malus,
            can_float: self.ai.nav.can_float,
            can_open_doors: self.ai.nav.can_open_doors,
            can_pass_doors: self.ai.nav.can_pass_doors,
            can_walk_over_fences: self.ai.nav.can_walk_over_fences,
            stands_on_lava: self.kind.flags.lava_walker,
        }
    }
    /// Mob.getMaxFallDistance (normal difficulty).
    pub fn max_fall_distance(&self) -> i32 {
        if self.ai.target.is_none() {
            return 3;
        }
        let mut sacrifice = (self.health - self.attrs.max_health * 0.33) as i32;
        sacrifice -= 4;
        sacrifice.max(0) + 3
    }
    pub fn malus(&self, t: PathType) -> f32 {
        self.ai.malus[t as usize].unwrap_or_else(|| t.malus())
    }
    pub fn set_malus(&mut self, t: PathType, v: f32) {
        self.ai.malus[t as usize] = Some(v);
    }
    fn max_path_length(&self) -> f32 {
        (self.attrs.follow_range as f32).max(self.ai.nav.required_path_length)
    }
    /// `PathNavigation.updatePathfinderMaxVisitedNodes`: after a follow range
    /// change and a new required path length.
    pub fn update_max_visited_nodes(&mut self) {
        self.ai.nav.max_visited_nodes = (self.max_path_length() * 16.0).floor() as i32;
    }
    /// `PathNavigation.setRequiredPathLength`.
    pub fn set_required_path_length(&mut self, length: f32) {
        self.ai.nav.required_path_length = length;
        self.update_max_visited_nodes();
    }
    fn can_update_path(&self) -> bool {
        match self.ai.nav.kind {
            NavKind::Ground | NavKind::WallClimber => self.on_ground || self.is_in_liquid(),
            NavKind::Water { breach } => breach || self.is_in_liquid(),
            NavKind::Flying | NavKind::Amphibious => true,
        }
    }
    fn temp_mob_pos(&self, w: &MobWorld) -> Vec3 {
        match self.ai.nav.kind {
            NavKind::Ground | NavKind::WallClimber => Vec3::new(
                self.position.x,
                f64::from(self.surface_y(w)),
                self.position.z,
            ),
            NavKind::Water { .. } | NavKind::Amphibious => Vec3::new(
                self.position.x,
                self.position.y + f64::from(self.height()) * 0.5,
                self.position.z,
            ),
            NavKind::Flying => self.position,
        }
    }
    fn surface_y(&self, w: &MobWorld) -> i32 {
        let (x, z) = (
            self.position.x.floor() as i32,
            self.position.z.floor() as i32,
        );
        if self.fluids.in_water && self.ai.nav.can_float {
            let mut y = self.position.y.floor() as i32;
            let start = y;
            let mut steps = 0;
            while w.facts.get(w.world.block(x, y, z)).fluid == crate::blocks::Fluid::Water {
                y += 1;
                steps += 1;
                if steps > 16 {
                    return start;
                }
            }
            y
        } else {
            (self.position.y + 0.5).floor() as i32
        }
    }
    /// GroundPathNavigation.findSurfacePosition.
    fn surface_position(&self, w: &MobWorld, mut p: [i32; 3]) -> [i32; 3] {
        let air = |p: [i32; 3]| w.facts.is_air(w.world.block(p[0], p[1], p[2]));
        let solid = |p: [i32; 3]| w.solid(p[0], p[1], p[2]);
        if air(p) {
            let mut q = [p[0], p[1] - 1, p[2]];
            while q[1] >= crate::chunk::MIN_Y && air(q) {
                q[1] -= 1;
            }
            if q[1] >= crate::chunk::MIN_Y {
                return [q[0], q[1] + 1, q[2]];
            }
            q[1] = p[1] + 1;
            while q[1] <= 319 && air(q) {
                q[1] += 1;
            }
            p = q;
        }
        if !solid(p) {
            return p;
        }
        let mut q = [p[0], p[1] + 1, p[2]];
        while q[1] <= 319 && solid(q) {
            q[1] += 1;
        }
        q
    }
    pub(super) fn create_path_raw(
        &mut self,
        w: &MobWorld,
        targets: &[[i32; 3]],
        reach: i32,
        max_len: f32,
    ) -> Option<Path> {
        if targets.is_empty()
            || self.position.y < f64::from(crate::chunk::MIN_Y)
            || !self.can_update_path()
        {
            return None;
        }
        if let (Some(path), Some(tp)) = (&self.ai.nav.path, self.ai.nav.target_pos)
            && !path.is_done()
            && targets.contains(&tp)
        {
            return Some(path.clone());
        }
        let pm = self.path_mob();
        let max_visited =
            (self.ai.nav.max_visited_nodes as f32 * self.ai.nav.max_visited_multiplier) as i32;
        let finder = Finder::new(w, &pm, self.ai.nav.evaluator());
        let path = finder.find(targets, max_len, reach, max_visited);
        if let Some(p) = &path {
            self.ai.nav.target_pos = Some(p.target);
            self.ai.nav.reach_range = reach;
            self.ai.nav.reset_stuck_timeout();
        }
        path
    }
    /// PathNavigation.createPath(BlockPos, reach) with the ground surface search.
    pub fn create_path_to(&mut self, w: &MobWorld, mut p: [i32; 3], reach: i32) -> Option<Path> {
        match self.ai.nav.kind {
            NavKind::Ground | NavKind::WallClimber => {
                if !w.world.loaded(p[0], p[2]) {
                    return None;
                }
                if self.ai.nav.kind == NavKind::WallClimber {
                    self.ai.nav.climb_to = Some(p);
                }
                if !self.ai.nav.below_surface {
                    p = self.surface_position(w, p);
                }
            }
            _ => {}
        }
        let max = self.max_path_length();
        self.create_path_raw(w, &[p], reach, max)
    }
    /// PathNavigation.createPath(Set<BlockPos>, reach): the nearest of
    /// several targets, without the ground surface search.
    pub fn create_path_to_any(
        &mut self,
        w: &MobWorld,
        targets: &[[i32; 3]],
        reach: i32,
    ) -> Option<Path> {
        let max = self.max_path_length();
        self.create_path_raw(w, targets, reach, max)
    }
    /// PathNavigation.createPath(Entity, reach).
    pub fn create_path_to_entity(
        &mut self,
        w: &MobWorld,
        at: [f64; 3],
        reach: i32,
    ) -> Option<Path> {
        let p = [
            at[0].floor() as i32,
            at[1].floor() as i32,
            at[2].floor() as i32,
        ];
        self.create_path_to(w, p, reach)
    }
    pub fn nav_move_to_path(&mut self, path: Option<Path>, speed: f64) -> bool {
        let Some(path) = path else {
            self.ai.nav.path = None;
            return false;
        };
        if !path.same_as(self.ai.nav.path.as_ref()) {
            self.ai.nav.path = Some(path);
        }
        if self.ai.nav.is_done() {
            return false;
        }
        if self.ai.nav.path.as_ref().is_none_or(|p| p.nodes.is_empty()) {
            return false;
        }
        self.ai.nav.speed_modifier = speed;
        self.ai.nav.last_stuck_check = self.ai.nav.tick;
        // getTempMobPos needs the world for ground navigation; position is close enough
        // for the first stuck sample and is refreshed 100 ticks later.
        self.ai.nav.last_stuck_pos = self.position;
        true
    }
    /// PathNavigation.moveTo(x, y, z, speed): a path to the containing block.
    pub fn nav_move_to(&mut self, w: &MobWorld, at: [f64; 3], speed: f64) -> bool {
        let p = [
            at[0].floor() as i32,
            at[1].floor() as i32,
            at[2].floor() as i32,
        ];
        let path = self.create_path_to(w, p, 1);
        self.nav_move_to_path(path, speed)
    }
    pub fn nav_move_to_entity(
        &mut self,
        w: &MobWorld,
        at: [f64; 3],
        reach: i32,
        speed: f64,
    ) -> bool {
        let path = self.create_path_to_entity(w, at, reach);
        if self.ai.nav.kind == NavKind::WallClimber && path.is_none() {
            // WallClimberNavigation.moveTo(Entity): climb straight towards it.
            self.ai.nav.climb_to = Some([
                at[0].floor() as i32,
                at[1].floor() as i32,
                at[2].floor() as i32,
            ]);
            self.ai.nav.speed_modifier = speed;
            return true;
        }
        path.is_some() && self.nav_move_to_path(path, speed)
    }
    pub fn nav_stop(&mut self) {
        self.ai.nav.path = None;
    }
    fn recompute_path(&mut self, w: &MobWorld) {
        if w.game_time - self.ai.nav.time_last_recompute <= 20 || !self.can_update_path() {
            self.ai.nav.delayed_recompute = true;
        } else if let Some(t) = self.ai.nav.target_pos {
            self.ai.nav.path = None;
            let reach = self.ai.nav.reach_range;
            self.ai.nav.path = self.create_path_to(w, t, reach);
            self.ai.nav.time_last_recompute = w.game_time;
            self.ai.nav.delayed_recompute = false;
        }
    }
    /// PathNavigation.tick.
    pub fn nav_tick(&mut self, w: &MobWorld) {
        self.ai.nav.tick += 1;
        if self.ai.nav.delayed_recompute {
            self.recompute_path(w);
        }
        if self.ai.nav.kind == NavKind::WallClimber && self.ai.nav.is_done() {
            if let Some(t) = self.ai.nav.climb_to {
                let width = f64::from(self.width());
                let center = |p: [i32; 3]| {
                    let (dx, dy, dz) = (
                        f64::from(p[0]) + 0.5 - self.position.x,
                        f64::from(p[1]) + 0.5 - self.position.y,
                        f64::from(p[2]) + 0.5 - self.position.z,
                    );
                    dx * dx + dy * dy + dz * dz < width * width
                };
                let same_level = [t[0], self.position.y.floor() as i32, t[2]];
                if !center(t) && (self.position.y <= f64::from(t[1]) || !center(same_level)) {
                    let speed = self.ai.nav.speed_modifier;
                    self.ai.mov.set_wanted(
                        f64::from(t[0]),
                        f64::from(t[1]),
                        f64::from(t[2]),
                        speed,
                    );
                } else {
                    self.ai.nav.climb_to = None;
                }
            }
            return;
        }
        if self.ai.nav.is_done() {
            return;
        }
        if self.can_update_path() {
            self.follow_the_path(w);
        } else if let Some(path) = &self.ai.nav.path
            && !path.is_done()
        {
            let mob = self.temp_mob_pos(w);
            let pos = path.entity_pos_at(path.next, self.width());
            let advance = match self.ai.nav.kind {
                NavKind::Flying => {
                    self.position.x.floor() == pos.x.floor()
                        && self.position.y.floor() == pos.y.floor()
                        && self.position.z.floor() == pos.z.floor()
                }
                _ => {
                    mob.y > pos.y
                        && !self.on_ground
                        && mob.x.floor() == pos.x.floor()
                        && mob.z.floor() == pos.z.floor()
                }
            };
            if advance && let Some(p) = &mut self.ai.nav.path {
                p.advance();
            }
        }
        if let Some(path) = &self.ai.nav.path
            && !path.is_done()
        {
            let target = path.entity_pos_at(path.next, self.width());
            let y = match self.ai.nav.kind {
                NavKind::Ground | NavKind::WallClimber => {
                    let (bx, by, bz) = (
                        target.x.floor() as i32,
                        target.y.floor() as i32,
                        target.z.floor() as i32,
                    );
                    if w.facts.is_air(w.world.block(bx, by - 1, bz)) {
                        target.y
                    } else {
                        w.floor_level(bx, by, bz)
                    }
                }
                _ => target.y,
            };
            let speed = self.ai.nav.speed_modifier;
            self.ai.mov.set_wanted(target.x, y, target.z, speed);
        }
    }
    fn follow_the_path(&mut self, w: &MobWorld) {
        let mob_pos = self.temp_mob_pos(w);
        let width = self.width();
        self.ai.nav.max_distance_to_waypoint = if width > 0.75 {
            width / 2.0
        } else {
            0.75 - width / 2.0
        };
        let vertical = match self.ai.nav.kind {
            NavKind::Water { .. } => 0.5,
            _ => 1.0,
        };
        let Some(path) = &self.ai.nav.path else {
            return;
        };
        let n = path.next_node();
        let xd = (self.position.x - (f64::from(n.x) + 0.5)).abs();
        let yd = (self.position.y - f64::from(n.y)).abs();
        let zd = (self.position.z - (f64::from(n.z) + 0.5)).abs();
        let max = f64::from(self.ai.nav.max_distance_to_waypoint);
        let close = xd < max && zd < max && yd < vertical;
        if (close || Navigation::can_cut_corner(n.kind) && self.should_target_next_node(w, mob_pos))
            && let Some(p) = &mut self.ai.nav.path
        {
            p.advance();
        }
        self.do_stuck_detection(w, mob_pos);
    }
    // Vanilla's NaN-sensitive `!(a < b)` form.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    fn should_target_next_node(&self, w: &MobWorld, mob: Vec3) -> bool {
        let Some(path) = &self.ai.nav.path else {
            return false;
        };
        if path.next + 1 >= path.nodes.len() {
            return false;
        }
        let n = path.nodes[path.next];
        let current = Vec3::new(f64::from(n.x) + 0.5, f64::from(n.y), f64::from(n.z) + 0.5);
        let d = |a: Vec3, b: Vec3| (a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2);
        if d(mob, current) >= 4.0 {
            return false;
        }
        if self.can_move_directly(w, mob, path.entity_pos_at(path.next, self.width())) {
            return true;
        }
        let m = path.nodes[path.next + 1];
        let next = Vec3::new(f64::from(m.x) + 0.5, f64::from(m.y), f64::from(m.z) + 0.5);
        let to_current = Vec3::new(current.x - mob.x, current.y - mob.y, current.z - mob.z);
        let to_next = Vec3::new(next.x - mob.x, next.y - mob.y, next.z - mob.z);
        let (cs, ns) = (to_current.length_squared(), to_next.length_squared());
        if !(ns < cs) && !(cs < 0.5) {
            return false;
        }
        let a = to_current.normalize();
        let b = to_next.normalize();
        a.x * b.x + a.y * b.y + a.z * b.z < 0.0
    }
    fn can_move_directly(&self, w: &MobWorld, from: Vec3, to: Vec3) -> bool {
        match self.ai.nav.kind {
            NavKind::Flying => w.clear_between(
                from,
                Vec3::new(to.x, to.y + f64::from(self.height()) * 0.5, to.z),
                true,
            ),
            NavKind::Water { .. } => w.clear_between(
                from,
                Vec3::new(to.x, to.y + f64::from(self.height()) * 0.5, to.z),
                false,
            ),
            NavKind::Amphibious if self.is_in_liquid() => w.clear_between(
                from,
                Vec3::new(to.x, to.y + f64::from(self.height()) * 0.5, to.z),
                false,
            ),
            _ => false,
        }
    }
    fn do_stuck_detection(&mut self, w: &MobWorld, mob: Vec3) {
        let speed = self.ai.speed;
        let nav = &mut self.ai.nav;
        if nav.tick - nav.last_stuck_check > 100 {
            let effective = if speed >= 1.0 { speed } else { speed * speed };
            let threshold = f64::from(effective * 100.0 * 0.25);
            let d = (mob.x - nav.last_stuck_pos.x).powi(2)
                + (mob.y - nav.last_stuck_pos.y).powi(2)
                + (mob.z - nav.last_stuck_pos.z).powi(2);
            if d < threshold * threshold {
                nav.stuck = true;
                nav.path = None;
            } else {
                nav.stuck = false;
            }
            nav.last_stuck_check = nav.tick;
            nav.last_stuck_pos = mob;
        }
        if let Some(path) = &nav.path
            && !path.is_done()
        {
            let n = path.next_node();
            let pos = [n.x, n.y, n.z];
            if pos == nav.timeout_node {
                nav.timeout_timer += w.game_time - nav.last_timeout_check;
            } else {
                nav.timeout_node = pos;
                let c = Vec3::new(f64::from(n.x) + 0.5, f64::from(n.y), f64::from(n.z) + 0.5);
                let dist =
                    ((mob.x - c.x).powi(2) + (mob.y - c.y).powi(2) + (mob.z - c.z).powi(2)).sqrt();
                nav.timeout_limit = if speed > 0.0 {
                    dist / f64::from(speed) * 20.0
                } else {
                    0.0
                };
            }
            if nav.timeout_limit > 0.0 && nav.timeout_timer as f64 > nav.timeout_limit * 3.0 {
                nav.reset_stuck_timeout();
                nav.path = None;
            }
            nav.last_timeout_check = w.game_time;
        }
    }
    /// PathNavigation.isStableDestination for the mob's navigation.
    pub fn stable_destination(&self, w: &MobWorld, p: [i32; 3]) -> bool {
        if self.species() == super::Species::Bee {
            return super::bee::stable_destination(w, p);
        }
        if self.species() == super::Species::Turtle
            && let Some(stable) = super::turtle::stable_destination(self, w, p)
        {
            return stable;
        }
        // StriderPathNavigation: lava too.
        if self.kind.flags.lava_walker && w.block_name(p[0], p[1], p[2]) == "minecraft:lava" {
            return true;
        }
        match self.ai.nav.kind {
            NavKind::Ground | NavKind::WallClimber => {
                w.facts
                    .get(w.world.block(p[0], p[1] - 1, p[2]))
                    .solid_render
            }
            NavKind::Water { .. } => !w.facts.get(w.world.block(p[0], p[1], p[2])).solid_render,
            NavKind::Flying => {
                w.data.state(w.world.block(p[0], p[1], p[2])).flags & super::data::STURDY_TOP != 0
            }
            NavKind::Amphibious => !w.facts.is_air(w.world.block(p[0], p[1] - 1, p[2])),
        }
    }
}
