"""Run the actual FOV method with a fake mod, without opening an OS window."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from test_world_input_capture import method

ROOT = Path(__file__).resolve().parents[1]


class ComputeFovHookTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_computed_fov_override_and_invalid_fallback(self):
        source = (ROOT / "crates/horizon-client/src/game/fov.rs").read_text(encoding="utf-8")
        actual = method(source.replace("pub(super) fn", "fn"), "fov_degrees")
        fixture = r"""
struct View;
impl View {
    fn calculate_fov(&self, partial: f32, setting: i32, scale: f32,
                     death: Option<i32>, fluid: (), panorama: bool) -> f32 {
        assert_eq!((partial, setting, scale, death, fluid, panorama), (0.25, 70, 1.0, None, (), false));
        70.0
    }
}
struct Settings { fov: f32 }
struct Game { view: View, settings: Settings, override_fov: Option<f32> }
impl Game {
    fn fov_effect_scale(&self) -> f32 { 1.0 }
    fn death_time(&self) -> Option<i32> { None }
    fn camera_fluid(&self, camera: [f64; 3]) { assert_eq!(camera, [1.0, 2.0, 3.0]); }
    fn zoomed_fov_degrees(&self, fov: f32) -> f32 { fov / 2.0 }
    fn native_compute_fov(&self, partial: f32, camera: [f64; 3], degrees: &mut f32) {
        assert_eq!(partial, 0.25); assert_eq!(camera, [1.0, 2.0, 3.0]); assert_eq!(*degrees, 35.0);
        if let Some(value) = self.override_fov { *degrees = value; }
    }
    METHOD
}
#[test]
fn unchanged_narrow_wide_and_invalid() {
    for (value, expected) in [(None, 35.0), (Some(15.0), 15.0), (Some(120.0), 120.0),
        (Some(1.0), 1.0), (Some(179.0), 179.0), (Some(0.0), 35.0), (Some(180.0), 35.0),
        (Some(-1.0), 35.0), (Some(f32::NAN), 35.0), (Some(f32::INFINITY), 35.0)] {
        let game = Game { view: View, settings: Settings { fov: 70.0 }, override_fov: value };
        assert_eq!(game.fov_degrees(0.25, [1.0, 2.0, 3.0]), expected);
    }
}
""".replace("METHOD", actual)
        with tempfile.TemporaryDirectory() as folder:
            src, exe = Path(folder) / "fov.rs", Path(folder) / "fov.exe"
            src.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", "--test", str(src), "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)
