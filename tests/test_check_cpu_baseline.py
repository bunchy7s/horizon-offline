import unittest

from tools import check_cpu_baseline as baseline

# llvm-objdump -d --no-show-raw-insn output: a stripped Windows program (only section labels) and an ELF one.
DISASSEMBLY = """
/tmp/horizon-client.exe:\tfile format coff-x86-64

Disassembly of section .text:

0000000140001000 <.text>:
140001000:     \tpopcntq\t%rdx, %rax
140001005:     \ttzcntl\t%ecx, %eax
14000100a:     \tvpaddd\t%ymm0, %ymm1, %ymm2
0000000140002000 <mi_bitmap_popcount>:
140002000:     \tlzcntq\t%r8, %r9
140002005:     \tmovq\t%rax, %rbx
140002008:     \tpopcnt\trax, rcx
"""


class ForbiddenUsesTests(unittest.TestCase):
    def test_reports_popcnt_and_lzcnt_with_their_symbol_and_nothing_else(self):
        uses = list(baseline.forbidden_uses(DISASSEMBLY.splitlines(keepends=True)))
        self.assertEqual(uses, [
            ("140001000", ".text", "popcnt"),
            ("140002000", "mi_bitmap_popcount", "lzcnt"),
            ("140002008", "mi_bitmap_popcount", "popcnt"),
        ])


if __name__ == "__main__":
    unittest.main()
