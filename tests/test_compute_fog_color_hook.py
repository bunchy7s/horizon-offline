"""Run the actual fog color dispatch with a fake mod, without a GPU/window."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from test_world_input_capture import method

ROOT = Path(__file__).resolve().parents[1]


class ComputeFogColorHookTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_rgb_override_fallback_and_alpha(self):
        source = (ROOT / "crates/horizon-client/src/game/render_tick.rs").read_text(encoding="utf-8")
        actual = method(source, "native_fog_color_now")
        frame = method(source.replace("pub(super) fn", "fn"), "frame_fog")
        self.assertIn("if self.title.is_none() { fog.color = self.native_fog_color_now", " ".join(frame.split()))
        fixture = r"""
#[derive(Clone, Copy, Debug, PartialEq)]
enum FogType { Atmospheric, Water, Lava, PowderSnow }
struct Game { replacement: Option<[f32; 3]> }
impl Game {
    fn native_compute_fog_color(&self, partial: f32, camera: [f64; 3],
                               _kind: FogType, rgb: &mut [f32; 3]) {
        assert_eq!((partial, camera, *rgb), (0.25, [1.0, 2.0, 3.0], [0.2, 0.4, 0.6]));
        if let Some(value) = self.replacement { *rgb = value; }
    }
    METHOD
}
#[test]
fn all_environments_valid_invalid_and_unchanged() {
    let original = [0.2, 0.4, 0.6, 0.75];
    for kind in [FogType::Atmospheric, FogType::Water, FogType::Lava, FogType::PowderSnow] {
        for (replacement, expected) in [(None, original),
            (Some([0.0, 1.0, 0.5]), [0.0, 1.0, 0.5, 0.75]),
            (Some([f32::NAN, 0.0, 0.0]), original),
            (Some([0.0, f32::INFINITY, 0.0]), original),
            (Some([0.0, 0.0, -0.1]), original),
            (Some([1.1, 0.0, 0.0]), original)] {
            assert_eq!(Game { replacement }.native_fog_color_now(0.25, [1.0, 2.0, 3.0], kind, original), expected);
        }
    }
}
""".replace("METHOD", actual)
        with tempfile.TemporaryDirectory() as folder:
            src, exe = Path(folder) / "fog.rs", Path(folder) / "fog.exe"
            src.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", "--test", str(src), "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)
