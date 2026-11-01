import json
from pathlib import Path
import unittest
from unittest import mock

from tools import port_publish as publish

DIFF = {"classes": {"changed": {"net/minecraft/world/food/FoodData": {
    "methods": {"added": [], "removed": [], "changed": ["tick(Lnet/minecraft/server/level/ServerPlayer;)V"]},
    "fields": {"added": ["exhaustionLevel F"], "removed": [], "changed": []}}}}}


def items():
    return [
        {"rank": 1, "kind": "protocol", "title": "Protocol: packets play/clientbound +1 -0 ~3",
         "files": ["crates/horizon-protocol/src/ids.rs"], "details": {"packets": {"play/clientbound": {
             "added": {"minecraft:punch": 3}, "removed": {}, "renumbered": {"minecraft:x": [1, 2]}}}}},
        {"rank": 2, "kind": "rerun", "title": "crates/w/src/food.rs: 1 vanilla classes changed", "harnesses": ["food"],
         "failed": [], "classes": {}},
        {"rank": 2, "kind": "harness-failed", "title": "Harness light: fails on 26.4", "failed": ["light"],
         "oracle": [], "harnesses": ["light"]},
        {"rank": 3, "kind": "unproved", "title": "crates/w/src/food.rs: 1 vanilla classes changed (1 methods, 1 named)",
         "files": ["crates/w/src/food.rs"], "classes": {"net/minecraft/world/food/FoodData": {
             "methods": 1, "named": ["tick"], "removed": False}}},
        {"rank": 3, "kind": "unproved", "title": "crates/c/src/sky.rs: 1 vanilla classes changed (2 methods, 0 named)",
         "classes": {"net/minecraft/client/Sky": {"methods": 2, "named": [], "removed": False}}},
        {"rank": 4, "kind": "content", "title": "New block: 2 entries", "entries": ["minecraft:a", "minecraft:b"]},
    ]


REPORT = {"old": "26.3", "new": "26.4", "summary": {"facts": {"protocol_version": [777, 780]}}, "items": items()}


class PublishTests(unittest.TestCase):
    def test_which_items_get_an_issue(self):
        self.assertEqual([publish.published(i) for i in items()], [True, False, True, True, False, False])

    def test_body_has_names_and_signatures_only(self):
        text = publish.body(items()[3], REPORT, DIFF, Path.home() / "code/horizon-parity/versions/diff-26.3-26.4")
        self.assertIn("`net.minecraft.world.food.FoodData`: 1 methods changed; Horizon's comments name tick", text)
        self.assertIn("methods changed: `tick(Lnet/minecraft/server/level/ServerPlayer;)V`", text)
        self.assertIn("fields added: `exhaustionLevel F`", text)
        self.assertIn("`~/code/horizon-parity/versions/diff-26.3-26.4`", text)
        self.assertTrue(text.startswith(publish.START) and text.rstrip().endswith(publish.END))

    def test_refuses_code(self):
        for line in ["```java\nint x = 1;\n```", "    return this.foodLevel;", "public void tick(Player player) {",
                     "- `if (x) { y(); }`", "import net.minecraft.world.food.FoodData;"]:
            with self.assertRaises(ValueError, msg=line):
                publish.assert_no_code(line)
        publish.assert_no_code("- `<init>(Lnet/minecraft/core/RegistryAccess;Lnet/minecraft/world/Foo;)V`, `a()V`")
        publish.assert_no_code("- `ParityData.java:45: cannot find symbol: variable WORLD (class RegistryLayer)`")

    def test_update_keeps_what_people_wrote(self):
        existing = f"Notes from a contributor.\n{publish.START}\nold\n{publish.END}\nMore notes."
        self.assertEqual(publish.merged(existing, f"{publish.START}\nnew\n{publish.END}\n"),
                         f"Notes from a contributor.\n{publish.START}\nnew\n{publish.END}\nMore notes.")
        self.assertEqual(publish.merged("no markers", "generated"), "generated")

    def test_plan_creates_updates_and_tracks(self):
        name = "[26.4] Protocol: packets play/clientbound +1 -0 ~3"
        actions = publish.plan(REPORT, DIFF, Path("/tmp/d"), {name: (12, "old body")})
        self.assertEqual([(a, t) for a, t, _, _ in actions], [
            ("update", name), ("create", "[26.4] Harness light: fails on 26.4"),
            ("create", "[26.4] crates/w/src/food.rs: 1 vanilla classes changed (1 methods, 1 named)"),
            ("create", "[26.4] Port to Minecraft 26.4")])
        tracking = actions[-1][2]
        self.assertIn("- [ ] #12 " + name, tracking)
        self.assertIn("- rank 4: New block: 2 entries", tracking)

    def test_apply_lists_the_new_issue_numbers_in_the_tracking_issue(self):
        calls = []

        def gh(*args):
            calls.append(args)
            if args[:2] == ("label", "list"):
                return "version-port\nport-26.4\n"
            if args[:2] == ("issue", "create"):
                return f"https://github.com/o/r/issues/{100 + sum(1 for c in calls if c[:2] == ('issue', 'create'))}\n"
            return ""

        actions = publish.plan(REPORT, DIFF, Path("/tmp/d"), {})
        with mock.patch.object(publish, "gh", side_effect=gh):
            bodies = {}
            real = publish.issue

            def issue(action, name, text, number, new):
                bodies[name] = text
                return real(action, name, text, number, new)

            with mock.patch.object(publish, "issue", side_effect=issue):
                urls = publish.apply(actions, REPORT, {})
        self.assertEqual(urls["[26.4] Port to Minecraft 26.4"], "https://github.com/o/r/issues/104")
        self.assertIn("- [ ] #101 [26.4] Protocol", bodies["[26.4] Port to Minecraft 26.4"])
        self.assertFalse([c for c in calls if c[:2] == ("label", "create")])

    def test_gh_and_kira_read_utf8_whatever_the_code_page(self):
        # gh and node print UTF-8; without an encoding, Windows decodes it with the code page (cp1252 here).
        def run(command, **kwargs):
            out = json.dumps({"body": "a → b"}, ensure_ascii=False).encode("utf-8")
            return mock.Mock(stdout=out.decode(kwargs.get("encoding") or "cp1252"))

        with mock.patch.object(publish.subprocess, "run", side_effect=run):
            self.assertEqual(json.loads(publish.gh("issue", "view"))["body"], "a → b")
            self.assertEqual(publish.kira("board_list", {})["body"], "a → b")


if __name__ == "__main__":
    unittest.main()
