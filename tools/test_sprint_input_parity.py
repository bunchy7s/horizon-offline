#!/usr/bin/env python3
"""Compare native LocalPlayer sprint decisions over evolving keyboard input."""
import itertools
import json
import os
from pathlib import Path
import platform
import random
import struct
import subprocess
try:
    from . import lab, test_rust_parity as base, test_model_baking_parity as baking
except ImportError:
    import lab, test_rust_parity as base, test_model_baking_parity as baking

def corpus():
    def step(keys,flags=0,food=20,window=7):return dict(keys=keys,flags=flags,food=food,window=window)
    def row(steps,initial_keys=0,sprint=False,trigger=0):return dict(steps=steps,initial_keys=initial_keys,sprint=sprint,trigger=trigger)
    # Complete combinations of the 17 environmental/ability/equipment queries.
    for flags in range(1<<17):
        yield row([step(65,flags),step(1,flags),step(0,flags)])
    for keys in range(128):
        for initial in [0,1,3,5,33,65]:
            for sprint in [False,True]:
                for trigger in [-2147483648,-1,0,1,2,7,8,2147483647]:
                    yield row([step(keys),step(keys),step(0)],initial,sprint,trigger)
    for window in range(11):
        for gap in range(13):
            for food in [-2147483648,-1,0,5,6,7,20,2147483647]:
                for sprint_key in [False,True]:
                    seq=[1,0]+[0]*gap+[65 if sprint_key else 1,1,0,1,1,0]
                    yield row([step(k,food=food,window=window) for k in seq])
    rng=random.Random(0x535052494e54)
    for _ in range(1000):
        yield row([step(rng.randrange(128),rng.randrange(1<<17),rng.choice([0,6,7,20]),rng.randrange(11)) for _ in range(40)],
            rng.randrange(128),bool(rng.randrange(2)),rng.randrange(-1,12))

def main():
    lock,jars,_=base.verify_artifacts();libraries=baking.client_libraries()
    build=lab.LOCAL/'rust-sprint-input';build.mkdir(exist_ok=True)
    report=build/'report.json';report.unlink(missing_ok=True)
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaSprintInputOracle.java'
    sources=[Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/test_model_baking_parity.py',lab.ROOT/'tools/lab.py',oracle,lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-client/Cargo.toml',lab.ROOT/'crates/horizon-client-input/src/input.rs',lab.ROOT/'crates/horizon-client/src/sprint.rs',lab.ROOT/'crates/horizon-client/src/lib.rs',lab.ROOT/'crates/horizon-client/examples/probe/sprint_probe.rs']
    tested=base.source_hashes(sources)
    def run(command,name):
        with (build/(name+'.log')).open('w') as log:subprocess.run(list(map(str,command)),cwd=lab.ROOT,check=True,timeout=600,stdout=log,stderr=subprocess.STDOUT)
    fixtures=list(corpus());operations=sum(len(f['steps']) for f in fixtures);inputs=build/'inputs.jsonl'
    inputs.write_text(''.join(json.dumps(f,separators=(',',':'))+'\n' for f in fixtures));input_hash=lab.digest(inputs,'sha256')
    classes=build/'classes';classes.mkdir(exist_ok=True)
    dependencies=os.pathsep.join(map(str,[jars['client'],*libraries]))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',dependencies,'-d',classes,oracle],'compile-java')
    run(['cargo','build','--locked','-p','horizon-client-lib','--example','client_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--locked','--no-deps','--format-version','1'],cwd=lab.ROOT))
    binary=Path(metadata['target_directory'])/'debug/examples'/('client_probe.exe' if os.name=='nt' else 'client_probe')
    run([binary,'sprint_probe',inputs,build/'rust.txt'],'rust')
    run(['java','-Xmx768M','-cp',str(classes)+os.pathsep+dependencies,'dev.horizonlab.parity.VanillaSprintInputOracle',inputs,build/'client.txt'],'client')
    matched=base.compare(build/'client.txt',build/'rust.txt')
    if matched!=operations+len(fixtures):raise AssertionError('Incomplete input output')
    base.ensure_sources_unchanged(sources,tested)
    if input_hash!=lab.digest(inputs,'sha256'):raise AssertionError('Input fixtures changed')
    lab.write_json(report,dict(schema=1,minecraft=lock['minecraft']['version'],manifest_sha1=lock['minecraft']['manifest_sha1'],platform=platform.platform(),rustc=subprocess.check_output(['rustc','--version'],text=True).strip(),java=subprocess.check_output(['java','--version'],text=True).splitlines()[0],fixtures=len(fixtures),operations=operations,sources_sha256=tested,input_sha256=input_hash,client_jar_sha256=lab.digest(jars['client'],'sha256'),matched_output_lines=matched,output_sha256=lab.digest(build/'rust.txt','sha256'),scope='Actual KeyboardInput.tick and LocalPlayer.aiStep prefix through sprint start/stop requests and sprintTriggerTime. A sentinel in jumpableVehicle stops before riding and parent/world movement. Native private canStartSprinting, isSprintingPossible and shouldStopRun/SwimSprinting execute, with controlled environmental/effect/vehicle/UI queries, actual FoodData/Abilities and actual ItemStack USE_EFFECTS. Compares evolving sprint state, timer, keyboard vector/shift and every ordered setSprinting request, including duplicate requests. All 17 query-flag combinations, all keyboard combinations with previous states, signed timer boundaries, food thresholds, configurable windows0..10, double taps and random sequences. Setter records requests; the separate scene gate covers actual attribute changes and movement. Parent movement, complete LocalPlayer ticks, GUI options persistence, networking and full gameplay parity remain unfinished.'))
    print(f'{matched:,} sprint phase output lines match the pinned client: {report}',flush=True)

if __name__=='__main__':main()
