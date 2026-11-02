"""Per-process scratch data for game integration tests (never a player's home)."""
from pathlib import Path
from contextlib import contextmanager
import os
import subprocess
import tempfile

try:
    from . import lab
except ImportError:
    import lab


def game_environment(base, root):
    root = Path(root)
    env = dict(base, HORIZON_TELEMETRY_URL='off')
    for key, folder in [('HORIZON_HOME', 'home'), ('HORIZON_LAUNCHER_HOME', 'launcher'),
                        ('XDG_DATA_HOME', 'data'), ('XDG_CONFIG_HOME', 'config')]:
        path = root / folder
        path.mkdir(parents=True, exist_ok=True)
        env[key] = str(path)
    return lab.game_env(env)


def run_game(command, **kwargs):
    """Run a headless game fixture with the same isolation as real-client tests."""
    with tempfile.TemporaryDirectory(prefix='hz-fixture-') as root:
        kwargs['env'] = game_environment(kwargs.pop('env', os.environ), root)
        if any(key in kwargs['env'] for key in
               ('HORIZON_DISPENSE_FIXTURE', 'HORIZON_SCULK_FIXTURE')):
            # These compare seeded rule/callback traces, not the client/server boundary.
            command = [*command, '--in-frame']
        return subprocess.run(command, **kwargs)


@contextmanager
def suite_lock():
    """Real-client CI suites share a GPU, even when parity.py runs harnesses in parallel."""
    try:
        from . import file_lock
    except ImportError:
        import file_lock
    with (Path(tempfile.gettempdir()) / 'horizon-integration-gpu.lock').open('a') as handle:
        file_lock.lock(handle)
        yield


def require_server_reports(resources):
    """Match Builtin.find: a registry report shadows later fallback directories."""
    resources = Path(resources)
    reports = next((p for p in (resources, resources / '../../datagen/out/reports',
                                resources / '../datagen/out/reports')
                    if (p / 'registries.json').is_file()), None)
    if reports is None or not (reports / 'commands.json').is_file():
        raise ValueError(f'incomplete prepared resources in {resources}: registries.json and '
                         'commands.json must be installed together; prepare the pinned resources again')
    return reports
