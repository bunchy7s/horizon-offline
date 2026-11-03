"""Game blob fallback through the actual production selection/caster methods."""
from pathlib import Path
import shutil
import subprocess
import sys
import unittest


ROOT = Path(__file__).resolve().parents[1]


class EntityShadowOwnership(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust to execute production methods")
    def test_compiled_caster_paths_select_the_blob_fallback(self):
        result = subprocess.run(
            [sys.executable, str(ROOT / "tools/test_entity_shadow_ownership.py")],
            cwd=ROOT,
            capture_output=True,
            text=True,
            timeout=60,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
