#!/usr/bin/env python3
"""Benchmark the actual gfx CPU upload path, isolated from GPU/window startup.

Run under the shared heavy-work flock. Uses a private .local Cargo workspace;
--ref selects an unmodified baseline, otherwise reads the working tree.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess

try:
    from . import lab
except ImportError:
    import lab

ROOT = Path(__file__).resolve().parents[1]


def function(source, name):
    start = source.index(f'fn {name}(')
    brace = source.index('{', start)
    depth = 1
    end = brace + 1
    while depth:
        depth += (source[end] == '{') - (source[end] == '}')
        end += 1
    return source[start:end]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ref')
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument('--test', action='store_true', help='run extracted production upload tests')
    mode.add_argument('--build-only', action='store_true', help='build without measuring')
    parser.add_argument('--label', default='candidate')
    parser.add_argument('--resources', type=Path, default=lab.RESOURCES)
    args = parser.parse_args()
    path = 'crates/horizon-client-render/src/gfx.rs'
    if args.ref:
        # gfx.rs moved from horizon-client to horizon-client-render; older refs have it at its old path.
        shown = subprocess.run(['git', 'show', f'{args.ref}:{path}'], cwd=ROOT, text=True, capture_output=True)
        source = shown.stdout if shown.returncode == 0 else subprocess.check_output(
            ['git', 'show', f'{args.ref}:crates/horizon-client/src/gfx.rs'], cwd=ROOT, text=True)
    else:
        source = (ROOT / path).read_text()
    build = ROOT / '.local/sprite-mips-bench' / args.label
    build.mkdir(parents=True, exist_ok=True)
    cached = 'struct AnimatedSprites' in source
    code = function(source, 'mip_chain') + '\n'
    if cached:
        code += source[source.index('const ANIMATION_MIP_BUDGET'):source.index('#[cfg(test)]\nmod sprite_upload_tests')]
    else:
        code += function(source, 'animate_sprites')
    baseline = subprocess.check_output(['git', 'show', '89b58991bbc02eb79b6965fc4ca56357fe2297d9:crates/horizon-client/src/gfx.rs'], cwd=ROOT, text=True)
    code += '\n' + function(baseline, 'animate_sprites').replace('fn animate_sprites(', 'fn reference_animate_sprites(', 1)
    if args.test:
        # sprites.rs holds it since the gfx -> sprites cut; older refs kept it in gfx.rs.
        code += '\n' + (function(source, 'sprite_mipmap_limit') if 'fn sprite_mipmap_limit' in source
                         else 'use sprites::sprite_mipmap_limit;')
        code += '\n' + source[source.index('#[cfg(test)]\nmod sprite_upload_tests'):]
    modules = '\n'.join(f'#[path = {json.dumps(str(ROOT / "crates/horizon-client-render/src" / (name + ".rs")))}] mod {name};'
                        for name in ('mipmap', 'sprites', 'fast_hash'))
    (build / 'main.rs').write_text(modules + '\nuse sprites::SpriteAnimation;\n'
                                  + code + '\n' + (ROOT / 'tools/bench_sprite_mips.rs').read_text())
    (build / 'Cargo.toml').write_text(f'''[package]
name = "sprite-mips-{args.label}"
version = "0.0.0"
edition = "2024"
[workspace]
[[bin]]
name = "sprite-mips-{args.label}"
path = "main.rs"
[dependencies]
horizon-assets = {{ path = "{ROOT / 'crates/horizon-assets'}" }}
png = "0.18"
serde_json = {{ version = "1.0.151", features = ["raw_value"] }}
log = "0.4"
[features]
cached = []
[profile.release]
lto = false
codegen-units = 16
''')
    # Include static sprites too: the largest layer sets the animation upload
    # resolution (the prepared straw-bed texture makes this a 64px array).
    names = [f'minecraft:{folder}/{image.stem}'
             for folder in ('block', 'item')
             for image in sorted((args.resources / f'assets/minecraft/textures/{folder}').glob('*.png'))]
    (build / 'names.txt').write_text('\n'.join(names))
    command = ['cargo', 'test' if args.test else 'build' if args.build_only else 'run', '--release', '--manifest-path', str(build / 'Cargo.toml')]
    if cached:
        command += ['--features', 'cached']
    if not args.test and not args.build_only:
        command += ['--', str(args.resources), str(build / 'names.txt')]
    subprocess.run(command, cwd=ROOT, check=True, env=dict(os.environ, CARGO_TARGET_DIR=str(ROOT / '.local/sprite-mips-bench/target')))


if __name__ == '__main__':
    main()
