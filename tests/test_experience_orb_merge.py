"""Run the real orb merge code (spawn and tick) with a fake mod."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from test_world_input_capture import method

ROOT = Path(__file__).resolve().parents[1]

FIXTURE = r"""
#[derive(Clone, Copy)] struct Vec3 { x: f64, y: f64, z: f64 }
impl Vec3 {
    fn new(x: f64, y: f64, z: f64) -> Self { Vec3 { x, y, z } }
    fn subtract(self, o: Vec3) -> Vec3 { Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z) }
    fn add_vector(self, o: Vec3) -> Vec3 { Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z) }
}
#[derive(Clone, Copy)] struct Aabb { lo: Vec3, hi: Vec3 }
impl Aabb {
    fn new(lo: Vec3, hi: Vec3) -> Self { Aabb { lo, hi } }
    fn inflate(self, d: Vec3) -> Aabb { Aabb::new(self.lo.subtract(d), self.hi.add_vector(d)) }
    fn intersects(self, o: Aabb) -> bool {
        self.lo.x < o.hi.x && self.hi.x > o.lo.x && self.lo.y < o.hi.y && self.hi.y > o.lo.y
            && self.lo.z < o.hi.z && self.hi.z > o.lo.z
    }
}
struct ExperienceOrb { id: u32, x: f64, value: u32, count: u32, age: u32, ticks: u32 }
impl ExperienceOrb {
    fn bounds(&self) -> Aabb {
        Aabb::new(Vec3::new(self.x - 0.25, 0.0, -0.25), Vec3::new(self.x + 0.25, 0.5, 0.25))
    }
    fn can_merge(&self, id: u32, value: u32) -> bool {
        self.value == value && (self.id as i32).wrapping_sub(id as i32) % 40 == 0
    }
}
struct Dim;
impl Dim { fn id(&self) -> &'static str { "minecraft:overworld" } }
struct Game { orbs: Vec<ExperienceOrb>, remote: Option<()>, dim: Dim, added: Vec<u32>,
              flip: bool, cancel: bool, asked: Vec<(u32, u32, u32, u32)>, merged: Vec<(u32, u32)> }
impl Game {
    fn native_experience_orb_can_merge(&mut self, _: &str, into: u32, from: u32, iv: u32, fv: u32,
                                       can_merge: &mut bool) {
        self.asked.push((into, from, iv, fv));
        if self.flip { *can_merge = !*can_merge; }
    }
    fn native_experience_orb_merge(&mut self, _: &str, into: u32, from: u32, _fv: u32,
                                   canceled: &mut bool) {
        self.merged.push((into, from));
        *canceled = self.cancel;
    }
    fn add_orb(&mut self, _: [f64; 3], value: u32) { self.added.push(value); }
    ORB_MERGES
    AWARD_ORB
    MERGE_ORBS
}
fn orb(id: u32, x: f64, value: u32, ticks: u32) -> ExperienceOrb {
    ExperienceOrb { id, x, value, count: 1, age: 10, ticks }
}
fn game(orbs: Vec<ExperienceOrb>, flip: bool, cancel: bool) -> Game {
    Game { orbs, remote: None, dim: Dim, added: vec![], flip, cancel, asked: vec![], merged: vec![] }
}
#[test]
fn spawn_merge_is_vanilla_unless_a_mod_decides() {
    // vanilla: same value, ids 40 apart -> merges
    let mut g = game(vec![orb(41, 0.0, 3, 0)], false, false);
    g.award_orb([0.0, 0.0, 0.0], 3, 1);
    assert_eq!((g.orbs[0].count, g.orbs[0].age, g.added.len()), (2, 0, 0));
    assert_eq!(g.asked, [(41, 0, 3, 3)]);
    // different value: vanilla refuses, a mod flips it
    let mut g = game(vec![orb(41, 0.0, 3, 0)], true, false);
    g.award_orb([0.0, 0.0, 0.0], 7, 1);
    assert_eq!(g.orbs[0].count, 2);
    // vanilla yes, a mod flips to no -> fresh orb
    let mut g = game(vec![orb(41, 0.0, 3, 0)], true, false);
    g.award_orb([0.0, 0.0, 0.0], 3, 1);
    assert_eq!((g.orbs[0].count, g.added.clone()), (1, vec![3]));
    // canceled -> fresh orb; out of reach is not asked
    let mut g = game(vec![orb(41, 0.0, 3, 0), orb(81, 5.0, 3, 0)], false, true);
    g.award_orb([0.0, 0.0, 0.0], 3, 1);
    assert_eq!((g.orbs[0].count, g.added.clone(), g.merged.clone()), (1, vec![3], vec![(41, 0)]));
    // a client of a server never merges or asks
    let mut g = game(vec![orb(41, 0.0, 3, 0)], false, false);
    g.remote = Some(());
    g.award_orb([0.0, 0.0, 0.0], 3, 1);
    assert_eq!((g.orbs[0].count, g.added.clone(), g.asked.len()), (1, vec![3], 0));
}
#[test]
fn tick_merge_is_vanilla_unless_a_mod_decides() {
    let two = |a, b| vec![orb(1, 0.0, 3, a), orb(41, 0.2, 3, b)];
    let mut g = game(two(1, 0), false, false);
    g.merge_orbs();
    assert_eq!((g.orbs.len(), g.orbs[0].count, g.orbs[0].age), (1, 2, 10));
    assert_eq!(g.asked, [(1, 41, 3, 3)]);
    // only an orb on its scan tick asks
    let mut g = game(two(0, 0), false, false);
    g.merge_orbs();
    assert_eq!((g.orbs.len(), g.asked.len()), (2, 0));
    // vanilla yes, mod no
    let mut g = game(two(1, 0), true, false);
    g.merge_orbs();
    assert_eq!(g.orbs.len(), 2);
    // different ids: vanilla no, mod yes
    let mut g = game(vec![orb(1, 0.0, 3, 1), orb(2, 0.2, 3, 0)], true, false);
    g.merge_orbs();
    assert_eq!((g.orbs.len(), g.orbs[0].count), (1, 2));
    // canceled merge keeps both
    let mut g = game(two(1, 0), false, true);
    g.merge_orbs();
    assert_eq!((g.orbs.len(), g.merged.clone()), (2, vec![(1, 41)]));
}
"""


class ExperienceOrbMergeTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_spawn_and_tick_merge_hooks(self):
        source = (ROOT / "crates/horizon-client/src/game.rs").read_text(encoding="utf-8")
        fixture = (FIXTURE.replace("ORB_MERGES", method(source, "orb_merges"))
                   .replace("AWARD_ORB", method(source, "award_orb"))
                   .replace("MERGE_ORBS", method(source, "merge_orbs")))
        with tempfile.TemporaryDirectory() as folder:
            src, exe = Path(folder) / "orb.rs", Path(folder) / "orb.exe"
            src.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", "--test", str(src), "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)
