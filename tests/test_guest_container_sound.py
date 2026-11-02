"""A container's open and close sounds must reach the player who opened it.

Two things used to go wrong here. The opener's own client played the sound and
the sound was also queued to the guests, so a hosted world played it twice. And
a guest opening a box never told the host's copy its opener count had changed,
so no lid opened and nobody heard the sound.

So this pins both ends of `level.playSound(null, ...)`: `container_sound` plays
exactly once (the guests get it through the capture relay, not a second queue),
and the guest's open path announces the first opener, which plays it for
everyone around.
"""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


def slice_fn(source, signature):
    """The function `signature` starts, up to its own closing brace."""
    start = source.index(signature)
    end = source.index("\n    }\n", start) + len("\n    }\n")
    return source[start:end]


def slice_module_fn(source, signature):
    start = source.index(signature)
    end = source.index("\n}\n", start) + len("\n}\n")
    return source[start:end]


@unittest.skipUnless(shutil.which("rustc"), "requires Rust")
class GuestContainerSoundTests(unittest.TestCase):
    def test_a_container_sound_plays_once_where_it_is_and_once_for_the_guests(self):
        source = (ROOT / "crates/horizon-client/src/game/storage.rs").read_text()
        edge = slice_fn(source, "    pub(super) fn container_edge_sound(")
        sound = slice_fn(source, "    pub(super) fn container_sound(")
        facing = slice_module_fn(source, "fn facing_step(")
        shulker = slice_module_fn(source, "pub(super) fn is_shulker_box(")
        methods = "\n".join(
            part.replace("pub(super) fn", "fn") for part in (edge, sound)
        )
        helpers = "\n".join(
            part.replace("pub(super) fn", "fn") for part in (facing, shulker)
        )
        fixture = r'''
#![allow(dead_code)]
use std::collections::HashMap;
struct Rng;
impl Rng {
    // 0.5 * 0.1 + 0.9: the pitch each container sound draws.
    fn next_float(&mut self) -> f32 { 0.5 }
}
struct World { random: Rng }
impl World {
    fn block(&self, _x: i32, _y: i32, _z: i32) -> u32 { 7 }
}
struct BlockState { properties: HashMap<String, String> }
struct Registry;
impl Registry {
    fn state(&self, _state: u32) -> Option<&BlockState> { None }
}
struct Assets { registry: Registry }
struct Pack { assets: Assets }
#[derive(Clone, Debug, PartialEq)]
struct Play { event: String, at: Option<[f64; 3]>, volume: f32, pitch: f32 }
struct Audio { capture: Option<Vec<()>>, played: Vec<Play> }
impl Audio {
    fn play(&mut self, event: &str, at: Option<[f64; 3]>, volume: f32, pitch: f32) {
        // The window's world: a hosted one captures the play for the guests.
        if self.capture.is_some() {
            self.capture.as_mut().unwrap().push(());
        }
        self.played.push(Play { event: event.to_owned(), at, volume, pitch });
    }
}
struct Facts { name: String }
struct Res { facts: Facts }
impl Res {
    fn facts(&self) -> &Self { self }
    fn get(&self, _state: u32) -> &Facts { &self.facts }
}
struct Game {
    block: &'static str,
    serving: bool,
    world: World,
    audio: Audio,
    pack: Pack,
    res: Res,
}
impl Game {
    fn new(block: &'static str, serving: bool) -> Self {
        Self {
            block,
            serving,
            world: World { random: Rng },
            audio: Audio {
                capture: serving.then(Vec::new),
                played: Vec::new(),
            },
            pack: Pack { assets: Assets { registry: Registry } },
            res: Res { facts: Facts { name: block.to_owned() } },
        }
    }
    /// No double chest: the sound plays at the block's centre.
    fn chest_partner(&self, _key: (i32, i32, i32)) -> Option<((i32, i32, i32), bool)> { None }
METHODS
}
HELPERS
fn main() {
    // A hosted world: the play is captured for the guests, and played here for
    // the player at the keyboard. Once, not twice.
    let mut game = Game::new("minecraft:red_shulker_box", true);
    game.container_edge_sound((1, 2, 3), true);
    assert_eq!(game.audio.played.len(), 1, "one sound, not one here and one queued: {:?}",
        game.audio.played);
    let open = &game.audio.played[0];
    assert_eq!(open.event, "minecraft:block.shulker_box.open");
    assert_eq!(open.at, Some([1.5, 2.5, 3.5]));
    assert!((open.volume - 0.5).abs() < 1e-6);
    assert!((open.pitch - 0.95).abs() < 1e-6);
    assert_eq!(game.audio.capture.as_ref().map(Vec::len), Some(1),
        "the capture relay carries the same sound to the guests");

    // Closing the last opener closes the lid with its own sound, once.
    game.container_edge_sound((1, 2, 3), false);
    assert_eq!(game.audio.played.len(), 2);
    assert_eq!(game.audio.played[1].event, "minecraft:block.shulker_box.close");

    // Chests take the same route.
    let mut chest = Game::new("minecraft:chest", true);
    chest.container_edge_sound((0, 0, 0), true);
    assert_eq!(chest.audio.played[0].event, "minecraft:block.chest.open");

    // A world nobody else is in captures nothing and still plays here.
    let mut alone = Game::new("minecraft:shulker_box", false);
    alone.container_edge_sound((0, 0, 0), true);
    assert_eq!(alone.audio.played.len(), 1);
    assert_eq!(alone.audio.played[0].event, "minecraft:block.shulker_box.open");
    assert!(alone.audio.capture.is_none());
}
'''.replace("METHODS", methods).replace("HELPERS", helpers)
        with tempfile.TemporaryDirectory(prefix="guest-container-sound-") as directory:
            path = Path(directory)
            (path / "main.rs").write_text(fixture)
            subprocess.run(["rustc", "--edition=2024", str(path / "main.rs"), "-o", str(path / "probe")],
                           check=True, capture_output=True, text=True)
            result = subprocess.run([str(path / "probe")], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_a_guests_open_announces_the_first_opener(self):
        """The host's copy has to count the guest's open, or the lid stays shut."""
        source = (ROOT / "crates/horizon-client/src/game/lan_actions.rs").read_text()
        opened = slice_fn(source, "    fn lan_open_container(")
        self.assertIn("self.container_openers_changed(pos, true)", opened,
                      "a guest's open must increment the opener count")
        self.assertIn("self.container_edge_sound(pos, true)", opened,
                      "the first opener plays the open sound for everyone")
        # The close path already did this; opening must not be the only half.
        self.assertIn("self.container_openers_changed(pos, false)", source,
                      "closing a guest's menu still decrements the count")


if __name__ == "__main__":
    unittest.main()
