#!/usr/bin/env python3
"""Execute the production blob selection and compiled-caster methods in isolation.

This does not assert source spelling or duplicate the decision under test. The
Rust statements are extracted unchanged; stand-ins supply the surrounding GPU
handles and atlas. The companion entity_shadow_ownership example builds actual
pipelines. Run this against an unfixed checkout with --source-root for the
before control. No game, assets, launcher or player data are used.
"""
import argparse
from pathlib import Path
import subprocess
import tempfile


def method(source, name):
    start = source.index(f"pub fn {name}(")
    opening = source.index("{", start)
    depth = 1
    end = opening + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end]


def program(root):
    gfx = (root / "crates/horizon-client-render/src/gfx.rs").read_text()
    post = (root / "crates/horizon-client-render/src/gfx/pack_post.rs").read_text()
    game = (root / "crates/horizon-client/src/game.rs").read_text()
    geometry = game.index("fn entity_geometry(")
    start = game.index("let pack_shadows =", geometry)
    end = game.index("if let Some(sprite) = shadow_sprite", start)
    guard = game[start:end]
    getter = method(gfx, "pack_entity_shadows") if "pub fn pack_entity_shadows(" in gfx else ""
    support = r'''
#![allow(dead_code)]
mod pack_entities {
    #[derive(Clone, Copy, Default)]
    pub struct Casters {
        pub entities: bool, pub only_player: bool, pub blocks: bool, pub only_lit: bool,
    }
}
mod post {
    pub struct Entities { pub shadow: [Option<()>; 2], pub only: (bool, bool) }
    pub struct PackPost { pub entities: Option<Entities>, pub shadow_targets: Option<()> }
    impl PackPost { __CASTERS__ }
}
struct Native { shadow: Option<()> }
struct Renderer { pack_post: Option<(i32, Option<post::PackPost>)>, native: Option<Native> }
impl Renderer { __NATIVE__ __GETTER__ }
#[derive(Clone, Copy, PartialEq)]
enum Dim { Overworld, Nether, End }
struct Settings { entity_shadows: bool }
struct Atlas { sprites: std::collections::HashMap<&'static str, usize> }
struct Gui { atlas: Atlas }
struct Game { dim: Dim, renderer: Option<Renderer>, settings: Settings, gui: Gui }
impl Game {
    fn selected_blob(&self) -> Option<usize> { __GUARD__ shadow_sprite }
}
fn scene(kind: &str, dim: Dim, enabled: bool, sprite: bool) -> Game {
    let casters = |entities: bool, only_player: bool, blocks: bool, targets: bool| {
        Some((0, Some(post::PackPost {
            entities: Some(post::Entities {
                shadow: [entities.then_some(()), blocks.then_some(())], only: (only_player, false),
            }),
            shadow_targets: targets.then_some(()),
        })))
    };
    let pack_post = match kind {
        "full" => casters(true, false, true, true),
        "player_only" => casters(true, true, true, true),
        "blocks_only" => casters(false, false, true, true),
        "failed_pipeline" => casters(false, false, false, true),
        "disabled_targets" => casters(true, false, true, false),
        "missing_entities" => Some((0, Some(post::PackPost { entities: None, shadow_targets: Some(()) }))),
        "pending" => Some((0, None)),
        _ => None,
    };
    Game {
        dim,
        renderer: (kind != "no_renderer").then_some(Renderer {
            pack_post,
            native: (kind == "native").then_some(Native { shadow: Some(()) }),
        }),
        settings: Settings { entity_shadows: enabled },
        gui: Gui { atlas: Atlas { sprites: if sprite {
            [("misc/shadow", 73)].into_iter().collect()
        } else { Default::default() } } },
    }
}
'''
    source = (support.replace("__CASTERS__", method(post, "shadow_casters"))
              .replace("__NATIVE__", method(gfx, "native_shadow"))
              .replace("__GETTER__", getter).replace("__GUARD__", guard))
    # Independent expected outcomes: a full standard entity caster owns the
    # mob shadow in each dimension. Native shadows own it only in the Overworld.
    rows = [
        ("full", [None, None, None]),
        ("player_only", [73, 73, 73]),
        ("blocks_only", [73, 73, 73]),
        ("failed_pipeline", [73, 73, 73]),
        ("disabled_targets", [73, 73, 73]),
        ("missing_entities", [73, 73, 73]),
        ("pending", [73, 73, 73]),
        ("no_pack", [73, 73, 73]),
        ("no_renderer", [73, 73, 73]),
        ("native", [None, 73, 73]),
    ]
    for kind, expected in rows:
        for dimension, wanted in zip(["Overworld", "Nether", "End"], expected):
            result = "None" if wanted is None else f"Some({wanted})"
            source += f'''
#[test] fn {kind}_{dimension.lower()}() {{
    assert_eq!(scene("{kind}", Dim::{dimension}, true, true).selected_blob(), {result});
    assert_eq!(scene("{kind}", Dim::{dimension}, false, true).selected_blob(), None);
    assert_eq!(scene("{kind}", Dim::{dimension}, true, false).selected_blob(), None);
}}
'''
    return source


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-root", type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="horizon-shadow-ownership-") as folder:
        source = Path(folder) / "ownership.rs"
        source.write_text(program(args.source_root))
        binary = Path(folder) / "ownership"
        subprocess.run(["rustc", "--edition=2024", "--test", str(source), "-o", str(binary)], check=True)
        return subprocess.run([str(binary), "--nocapture"]).returncode


if __name__ == "__main__":
    raise SystemExit(main())
