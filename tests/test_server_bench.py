import unittest

import re

from types import SimpleNamespace

from tools.server_bench import DONE, PUMPKIN_READY, TICK_AVERAGE, TICK_PERCENTILES, pumpkin_config, work


def run(server, bots, sent, new, waited=0.0, behaviour="mixed"):
    return {"server": server, "behaviour": behaviour, "bots": bots, "chunks_per_second": sent,
            "unique_per_second": new, "waited_ticks_per_second": waited}


class ServerBenchWorkTests(unittest.TestCase):
    def test_shares_of_the_busiest_server(self):
        [row] = work([run("horizon", 10, 98.2, 36.5), run("vanilla", 10, 24.3, 19.1, 12.0)])
        self.assertFalse(row["equal"])
        self.assertEqual(row["servers"]["horizon"]["chunks_per_second"], {"value": 98.2, "share": 1.0})
        self.assertEqual(row["servers"]["vanilla"]["chunks_per_second"]["share"], 0.247)
        self.assertEqual(row["servers"]["vanilla"]["waited_ticks_per_second"], 12.0)

    def test_equal_within_ten_percent_or_a_chunk(self):
        [row] = work([run("horizon", 25, 236, 28.9), run("vanilla", 25, 252, 28.9)])
        self.assertTrue(row["equal"])
        # One bot: 0.3 against 1.1 new chunks a second is within a chunk.
        [row] = work([run("horizon", 1, 9.9, 0.3), run("vanilla", 1, 9.9, 1.1)])
        self.assertTrue(row["equal"])

    def test_only_counts_run_on_several_servers(self):
        failed = dict(run("vanilla", 50, 0, 0), error="watchdog")
        self.assertEqual(work([run("horizon", 50, 450, 10), failed, run("horizon", 1, 9, 0.3)]), [])

    def test_each_round_is_compared_on_its_own(self):
        rows = work([dict(run("horizon", 20, 236, 111), round=0), dict(run("pumpkin", 20, 190, 69), round=0),
                     dict(run("horizon", 20, 237, 112), round=1), dict(run("pumpkin", 20, 237, 112), round=1)])
        self.assertEqual([(r["round"], r["equal"]) for r in rows], [(0, False), (1, True)])


class ServerBenchLogTests(unittest.TestCase):
    def test_every_server_says_it_is_up_the_same_way(self):
        for line in ['[05:16:10] [Server thread/INFO]: Done (9.762s)! For help, type "help"',
                     '[02:53:58 INFO]: Done (29.649s)! For help, type "help"']:
            self.assertTrue(re.search(DONE, line), line)
        self.assertFalse(re.search(DONE, '[02:53:58 INFO]: Preparing spawn area: Done (12%)'))

    def test_paper_answers_tick_query_as_vanilla_does(self):
        # Paper 26.3 build 205, from a smoke run of this tool.
        self.assertEqual(TICK_AVERAGE.search("Average time per tick: 14.4ms (Target: 50.0ms)").group(1), "14.4")
        self.assertEqual(TICK_PERCENTILES.search(
            "[02:54:10 INFO]: System chat: Percentiles: P50: 1.0ms P95: 34.6ms P99: 764.4ms. Sample: 100").groups(),
            ("1.0", "34.6", "764.4"))
    def test_pumpkin_is_up_once_it_listens(self):
        line = "16:39:19  INFO Server is now running. Connect using port: Java Edition: 127.0.0.1:39811"
        self.assertTrue(re.search(PUMPKIN_READY, line))


# A configuration in the shape Pumpkin writes (made up here: the keys this tool sets).
PUMPKIN_DEFAULTS = '''seed = "17"
default_gamemode = "Survival"
spawn_protection = 16

[logging]
color = true

[networking.query]
enabled = true
address = "0.0.0.0:25565"

[networking.java]
enabled = true
address = "0.0.0.0:25565"
encryption = true
online_mode = true
view_distance = 16
simulation_distance = 10

[networking.bedrock]
enabled = true
address = "0.0.0.0:19132"
online_mode = true
view_distance = 16

[plugins]
enabled = true

[telemetry]
enabled = true
endpoint = "https://example.org/heartbeat"
'''


class PumpkinConfigTests(unittest.TestCase):
    def test_the_benchmark_settings_on_loopback_with_the_extras_off(self):
        text = pumpkin_config(PUMPKIN_DEFAULTS, 27001, SimpleNamespace(seed=12345))
        self.assertIn('seed = "12345"', text)
        self.assertIn('default_gamemode = "Creative"', text)
        self.assertIn('spawn_protection = 0', text)
        java = text.split('[networking.java]')[1].split('[')[0]
        for line in ('address = "127.0.0.1:27001"', 'encryption = false', 'online_mode = false',
                     'view_distance = 10', 'simulation_distance = 10'):
            self.assertIn(line, java)
        self.assertIn('[networking.query]\nenabled = false\naddress = "127.0.0.1:27002"', text)
        self.assertIn('[networking.bedrock]\nenabled = false', text)
        self.assertIn('[plugins]\nenabled = false', text)
        self.assertIn('[telemetry]\nenabled = false\nendpoint = ""', text)
        self.assertNotIn('0.0.0.0:25565', text)
        # Only its own section's keys change: Bedrock keeps its (it is off).
        self.assertIn('address = "0.0.0.0:19132"\nonline_mode = true', text)


if __name__ == "__main__":
    unittest.main()
