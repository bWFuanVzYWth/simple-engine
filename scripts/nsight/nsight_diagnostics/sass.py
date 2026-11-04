# SPDX-FileCopyrightText: Copyright (c) 2026 linlin
# SPDX-License-Identifier: GPL-3.0-only
"""迭代索引 nvdisasm 文本；只描述静态指令与原始行注释，不推断运行成本。

支持 `.section .text.<symbol>`、`//## File "...", line N` 和 `/*offset*/` 指令。
函数偏移只作精确匹配；不把相邻指令归给未知 PC。行注释可能指向 SPIR-V 文本，也可能
指向其他文件；不读取该文件或推断高级源码。LDL/STL 不自动归类为 spill。
"""

from collections import Counter
from pathlib import Path
import re


_SECTION = re.compile(r'^\s*\.section\s+(?:"([^"]+)"|([^,\s]+))')
_SOURCE = re.compile(r'^\s*//##\s+File\s+"(.*)",\s*line\s+(\d+)\s*$')
_INSTRUCTION = re.compile(
    r'^\s*/\*([0-9a-fA-F]+)\*/\s*'
    r'((?:@!?[A-Za-z_][A-Za-z_0-9]*\s+)?'
    r'([A-Za-z][A-Za-z_0-9]*(?:\.[A-Za-z_0-9]+)*)(?:\s|;|$).*)$'
)


def parse_sass(text):
    """索引字符串，返回可 JSON 序列化的 functions／diagnostics／limitations。

    functions 以原始 symbol 为键；instructions 以 `0x` 加 16 位十六进制偏移为键，
    每项保存全部 records。opcodeCounts 按基础 opcode 汇总，mnemonic 保留修饰符。
    重复 `(symbol,offset)` 不覆盖：该函数 status=ambiguous，总数置 None，并保留
    observed 计数；精确查询只对该地址返回 None。其余函数和地址仍可查询。
    """
    functions, diagnostics = {}, []
    current = None
    source_file, source_line = None, None
    for number, line in enumerate(text.splitlines(), 1):
        section = _SECTION.match(line)
        if section:
            name = section.group(1) or section.group(2)
            current = None
            source_file, source_line = None, None
            if name.startswith('.text.') and len(name) > len('.text.'):
                symbol = name[len('.text.'):]
                current = functions.setdefault(symbol, {
                    'symbol': symbol, 'instructions': {}, 'observedInstructionCount': 0,
                    'observedOpcodeCounts': Counter(),
                })
            continue
        if current is None:
            continue
        source = _SOURCE.match(line)
        if source:
            source_file, source_line = source.group(1), int(source.group(2))
            continue
        instruction = _INSTRUCTION.match(line)
        if instruction is None:
            continue
        offset = int(instruction.group(1), 16)
        mnemonic = instruction.group(3)
        opcode = mnemonic.split('.', 1)[0]
        body = instruction.group(2).strip()
        if ';' in body:
            body = body.split(';', 1)[0].rstrip() + ';'
        key = f'0x{offset:016x}'
        record = {
            'offset': offset, 'offsetHex': key, 'opcode': opcode, 'mnemonic': mnemonic,
            'instruction': body, 'sourceFile': source_file, 'sourceLine': source_line,
            'sassLine': number,
        }
        current['instructions'].setdefault(key, []).append(record)
        current['observedInstructionCount'] += 1
        current['observedOpcodeCounts'][opcode] += 1
    for function in functions.values():
        ambiguous = [key for key, records in function['instructions'].items() if len(records) > 1]
        observed = dict(sorted(function['observedOpcodeCounts'].items()))
        function.update(
            status='ambiguous' if ambiguous else 'indexed', ambiguousOffsets=ambiguous,
            instructionCount=None if ambiguous else function['observedInstructionCount'],
            opcodeCounts=None if ambiguous else observed, observedOpcodeCounts=observed,
        )
        for key in ambiguous:
            diagnostics.append({
                'symbol': function['symbol'], 'offsetHex': key,
                'reason': 'duplicate instruction offset; attribution is ambiguous',
                'sassLines': [record['sassLine'] for record in function['instructions'][key]],
            })
    return {
        'functions': functions, 'diagnostics': diagnostics,
        'limitations': [
            'Only parsed static instruction occurrences are counted; no dynamic traffic or time is inferred.',
            'Source locations are nvdisasm annotations, not independently verified high-level source mappings.',
            'LDL/STL do not by themselves prove register spilling.',
            'Lookup requires an exact symbol and instruction offset; unknown and ambiguous locations remain unavailable.',
        ],
    }


def read_sass(path):
    """从 UTF-8 文本路径建立相同索引；不可解码字节按替换字符显示，IO 错误原样传播。"""
    return parse_sass(Path(path).read_text(encoding='utf-8', errors='replace'))


def lookup_instruction(index, symbol, offset):
    """精确查询函数相对整数偏移；未知 symbol、非指令边界或重复地址返回 None。"""
    function = index['functions'].get(symbol)
    if function is None:
        return None
    records = function['instructions'].get(f'0x{offset:016x}', [])
    return records[0] if len(records) == 1 else None
