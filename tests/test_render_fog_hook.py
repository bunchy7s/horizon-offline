"""Exercise the actual final-fog validation/dispatch with a fake mod."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from test_world_input_capture import method

ROOT = Path(__file__).resolve().parents[1]


class RenderFogHookTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_valid_parameters_and_atomic_invalid_fallback(self):
        source = (ROOT / "crates/horizon-client/src/game/render_tick.rs").read_text(encoding="utf-8")
        actual = method(source, "native_fog_now")
        fixture = r"""
#[derive(Clone, Copy)]
enum FogType { Water }
#[derive(Clone, Copy, Debug, PartialEq)]
struct FogData {
    color: [f32; 4], environmental_start: f32, environmental_end: f32,
    render_distance_start: f32, render_distance_end: f32, sky_end: f32, cloud_end: f32,
}
struct Game { replacement: Option<FogData> }
impl Game {
    fn native_render_fog(&self, partial: f32, camera: [f64; 3], kind: FogType, fog: &mut FogData) {
        assert_eq!((partial, camera), (0.5, [1.0, 2.0, 3.0]));
        assert!(matches!(kind, FogType::Water));
        assert_eq!(fog.environmental_start, -8.0);
        if let Some(value) = self.replacement { *fog = value; }
    }
    METHOD
}
#[test]
fn changed_unchanged_and_invalid() {
    let original = FogData { color: [0.2, 0.3, 0.4, 1.0], environmental_start: -8.0,
        environmental_end: 24.0, render_distance_start: 230.0, render_distance_end: 256.0,
        sky_end: 24.0, cloud_end: 24.0 };
    let run = |replacement| Game { replacement }.native_fog_now(0.5, [1.0, 2.0, 3.0], FogType::Water, original);
    assert_eq!(run(None), original);
    let changed = FogData { color: [1.0, 0.0, 0.5, 0.5], environmental_start: -16.0,
        environmental_end: 48.0, render_distance_start: 512.0, render_distance_end: 1024.0,
        sky_end: 0.0, cloud_end: 80.0 };
    assert_eq!(run(Some(changed)), changed);
    for invalid in [FogData { environmental_start: 48.0, ..changed },
        FogData { environmental_end: f32::NAN, ..changed },
        FogData { render_distance_end: 512.0, ..changed },
        FogData { render_distance_start: f32::NEG_INFINITY, ..changed },
        FogData { sky_end: -1.0, ..changed }, FogData { cloud_end: f32::INFINITY, ..changed },
        FogData { cloud_end: -1.0, ..changed },
        FogData { color: [f32::NAN, 0.0, 0.0, 1.0], ..changed },
        FogData { color: [0.0, 0.0, 0.0, 1.1], ..changed }] {
        assert_eq!(run(Some(invalid)), original);
    }
}
""".replace("METHOD", actual)
        with tempfile.TemporaryDirectory() as folder:
            src, exe = Path(folder) / "fog.rs", Path(folder) / "fog.exe"
            src.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", "--test", str(src), "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)
