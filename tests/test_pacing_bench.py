import unittest

from tools.pacing_bench import stats


class StatsTests(unittest.TestCase):
    def test_lows_percentiles_and_hitch_counts(self):
        # 998 frames of 5 ms, one of 40 ms and one of 100 ms.
        frames = [5.0] * 998 + [40.0, 100.0]
        s = stats(frames)
        self.assertEqual(s["frames"], 1000)
        self.assertEqual(s["p50_ms"], 5.0)
        self.assertEqual(s["p99_ms"], 5.0)
        self.assertEqual(s["p999_ms"], 40.0)
        self.assertEqual(s["max_ms"], 100.0)
        # The slowest 1% (10 frames): 8 x 5 + 40 + 100 = 180 ms, 18 ms each.
        self.assertAlmostEqual(s["low_1pct_fps"], round(1000 / 18.0, 1))
        # The slowest 0.1% is the single 100 ms frame.
        self.assertEqual(s["low_01pct_fps"], 10.0)
        self.assertEqual(s["over_33ms"], 2)
        self.assertEqual(s["over_2x_median"], 2)

    def test_no_frames(self):
        self.assertEqual(stats([]), {"frames": 0})


if __name__ == "__main__":
    unittest.main()
