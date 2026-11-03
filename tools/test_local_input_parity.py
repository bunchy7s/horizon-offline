#!/usr/bin/env python3
"""Compare native keyboard direction and LocalPlayer input-speed preparation."""
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

def bits(value):return struct.pack('>f',value).hex()
def corpus():
    def row(keys,using,riding,slow,item,sneak,vector=None):
        result=dict(keys=keys,using=using,riding=riding,slow=slow,item=bits(item),sneak=bits(sneak))
        if vector is not None:result['vector']=list(map(bits,vector))
        return result
    for keys,using,riding,slow in itertools.product(range(128),[False,True],[False,True],[False,True]):
        for item,sneak in [(1,1),(.2,.3),(0,.3),(.2,0),(.5,.5),(.8,.9),(2,2),(-.2,.3),(.2,-.3)]:
            yield row(keys,using,riding,slow,item,sneak)
    special=[0,-0.0,1,-1,1e-23,1e-20,1e-4,.98,1e10,1e20,float('inf'),float('-inf'),float('nan')]
    for x,z in itertools.product(special,special):
        for item,sneak in [(1,1),(0,.3),(.2,.3),(float('nan'),1)]:
            yield row(0,True,False,True,item,sneak,(x,z))
    rng=random.Random(0x494e505554)
    for _ in range(4096):
        yield row(rng.randrange(128),bool(rng.randrange(2)),bool(rng.randrange(2)),bool(rng.randrange(2)),rng.uniform(0,2),rng.uniform(0,1),(rng.uniform(-2,2),rng.uniform(-2,2)))

def main():
    lock,jars,_=base.verify_artifacts();libraries=baking.client_libraries()
    build=lab.LOCAL/'rust-local-input';build.mkdir(exist_ok=True)
    report=build/'report.json';report.unlink(missing_ok=True)
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaLocalInputOracle.java'
    sources=[Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/test_model_baking_parity.py',lab.ROOT/'tools/lab.py',oracle,lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-client/Cargo.toml',lab.ROOT/'crates/horizon-client-input/src/input.rs',lab.ROOT/'crates/horizon-client/src/lib.rs',lab.ROOT/'crates/horizon-client/examples/probe/input_probe.rs']
    tested=base.source_hashes(sources)
    def run(command,name):
        with (build/(name+'.log')).open('w') as log:subprocess.run(list(map(str,command)),cwd=lab.ROOT,check=True,timeout=600,stdout=log,stderr=subprocess.STDOUT)
    fixtures=list(corpus());inputs=build/'inputs.jsonl'
    inputs.write_text(''.join(json.dumps(f,separators=(',',':'))+'\n' for f in fixtures));input_hash=lab.digest(inputs,'sha256')
    classes=build/'classes';classes.mkdir(exist_ok=True)
    dependencies=os.pathsep.join(map(str,[jars['client'],*libraries]))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',dependencies,'-d',classes,oracle],'compile-java')
    run(['cargo','build','--locked','-p','horizon-client-lib','--example','client_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--locked','--no-deps','--format-version','1'],cwd=lab.ROOT))
    binary=Path(metadata['target_directory'])/'debug/examples'/('client_probe.exe' if os.name=='nt' else 'client_probe')
    run([binary,'input_probe',inputs,build/'rust.txt'],'rust')
    run(['java','-Xmx768M','-cp',str(classes)+os.pathsep+dependencies,'dev.horizonlab.parity.VanillaLocalInputOracle',inputs,build/'client.txt'],'client')
    matched=base.compare(build/'client.txt',build/'rust.txt')
    if matched!=len(fixtures):raise AssertionError('Incomplete input output')
    base.ensure_sources_unchanged(sources,tested)
    if input_hash!=lab.digest(inputs,'sha256'):raise AssertionError('Input fixtures changed')
    lab.write_json(report,dict(schema=1,minecraft=lock['minecraft']['version'],manifest_sha1=lock['minecraft']['manifest_sha1'],platform=platform.platform(),rustc=subprocess.check_output(['rustc','--version'],text=True).strip(),java=subprocess.check_output(['java','--version'],text=True).splitlines()[0],fixtures=len(fixtures),sources_sha256=tested,input_sha256=input_hash,client_jar_sha256=lab.digest(jars['client'],'sha256'),matched_output_lines=matched,output_sha256=lab.digest(build/'rust.txt','sha256'),scope='Actual KeyboardInput.tick direction and private LocalPlayer.modifyInput compared by float bits. All 128 key combinations, input-use/riding/slow gates, supplied use-effects and sneaking multipliers, analogue vectors, signed zeros and nonfinite arithmetic. Native item speed reads a real ItemStack USE_EFFECTS component. Does not establish the full LocalPlayer tick, input remapping, sprint eligibility, poses, camera bob or network lifecycle.'))
    print(f'{matched:,} local input cases match the pinned client: {report}',flush=True)

if __name__=='__main__':main()
