"""bench/bench.py: the parsers, the report table and the per-server setup (no server is started)."""

import importlib.util
from pathlib import Path
import tempfile
import types
import unittest

SPEC = importlib.util.spec_from_file_location('bench', Path(__file__).resolve().parent.parent / 'bench/bench.py')
bench = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(bench)


def args(**kw):
    base = dict(seed=777, view_distance=8, pre_generated=None, pumpkin_config=None, exe='pumpkin', jar='server.jar',
                heap='2g', bots=5, behaviour='mixed', warmup=1, join_every=0.5)
    base.update(kw)
    return types.SimpleNamespace(**base)


class ParserTests(unittest.TestCase):
    def test_tick_query(self):
        lines = ['[Server] Average time per tick: 12.5 ms', 'Target: 50.0 ms', 'P50: 11.0 ms P95: 20.5 ms P99: 31.0 ms']
        self.assertEqual(bench.parse_tick_query(lines), {'average': 12.5, 'p50': 11.0, 'p95': 20.5, 'p99': 31.0})
        self.assertIsNone(bench.parse_tick_query(lines[:1]))

    def test_gametime_and_tps(self):
        self.assertEqual(bench.parse_gametime(['x', 'The time is 4200']), 4200)
        self.assertIsNone(bench.parse_gametime(['nothing']))
        self.assertEqual(bench.tps(1000, 2200, 60), 20.0)
        self.assertIsNone(bench.tps(None, 2200, 60))

    def test_cpu_seconds_and_percentile(self):
        self.assertEqual(bench.cpu_seconds('1-00:00:01.5'), 86401.5)
        self.assertEqual(bench.cpu_seconds('02:03'), 123.0)
        self.assertEqual(bench.percentile([5, 1, 3], 50), 3)
        self.assertIsNone(bench.percentile([], 50))

    def test_bot_output_split(self):
        samples, summary = bench.split_bot_output(['{"t": 1, "online": 2}\n', '{\n', '  "joined": 2\n', '}\n'])
        self.assertEqual(samples, [{'t': 1, 'online': 2}])
        self.assertEqual(summary, {'joined': 2})

    def test_bandwidth_divides_by_each_bots_time_online(self):
        # Bot 0 is on for 100 s, bot 1 for 90 s (it joined 10 s later), bot 2 failed and counts for nothing.
        per_bot = [{'bytes_in': 1000, 'bytes_out': 100}, {'bytes_in': 900, 'bytes_out': 90},
                   {'bytes_in': 50, 'bytes_out': 5, 'error': 'refused'}]
        self.assertEqual(bench.bytes_per_player_second(per_bot, 100, 10), {'bytes_in': 10, 'bytes_out': 1})
        self.assertIsNone(bench.bytes_per_player_second([{'joined_after_ms': 80}], 100, 10))


class SetupTests(unittest.TestCase):
    def test_pumpkin_config_and_pre_generated_world(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            world = tmp / 'src'
            world.mkdir()
            (world / 'level.dat').write_text('x')
            run = tmp / 'run'
            run.mkdir()
            command = bench.prepare('pumpkin', run, 25599, args(pre_generated=str(world)))
            self.assertEqual(len(command), 1)
            config = (run / 'config/configuration.toml').read_text()
            self.assertIn('127.0.0.1:25599', config)
            self.assertIn('[networking.bedrock]\nenabled = false', config)
            self.assertIn('enabled = false', (run / 'config/features.toml').read_text())
            self.assertEqual((run / 'world/level.dat').read_text(), 'x')

    def test_vanilla_properties(self):
        with tempfile.TemporaryDirectory() as tmp:
            command = bench.prepare('vanilla', tmp, 25598, args())
            text = (Path(tmp) / 'server.properties').read_text()
            for line in ('server-ip=127.0.0.1', 'server-port=25598', 'level-seed=777', 'online-mode=false'):
                self.assertIn(line, text)
            self.assertIn('-Xmx2g', command)
            self.assertEqual((Path(tmp) / 'eula.txt').read_text(), 'eula=true\n')


class TableTests(unittest.TestCase):
    def test_table_marks_missing_numbers(self):
        run = {'server': 'horizon', 'bots': 5, 'tps': 20.0, 'mspt': {'average': 3.0, 'p99': 9.0},
               'bytes_per_player_second': None, 'join_ms': {'p50': 80, 'worst': 120}}
        text = bench.table([run, {'server': 'pumpkin', 'error': 'no start\nmore'}])
        self.assertIn('| TPS | 20.0 |', text)
        self.assertIn('| Bytes in per player and s online | - |', text)
        self.assertIn('pumpkin failed: no start', text)


if __name__ == '__main__':
    unittest.main()
