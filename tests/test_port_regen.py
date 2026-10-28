import subprocess
import unittest

from tools import port_regen as regen

LOCK = {"minecraft": {"version": "26.3", "manifest_sha1": "m3", "accepted_manifest_sha1": ["m3old"],
                      "accepted_manifest_note": "republished", "client_sha1": "c3", "server_sha1": "s3",
                      "java_major": 25},
        "vineflower": {"version": "1.12.0"}}


def manifest(version, client, server, java=25):
    return {"id": version, "downloads": {"client": {"sha1": client}, "server": {"sha1": server}},
            "javaVersion": {"majorVersion": java}}


class PinTests(unittest.TestCase):
    def test_another_version_gets_a_new_block(self):
        pinned = regen.pinned_lock(LOCK, manifest("26.4", "c4", "s4", 26), "m4")
        self.assertEqual(pinned["minecraft"], {"version": "26.4", "manifest_sha1": "m4", "client_sha1": "c4",
                                               "server_sha1": "s4", "java_major": 26})
        self.assertEqual(pinned["vineflower"], LOCK["vineflower"])

    def test_the_same_version_keeps_its_block(self):
        for sha1 in ["m3", "m3old"]:
            self.assertIs(regen.pinned_lock(LOCK, manifest("26.3", "c3", "s3"), sha1), LOCK)

    def test_the_same_version_with_other_jars_is_repinned(self):
        self.assertEqual(regen.pinned_lock(LOCK, manifest("26.3", "c3b", "s3"), "m3b")["minecraft"]["client_sha1"],
                         "c3b")


class CompileErrorTests(unittest.TestCase):
    OUTPUT = """/x/mods/horizon/src/test/java/dev/horizonlab/parity/ParityData.java:45: error: cannot find symbol
        var context = TagLoader.buildUpdatedLookups(layers.getAccessForLoading(RegistryLayer.WORLD), staticTags);
                                                                                            ^
  symbol:   variable WORLD
  location: class RegistryLayer
/x/VanillaCommandParseOracle.java:12: error: incompatible types: int cannot be converted to String
2 errors
"""

    def test_names_the_symbols(self):
        self.assertEqual(regen.compile_errors(self.OUTPUT), [
            "ParityData.java:45: cannot find symbol: variable WORLD (class RegistryLayer)",
            "VanillaCommandParseOracle.java:12: incompatible types: int cannot be converted to String"])

    def test_nothing_from_other_failures(self):
        self.assertEqual(regen.compile_errors("expected the root's order to differ"), [])


class InterpreterTests(unittest.TestCase):
    def test_python_is_an_interpreter_that_runs(self):
        found = regen.python()
        done = subprocess.run([found, "-c", "import sys; sys.exit(sys.version_info < (3,))"],
                              capture_output=True)
        self.assertEqual(done.returncode, 0, f"{found} is not a Python 3 that runs")


if __name__ == "__main__":
    unittest.main()
