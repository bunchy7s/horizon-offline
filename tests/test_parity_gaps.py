import re
import unittest
from pathlib import Path

from tools import parity_gaps as pg

README = Path(__file__).resolve().parents[1] / "README.md"


def lists(*opens):
    return [{"number": 3512 + i, "area": f"Area {i}", "open": n, "closed": 1} for i, n in enumerate(opens)]


class CountTests(unittest.TestCase):
    def test_rows(self):
        body = "Intro\n\n- [ ] `A.b` (missing): x\n- [x] `C.d` (gap): y\n- [X] `E.f`\n* [ ] `G.h`\n  - [ ] nested\n"
        self.assertEqual(pg.rows(body), (3, 2))
        self.assertEqual(pg.rows(None), (0, 0))

    def test_area_from_the_title(self):
        self.assertEqual(pg.area("CRUCIAL: Player-visible port gaps: Blocks, redstone and fluids (86)"),
                         "Blocks, redstone and fluids")


class ReadmeTests(unittest.TestCase):
    def test_block_round_trip(self):
        block = pg.block(lists(5, 20, 0))
        self.assertIn("**Player-visible gaps: 25 open** of the 28 ", block)
        self.assertLess(block.index("Area 1 [20]"), block.index("Area 0 [5]"))  # the biggest first
        self.assertEqual(pg.written(block), (25, [20, 5, 0]))

    def test_rewrite_replaces_the_block_and_the_badge(self):
        text = ("<p>\n  <img src=\"https://img.shields.io/badge/vanilla%20parity-100%25-F0795A\" alt=\"x\">\n</p>\n\n"
                "## Project status\n\nThe table.\n")
        once = pg.rewrite(text, pg.block(lists(7)), 7)
        self.assertIn("player--visible%20gaps-7%20open-", once)
        self.assertIn("## Project status\n\n**Player-visible gaps: 7 open**", once)
        twice = pg.rewrite(once, pg.block(lists(3)), 3)
        self.assertEqual(twice.count("**Player-visible gaps:"), 1)
        self.assertIn("player--visible%20gaps-3%20open-", twice)
        self.assertEqual(pg.written(twice)[0], 3)

    def test_the_readme_agrees_with_itself(self):
        text = README.read_text(encoding="utf-8")
        total, counts = pg.written(text)
        self.assertEqual(len(counts), len(pg.ISSUES))
        self.assertEqual(total, sum(counts), "the README's gap total is the sum of its areas")
        badge = pg.BADGE.search(text)
        self.assertIsNotNone(badge, "the README's gap badge is missing")
        self.assertEqual(int(badge.group(2)), total, "the gap badge must match the block")
        status = text.split("## Project status", 1)[1]
        self.assertTrue(status.lstrip().startswith("**Player-visible gaps:"), "the gaps come first under Project status")
        self.assertNotIn("<!--", status.split("###", 1)[0], "the website prints HTML comments of this intro")
        self.assertTrue(re.search(r"^### Java parity: \d+%$", status, re.M), "the site still reads this heading")


if __name__ == "__main__":
    unittest.main()
