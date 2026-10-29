#!/usr/bin/env python3
"""Compare real model-shaped breaking geometry and projected UVs with vanilla 26.3."""
import json
import itertools
try:
    from . import lab, oracle_harness as harness, test_model_baking_parity as baking, test_rust_parity as base, test_section_mesh_parity as mesh
except ImportError:
    import lab, oracle_harness as harness, test_model_baking_parity as baking, test_rust_parity as base, test_section_mesh_parity as mesh

BLOCKS = ["stone", "oak_slab", "stone_slab", "oak_stairs", "stone_stairs", "oak_fence", "iron_bars", "glass_pane", "white_stained_glass_pane", "glass", "tinted_glass", "cobblestone_wall", "oak_fence_gate", "oak_trapdoor", "iron_trapdoor", "oak_door", "white_carpet", "moss_carpet", "pale_moss_carpet", "snow", "cake", "candle", "sea_pickle", "dandelion", "tall_grass", "bamboo", "torch", "wall_torch", "lantern", "chain", "end_rod", "lightning_rod", "lever", "stone_button", "repeater", "comparator", "farmland", "enchanting_table", "lectern", "scaffolding", "anvil", "brewing_stand", "hopper", "cauldron", "composter", "potted_fern", "lily_pad", "sunflower", "chest", "air", "water"]


def cases(resources):
    states=mesh.States(resources)
    ids=sorted({i for block in BLOCKS for i in states.by_name.get("minecraft:"+block, [])})
    for state, (pos, camera) in itertools.product(ids, [([2,97,3],[.5,98.62,.5]), ([-23,11,-19],[-25.25,12.125,-15.5])]):
        yield dict(state=state,pos=pos,camera=camera)


def canonical(quads):
    result=[]
    for quad in quads:
        # Only cyclic rotation is allowed, so reversed winding cannot pass.
        rotations=[quad[i:]+quad[:i] for i in range(4)]
        result.append(min(rotations,key=lambda q:tuple(round(v/1e-4) for p in q for v in p[:3])))
    return sorted(result,key=lambda q:tuple(round(v/1e-4) for p in q for v in p[:3]))


def main():
    lock,jars,_=base.verify_artifacts();libraries=baking.client_libraries()
    resources=lab.RESOURCES;build=lab.LOCAL/"breaking-model-parity";build.mkdir(exist_ok=True)
    paths=[lab.ROOT/"tools/test_breaking_model_parity.py",lab.ROOT/"crates/horizon-client-render/src/breaking.rs",lab.ROOT/"crates/horizon-client/src/game.rs",lab.ROOT/"crates/horizon-client-render/src/gfx.rs",lab.ROOT/"crates/horizon-client-render/src/entity.wgsl",lab.ROOT/"crates/horizon-client-render/src/models/catalog.rs",lab.ROOT/"crates/horizon-client-render/src/mesher.rs",lab.ROOT/"crates/horizon-client-render/src/camera.rs",lab.ROOT/"crates/horizon-client/examples/probe/breaking_fixture.rs",lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBreakingModelOracle.java",lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaSectionMeshOracle.java",lab.ROOT/"Cargo.lock"]
    tested=base.source_hashes(paths)
    rows=[dict(case=i,**row) for i,row in enumerate(cases(resources))]
    inputs=build/"inputs.jsonl";inputs.write_text("".join(json.dumps(row)+"\n" for row in rows))
    classes,dependencies,_=harness.compile_oracle(build,"VanillaBreakingModelOracle",jars,libraries,lock["minecraft"]["java_major"])
    command=harness.build_probe(build,"breaking_fixture")
    harness.run_logged([*command,resources,inputs,build/"rust.jsonl"],build,"rust")
    harness.java_run(build,classes,dependencies,"VanillaBreakingModelOracle",[inputs,build/"client.jsonl"],"client")
    count=vertices=0;max_error=0.0
    with (build/"client.jsonl").open() as native, (build/"rust.jsonl").open() as rust:
        for v,r in itertools.zip_longest(native,rust):
            assert v is not None and r is not None,"output length differs"
            v,r=json.loads(v),json.loads(r)
            assert v["case"]==r["case"],"case order differs"
            assert v["translucent"]==r["translucent"],(rows[count],"wrong render phase",v["translucent"],r["translucent"])
            vq,rq=canonical(v["quads"]),canonical(r["quads"])
            assert len(vq)==len(rq),(rows[count],"quad count",len(vq),len(rq))
            for a,b in zip(vq,rq):
                for vp,rp in zip(a,b):
                    for axis,(expected,got) in enumerate(zip(vp,rp)):
                        error=abs(expected-got);max_error=max(max_error,error)
                        assert error<=1e-4,(rows[count],axis,expected,got,error)
                    vertices+=1
            count+=1
    assert count==len(rows);base.ensure_sources_unchanged(paths,tested)
    lab.write_json(build/"report.json",dict(cases=count,vertices=vertices,max_error=max_error,sources_sha256=tested,client_jar_sha256=lab.digest(jars["client"],"sha256"),scope="Selected baked model quads, block offsets, winding, projected crack UVs, white/fullbright vertices and translucent phase. Not complete framebuffer or block-entity overlay parity."))
    print(f"{count:,} breaking model cases / {vertices:,} vertices match vanilla; max error {max_error:g}")


if __name__=="__main__":
    main()
