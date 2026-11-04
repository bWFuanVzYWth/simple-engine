# SPDX-FileCopyrightText: Copyright (c) 2026 linlin
# SPDX-License-Identifier: GPL-3.0-only
"""redacted RT ELF 的具名范围；代码缺失时只保留符号证据，不生成指令或资源结论。"""

import bisect
from collections import Counter
from pathlib import Path
import struct

from . import FormatError
from .format import require, sha256
from .shaders import decode_blob, unpack_blob


def _redacted_symbols(data):
    require(data, 0, 64, "redacted ELF64 header")
    if data[:7] != b"\x7fELF\x02\x01\x01":
        raise FormatError("redacted ranges require ELF64 little-endian version 1")
    elf_type, machine = struct.unpack_from("<HH", data, 16)
    if machine != 190 or elf_type not in (1, 2):
        raise FormatError("unreviewed redacted ELF type/machine")
    shoff = struct.unpack_from("<Q", data, 40)[0]
    stride, count, names_index = struct.unpack_from("<3H", data, 58)
    if stride != 64 or not count or names_index >= count:
        raise FormatError("unsupported redacted ELF section table")
    require(data, shoff, count * stride, "redacted ELF sections")
    sections = [struct.unpack_from("<IIQQQQIIQQ", data, shoff + i * stride) for i in range(count)]
    for section in sections:
        if section[1] != 8:
            require(data, section[4], section[5], "redacted ELF section data")

    def strings(index):
        if index >= count or sections[index][1] != 3:
            raise FormatError("redacted ELF string table unavailable")
        section = sections[index]
        return data[section[4]:section[4] + section[5]]

    def string(table, offset):
        if offset >= len(table):
            raise FormatError("redacted ELF string offset out of bounds")
        end = table.find(b"\0", offset)
        if end < 0:
            raise FormatError("unterminated redacted ELF string")
        return table[offset:end].decode("utf-8", errors="replace")

    section_names = strings(names_index)
    ranges, excluded = [], Counter()
    for section in sections:
        if section[1] != 2:
            continue
        if section[9] != 24 or section[5] % 24:
            raise FormatError("invalid redacted ELF symbol table")
        table = strings(section[6])
        for offset in range(section[4], section[4] + section[5], 24):
            name, info, other, index, value, size = struct.unpack_from("<IBBHQQ", data, offset)
            if info & 15 != 2:
                continue
            if not index or not value or not size:
                excluded["undefinedOrZeroValueOrSize"] += 1
                continue
            if index >= count:
                raise FormatError("redacted ELF function references invalid section")
            symbol = string(table, name)
            if not symbol:
                excluded["unnamed"] += 1
                continue
            code = sections[index]
            section_name = string(section_names, code[0])
            if elf_type != 2 or code[1] != 8 or code[3:6] != (0, 0, 0) or section_name:
                excluded["unreviewedSymbolAddressLayout"] += 1
                continue
            if value + size > 1 << 64:
                raise FormatError("redacted ELF function range overflows u64")
            ranges.append({"symbol": symbol, "gpuVA": value, "bytes": size,
                "addressMode": "captureAbsoluteVA/redactedSymbol", "codeAvailable": False,
                "symbolEntryOffset": offset, "sectionIndex": index, "sectionType": code[1],
                "sectionName": section_name, "sectionBytes": code[5], "stOther": other})
    return ranges, dict(excluded)


def _deduplicate(ranges):
    grouped = {}
    for item in ranges:
        key = (item["device"], item["gpuVA"], item["bytes"])
        row = grouped.setdefault(key, {"device": key[0], "gpuVA": key[1], "bytes": key[2],
            "kind": "rtCoreInternalRedactedCubin", "codeAvailable": False,
            "addressMode": "captureAbsoluteVA/redactedSymbol", "symbols": [], "sources": []})
        for symbol in item.get("symbols", [item.get("symbol")]):
            if symbol is not None and symbol not in row["symbols"]:
                row["symbols"].append(symbol)
        for source in item["sources"]:
            if source not in row["sources"]:
                row["sources"].append(source)
    for row in grouped.values():
        row["symbol"] = row["symbols"][0] if len(row["symbols"]) == 1 else None
    return list(grouped.values())


def extract_internal_ranges(trace, out, mapping):
    """导出已审查模块的 encoded／redacted ELF，返回标准 symtab 范围与来源。

    out 为本次输出根目录；内部使用 internal-rt/，文件独占创建，不覆盖已有证据。
    每个 device/blob 只解码一次，重复 module 引用保留为来源。mapping 由调用方提供；
    ELF 校验不等于独立原始 CUBIN 身份验证。仅接受具名、非零 STT_FUNC 的已观察
    redacted 地址布局：ET_EXEC、空名且零长的 SHT_NOBITS。匿名／相对符号不猜地址。
    不 normalize、反汇编或推断 REG；bytes 是符号范围长度，不是捕获内代码长度。
    I/O／格式失败留在对象诊断中，已导出的字节保留；不加载 ELF，也不执行 GPU。
    """
    out = Path(out)
    folder = out / "internal-rt"
    folder.mkdir(exist_ok=True)
    objects, cached = [], {}
    for device, item in enumerate(trace.devices):
        if not item.HasField("ShaderProfilerReport"):
            continue
        report = item.ShaderProfilerReport
        for module, entry in enumerate(report.pcCountersInfo.pcSamplingSession.modules):
            if not entry.HasField("rtCoreInternalRedactedCubin"):
                continue
            internal = entry.rtCoreInternalRedactedCubin
            if (not internal.HasField("cubin") or not internal.cubin.HasField("ref") or
                    not internal.cubin.ref.HasField("idx")):
                objects.append({"device": device, "blob": None, "modules": [module], "status": "unavailable",
                    "kind": "rtCoreInternalRedactedCubin", "reason": "redacted CUBIN blob reference missing",
                    "errorType": "FormatError"})
                continue
            index = internal.cubin.ref.idx
            key = (device, index)
            if key in cached:
                cached[key]["modules"].append(module)
                continue
            obj = {"device": device, "blob": index, "modules": [module], "status": "unavailable",
                   "kind": "rtCoreInternalRedactedCubin"}
            cached[key] = obj
            objects.append(obj)
            try:
                blobs = report.blobTable.blobs
                if index < 0 or index >= len(blobs):
                    raise FormatError("redacted CUBIN blob reference out of bounds")
                blob = blobs[index]
                raw = unpack_blob(blob)
                stem = f"device-{device}-blob-{index}"
                encoded = folder / (stem + ".encoded.bin")
                with encoded.open("xb") as file:
                    file.write(raw)
                obj.update(compression=blob.compression, obfuscation=blob.obfuscation,
                           encodedPath=encoded.relative_to(out).as_posix(), encodedSha256=sha256(raw))
                data = decode_blob(blob, mapping)
                target = folder / (stem + ".redacted.cubin")
                with target.open("xb") as file:
                    file.write(data)
                obj.update(path=target.relative_to(out).as_posix(), decodedSha256=sha256(data), bytes=len(data))
                ranges, excluded = _redacted_symbols(data)
                obj.update(status="decoded", symbolRanges=ranges, excludedSymbols=excluded)
            except (OSError, FormatError) as error:
                obj.update(reason=str(error), errorType=type(error).__name__)
    ranges = []
    for obj in objects:
        for row in obj.get("symbolRanges", []):
            ranges.append({"device": obj["device"], "symbol": row["symbol"], "gpuVA": row["gpuVA"],
                "bytes": row["bytes"], "sources": [{"device": obj["device"], "blob": obj["blob"],
                    "modules": list(obj["modules"]), "decodedSha256": obj["decodedSha256"], **row}]})
    failures = [{"device": obj["device"], "blob": obj["blob"], "reason": obj["reason"],
                 "errorType": obj["errorType"]} for obj in objects if obj["status"] != "decoded"]
    return {"status": "partial" if failures else "decoded" if objects else "unavailable",
            "deviceCount": len(trace.devices), "gpuCount": len(trace.gpus),
            "mappingSha256": sha256(mapping) if mapping is not None else None,
            "objects": objects, "ranges": _deduplicate(ranges), "failures": failures,
            "limitations": ["Names and ranges are captured redacted symbols, not inferred traversal coverage.",
                "Code, source, registers, private RT metadata semantics and exact internal algorithms remain unavailable.",
                "Valid ELF structure and provided XOR decoding do not independently verify original internal CUBIN identity."]}


def _lookup_index(ranges):
    ordered = sorted((row["gpuVA"], row["gpuVA"] + row["bytes"], index)
                     for index, row in enumerate(ranges) if row["gpuVA"] is not None)
    ends, maximum = [], 0
    for _, end, _ in ordered:
        maximum = max(maximum, end)
        ends.append(maximum)
    return ordered, [row[0] for row in ordered], ends


def _matches(pc, index):
    ordered, starts, ends = index
    i = bisect.bisect_right(starts, pc) - 1
    result = []
    while i >= 0 and ends[i] > pc:
        if pc < ordered[i][1]:
            result.append(ordered[i][2])
        i -= 1
    return sorted(result)


def attribute_internal_pcs(shaders, metadata):
    """只对实际导出的 topPcs 逐项归因，返回独立 artifact，不修改旧函数或计数。

    相同 device/VA/size 的范围合并并保留全部来源；名称冲突或非相同范围重叠标
    ambiguous，不挑选一个。与原 functions 相交也不覆盖已有身份。只支持单 GPU／
    单 device 地址域。allPcsExported 仅表示导出 samples 之和等于该窗口 totalSamples；
    缺失 topPcs 为不完整，部分 top N 不外推。attributionStatus=complete 还要求所有
    PC 唯一归入内部或旧函数。sampleCounts.unattributed 仅统计已导出且两种范围都
    未覆盖的样本，含义独立于旧 unmappedSamples；代码缺失不记为零代码。
    """
    ranges = _deduplicate(metadata["ranges"])
    lookup = _lookup_index(ranges)
    existing = _lookup_index(shaders.get("functions", []))
    namespace_reviewed = metadata["deviceCount"] == 1 and metadata["gpuCount"] == 1
    windows = []
    for window in shaders.get("pcWindows", []):
        histogram = window.get("pcHistogram", {})
        pcs = histogram.get("topPcs", [])
        exported = sum(pc["samples"] for pc in pcs)
        all_exported = "topPcs" in histogram and exported == window["totalSamples"]
        rows, categories, totals = [], Counter(), Counter()
        for pc in pcs:
            address, count = int(pc["pc"], 16), pc["samples"]
            matches = _matches(address, lookup) if namespace_reviewed else []
            prior = _matches(address, existing) if namespace_reviewed else []
            ambiguous = len(matches) > 1 or bool(matches and prior) or any(ranges[i]["symbol"] is None for i in matches)
            status = ("unavailable" if not namespace_reviewed else "ambiguous" if ambiguous else
                      "internal" if matches else "existingFunction" if prior else "unattributed")
            totals[status] += count
            if status == "internal":
                categories[ranges[matches[0]]["symbol"]] += count
            rows.append({"pc": pc["pc"], "samples": count, "status": status,
                         "rangeIndices": matches, "existingFunctionIndices": prior})
        complete = (all_exported and namespace_reviewed and not totals["ambiguous"] and
                    not totals["unattributed"] and metadata["status"] == "decoded")
        windows.append({"windowId": window["windowId"], "totalSamples": window["totalSamples"],
            "exportedSamples": exported, "allPcsExported": all_exported,
            "attributionStatus": "complete" if complete else "partial" if namespace_reviewed else "unavailable",
            "sampleCounts": dict(totals), "categorySamples": dict(categories), "pcs": rows})
    return {"status": "decoded" if metadata["status"] == "decoded" and windows and all(
                row["attributionStatus"] == "complete" for row in windows) else "partial",
            "metadata": {**metadata, "ranges": ranges}, "windows": windows,
            "limitations": ["Only exported PC samples are attributed; incomplete top lists are never extrapolated.",
                "Original functions, indices and unmappedSamples are unchanged.",
                "Sample frequencies and captured symbol labels do not determine time, memory traffic or internal code behavior."]}
