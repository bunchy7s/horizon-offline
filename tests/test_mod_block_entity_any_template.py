"""Run the real mod block entity checks (removal, piston, render list) with fakes."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from test_world_input_capture import method

ROOT = Path(__file__).resolve().parents[1]
LEVEL = ROOT / "crates/horizon-world/src/level"
CLIENT = ROOT / "crates/horizon-client/src"


def between(source, start, end):
    i = source.index(start) + len(start)
    return source[i:source.index(end, i)]


FIXTURE = r"""#![allow(unused)]
mod state_flags { pub const BLOCK_ENTITY: u32 = 1; }
#[derive(Clone, Copy, PartialEq, Debug)] struct BlockStateId(u32);
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)] struct BlockPos { x: i32, y: i32, z: i32 }
enum BlockEntity { Mod(u8), Chest(u8) }
struct Facts { block_entity: bool }
struct Res;
impl Res { fn facts(&self) -> &Self { self } fn get(&self, s: BlockStateId) -> Facts { Facts { block_entity: s.0 == 1 } } }
struct Blocks { templated: Vec<BlockStateId> }
impl Blocks { fn has_flag(&self, s: BlockStateId, _f: u32) -> bool { self.templated.contains(&s) } }
#[derive(Default)] struct Entities(std::collections::HashMap<(i32, i32, i32), BlockEntity>);
impl std::ops::Deref for Entities { type Target = std::collections::HashMap<(i32, i32, i32), BlockEntity>; fn deref(&self) -> &Self::Target { &self.0 } }
// BlockEntities::mods_near has its own test in horizon-client-render; here every mod entity is near.
impl Entities {
    fn has_mods(&self) -> bool { self.0.values().any(|e| matches!(e, BlockEntity::Mod(_))) }
    fn mods_near(&self, _x: f64, _z: f64, _r: f64) -> impl Iterator<Item = &(i32, i32, i32)> {
        self.0.iter().filter(|(_, e)| matches!(e, BlockEntity::Mod(_))).map(|(p, _)| p)
    }
}
struct DVec3 { x: f64, z: f64 }
const VIEW_DISTANCE: f64 = 64.0;
struct World { block_entities: Entities }
impl World { fn block(&self, x: i32, y: i32, z: i32) -> BlockStateId { BlockStateId((x != 0) as u32) } }
struct GameLevel { world: World }
impl GameLevel {
    HAS_MOD
}
struct Game { world: World, res: Res }
impl Game {
    UNTEMPLATED
}
struct Sim { b: Blocks, level: GameLevel }
impl Sim {
    fn keeps_block_entity(&self, _n: BlockStateId, _o: BlockStateId) -> bool { false }
    fn removes(&self, block_changed: bool, old: BlockStateId, state: BlockStateId, pos: BlockPos) -> bool {
        block_changed
            REMOVAL
    }
    fn pushable_tail(&self, state: BlockStateId, pos: BlockPos) -> bool {
        let b = &self.b;
        PISTON
    }
}
fn at(x: i32) -> BlockPos { BlockPos { x, y: 64, z: 0 } }
fn level(entities: &[(i32, BlockEntity)]) -> GameLevel {
    GameLevel { world: World { block_entities: Entities(entities.iter()
        .map(|(x, e)| ((*x, 64, 0), match e { BlockEntity::Mod(_) => BlockEntity::Mod(0), BlockEntity::Chest(_) => BlockEntity::Chest(0) }))
        .collect()) } }
}
#[test]
fn a_mod_block_entity_on_a_template_without_one_is_removed_and_not_pushed() {
    let stone = BlockStateId(0);
    let sim = Sim { b: Blocks { templated: vec![BlockStateId(1)] }, level: level(&[(5, BlockEntity::Mod(0))]) };
    // stone template, a mod entity here: removed with its block, never pushed
    assert!(sim.removes(true, stone, BlockStateId(9), at(5)));
    assert!(!sim.pushable_tail(stone, at(5)));
    // stone with no entity: nothing to remove, pushable as before
    assert!(!sim.removes(true, stone, BlockStateId(9), at(6)));
    assert!(sim.pushable_tail(stone, at(6)));
    // the same block replaced by itself (state change) keeps it
    assert!(!sim.removes(false, stone, BlockStateId(9), at(5)));
    // a template that has one still removes it, as before
    assert!(sim.removes(true, BlockStateId(1), BlockStateId(9), at(6)));
    assert!(!sim.pushable_tail(BlockStateId(1), at(6)));
    // a vanilla entity on a template without the flag is not a mod's
    let sim = Sim { b: Blocks { templated: vec![] }, level: level(&[(5, BlockEntity::Chest(0))]) };
    assert!(!sim.removes(true, stone, BlockStateId(9), at(5)));
}
#[test]
fn the_render_list_holds_mod_entities_of_templates_without_one() {
    let game = Game {
        world: World { block_entities: Entities([((0, 64, 0), BlockEntity::Mod(0)), ((1, 64, 0), BlockEntity::Mod(0)),
                                        ((2, 64, 0), BlockEntity::Chest(0))].into_iter().collect()) },
        res: Res,
    };
    // x == 0 gives state 0 (template without an entity); x != 0 state 1 (has one: the mesher lists it)
    let mut listed = game.untemplated_mod_block_entities(DVec3 { x: 0.0, z: 0.0 });
    listed.sort();
    assert_eq!(listed, vec![([0, 64, 0], 0)]);
}
"""


class ModBlockEntityAnyTemplateTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_removal_piston_and_render_list(self):
        game = (CLIENT / "game.rs").read_text(encoding="utf-8")
        draw = (CLIENT / "game/native_block_entity_draw.rs").read_text(encoding="utf-8")
        update = (LEVEL / "update.rs").read_text(encoding="utf-8")
        pistons = (LEVEL / "pistons.rs").read_text(encoding="utf-8")
        removal = between(update, "} else if block_changed\n", "        {\n            // BlockEntity.preRemoveSideEffects")
        tail = between(pistons, "        } else if b.table.bool(state, b.props.extended) {\n            return false;\n        }\n", "    }\n    fn is_slime_or_honey")
        fixture = (FIXTURE.replace("HAS_MOD", method(game, "has_mod_block_entity"))
                   .replace("UNTEMPLATED", method(draw.replace("pub(super) fn", "fn"), "untemplated_mod_block_entities"))
                   .replace("REMOVAL", removal.replace("self.b.has_flag(old, super::data::state_flags::BLOCK_ENTITY)", "self.b.has_flag(old, state_flags::BLOCK_ENTITY)"))
                   .replace("PISTON", tail.replace("super::data::state_flags::", "state_flags::")))
        fixture = fixture.replace("fn has_mod_block_entity(&self, pos: BlockPos)", "fn has_mod_block_entity(&self, pos: BlockPos)")
        with tempfile.TemporaryDirectory() as folder:
            src, exe = Path(folder) / "mbe.rs", Path(folder) / "mbe.exe"
            src.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", "--test", str(src), "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)
