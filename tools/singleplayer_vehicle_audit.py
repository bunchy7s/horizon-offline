#!/usr/bin/env python3
"""Restore vanilla RootVehicle.Entity, attach to its passenger UUID, and save/reopen.

A fresh world's player file is given a horse with a pig passenger, present only
in RootVehicle.Entity, as ServerPlayer.saveParentVehicle writes it. Attach names
the pig rather than the root horse. Both loading this fixture and reopening its
Horizon save must retain one horse, one pig, and the correct mounted player.
Every process uses fresh scratch homes; only the fixture's saves are copied.
"""
import argparse
import gzip
import os
import shutil
import struct
import subprocess
import tempfile
import time
from pathlib import Path
import lab, anvil_dump
from agent_control_e2e import Cli
def text(v):
 b=v.encode();return struct.pack('>H',len(b))+b
def tag(t,n,b):return bytes([t])+text(n)+b
def uuid(n,v):return tag(11,n,struct.pack('>i4i',4,*v))
def entity(kind,u,pos,passengers=b''):
 return tag(8,'id',text('minecraft:'+kind))+uuid('UUID',u)+tag(9,'Pos',struct.pack('>bi3d',6,3,*pos))+tag(9,'Rotation',struct.pack('>bi2f',5,2,0,0))+tag(1,'NoAI',b'\1')+tag(1,'Tame',b'\1')+tag(5,'Health',struct.pack('>f',20))+passengers+b'\0'
def run(mode, bins, resources):
 with tempfile.TemporaryDirectory(prefix='vehicle-proof-') as tmp:
  work=Path(tmp); previous=None
  for stage in range(3):
   env=lab.game_env({k:v for k,v in os.environ.items() if k not in ('HORIZON_HOME','HORIZON_LAUNCHER_HOME','XDG_DATA_HOME','XDG_CONFIG_HOME')})
   home=Path(env['HORIZON_HOME'])
   if previous:shutil.copytree(previous,home/'saves')
   sock=work/f'{stage}.sock';cli=Cli(bins/'horizon-ctl',sock,home,env)
   args=[str(bins/'horizon-client'),'--resources',str(resources),'--world','VehicleProof','--creative','--offscreen','--render-distance','2','--control','--control-socket',str(sock)]
   if mode=='in-frame':args+=['--in-frame']
   with (work/f'{stage}.log').open('w') as out:
    p=subprocess.Popen(args,cwd=home,env=env,stdout=out,stderr=out)
    try:
     deadline=time.monotonic()+120
     while not sock.exists():
      if p.poll() is not None or time.monotonic()>deadline:raise RuntimeError((work/f'{stage}.log').read_text()[-2500:])
      time.sleep(.1)
     cli('wait_ready',seconds=240)
     if stage:
      for _ in range(30):
       cli('wait',ticks=5)
       status=cli('status');riding=status.get('riding')
       if riding is not None:break
      pigs=cli('entities',radius=32,type='pig')['entities'];horses=cli('entities',radius=32,type='horse')['entities']
      assert len(pigs)==len(horses)==1,(mode,stage,pigs,horses)
      assert riding==pigs[0]['id'],(mode,stage,riding,pigs,horses)
      print('PASS',mode,stage,'rides passenger pig; one pig and one root horse',flush=True)
     cli('quit');assert p.wait(timeout=120)==0
    finally:
     if p.poll() is None:p.terminate();p.wait(timeout=30)
   if stage==0:
    player=next((home/'saves'/'VehicleProof'/'players'/'data').glob('*.dat'))
    saved=anvil_dump.read_dat(player);pos=tuple(v[1] for v in saved['Pos'])
    child=entity('pig',[55,66,77,88],pos)
    root=entity('horse',[11,22,33,44],pos,tag(9,'Passengers',struct.pack('>bi',10,1)+child))
    raw=gzip.decompress(player.read_bytes());assert raw[-1:]==b'\0'
    wrapper=tag(10,'RootVehicle',uuid('Attach',[55,66,77,88])+tag(10,'Entity',root)+b'\0')
    player.write_bytes(gzip.compress(raw[:-1]+wrapper+b'\0'))
   previous=home/'saves'
if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bin-dir', type=Path, required=True)
    parser.add_argument('--resources', type=Path, default=lab.RESOURCES)
    parser.add_argument('--mode', choices=['server', 'in-frame', 'both'], default='both')
    args = parser.parse_args()
    for mode in (('server', 'in-frame') if args.mode == 'both' else (args.mode,)):
        run(mode, args.bin_dir.resolve(), args.resources.resolve())
