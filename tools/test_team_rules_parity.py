#!/usr/bin/env python3
"""Compare the team rules Horizon's gameplay uses with vanilla's.

The Java oracle (VanillaTeamRulesOracle) runs vanilla's own
Entity.doTeamsAllowDamage (friendly fire: player attacks, explosions,
pufferfish, slimes), Entity.isAlliedTo (targeting, sweeping), the
EntitySelector.pushableBy predicate (collision rules), Entity.isInvisibleTo
(team mates who see friendly invisibles) and LivingEntityRenderer.shouldShowName
(name tag visibility, with a stand-in client for the viewer, camera and F1) on
stand-in entities, for every combination of two teams' options, the three
entities' teams (none, either team) and the flags involved.
`examples/probe/team_rules_fixture` answers from `horizon_client::scoreboard`.

  test_team_rules_parity.py [--count N]
"""
import argparse
import itertools
import json
import os
import random
import subprocess
import sys
from pathlib import Path

try:
    from . import lab, test_rust_parity as base, test_model_baking_parity as baking
except ImportError:
    import lab
    import test_rust_parity as base
    import test_model_baking_parity as baking

PARITY = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity'
RULES = ['always', 'never', 'pushOtherTeams', 'pushOwnTeam']
VISIBILITY = ['always', 'never', 'hideForOtherTeams', 'hideForOwnTeam']


def cases(count):
    rng = random.Random(0x5445414d)
    out = []
    teams = [None, 'A', 'B']
    # Every pair of collision rules and name tag visibilities with every team assignment.
    for ra, rb, va, own, other, viewer in itertools.product(RULES, RULES, VISIBILITY, teams, teams, teams):
        out.append((ra, rb, va, own, other, viewer))
    rows = []
    for n in range(max(count, len(out))):
        ra, rb, va, own, other, viewer = out[n] if n < len(out) else (
            rng.choice(RULES), rng.choice(RULES), rng.choice(VISIBILITY), rng.choice(teams), rng.choice(teams), rng.choice(teams))
        rows.append({
            'case': n,
            'A': {'ff': rng.random() < 0.5, 'si': rng.random() < 0.5, 'collision': ra, 'nametag': va},
            'B': {'ff': rng.random() < 0.5, 'si': rng.random() < 0.5, 'collision': rb, 'nametag': rng.choice(VISIBILITY)},
            'own': own, 'other': other, 'viewer': viewer,
            'pushable': rng.random() < 0.8, 'spectator': rng.random() < 0.15, 'invisible': rng.random() < 0.4,
            'discrete': rng.random() < 0.3, 'vehicle': rng.random() < 0.15, 'viewer_spectator': rng.random() < 0.15,
            'camera_is_entity': rng.random() < 0.1, 'hud_hidden': rng.random() < 0.15,
            'distance': rng.choice([0.0, 100.0, 1023.99, 1024.0, 4095.0, 5000.0]),
        })
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--count', type=int, default=6000)
    args = parser.parse_args()
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'team-rules-parity'
    build.mkdir(parents=True, exist_ok=True)
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['client'], *libraries]))

    def run(command, name):
        with (build / (name + '.log')).open('w') as log:
            result = subprocess.run(list(map(str, command)), cwd=lab.ROOT, timeout=3600, stdout=log, stderr=subprocess.STDOUT)
        if result.returncode:
            print((build / (name + '.log')).read_text()[-4000:])
            sys.exit(f'{name} failed')

    chosen = cases(args.count)
    inputs = build / 'inputs.jsonl'
    inputs.write_text(''.join(json.dumps(c) + '\n' for c in chosen))
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-nowarn', '-cp', dependencies, '-d', classes,
         PARITY / 'VanillaTeamRulesOracle.java'], 'compile-java')
    run(['java', '-Xmx1G', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaTeamRulesOracle',
         inputs, build / 'vanilla.jsonl'], 'vanilla')
    run(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client-lib', '--example', 'client_probe'], 'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'], cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'fast/examples' / ('client_probe.exe' if os.name=='nt' else 'client_probe')
    run([binary, 'team_rules_fixture',inputs, build / 'rust.jsonl'], 'rust')
    vanilla = [json.loads(l) for l in (build / 'vanilla.jsonl').read_text().splitlines()]
    rust = [json.loads(l) for l in (build / 'rust.jsonl').read_text().splitlines()]
    assert len(vanilla) == len(rust) == len(chosen)
    errors = [v for v in vanilla if 'error' in v]
    bad = [(c, v, r) for c, v, r in zip(chosen, vanilla, rust) if 'error' not in v and v != r]
    for v in errors[:3]:
        print('ORACLE ERROR', v)
    for c, v, r in bad[:5]:
        print('MISMATCH', json.dumps(c))
        print('   vanilla', json.dumps(v))
        print('   horizon', json.dumps(r))
    print(f'{len(vanilla) - len(bad) - len(errors)}/{len(vanilla)} team rule cases match; {len(errors)} oracle errors')
    if bad or errors:
        sys.exit(1)
    print('team rules parity ok')


if __name__ == '__main__':
    main()
