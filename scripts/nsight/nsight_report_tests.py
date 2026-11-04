# SPDX-FileCopyrightText: Copyright (c) 2026 linlin
# SPDX-License-Identifier: GPL-3.0-only
"""报告的缺失证据、指标口径与 PC 指令对应。"""

import json
from pathlib import Path
import tempfile
import unittest

from nsight_diagnostics.report import markdown_report


class ReportTests(unittest.TestCase):
    def test_missing_components_stay_unavailable(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            markdown_report({"status": "failed", "toolVersion": "test", "errors": [
                {"component": "input", "reason": "损坏输入"}]}, None, None, out)
            text = (out / "report.md").read_text(encoding="utf-8")
            self.assertIn("损坏输入", text)
            self.assertNotIn("[counters.json]", text)
            self.assertEqual(json.loads((out / "sass-analysis.json").read_text(encoding="utf-8"))["artifacts"], [])

    def test_counter_null_does_not_become_zero(self):
        counters = {"windows": [{"windowId": "frame", "coverage": 1, "metrics": {"missing.pct": {
            "durationWeightedMean": None, "overlapProratedSum": None, "unit": "percent", "validDurationNs": 0}}}],
            "unavailableMetrics": ["missing.pct"]}
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            markdown_report({"status": "partial", "toolVersion": "test"}, counters, None, out)
            text = (out / "report.md").read_text(encoding="utf-8")
            self.assertIn("| missing.pct | null | null | percent | 0 |", text)

    def test_missing_register_metadata_stays_null(self):
        shaders = {"functions": [{"symbol": "kernel", "cudaResourceMetadata": None}],
                   "pcWindows": [{"windowId": "dispatch", "totalSamples": 1, "unmappedSamples": 0,
                                  "activeFunctions": [{"functionIndex": 0, "samples": 1}]}]}
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            markdown_report({"status": "partial", "toolVersion": "test"}, None, shaders, out)
            self.assertIn("| 0 | kernel | null | 1 |", (out / "report.md").read_text(encoding="utf-8"))

    def test_pc_annotation_requires_exact_instruction(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            sass = out / "kernel.txt"
            sass.write_text('.section .text.kernel,"ax",@progbits\n'
                            '//## File "module.spv", line 7\n'
                            '/*0010*/ @P0 LDG.E R2, [R4.64];\n', encoding="utf-8")
            shaders = {"objects": [{"device": 0, "module": 2, "disassembly": {
                "success": True, "stdoutPath": str(sass)}}],
                "functions": [{"device": 0, "module": 2, "symbol": "kernel"}],
                "pcWindows": [{"windowId": "dispatch", "totalSamples": 5, "unmappedSamples": 0,
                    "activeFunctions": [], "pcHistogram": {"status": "decoded", "topPcs": [
                        {"pc": "0x1010", "samples": 3, "functionIndex": 0, "functionOffset": 16},
                        {"pc": "0x1018", "samples": 2, "functionIndex": 0, "functionOffset": 24}]}}]}
            markdown_report({"status": "decoded", "toolVersion": "test"}, None, shaders, out)
            j = json.loads((out / "sass-analysis.json").read_text(encoding="utf-8"))
            top = j["windows"][0]["topPcs"]
            self.assertEqual(top[0]["instruction"]["sourceLine"], 7)
            self.assertIsNone(top[1]["instruction"])

    def test_older_source_summary_has_unknown_function_coverage(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            (out / "source-analysis.json").write_text(json.dumps({"windows": [{
                "windowId": "dispatch", "totalSamples": 1, "exportedPcSamples": 1,
                "sourceMappedSamples": 1, "allPcsExported": True, "functions": [], "sourceLines": []}],
                "unavailable": []}), encoding="utf-8")
            markdown_report({"status": "partial", "toolVersion": "test"}, None,
                            {"sourceCorrelation": {"path": "source-analysis.json"}}, out)
            self.assertIn("有函数作用域 `unknown`", (out / "report.md").read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
