# SPDX-FileCopyrightText: Copyright (c) 2026 linlin
# SPDX-License-Identifier: GPL-3.0-only
"""将已导出的证据汇总为报告；不从静态指令或 PC 频次推断动态瓶颈。"""

import json
from collections import Counter
from pathlib import Path

from .format import sha256, write_json
from .sass import lookup_instruction, read_sass


MAX_SASS_BYTES = 64 * 1024**2


def _cell(value):
    return str(value).replace("|", "\\|").replace("\r", " ").replace("\n", " ")


def _number(value):
    return "null" if value is None else f"{value:.6g}"


def _reason_text(rows, total):
    counts = Counter()
    for row in rows:
        counts[row["metric"] or "unknown"] += row["samples"]
    return ", ".join(f"{name.removeprefix('smsp__pcsamp_warps_issue_stalled_')}: {100 * count / total:.2f}%"
                     for name, count in counts.most_common(3)) if total and rows else "unknown"


def _table(lines, headings, rows):
    lines.extend(["| " + " | ".join(headings) + " |", "|" + "---|" * len(headings)])
    lines.extend("| " + " | ".join(_cell(cell) for cell in row) + " |" for row in rows)
    lines.append("")


def _sass_evidence(shaders, out):
    artifacts, indices, unavailable = [], {}, []
    for obj in shaders.get("objects", []):
        tool = obj.get("disassembly", {})
        if not tool.get("success"):
            continue
        path = Path(tool["stdoutPath"]).resolve()
        try:
            relative = path.relative_to(out.resolve())
            if path.stat().st_size > MAX_SASS_BYTES:
                raise ValueError("SASS 文本超过 64 MiB 索引上限")
            index = read_sass(path)
            key = (obj["device"], obj.get("module"), obj.get("codeBlock"))
            indices[key] = index
            artifacts.append({"path": relative.as_posix(), "sha256": sha256(path.read_bytes()),
                              "device": obj["device"], "module": obj.get("module"),
                              "codeBlock": obj.get("codeBlock"), "functions": [
                                  {"symbol": symbol, "instructionCount": row["instructionCount"],
                                   "opcodeCounts": row["opcodeCounts"], "status": row["status"]}
                                  for symbol, row in index["functions"].items()]})
        except (OSError, ValueError) as error:
            unavailable.append({"path": str(path), "reason": str(error)})
    windows = []
    functions = shaders.get("functions", [])
    for window in shaders.get("pcWindows", []):
        histogram = window.get("pcHistogram", {})
        top = []
        for pc in histogram.get("topPcs", []):
            row = dict(pc)
            owner = pc.get("functionIndex")
            if owner is not None and 0 <= owner < len(functions):
                function = functions[owner]
                index = indices.get((function["device"], function.get("module"), function.get("codeBlock")))
                symbol = function.get("symbol")
                if index and symbol is None and len(index["functions"]) == 1:
                    symbol = next(iter(index["functions"]))
                row["instruction"] = lookup_instruction(index, symbol, pc["functionOffset"]) if index else None
            else:
                row["instruction"] = None
            top.append(row)
        windows.append({"windowId": window["windowId"], "totalSamples": window["totalSamples"],
                        "histogramStatus": histogram.get("status", "notRequested"), "topPcs": top})
    return {"artifacts": artifacts, "windows": windows, "unavailable": unavailable,
            "limitations": ["opcodeCounts 是静态指令数量，不是动态请求、流量或耗时。",
                            "sourceFile/sourceLine 原样来自 nvdisasm，可能标注 SPIR-V 汇编而非 Slang。",
                            "PC 是被采样位置；等待结果的消费者不必是访存生产者。",
                            "LDL/STL 可能来自 RT continuation；CUDA STACK/LOCAL=0 不证明没有这些访问。"]}


def markdown_report(summary, counters, shaders, out):
    """生成 report.md 与有界 SASS 索引摘要；缺失组件用 null／诊断呈现。"""
    out = Path(out)
    counters, shaders = counters or {}, shaders or {}
    evidence = _sass_evidence(shaders, out)
    write_json(out / "sass-analysis.json", evidence)
    source = summary.get("provenance", {}).get("input", {})
    lines = ["# GPU Trace 离线报告", "", f"状态：`{summary['status']}`；工具 `{summary['toolVersion']}`。",
             "", f"输入：`{_cell(source.get('path', 'unknown'))}`。",
             f"SHA256：`{source.get('sha256', 'unknown')}`；分析前后相同："
             f"`{summary.get('provenance', {}).get('inputUnchanged', 'unknown')}`。", "",
             "完整状态见 [summary.json](summary.json)，指令对应见 [SASS 索引](sass-analysis.json)。", ""]
    if counters:
        lines.extend(["计数器证据：[counters.json](counters.json)。", ""])
    if shaders:
        lines.extend(["Shader 证据：[shaders.json](shaders.json)。", ""])
    if shaders.get("sourceCorrelation"):
        lines.extend(["高级源码关联：[source-analysis.json](source-analysis.json)。", ""])
    if shaders.get("internalRt"):
        lines.extend(["内部 RT 具名范围：[internal-rt.json](internal-rt.json)。", ""])
    for error in summary.get("errors", []):
        lines.append(f"- {_cell(error['component'])}：{_cell(error['reason'])}")
    if summary.get("errors"):
        lines.append("")
    lines.extend(["## GPU 时间与覆盖", "", "区间是 GPU 墙钟；并发或邻近帧调用不能相加为整帧耗时。", ""])
    metric_windows = {w["windowId"]: w for w in counters.get("windows", [])}
    rows = []
    windows = summary.get("gpuWindows", {})
    for window in [*windows.get("frames", []), *windows.get("dispatches", [])]:
        metric = metric_windows.get(window["id"], {})
        duration = window.get("durationNs")
        rows.append((window["id"], window.get("function", "frame"),
                     _number(duration / 1e6 if duration is not None else None),
                     _number(metric.get("coverage")), window.get("containedFrameIds", "—")))
    _table(lines, ["窗口", "调用", "ms", "counter 覆盖率", "所属捕获帧"], rows)
    for window in counters.get("windows", []):
        if not window.get("coverage"):
            continue
        lines.extend([f"### `{window['windowId']}`", ""])
        _table(lines, ["metric", "时间加权均值", "分摊 sum", "单位", "有效 ns"], [
            (name, _number(value["durationWeightedMean"]), _number(value["overlapProratedSum"]),
             value["unit"], value["validDurationNs"]) for name, value in window.get("metrics", {}).items()])
        ratio = window.get("threadInstOver32WarpInst")
        if ratio is not None:
            lines.extend([f"成对指令计数的平均活跃 lanes：`{32 * ratio:.6g}/32`；比率 `{100 * ratio:.6g}%`。"
                          "这不是 resident occupancy，也不等于逐样本百分比的时间均值。", ""])
    unavailable = counters.get("unavailableMetrics", [])
    if unavailable:
        lines.extend(["以下指标没有可用值；不能解释为零，也不能靠离线求值补出未采集的 counter：", ""])
        lines.extend(f"- `{name}`" for name in unavailable)
        lines.append("")
    lines.extend(["## Shader 与 PC", "", "PC 样本比例不是耗时比例、调用数或动态指令数。"
                  "寄存器 metadata 不等于 peak live state；未映射 PC 不全部归为 traversal。", ""])
    functions = shaders.get("functions", [])
    for window in shaders.get("pcWindows", []):
        if not window["totalSamples"]:
            continue
        total = window["totalSamples"]
        lines.extend([f"### `{window['windowId']}`", "",
                      f"总样本 `{total}`；未映射 `{window['unmappedSamples']}`。", ""])
        reasons = window.get("pcReasons")
        if reasons is not None:
            lines.extend([f"具名 reason 状态：`{reasons['status']}`。比例分母是本窗口 PC samples；"
                          "不是耗时、依赖 producer 或动态指令比例。", ""])
            if "counts" in reasons:
                _table(lines, ["raw code", "metric", "样本", "占窗口样本"], [
                    (r["rawReasonCode"], r["metric"] or "unknown", r["samples"],
                     f"{100 * r['samples'] / total:.4f}%") for r in reasons["counts"]])
            elif reasons.get("reason"):
                lines.extend([_cell(reasons["reason"]), ""])
        rows = []
        for active in window.get("activeFunctions", []):
            owner = active["functionIndex"]
            function = functions[owner]
            reg = (function.get("cudaResourceMetadata") or {}).get("REG",
                      (function.get("capturedMetadata") or {}).get("registersPerThread"))
            rows.append((owner, function.get("symbol") or function.get("shaderType", "unknown"),
                         _number(reg), active["samples"], f"{100 * active['samples'] / total:.4f}%"))
        _table(lines, ["函数索引", "函数", "REG", "样本", "样本比例"], rows)
        annotated = next(w for w in evidence["windows"] if w["windowId"] == window["windowId"])
        if annotated["topPcs"]:
            rows = []
            for pc in annotated["topPcs"][:32]:
                instruction = pc.get("instruction") or {}
                location = (f"{instruction['sourceFile']}:{instruction['sourceLine']}"
                            if instruction.get("sourceFile") else "unknown")
                rows.append((pc["pc"], pc.get("functionIndex"), pc["samples"],
                             instruction.get("instruction", "unknown"), location))
            _table(lines, ["PC", "函数索引", "样本", "SASS", "原始行标注"], rows)
        elif window.get("pcHistogram", {}).get("reason"):
            lines.extend(["PC histogram：" + window["pcHistogram"]["reason"], ""])
    if shaders.get("internalRt"):
        internal = json.loads((out / "internal-rt.json").read_text(encoding="utf-8"))
        lines.extend(["## 内部 RT 符号归属", "", "仅按捕获 redacted ELF 的具名范围关联 PC；"
                      "没有内部代码、REG 或访存行为。旧 functions 索引及 unmappedSamples 含义保持原样。"
                      "allPcsExported 只表示导出覆盖；complete 还要求唯一范围覆盖，无 unattributed。", ""])
        for window in internal["windows"]:
            total = window["totalSamples"]
            if not total:
                continue
            lines.extend([f"### `{window['windowId']}`", "",
                          f"归属状态 `{window['attributionStatus']}`；导出样本 `{window['exportedSamples']}/{total}`；"
                          f"全部 PC 已导出 `{window['allPcsExported']}`。分类仅统计唯一范围，重叠保留 ambiguous。", ""])
            _table(lines, ["捕获符号名", "样本", "占窗口样本"], [
                (name, count, f"{100 * count / total:.4f}%")
                for name, count in sorted(window["categorySamples"].items(), key=lambda item: -item[1])])
            _table(lines, ["归属类别", "已导出样本"], window["sampleCounts"].items())
    if shaders.get("sourceCorrelation"):
        source_analysis = json.loads((out / "source-analysis.json").read_text(encoding="utf-8"))
        lines.extend(["## 高级源码关联", "", "模块配对由调用方明确指定，锚点不代替身份验证。"
                      "以下是调试位置归属的样本，不能当作耗时或 dynamic instruction 比例。", ""])
        for window in source_analysis["windows"]:
            if not window["totalSamples"]:
                continue
            lines.extend([f"### `{window['windowId']}`", "",
                          f"窗口总样本 `{window['totalSamples']}`；导出 PC 覆盖 `{window['exportedPcSamples']}`，"
                          f"有高级行 `{window['sourceMappedSamples']}`，有函数作用域 `{window.get('functionMappedSamples', 'unknown')}`。"
                          f"全部 PC 已导出：`{window['allPcsExported']}`；缺失部分不外推。", ""])
            _table(lines, ["当前函数作用域", "样本", "占窗口样本", "前三 reason（分母为该行样本）", "声明文件"], [
                (r["function"], r["samples"], f"{r['percentOfWindowSamples']:.4f}%",
                 _reason_text(r.get("reasonSamples", []), r["samples"]), r["sourceFile"])
                for r in window["functions"][:20]])
            _table(lines, ["函数", "文件:行", "样本", "占窗口样本"], [
                (r["function"], f"{r['sourceFile']}:{r['sourceLine']}", r["samples"],
                 f"{r['percentOfWindowSamples']:.4f}%") for r in window["sourceLines"][:20]])
        for unavailable in source_analysis["unavailable"]:
            lines.extend(["源码关联不可用：" + _cell(unavailable["reason"]), ""])
    lines.extend(["## 静态 SASS", "", "下表只统计代码中出现的指令；不能据此分摊 LSU 流量。", ""])
    rows = []
    for artifact in evidence["artifacts"]:
        for function in artifact["functions"]:
            counts = function["opcodeCounts"]
            rows.append((function["symbol"], function["instructionCount"],
                         *(counts.get(op, 0) if counts is not None else "unknown"
                           for op in ("LDL", "STL", "LDG", "STG", "TEX", "TLD"))))
    _table(lines, ["函数", "全部", "LDL", "STL", "LDG", "STG", "TEX", "TLD"], rows)
    lines.extend(["## 证据边界", ""])
    lines.extend("- " + item for item in evidence["limitations"])
    if any(o.get("identityEvidence", "").startswith("derivedFromReference") for o in shaders.get("objects", [])):
        lines.append("- 至少一个 SPIR-V 是同一 encoded/plain 参考对派生；自身 SHA 相等不算独立身份验证。")
    if shaders.get("xorMappingIndependentTargetValidation"):
        lines.append("- 其他模块有独立全长 SHA 匹配；这不自动验证参考长度内每个位置的 XOR 映射。")
    lines.extend("- SASS 索引不可用：" + _cell(item["reason"]) for item in evidence["unavailable"])
    (out / "report.md").write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    return {"path": "report.md", "sassPath": "sass-analysis.json",
            "sassIndexUnavailableCount": len(evidence["unavailable"])}
