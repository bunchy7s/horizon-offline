"""The release comparison must tolerate only known variation and still catch regressions."""
import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest

from tools.check_prepare_exporters import verify


class PrepareExportersTests(unittest.TestCase):
    def test_known_noise_does_not_mask_changed_resources(self):
        with tempfile.TemporaryDirectory() as temporary:
            # Two isolated runs, then two batched ones (with and without the second server JVM).
            folders = [Path(temporary) / str(i) for i in range(4)]
            for i, folder in enumerate(folders):
                folder.mkdir()
                (folder / 'stable.nbt').write_bytes(b'unchanged')
                (folder / 'horizon-resources.json').write_text('{"schema":7}')
                (folder / 'block-behaviours.json').write_text(json.dumps({
                    'fields': {'shape': f'<Block$$Lambda/0x{i:08x}>', 'solid': True}}))
                (folder / 'creative-inventory.json').write_text(json.dumps({
                    'tabs': [{}, {'tags': [['a', 'b'] if i < 2 else ['b', 'a']], 'items': ['stone']}]}))
            with contextlib.redirect_stdout(io.StringIO()):
                verify(folders)
                for name, changed in [
                    ('stable.nbt', b'changed'),
                    ('block-behaviours.json', b'{"fields":{"shape":"<Block$$Lambda/0x1234>","solid":false}}'),
                    ('creative-inventory.json', b'{"tabs":[{},{"tags":[["a","b"]],"items":["dirt"]}]}'),
                    ('horizon-resources.json', b'{"schema":8}'),
                ]:
                    for folder in folders[2:]:
                        path = folder / name
                        original = path.read_bytes()
                        path.write_bytes(changed)
                        with self.subTest(resource=name, run=folder.name), self.assertRaises(SystemExit):
                            verify(folders)
                        path.write_bytes(original)
