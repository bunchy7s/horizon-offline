"""Exercise Entity motion delivery through the live multiplayer handler."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(shutil.which("rustc"), "requires Rust")
class RemoteMotionTests(unittest.TestCase):
    def test_mob_velocity_reaches_squid_animation(self):
        source = (ROOT / "crates/horizon-client/src/game/remote.rs").read_text()
        start = source.index("    fn remote_motion(")
        end = source.index("    fn apply_remote_health(", start)
        fixture = r'''
#![allow(dead_code)]
use std::collections::HashMap;
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Vec3(f64, f64, f64);
impl Vec3 { fn new(x: f64, y: f64, z: f64) -> Self { Self(x,y,z) } }
#[derive(Default)]
struct Entity { id: u32, velocity: Vec3 }
#[derive(Default)]
struct Remote { pending_items: HashMap<i32, ((), Vec3)> }
#[derive(Default)]
struct Creatures { projectiles: Vec<Entity> }
#[derive(Default)]
struct Potions;
impl Potions { fn remote_thrown_motion(&mut self, _id: u32, _v: Vec3) -> bool { false } }
#[derive(Default)]
struct End;
impl End { fn remote_eye_motion(&mut self, _id: u32, _v: [f64; 3]) {} }
#[derive(Default)]
struct Game {
    mobs: Vec<Entity>, dropped: Vec<Entity>, falling: Vec<Entity>, tnts: Vec<Entity>, orbs: Vec<Entity>,
    creatures: Creatures, dragon_fireballs: Vec<Entity>, tridents: Vec<Entity>,
    rockets: Vec<Entity>, remote: Option<Remote>,
    potions: Potions, end: End,
}
impl Game {
fn remote_dragon_motion(&mut self, _id: i32, _velocity: [f64; 3]) {}
fn remote_hook_motion(&mut self, _id: u32, _v: Vec3) -> bool { false }
METHOD
}
fn main() {
    let mut game = Game::default();
    game.mobs.push(Entity { id: 42, ..Entity::default() });
    game.dropped.push(Entity { id: 43, ..Entity::default() });
    // AddEntity initial motion and subsequent SetEntityMotion share this handler.
    for motion in [[0.12, -0.04, 0.08], [-0.05, 0.03, 0.2], [0.0; 3]] {
        game.remote_motion(42, motion);
        assert_eq!(game.mobs[0].velocity, Vec3::new(motion[0], motion[1], motion[2]));
        assert_eq!(game.dropped[0].velocity, Vec3::default());
    }
    game.remote_motion(43, [0.1, 0.2, 0.3]);
    assert_eq!(game.dropped[0].velocity, Vec3::new(0.1, 0.2, 0.3));
    game.remote_motion(999, [1.0; 3]); // Untracked entity is harmless.
}
'''.replace("METHOD", source[start:end])
        with tempfile.TemporaryDirectory(prefix="remote-motion-") as directory:
            path = Path(directory)
            (path / "main.rs").write_text(fixture)
            subprocess.run(["rustc", "--edition=2024", str(path / "main.rs"), "-o", str(path / "probe")],
                           check=True, capture_output=True, text=True)
            result = subprocess.run([str(path / "probe")], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
