from pathlib import Path
import unittest
from unittest import mock

from tools import interaction_lab


class CargoExampleTests(unittest.TestCase):
    def test_client_probe_uses_the_reusable_library_package(self):
        with mock.patch.object(interaction_lab.subprocess, 'run') as run:
            interaction_lab.cargo_example('horizon-client', 'menu_fixture', Path('cases.txt'), profile='fast')
        run.assert_called_once_with(
            ['cargo', 'run', '-q', '-j3', '--profile', 'fast', '-p', 'horizon-client-lib',
             '--example', 'client_probe', '--', 'menu_fixture', 'cases.txt'],
            check=True, cwd=interaction_lab.lab.ROOT)

    def test_other_probe_packages_are_unchanged(self):
        with mock.patch.object(interaction_lab.subprocess, 'run') as run:
            interaction_lab.cargo_example('horizon-world', 'loot_probe', 42)
        run.assert_called_once_with(
            ['cargo', 'run', '-q', '-j3', '-p', 'horizon-world', '--example', 'world_probe', '--', 'loot_probe', '42'],
            check=True, cwd=interaction_lab.lab.ROOT)
