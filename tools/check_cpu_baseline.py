#!/usr/bin/env python3
"""Fail when an x86_64 release binary uses an instruction that baseline x86-64 CPUs lack.

The game must start on players' oldest PCs: x86-64 with SSE2 only (Core 2, AMD K8). A
single unguarded POPCNT killed it with 0xc000001d before `main` on a Core 2 (#7264):
mimalloc's C code, built by MSVC, whose `__popcnt` intrinsic is the instruction whatever
the target.

Only POPCNT and LZCNT are checked. They are what C compilers emit for count intrinsics, and
no runtime-dispatched code in the game uses them today. AVX, BMI2, SSSE3 and SSE4 do appear,
always behind a CPUID check (ring's assembly, memchr, half's F16C, compiler_builtins' FMA,
the MSVC C runtime), and the Windows programs have no symbols to tell those functions from
unguarded ones. `tzcnt` is not checked either: LLVM writes `rep bsf` for trailing_zeros,
which disassembles as tzcnt and runs as bsf on every CPU.

Needs llvm-objdump: $LLVM_OBJDUMP, rustup's llvm-tools component, or one on PATH.
"""

import argparse
import glob
import os
import re
import shutil
import subprocess
import sys

FORBIDDEN = ("popcnt", "lzcnt")
LABEL = re.compile(r"^([0-9a-f]+) <(.*)>:$")
INSTRUCTION = re.compile(r"^\s*([0-9a-f]+):\s+(\S+)")


def find_objdump():
    if os.environ.get("LLVM_OBJDUMP"):
        return os.environ["LLVM_OBJDUMP"]
    try:
        sysroot = subprocess.run(["rustc", "--print", "sysroot"], capture_output=True, text=True,
                                 check=True).stdout.strip()
        found = glob.glob(os.path.join(sysroot, "lib", "rustlib", "*", "bin", "llvm-objdump*"))
        if found:
            return found[0]
    except (OSError, subprocess.CalledProcessError):
        pass
    return shutil.which("llvm-objdump")


def forbidden_uses(lines):
    """(address, enclosing symbol, mnemonic) of each forbidden instruction in llvm-objdump -d output."""
    symbol = "?"
    for line in lines:
        line = line.rstrip("\n")
        label = LABEL.match(line)
        if label:
            symbol = label.group(2)
            continue
        insn = INSTRUCTION.match(line)
        if not insn:
            continue
        mnemonic = insn.group(2)
        # AT&T syntax adds the operand size: popcntq, lzcntl.
        base = mnemonic[:-1] if mnemonic[-1:] in "wlq" and mnemonic[:-1] in FORBIDDEN else mnemonic
        if base in FORBIDDEN:
            yield insn.group(1), symbol, base


def check(objdump, path):
    proc = subprocess.Popen([objdump, "-d", "--no-show-raw-insn", path], stdout=subprocess.PIPE,
                            text=True, errors="replace")
    uses = list(forbidden_uses(proc.stdout))
    if proc.wait() != 0:
        sys.exit(f"llvm-objdump failed on {path}")
    for address, symbol, mnemonic in uses[:20]:
        print(f"{path}: {mnemonic} at 0x{address} in {symbol}")
    if len(uses) > 20:
        print(f"{path}: ... {len(uses) - 20} more")
    return not uses


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("binaries", nargs="+")
    args = parser.parse_args()
    objdump = find_objdump()
    if not objdump:
        sys.exit("llvm-objdump not found: rustup component add llvm-tools, or set LLVM_OBJDUMP")
    failed = [path for path in args.binaries if not check(objdump, path)]
    if failed:
        print(f"::error::{', '.join(failed)}: {'/'.join(FORBIDDEN)} without a CPU check crash CPUs "
              "older than Nehalem (#7264); find the C library or target_feature that emits them")
        sys.exit(1)
    print(f"no {'/'.join(FORBIDDEN)} in {len(args.binaries)} binaries")


if __name__ == "__main__":
    main()
