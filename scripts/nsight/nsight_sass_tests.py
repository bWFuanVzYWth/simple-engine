# SPDX-FileCopyrightText: Copyright (c) 2026 linlin
# SPDX-License-Identifier: GPL-3.0-only
"""合成 nvdisasm 文本的静态索引行为；不依赖 NVIDIA 工具或 capture。"""

import json
from pathlib import Path
import tempfile
import unittest

from nsight_diagnostics.sass import lookup_instruction, parse_sass, read_sass


class SassTest(unittest.TestCase):
    def test_function_offsets_and_opcode_modifiers_remain_distinct(self):
        index = parse_sass('''
 .section .text.first,"ax",@progbits
 //## File "first.spv", line 90
 /*0000*/ @!P0 LDG.E.128.CONSTANT R4, [R2] ;
 /*0010*/ LDL.64 R0, [R1+-0x10] ;
 /*0020*/ STL.128 [R1], R4 ;
 .section .text.second,"ax",@progbits
 /*0000*/ TEX.B.LZ R0, R2, R4, 0x0, 2D, 0xf ;
 /*0010*/ TLD.B.LZ R0, R2, R4, 0x0, 2D, 0xf ;
 /*0020*/ STG.E.128 [R2], R4 ;
''')
        first = lookup_instruction(index, 'first', 0)
        self.assertEqual(first['mnemonic'], 'LDG.E.128.CONSTANT')
        self.assertEqual(first['sourceFile'], 'first.spv')
        self.assertEqual(first['sourceLine'], 90)
        self.assertTrue(first['instruction'].startswith('@!P0 LDG'))
        self.assertEqual(index['functions']['first']['opcodeCounts'], {'LDG': 1, 'LDL': 1, 'STL': 1})
        self.assertEqual(index['functions']['second']['opcodeCounts'], {'STG': 1, 'TEX': 1, 'TLD': 1})
        self.assertEqual(lookup_instruction(index, 'second', 0)['opcode'], 'TEX')
        self.assertIsNone(lookup_instruction(index, 'second', 0)['sourceLine'])
        self.assertEqual(index['functions']['second']['instructionCount'], 3)

    def test_line_annotations_keep_embedded_quotes_and_persist_until_changed(self):
        index = parse_sass('''
 .section .text.kernel,"ax",@progbits
 //## File "./"stage>rgs"fmt>spir"Hash>abcdef/rgen.spv", line 1234
 /*0000*/ MOV R0, R1 ;
 .L_label:
 /*0010*/ FADD R2, R0, R1 ;
 //## File "path, with spaces/source.slang", line 17
 /*0020*/ EXIT ;
''')
        self.assertEqual(lookup_instruction(index, 'kernel', 0x10)['sourceFile'],
                         './"stage>rgs"fmt>spir"Hash>abcdef/rgen.spv')
        self.assertEqual(lookup_instruction(index, 'kernel', 0x10)['sourceLine'], 1234)
        self.assertEqual(lookup_instruction(index, 'kernel', 0x20)['sourceLine'], 17)

    def test_duplicate_instruction_address_is_preserved_but_never_guessed(self):
        index = parse_sass('''
 .section .text.kernel,"ax",@progbits
 /*0000*/ MOV R0, R1 ;
 /*0000*/ LDG.E R0, [R1] ;
 /*0010*/ EXIT ;
''')
        function = index['functions']['kernel']
        self.assertIsNone(lookup_instruction(index, 'kernel', 0))
        self.assertEqual(lookup_instruction(index, 'kernel', 0x10)['opcode'], 'EXIT')
        self.assertEqual(function['status'], 'ambiguous')
        self.assertIsNone(function['instructionCount'])
        self.assertIsNone(function['opcodeCounts'])
        self.assertEqual(function['observedInstructionCount'], 3)
        self.assertEqual(len(function['instructions']['0x0000000000000000']), 2)
        self.assertEqual(index['diagnostics'][0]['offsetHex'], '0x0000000000000000')

    def test_data_directives_encoding_and_other_sections_are_not_instructions(self):
        index = parse_sass('''
 .section .debug_line,"",@progbits
 /*0000*/ .byte 0x00, 0x01
 /*0010*/ LDG.E R0, [R1] ;
 .section ".text.kernel","ax",@progbits
 /*0000*/ MOV R0, R1 ; /* 0x0000000000000000 */
 /*0010*/ /* 0x0000000000000000 */
 /*0020*/ .byte 0x00
 .section .nv.info.kernel,"",@progbits
 /*0030*/ STL [R0], R1 ;
''')
        function = index['functions']['kernel']
        self.assertEqual(function['instructionCount'], 1)
        self.assertEqual(lookup_instruction(index, 'kernel', 0)['instruction'], 'MOV R0, R1;')
        self.assertIsNone(lookup_instruction(index, 'kernel', 0x30))

    def test_unknown_pc_does_not_inherit_the_preceding_instruction(self):
        index = parse_sass('.section .text.kernel,"ax",@progbits\n/*0010*/ EXIT ;\n')
        for symbol, offset in (('absent', 0x10), ('kernel', 0), ('kernel', 0x11), ('kernel', -1)):
            with self.subTest(symbol=symbol, offset=offset):
                self.assertIsNone(lookup_instruction(index, symbol, offset))

    def test_path_input_and_json_round_trip_keep_the_same_evidence(self):
        text = '.section .text.kernel,"ax",@progbits\n//## File "测试.spv", line 8\n/*0000*/ EXIT ;\n'
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'kernel.sass.txt'
            path.write_text(text, encoding='utf-8')
            index = read_sass(path)
        self.assertEqual(index, parse_sass(text))
        restored = json.loads(json.dumps(index, ensure_ascii=False, allow_nan=False))
        self.assertEqual(lookup_instruction(restored, 'kernel', 0)['sourceFile'], '测试.spv')


if __name__ == '__main__':
    unittest.main()
