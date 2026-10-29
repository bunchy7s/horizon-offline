import os
import subprocess
import unittest
from pathlib import Path
from unittest import mock
from tools import test_world_spawn_parity as spawn

LOCK = {'minecraft': {'server_sha1': 'a' * 40}}
COMMAND = ['java', '-Xmx2g', '-Dmax.bg.threads=1', '-cp', '/one/server.jar', 'net.minecraft.server.Main', 'nogui']


def cache(environment, lock=LOCK, command=COMMAND, classpath='/one/server.jar', java='openjdk version "25"'):
    version = subprocess.CompletedProcess([], 0, stdout='', stderr=java)
    with mock.patch.dict(os.environ, environment, clear=True), mock.patch.object(spawn.subprocess, 'run',
                                                                                return_value=version):
        return spawn.vanilla_cache(lock, command, classpath)


class VanillaCacheTests(unittest.TestCase):
    def test_only_under_oracle_reuse(self):
        self.assertIsNone(cache({}))
        self.assertIsNone(cache({'PARITY_SHIM_CACHE': '/cache', 'PARITY_JAVA_CACHE': '0'}))
        self.assertEqual(cache({'PARITY_SHIM_CACHE': '/cache'})(4242).parent, Path('/cache'))

    def test_key_holds_what_vanilla_depends_on(self):
        reuse = {'PARITY_SHIM_CACHE': '/cache'}
        record = cache(reuse)
        self.assertEqual(record(4242), record(4242))
        self.assertNotEqual(record(4242), record(777))
        # The classpath's place on this machine isn't part of it: verify_artifacts checked the jars.
        moved = ['/two/server.jar' if part == '/one/server.jar' else part for part in COMMAND]
        self.assertEqual(cache(reuse, command=moved, classpath='/two/server.jar')(4242), record(4242))
        self.assertNotEqual(cache(reuse, lock={'minecraft': {'server_sha1': 'b' * 40}})(4242), record(4242))
        self.assertNotEqual(cache(reuse, java='openjdk version "26"')(4242), record(4242))
        threads = [part.replace('threads=1', 'threads=2') for part in COMMAND]
        self.assertNotEqual(cache(reuse, command=threads)(4242), record(4242))

    def test_properties_leave_out_the_port(self):
        self.assertNotIn('server-port', spawn.properties(4242))
        self.assertEqual(spawn.properties(4242)['level-seed'], '4242')


if __name__ == '__main__':
    unittest.main()
