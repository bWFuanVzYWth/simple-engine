# SPDX-FileCopyrightText: Copyright (c) 2026 linlin
# SPDX-License-Identifier: GPL-3.0-only
"""按显式 CUBIN/SPIR-V 配对关联已导出 PC，不推断私有 stall reason。"""

from collections import Counter
from pathlib import Path
import re
import subprocess

from . import FormatError
from .format import write_json
from .sass import lookup_instruction, read_sass
from .shaders import run_tool
from .spirv_debug import lookup_location, read_spirv_debug


MAX_LISTING_BYTES = 64 * 1024**2


def source_pair(value):
    """解析 device:cubinModule=spirvModule；只接受非负十进制索引。"""
    left, separator, right = value.partition("=")
    device, colon, module = left.partition(":")
    if separator != "=" or colon != ":" or not all(part.isascii() and part.isdecimal()
                                                     for part in (device, module, right)):
        raise ValueError("source pair 须为 device:cubinModule=spirvModule，例如 0:9=4")
    return int(device), int(module), int(right)


def correlate_sources(shaders, out, spirv_dis, pairs):
    """导出 SPIR-V listing 和样本关联；调用方显式指定同一 device 的模块配对。

    支持的行号约定为 spirv-dis 默认五行 header、每条指令一逻辑行；OpString 内嵌换行
    由 debug reader 折叠。保留首条 SASS 行指向 SPIR-V OpFunction 的配对锚点；不匹配
    时停止该模块关联。高级源码与内联位置来自标准 NonSemantic.Shader.DebugInfo.100。
    """
    out, spirv_dis = Path(out), Path(spirv_dis)
    source_dir = out / "sources"
    source_dir.mkdir()
    objects = {(o["device"], o["module"]): o for o in shaders.get("objects", []) if "module" in o}
    associations, failures, indices = [], [], {}
    for device, cubin_module, spv_module in pairs:
        identity = {"device": device, "cubinModule": cubin_module, "spirvModule": spv_module}
        try:
            cubin, spirv = objects.get((device, cubin_module)), objects.get((device, spv_module))
            if cubin is None or cubin["kind"] != "rtCoreUserCubin":
                raise FormatError("配对 CUBIN 不是已导出的 RT 用户模块")
            if spirv is None or spirv["kind"] != "graphicsAPIModule":
                raise FormatError("配对 SPIR-V 模块未导出")
            tool = cubin.get("disassembly") or {}
            if not tool.get("success"):
                raise FormatError("配对 CUBIN 没有成功的 nvdisasm 文本")
            key = (device, cubin_module)
            if key in indices:
                raise FormatError("同一 CUBIN 不支持多个源码配对")
            destination = source_dir / f"device-{device}-module-{spv_module}.spvasm"
            result = run_tool(spirv_dis, [str(out / "shaders" / spirv["path"])], destination)
            if not result["success"]:
                raise FormatError("spirv-dis 未成功输出，详见工具日志")
            if any(Path(path).stat().st_size > MAX_LISTING_BYTES
                   for path in (result["stdoutPath"], tool["stdoutPath"])):
                raise FormatError("源码关联文本超过每份 64 MiB 索引上限")
            debug = read_spirv_debug(result["stdoutPath"])
            if debug["status"] == "unavailable":
                raise FormatError("SPIR-V 没有受支持的 NonSemantic.Shader.DebugInfo.100 调试信息")
            sass = read_sass(tool["stdoutPath"])
            anchors = []
            for symbol, function in sass["functions"].items():
                first = min((r for records in function["instructions"].values() for r in records),
                            key=lambda r: r["offset"], default=None)
                if (first is not None and first.get("sourceLine") is not None
                        and (first.get("sourceFile") or "").endswith(".spv")):
                    line = first["sourceLine"]
                    # 具体操作名由 debug reader 的逻辑行索引提供，不按相邻源码猜测。
                    instruction = debug["logicalLines"][line - 1] if 1 <= line <= len(debug["logicalLines"]) else ""
                    anchors.append({"symbol": symbol, "logicalLine": line, "file": first["sourceFile"],
                                    "isFunction": re.match(r"^\s*%\S+\s*=\s*OpFunction\s+", instruction) is not None})
            if not anchors or not any(a["isFunction"] for a in anchors):
                raise FormatError("未验证 SASS 首指令到 SPIR-V OpFunction 的行号配对")
            files = {anchor["file"] for anchor in anchors}
            if len(files) != 1:
                raise FormatError("同一 CUBIN 的 SPIR-V IL 文件关联有歧义")
            il_file = next(iter(files))
            indices[key] = (sass, debug, il_file, spv_module)
            associations.append({**identity, "status": "partial" if debug.get("diagnostics") or sass.get("diagnostics")
                                 else "correlated", "spirvDisassembly": result,
                                 "anchors": anchors, "identityEvidence": spirv.get("identityEvidence"),
                                 "debugDiagnostics": debug.get("diagnostics", []),
                                 "sassDiagnostics": sass.get("diagnostics", [])})
        except (OSError, ValueError, subprocess.SubprocessError) as error:
            failures.append({**identity, "reason": str(error)})
    functions, windows = shaders.get("functions", []), []
    for window in shaders.get("pcWindows", []):
        by_function, by_source, pcs = Counter(), Counter(), []
        function_names, function_reasons, source_reasons = {}, {}, {}
        covered, mapped, function_mapped = 0, 0, 0
        for pc in window.get("pcHistogram", {}).get("topPcs", []):
            covered += pc["samples"]
            owner, location = pc.get("functionIndex"), None
            if owner is not None and 0 <= owner < len(functions):
                function = functions[owner]
                pair = indices.get((function["device"], function.get("module")))
                if pair is not None:
                    sass, debug, il_file, spv_module = pair
                    instruction = lookup_instruction(sass, function.get("symbol"), pc["functionOffset"])
                    if instruction and instruction.get("sourceLine") and instruction.get("sourceFile") == il_file:
                        location = lookup_location(debug, instruction["sourceLine"])
            if location is not None and location.get("function") is not None:
                function_mapped += pc["samples"]
                function_key = (function["device"], spv_module,
                                location.get("functionSourceFile") or location.get("sourceFile"),
                                location.get("functionId") or location["function"])
                function_names[function_key] = location["function"]
                by_function[function_key] += pc["samples"]
                for reason in pc.get("reasonSamples", []):
                    function_reasons.setdefault(function_key, Counter())[reason["rawReasonCode"], reason["metric"]] += reason["samples"]
            if (location is not None and isinstance(location.get("sourceFile"), str)
                    and isinstance(location.get("sourceLine"), int) and location["sourceLine"] > 0):
                mapped += pc["samples"]
                source_key = (function["device"], spv_module,
                              location["sourceFile"], location["sourceLine"], location.get("function"))
                by_source[source_key] += pc["samples"]
                for reason in pc.get("reasonSamples", []):
                    source_reasons.setdefault(source_key, Counter())[reason["rawReasonCode"], reason["metric"]] += reason["samples"]
            pcs.append({**pc, "location": location})
        total = window["totalSamples"]
        windows.append({"windowId": window["windowId"], "totalSamples": total,
                        "exportedPcSamples": covered, "sourceMappedSamples": mapped,
                        "functionMappedSamples": function_mapped,
                        "allPcsExported": covered == total,
                        "functions": [{"device": key[0], "spirvModule": key[1], "sourceFile": key[2],
                                       "function": function_names[key], "functionId": key[3],
                                       "samples": count, "percentOfWindowSamples": 100 * count / total if total else None,
                                       "reasonSamples": [{"rawReasonCode": code, "metric": metric, "samples": value}
                                                         for (code, metric), value in sorted(function_reasons.get(key, {}).items())]}
                                      for key, count in by_function.most_common()],
                        "sourceLines": [{"device": key[0], "spirvModule": key[1], "sourceFile": key[2],
                                         "sourceLine": key[3], "function": key[4],
                                         "samples": count, "percentOfWindowSamples": 100 * count / total if total else None,
                                         "reasonSamples": [{"rawReasonCode": code, "metric": metric, "samples": value}
                                                           for (code, metric), value in sorted(source_reasons.get(key, {}).items())]}
                                        for key, count in by_source.most_common()], "pcs": pcs})
    result = {"status": "partial" if failures or any(a["status"] == "partial" for a in associations)
              else "correlated", "associations": associations,
              "unavailable": failures, "windows": windows,
              "limitations": ["模块配对由调用方明确提供；锚点验证不能代替完整模块身份验证。",
                              "高级位置是编译器调试关联，优化后的指令不一定有唯一高级语义。",
                              "只统计已导出的 topPcs；allPcsExported 为 false 时不外推缺失样本。",
                              "函数样本按当前 source location 归属，不等同 inclusive CPU/GPU 耗时。",
                              "reasonSamples 只继承 PC 已审查 profile 的分类；未启用／不支持时不补猜，也不推导 SB producer 归因。"]}
    write_json(out / "source-analysis.json", result)
    return result
