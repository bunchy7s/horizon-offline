"""A rocket boosting a server client's own player, run through the live methods.

Vanilla reference: FireworkRocketEntity.tick after the player's tick in ClientLevel.tickEntities;
Entity.getHandHoldingItemAngle.
"""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
GAME = ROOT / "crates/horizon-client/src/game"


def lift(path, start, end):
    source = path.read_text()
    a = source.index(start)
    return source[a : source.index(end, a)].replace("pub(super) ", "")


@unittest.skipUnless(shutil.which("rustc"), "requires Rust")
class RemoteOwnRocketTests(unittest.TestCase):
    def test_the_spark_follows_the_moved_player_at_the_rocket_hand(self):
        methods = lift(
            GAME / "fireworks.rs",
            "    /// `Entity.getHandHoldingItemAngle",
            "    /// `FireworkRocketItem.use` while gliding",
        ) + lift(
            GAME / "remote_projectiles.rs",
            "    /// This client's own player's entity id",
            "    /// One tick of interpolation",
        )
        fixture = r'''
#![allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Vec3 { x: f64, y: f64, z: f64 }
impl Vec3 {
    fn new(x: f64, y: f64, z: f64) -> Self { Self { x, y, z } }
    fn add_vector(self, o: Vec3) -> Self { Self::new(self.x + o.x, self.y + o.y, self.z + o.z) }
}
// Entity.calculateViewVector(pitch, yaw).
fn view_vector(yaw: f32, pitch: f32) -> [f64; 3] {
    let (p, y) = (f64::from(pitch).to_radians(), f64::from(-yaw).to_radians());
    [y.sin() * p.cos(), -p.sin(), y.cos() * p.cos()]
}
struct ItemStack { item: String }
struct Remote { entity_id: i32 }
mod horizon_core {
    pub mod random {
        // Gaussians in turn from a list.
        pub struct LegacyRandom(pub Vec<f64>);
        impl LegacyRandom {
            pub fn next_gaussian(&mut self) -> f64 { self.0.remove(0) }
        }
    }
}
struct World { random: horizon_core::random::LegacyRandom }
enum ParticleType { Firework }
struct ParticleOptions;
impl ParticleOptions { fn simple(_: ParticleType) -> Self { Self } }
#[derive(Default)]
struct Particles { added: Vec<([f64; 3], [f64; 3])> }
impl Particles {
    fn add(&mut self, _: &ParticleOptions, at: [f64; 3], v: [f64; 3]) { self.added.push((at, v)); }
}
struct Clock { runs: bool }
impl Clock {
    fn runs_normally(&self) -> bool { self.runs }
    fn world_runs(&mut self) -> bool { panic!("TickRateManager.tick a second time in one client tick") }
}
struct Settings { main_hand_left: bool }
struct Player { position: Vec3, velocity: Vec3, fall_flying: bool }
#[derive(Default)]
struct Rocket { attached_to: Option<u32>, position: Vec3, velocity: Vec3 }
struct Game {
    remote: Option<Remote>, clock: Clock, player: Option<Player>, settings: Settings,
    hotbar: Option<ItemStack>, offhand: Option<ItemStack>, selected: usize,
    rockets: Vec<Rocket>, yaw: f32, pitch: f32, world: World, particles: Particles,
}
impl Game {
fn stack_in(&self, slot: usize) -> Option<&ItemStack> { self.hotbar.as_ref().filter(|_| slot == 3) }
METHODS
}
fn rocket() -> Option<ItemStack> { Some(ItemStack { item: "minecraft:firework_rocket".into() }) }
fn game(runs: bool) -> Game {
    Game {
        remote: Some(Remote { entity_id: 7 }),
        clock: Clock { runs },
        // Where the player's own tick has just moved it, facing west.
        player: Some(Player {
            position: Vec3::new(10.0, 70.0, 20.0),
            velocity: Vec3::new(0.0, -0.2, 1.0),
            fall_flying: true,
        }),
        settings: Settings { main_hand_left: false },
        hotbar: rocket(), offhand: None, selected: 3,
        rockets: vec![
            Rocket { attached_to: Some(7), ..Rocket::default() },
            Rocket { attached_to: Some(8), position: Vec3::new(1.0, 2.0, 3.0), ..Rocket::default() },
            Rocket { attached_to: None, position: Vec3::new(4.0, 5.0, 6.0), ..Rocket::default() },
        ],
        yaw: 90.0, pitch: 0.0,
        world: World { random: horizon_core::random::LegacyRandom(vec![3.0, -2.0, 3.0, -2.0]) },
        particles: Particles::default(),
    }
}
fn near(a: [f64; 3], b: [f64; 3]) -> bool { a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-4) }
fn at(v: Vec3) -> [f64; 3] { [v.x, v.y, v.z] }
// The rocket's spot after one tick with this hand setup.
fn hand(main_hand_left: bool, main: Option<ItemStack>, offhand: Option<ItemStack>) -> [f64; 3] {
    let mut g = game(true);
    g.settings.main_hand_left = main_hand_left;
    g.hotbar = main;
    g.offhand = offhand;
    g.tick_own_rockets();
    at(g.rockets[0].position)
}
fn main() {
    let mut g = game(true);
    g.tick_own_rockets();
    // Looking west (-x): movement + look * 0.1 + (look * 1.5 - movement) * 0.5.
    let boosted = [-0.85, -0.1, 0.5];
    assert!(near(at(g.player.as_ref().unwrap().velocity), boosted), "{:?}", g.player.as_ref().unwrap().velocity);
    assert_eq!(g.player.as_ref().unwrap().position, Vec3::new(10.0, 70.0, 20.0));
    // Facing west the right hand is north: half a block out at yaw + 80.
    let right = [10.0 - 0.0868, 70.0, 20.0 - 0.4924];
    assert!(near(at(g.rockets[0].position), right), "{:?}", g.rockets[0].position);
    assert!(near(at(g.rockets[0].velocity), boosted));
    // One firework spark at the rocket, drifting against the boosted motion
    // with a gaussian spread of 0.05 across it.
    let sparks = &g.particles.added;
    assert_eq!(sparks.len(), 1);
    assert!(near(sparks[0].0, right) && near(sparks[0].1, [0.15, 0.05, -0.1]), "{:?}", sparks);
    // Rockets of another entity, or free ones, tick with the other entities.
    assert_eq!(g.rockets[1].position, Vec3::new(1.0, 2.0, 3.0));
    assert_eq!(g.rockets[2].position, Vec3::new(4.0, 5.0, 6.0));

    // The other arm: a left main arm, or a rocket only in the offhand.
    let left = [10.0 - 0.0868, 70.0, 20.0 + 0.4924];
    assert!(near(hand(true, rocket(), None), left));
    assert!(near(hand(false, None, rocket()), left));
    assert!(near(hand(true, None, rocket()), right));
    assert!(near(hand(false, rocket(), rocket()), right));

    // Not gliding: it follows the player itself, without boosting it.
    let mut g = game(true);
    g.player.as_mut().unwrap().fall_flying = false;
    g.tick_own_rockets();
    assert_eq!(g.player.as_ref().unwrap().velocity, Vec3::new(0.0, -0.2, 1.0));
    assert_eq!(g.rockets[0].position, Vec3::new(10.0, 70.0, 20.0));
    assert_eq!(g.particles.added.len(), 1);

    // A frozen level ticks no entities.
    let mut g = game(false);
    g.tick_own_rockets();
    assert!(g.particles.added.is_empty());
    assert_eq!(g.player.as_ref().unwrap().velocity, Vec3::new(0.0, -0.2, 1.0));
}
'''.replace("METHODS", methods)
        with tempfile.TemporaryDirectory(prefix="remote-own-rockets-") as directory:
            path = Path(directory)
            (path / "main.rs").write_text(fixture)
            built = subprocess.run(
                ["rustc", "--edition=2024", str(path / "main.rs"), "-o", str(path / "probe")],
                capture_output=True,
                text=True,
            )
            self.assertEqual(built.returncode, 0, built.stderr)
            result = subprocess.run([str(path / "probe")], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
