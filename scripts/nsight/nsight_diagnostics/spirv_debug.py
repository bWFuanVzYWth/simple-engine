# SPDX-FileCopyrightText: Copyright (c) 2026 linlin
# SPDX-License-Identifier: GPL-3.0-only
"""迭代读取 spirv-dis 的 NonSemantic.Shader.DebugInfo.100 源位置，不推断执行成本。

操作数依据 Khronos SPIRV-Headers 的 DebugInfo.100 grammar（SDK rev6），作用域边界依据
https://github.khronos.org/SPIRV-Registry/nonsemantic/NonSemantic.Shader.DebugInfo.html 。
只解析明确的 100 profile；不是 SPIR-V 验证器，不验证类型、变量位置或完整调试表达式。
logical 行将引号内物理换行折为转义换行，保留引号外所有行（包括 spirv-dis 五行头）。
"""

import json
from pathlib import Path
import re


PROFILE = 'NonSemantic.Shader.DebugInfo.100'
_OP = re.compile(r'^\s*(?:(%\S+)\s*=\s*)?(Op[A-Za-z0-9]+)(?:\s+(.*))?\s*$')
_ARITIES = {
    'DebugInfoNone': (0, 0), 'DebugSource': (1, 2), 'DebugCompilationUnit': (4, 4),
    'DebugFunction': (9, 10), 'DebugLexicalBlock': (4, 5),
    'DebugLexicalBlockDiscriminator': (3, 3), 'DebugTypeComposite': (9, None),
    'DebugInlinedAt': (2, 3), 'DebugScope': (1, 2), 'DebugNoScope': (0, 0),
    'DebugLine': (5, 5), 'DebugNoLine': (0, 0), 'DebugFunctionDefinition': (2, 2),
}
_KNOWN = set('''DebugInfoNone DebugCompilationUnit DebugTypeBasic DebugTypePointer
DebugTypeQualifier DebugTypeArray DebugTypeVector DebugTypedef DebugTypeFunction DebugTypeEnum
DebugTypeComposite DebugTypeMember DebugTypeInheritance DebugTypePtrToMember DebugTypeTemplate
DebugTypeTemplateParameter DebugTypeTemplateTemplateParameter DebugTypeTemplateParameterPack
DebugGlobalVariable DebugFunctionDeclaration DebugFunction DebugLexicalBlock
DebugLexicalBlockDiscriminator DebugScope DebugNoScope DebugInlinedAt DebugLocalVariable
DebugInlinedVariable DebugDeclare DebugValue DebugOperation DebugExpression DebugMacroDef
DebugMacroUndef DebugImportedEntity DebugSource DebugFunctionDefinition DebugSourceContinued
DebugLine DebugNoLine DebugBuildIdentifier DebugStoragePath DebugEntryPoint DebugTypeMatrix'''.split())
_TERMINATORS = {
    'OpBranch', 'OpBranchConditional', 'OpSwitch', 'OpReturn', 'OpReturnValue', 'OpKill',
    'OpUnreachable', 'OpTerminateInvocation', 'OpIgnoreIntersectionKHR', 'OpTerminateRayKHR',
    'OpEmitMeshTasksEXT',
}


def _logical_lines(text):
    lines, current = [], []
    quoted, escaped, comment = False, False, False
    for char in text.replace('\r\n', '\n'):
        if char == '\n':
            if quoted:
                current.append('\\n')
            else:
                lines.append(''.join(current))
                current = []
            escaped = False
            comment = False
            continue
        current.append(char)
        if comment:
            continue
        if not quoted and char == ';':
            comment = True
        elif escaped:
            escaped = False
        elif quoted and char == '\\':
            escaped = True
        elif char == '"':
            quoted = not quoted
    if quoted:
        raise ValueError('unterminated quoted string in spirv-dis listing')
    if current:
        lines.append(''.join(current))
    return lines


class _Resolver:
    def __init__(self, strings, constants, nodes, diagnostics):
        self.strings, self.constants, self.nodes = strings, constants, nodes
        self.diagnostics = diagnostics
        self.reported = set()
        self.scopes, self.chains = {}, {}

    def issue(self, reason, identifier=None, line=None):
        key = (reason, identifier, line)
        if key not in self.reported:
            self.reported.add(key)
            self.diagnostics.append({'reason': reason, 'id': identifier, 'logicalLine': line})

    def number(self, identifier):
        value = self.constants.get(identifier)
        if value is None:
            self.issue('expected a nonnegative 32-bit integer OpConstant', identifier)
        return value

    def source(self, identifier):
        node = self.nodes.get(identifier)
        if node is None or node['name'] != 'DebugSource':
            self.issue('missing or unsupported DebugSource', identifier)
            return None
        value = self.strings.get(node['args'][0])
        if value is None:
            self.issue('DebugSource File is not a known OpString', identifier, node['line'])
        return value

    def scope(self, identifier):
        if identifier is None:
            return None
        if identifier in self.scopes:
            return self.scopes[identifier]
        at, seen = identifier, set()
        result = {'function': None, 'functionId': None, 'functionSourceFile': None, 'sourceFile': None}
        while at is not None:
            if at in seen:
                self.issue('cyclic lexical scope chain', at)
                result = None
                break
            seen.add(at)
            node = self.nodes.get(at)
            if node is None:
                self.issue('missing lexical scope', at)
                result = None
                break
            name, args = node['name'], node['args']
            if name == 'DebugInfoNone':
                result = None
                break
            if name in ('DebugFunction', 'DebugTypeComposite'):
                source, parent = args[2], args[5]
                if name == 'DebugFunction' and result['functionId'] is None:
                    result['function'] = self.strings.get(args[0])
                    result['functionId'] = at
                    result['functionSourceFile'] = self.source(source)
                    if result['function'] is None:
                        self.issue('DebugFunction Name is not a known OpString', at, node['line'])
            elif name == 'DebugLexicalBlock':
                source, parent = args[0], args[3]
            elif name == 'DebugLexicalBlockDiscriminator':
                source, parent = args[0], args[2]
            elif name == 'DebugCompilationUnit':
                source, parent = args[2], None
            else:
                self.issue('unsupported lexical scope instruction', at, node['line'])
                result = None
                break
            path = self.source(source)
            if result['sourceFile'] is None:
                result['sourceFile'] = path
            at = parent
        self.scopes[identifier] = result
        return result

    def inline_chain(self, identifier):
        if identifier is None:
            return []
        if identifier in self.chains:
            return self.chains[identifier]
        at, seen, result = identifier, set(), []
        while at is not None:
            if at in seen:
                self.issue('cyclic DebugInlinedAt chain', at)
                result = None
                break
            seen.add(at)
            node = self.nodes.get(at)
            if node is not None and node['name'] == 'DebugInfoNone':
                break
            if node is None or node['name'] != 'DebugInlinedAt':
                self.issue('missing or unsupported DebugInlinedAt', at)
                result = None
                break
            args = node['args']
            scope, line = self.scope(args[1]), self.number(args[0])
            if scope is None or line is None:
                result = None
                break
            result.append({'sourceFile': scope['sourceFile'], 'sourceLine': line or None,
                           'function': scope['function'], 'functionId': scope['functionId'],
                           'functionSourceFile': scope['functionSourceFile'], 'inlinedAtId': at})
            at = args[2] if len(args) == 3 else None
        self.chains[identifier] = result
        return result

    def line(self, args):
        path = self.source(args[0])
        numbers = [self.number(value) for value in args[1:]]
        if path is None or any(value is None for value in numbers):
            return None
        start, end, column, column_end = numbers
        if end < start or (start == end and column_end < column):
            self.issue('invalid DebugLine source range', args[0])
            return None
        return {'sourceFile': path, 'sourceLine': start or None, 'sourceLineEnd': end or None,
                'sourceColumn': column, 'sourceColumnEnd': column_end}


def parse_spirv_debug(text):
    """读取原始或已折叠的 spirv-dis 文本，返回 JSON 友好的 logicalLines／locations／diagnostics。

    locations 的键是从1开始的 logical 行号字符串；只索引函数内指令。sourceLine=0 按规范
    转为 None；DebugNoLine 与 DebugNoScope 独立清除状态，block 末清除两者，不补猜前一块。
    function／functionId／functionSourceFile 是最近 DebugFunction 的名称／id／声明文件，
    与 DebugLine 的 sourceFile 独立；spirvFunction 是包含指令的 SPIR-V id。
    inlinedCallChain 按最近 caller 到最外 caller 排列；未知／循环链为 None，并保留诊断。
    此处的源码映射来自模块调试记录，不证明实际 specialization 或 PC 的采样耗时。
    """
    diagnostics = []
    try:
        lines = _logical_lines(text)
    except ValueError as error:
        return {'profile': None, 'status': 'unavailable', 'logicalLines': [], 'locations': {},
                'diagnostics': [{'reason': str(error)}]}
    imports, strings, types, constants, nodes, instructions = {}, {}, {}, {}, {}, []
    seen_ids, invalid_ids = set(), set()
    for number, raw in enumerate(lines, 1):
        matched = _OP.match(raw)
        if matched is None:
            continue
        identifier, opcode, payload = matched.groups()
        payload = payload or ''
        instructions.append((number, identifier, opcode, payload))
        if identifier is not None:
            if identifier in seen_ids:
                invalid_ids.add(identifier)
                diagnostics.append({'reason': 'duplicate SPIR-V result id', 'id': identifier,
                                    'logicalLine': number})
            seen_ids.add(identifier)
        try:
            if opcode in ('OpString', 'OpExtInstImport'):
                value = json.loads(payload, strict=False)
                if not isinstance(value, str):
                    raise ValueError('expected a quoted string')
                (strings if opcode == 'OpString' else imports)[identifier] = value
            elif opcode == 'OpTypeInt':
                args = payload.split()
                if len(args) == 2:
                    types[identifier] = (int(args[0]), int(args[1]))
            elif opcode == 'OpConstant':
                args = payload.split()
                if len(args) == 2 and types.get(args[0], (None,))[0] == 32:
                    value = int(args[1], 0)
                    if 0 <= value <= 0xffffffff:
                        constants[identifier] = value
        except (ValueError, TypeError) as error:
            diagnostics.append({'reason': str(error), 'id': identifier, 'logicalLine': number})
    supported = {identifier for identifier, name in imports.items() if name == PROFILE}
    unknown = [name for name in imports.values()
               if name.startswith('NonSemantic.Shader.DebugInfo.') and name != PROFILE]
    if not supported or unknown or invalid_ids:
        diagnostics.append({'reason': 'unsupported or ambiguous shader debug profile',
                            'imports': list(imports.values())})
        return {'profile': None, 'status': 'unavailable', 'logicalLines': lines,
                'locations': {}, 'diagnostics': diagnostics}
    events = {}
    for number, identifier, opcode, payload in instructions:
        if opcode != 'OpExtInst':
            continue
        args = payload.split()
        if len(args) < 3 or args[1] not in supported:
            continue
        name, operands = args[2], args[3:]
        valid = name in _KNOWN
        if valid and name in _ARITIES:
            low, high = _ARITIES[name]
            valid = len(operands) >= low and (high is None or len(operands) <= high)
        if not valid:
            diagnostics.append({'reason': 'unknown instruction or invalid debug operand count',
                                'id': identifier, 'instruction': name, 'logicalLine': number})
            events[number] = None
            continue
        node = {'name': name, 'args': operands, 'line': number}
        nodes[identifier] = node
        events[number] = node
    resolver = _Resolver(strings, constants, nodes, diagnostics)
    locations = {}
    function, in_block, scope_id, inline_id, location = None, False, None, None, None
    for number, identifier, opcode, payload in instructions:
        if opcode == 'OpFunction':
            function, in_block = identifier, False
            scope_id, inline_id, location = None, None, None
        elif opcode == 'OpLabel':
            in_block = True
            scope_id, inline_id, location = None, None, None
        event = events.get(number)
        if number in events:
            if event is None:
                scope_id, inline_id, location = None, None, None
            elif event['name'] in ('DebugScope', 'DebugNoScope', 'DebugLine', 'DebugNoLine'):
                if not in_block:
                    resolver.issue('debug scope/line instruction outside a basic block', identifier, number)
                    scope_id, inline_id, location = None, None, None
                elif event['name'] == 'DebugScope':
                    scope_id = event['args'][0]
                    inline_id = event['args'][1] if len(event['args']) == 2 else None
                elif event['name'] == 'DebugNoScope':
                    scope_id, inline_id = None, None
                elif event['name'] == 'DebugLine':
                    location = resolver.line(event['args'])
                else:
                    location = None
            continue
        if function is not None:
            scope, chain = resolver.scope(scope_id), resolver.inline_chain(inline_id)
            known = location is not None and location['sourceLine'] is not None
            row = {'logicalLine': number, 'sourceFile': None, 'sourceLine': None,
                   'sourceLineEnd': None, 'sourceColumn': None, 'sourceColumnEnd': None,
                   'function': scope['function'] if scope else None, 'spirvFunction': function,
                   'functionId': scope['functionId'] if scope else None,
                   'functionSourceFile': scope['functionSourceFile'] if scope else None,
                   'inlinedCallChain': chain,
                   'status': 'located' if known else 'scopeOnly' if scope else 'unavailable'}
            if location is not None:
                row.update(location)
            if (scope_id is not None and scope is None) or chain is None:
                row['status'] = 'partial'
            locations[str(number)] = row
        if opcode in _TERMINATORS or opcode == 'OpFunctionEnd':
            in_block = False
            scope_id, inline_id, location = None, None, None
        if opcode == 'OpFunctionEnd':
            function = None
    return {'profile': PROFILE, 'status': 'partial' if diagnostics else 'decoded',
            'logicalLines': lines, 'locations': locations, 'diagnostics': diagnostics}


def read_spirv_debug(path):
    """读取 UTF-8 spirv-dis listing；IO／编码错误原样传播，不运行反汇编器。"""
    return parse_spirv_debug(Path(path).read_text(encoding='utf-8'))


def lookup_location(index, logical_line):
    """精确查询从1开始的 logical SPV 行；未索引行或不支持的 profile 返回 None。"""
    return index['locations'].get(str(logical_line))
