# SPDX-FileCopyrightText: Copyright (c) 2026 linlin
# SPDX-License-Identifier: GPL-3.0-only
"""使用合成 DebugInfo.100 操作数核对位置、作用域与失败边界，不依赖 capture。"""

import json
from pathlib import Path
import tempfile
import unittest

from nsight_diagnostics.spirv_debug import lookup_location, parse_spirv_debug, read_spirv_debug


HEADER = '; SPIR-V\n; Version: 1.4\n; Generator: fixture\n; Bound: 100\n; Schema: 0\n'
PREFIX = HEADER + '''%dbg = OpExtInstImport "NonSemantic.Shader.DebugInfo.100"
%uint = OpTypeInt 32 0
%zero = OpConstant %uint 0
%one = OpConstant %uint 1
%two = OpConstant %uint 2
%three = OpConstant %uint 3
%ten = OpConstant %uint 10
%twenty = OpConstant %uint 20
%name = OpString "callee"
%callerName = OpString "caller"
%outerName = OpString "outer"
%file = OpString "C:\\作品\\module.slang"
%source = OpExtInst %void %dbg DebugSource %file
%unit = OpExtInst %void %dbg DebugCompilationUnit %one %one %source %one
%callee = OpExtInst %void %dbg DebugFunction %name %type %source %one %zero %unit %name %zero %one
%caller = OpExtInst %void %dbg DebugFunction %callerName %type %source %two %zero %unit %callerName %zero %two
%outer = OpExtInst %void %dbg DebugFunction %outerName %type %source %three %zero %unit %outerName %zero %three
'''.replace('C:\\作品\\module.slang', 'C:\\\\作品\\\\module.slang')


def at(index, needle):
    line = next(number for number, text in enumerate(index['logicalLines'], 1) if needle in text)
    return lookup_location(index, line)


class SpirvDebugTests(unittest.TestCase):
    def test_explicit_source_line_and_nested_inline_callers(self):
        index = parse_spirv_debug(PREFIX + '''
%block = OpExtInst %void %dbg DebugLexicalBlock %source %one %zero %callee
%outerSite = OpExtInst %void %dbg DebugInlinedAt %twenty %outer
%innerSite = OpExtInst %void %dbg DebugInlinedAt %ten %caller %outerSite
%main = OpFunction %void None %type
%entry = OpLabel
%s = OpExtInst %void %dbg DebugScope %block %innerSite
%l = OpExtInst %void %dbg DebugLine %source %two %three %one %two
%value = OpIAdd %uint %one %two
OpReturn
OpFunctionEnd
''')
        self.assertEqual(index['status'], 'decoded')
        row = at(index, '%value =')
        self.assertEqual(row['sourceFile'], 'C:\\作品\\module.slang')
        self.assertEqual((row['sourceLine'], row['sourceLineEnd']), (2, 3))
        self.assertEqual((row['sourceColumn'], row['sourceColumnEnd']), (1, 2))
        self.assertEqual(row['function'], 'callee')
        self.assertEqual(row['spirvFunction'], '%main')
        self.assertEqual([(site['function'], site['sourceLine']) for site in row['inlinedCallChain']],
                         [('caller', 10), ('outer', 20)])

    def test_no_line_and_no_scope_clear_independently(self):
        index = parse_spirv_debug(PREFIX + '''
%main = OpFunction %void None %type
%entry = OpLabel
%s = OpExtInst %void %dbg DebugScope %callee
%l = OpExtInst %void %dbg DebugLine %source %two %two %zero %one
%a = OpIAdd %uint %one %two
%ns = OpExtInst %void %dbg DebugNoScope
%b = OpIAdd %uint %one %two
%s2 = OpExtInst %void %dbg DebugScope %caller
%nl = OpExtInst %void %dbg DebugNoLine
%c = OpIAdd %uint %one %two
OpReturn
OpFunctionEnd
''')
        self.assertIsNone(at(index, '%b =')['function'])
        self.assertEqual(at(index, '%b =')['sourceLine'], 2)
        self.assertEqual(at(index, '%c =')['function'], 'caller')
        self.assertIsNone(at(index, '%c =')['sourceLine'])
        self.assertEqual(at(index, '%c =')['status'], 'scopeOnly')
        self.assertEqual(at(index, '%c =')['functionId'], '%caller')
        self.assertEqual(at(index, '%c =')['functionSourceFile'], 'C:\\作品\\module.slang')

    def test_control_flow_and_function_boundaries_do_not_inherit_source(self):
        index = parse_spirv_debug(PREFIX + '''
%main = OpFunction %void None %type
%entry = OpLabel
%s = OpExtInst %void %dbg DebugScope %callee
%l = OpExtInst %void %dbg DebugLine %source %two %two %zero %one
OpBranch %next
%next = OpLabel
%value = OpIAdd %uint %one %two
OpReturn
OpFunctionEnd
%second = OpFunction %void None %type
%entry2 = OpLabel
%secondValue = OpIAdd %uint %one %two
OpReturn
OpFunctionEnd
''')
        for marker in ('%value =', '%secondValue ='):
            row = at(index, marker)
            self.assertIsNone(row['sourceLine'])
            self.assertIsNone(row['function'])

    def test_multiline_opstring_keeps_logical_numbering_and_escapes(self):
        text = PREFIX + '''%text = OpString "first line
second \\"quoted\\" line"
%main = OpFunction %void None %type
%entry = OpLabel
OpReturn
OpFunctionEnd
'''
        index = parse_spirv_debug(text)
        collapsed = '\n'.join(index['logicalLines']) + '\n'
        reparsed = parse_spirv_debug(collapsed)
        self.assertEqual(reparsed, index)
        self.assertEqual(index['logicalLines'][:5], HEADER.splitlines())
        string_line = next(line for line in index['logicalLines'] if line.startswith('%text ='))
        self.assertIn('first line\\nsecond', string_line)
        self.assertIsNone(lookup_location(index, 1))
        self.assertIsNone(lookup_location(index, len(index['logicalLines']) + 1))

    def test_unknown_profile_and_unknown_scope_never_guess_a_callee(self):
        unknown = parse_spirv_debug(PREFIX.replace('DebugInfo.100', 'DebugInfo.101'))
        self.assertEqual(unknown['status'], 'unavailable')
        self.assertEqual(unknown['locations'], {})
        index = parse_spirv_debug(PREFIX + '''
%main = OpFunction %void None %type
%entry = OpLabel
%s = OpExtInst %void %dbg DebugScope %missing
%l = OpExtInst %void %dbg DebugLine %source %two %two %zero %one
%value = OpIAdd %uint %one %two
OpReturn
OpFunctionEnd
''')
        row = at(index, '%value =')
        self.assertEqual(row['sourceLine'], 2)
        self.assertIsNone(row['function'])
        self.assertEqual(row['status'], 'partial')
        self.assertTrue(any('missing lexical scope' in d['reason'] for d in index['diagnostics']))

    def test_scope_and_inline_cycles_are_diagnosed_iteratively(self):
        for mode in ('scope', 'inline'):
            scope = '%cycle' if mode == 'scope' else '%callee'
            extra = ('%cycle = OpExtInst %void %dbg DebugLexicalBlock %source %one %zero %cycle\n'
                     if mode == 'scope' else
                     '%cycle = OpExtInst %void %dbg DebugInlinedAt %ten %caller %cycle\n')
            inline = '' if mode == 'scope' else ' %cycle'
            index = parse_spirv_debug(PREFIX + extra + f'''
%main = OpFunction %void None %type
%entry = OpLabel
%s = OpExtInst %void %dbg DebugScope {scope}{inline}
%value = OpIAdd %uint %one %two
OpReturn
OpFunctionEnd
''')
            with self.subTest(mode=mode):
                self.assertEqual(at(index, '%value =')['status'], 'partial')
                self.assertTrue(any('cyclic' in d['reason'] for d in index['diagnostics']))

    def test_unknown_inner_function_name_does_not_fall_back_to_outer_function(self):
        index = parse_spirv_debug(PREFIX + '''
%inner = OpExtInst %void %dbg DebugFunction %missing %type %source %one %zero %outer %missing %zero %one
%main = OpFunction %void None %type
%entry = OpLabel
%s = OpExtInst %void %dbg DebugScope %inner
%value = OpIAdd %uint %one %two
OpReturn
OpFunctionEnd
''')
        row = at(index, '%value =')
        self.assertIsNone(row['function'])
        self.assertEqual(row['functionId'], '%inner')
        self.assertTrue(any('DebugFunction Name' in d['reason'] for d in index['diagnostics']))

    def test_invalid_line_operands_and_zero_line_are_not_real_source_lines(self):
        for operands, diagnostic in (('%source %three %two %zero %one', True),
                                     ('%source %missing %two %zero %one', True),
                                     ('%source %zero %zero %zero %one', False)):
            index = parse_spirv_debug(PREFIX + f'''
%main = OpFunction %void None %type
%entry = OpLabel
%s = OpExtInst %void %dbg DebugScope %callee
%l = OpExtInst %void %dbg DebugLine {operands}
%value = OpIAdd %uint %one %two
OpReturn
OpFunctionEnd
''')
            with self.subTest(operands=operands):
                self.assertIsNone(at(index, '%value =')['sourceLine'])
                self.assertEqual(bool(index['diagnostics']), diagnostic)

    def test_unknown_debug_instruction_breaks_location_instead_of_inheriting(self):
        index = parse_spirv_debug(PREFIX + '''
%main = OpFunction %void None %type
%entry = OpLabel
%s = OpExtInst %void %dbg DebugScope %callee
%l = OpExtInst %void %dbg DebugLine %source %two %two %zero %one
%future = OpExtInst %void %dbg DebugFutureLocation %source
%value = OpIAdd %uint %one %two
OpReturn
OpFunctionEnd
''')
        self.assertIsNone(at(index, '%value =')['sourceLine'])
        self.assertIsNone(at(index, '%value =')['function'])
        self.assertEqual(index['status'], 'partial')

    def test_comments_do_not_open_strings_and_unterminated_input_is_unavailable(self):
        self.assertEqual(parse_spirv_debug('; comment with lone "\n' + PREFIX)['status'], 'decoded')
        result = parse_spirv_debug(PREFIX + '%text = OpString "unfinished')
        self.assertEqual(result['status'], 'unavailable')
        self.assertEqual(result['locations'], {})

    def test_file_and_json_round_trip_preserve_exact_lookup(self):
        text = PREFIX + '''
%main = OpFunction %void None %type
%entry = OpLabel
%s = OpExtInst %void %dbg DebugScope %callee
%l = OpExtInst %void %dbg DebugLine %source %two %two %zero %one
%value = OpIAdd %uint %one %two
OpReturn
OpFunctionEnd
'''
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'module.spvasm'
            path.write_text(text, encoding='utf-8')
            index = read_spirv_debug(path)
        restored = json.loads(json.dumps(index, ensure_ascii=False, allow_nan=False))
        self.assertEqual(at(restored, '%value ='), at(index, '%value ='))


if __name__ == '__main__':
    unittest.main()
