# SPDX-FileCopyrightText: Copyright (c) 2026 linlin
# SPDX-License-Identifier: GPL-3.0-only
"""显式源码配对、精确 IL 文件及不完整 PC 覆盖的行为。"""

from pathlib import Path
import tempfile
import unittest
from unittest import mock

from nsight_diagnostics.correlation import correlate_sources, source_pair


def fixture(root, il_file="module.spv"):
    listing = root / "listing.txt"
    listing.write_text("%entry = OpFunction %void None %type\nOpLoad %uint %address\n", encoding="utf-8")
    sass = root / "kernel.txt"
    sass.write_text('.section .text.kernel,"ax",@progbits\n'
                    '//## File "module.spv", line 1\n/*0000*/ NOP;\n'
                    f'//## File "{il_file}", line 2\n/*0010*/ LDG.E R2, [R4.64];\n', encoding="utf-8")
    shaders = {"objects": [{"device": 0, "module": 9, "kind": "rtCoreUserCubin", "disassembly": {
                   "success": True, "stdoutPath": str(sass)}},
                {"device": 0, "module": 4, "kind": "graphicsAPIModule", "path": "source.spv"}],
               "functions": [{"device": 0, "module": 9, "symbol": "kernel"}],
               "pcWindows": [{"windowId": "dispatch", "totalSamples": 5, "pcHistogram": {"topPcs": [
                   {"pc": "0x1010", "functionIndex": 0, "functionOffset": 16, "samples": 3}]}}]}
    index = {"status": "decoded", "logicalLines": listing.read_text(encoding="utf-8").splitlines(), "diagnostics": []}
    return listing, shaders, index


class CorrelationTests(unittest.TestCase):
    def test_scope_only_is_not_a_source_line(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            listing, shaders, index = fixture(root)
            with mock.patch("nsight_diagnostics.correlation.run_tool", return_value={
                    "success": True, "stdoutPath": str(listing)}), \
                    mock.patch("nsight_diagnostics.correlation.read_spirv_debug", return_value=index), \
                    mock.patch("nsight_diagnostics.correlation.lookup_location", return_value={
                        "sourceFile": "shader.slang", "sourceLine": None, "function": "shade", "status": "scopeOnly"}):
                result = correlate_sources(shaders, root, root / "spirv-dis", [(0, 9, 4)])
            window = result["windows"][0]
            self.assertEqual(window["sourceMappedSamples"], 0)
            self.assertEqual(window["functionMappedSamples"], 3)
            self.assertEqual(window["exportedPcSamples"], 3)
            self.assertFalse(window["allPcsExported"])

    def test_other_source_file_is_not_treated_as_spirv(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            listing, shaders, index = fixture(root, "shader.slang")
            with mock.patch("nsight_diagnostics.correlation.run_tool", return_value={
                    "success": True, "stdoutPath": str(listing)}), \
                    mock.patch("nsight_diagnostics.correlation.read_spirv_debug", return_value=index), \
                    mock.patch("nsight_diagnostics.correlation.lookup_location") as lookup:
                result = correlate_sources(shaders, root, root / "spirv-dis", [(0, 9, 4)])
            lookup.assert_not_called()
            self.assertEqual(result["windows"][0]["sourceMappedSamples"], 0)

    def test_module_pair_requires_explicit_nonnegative_indices(self):
        self.assertEqual(source_pair("0:9=4"), (0, 9, 4))
        with self.assertRaises(ValueError):
            source_pair("0:-9=4")

    def test_debug_ids_remain_local_to_their_spirv_module(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            listing, shaders, index = fixture(root)
            shaders["objects"].extend([
                {**shaders["objects"][0], "module": 10}, {**shaders["objects"][1], "module": 5}])
            shaders["functions"].append({**shaders["functions"][0], "module": 10})
            shaders["pcWindows"][0]["pcHistogram"]["topPcs"].append(
                {"pc": "0x2010", "functionIndex": 1, "functionOffset": 16, "samples": 2})
            with mock.patch("nsight_diagnostics.correlation.run_tool", return_value={
                    "success": True, "stdoutPath": str(listing)}), \
                    mock.patch("nsight_diagnostics.correlation.read_spirv_debug", return_value=index), \
                    mock.patch("nsight_diagnostics.correlation.lookup_location", side_effect=[
                        {"sourceFile": "shader.slang", "sourceLine": 2, "function": "first", "functionId": "%7"},
                        {"sourceFile": "shader.slang", "sourceLine": 2, "function": "second", "functionId": "%7"}]):
                result = correlate_sources(shaders, root, root / "spirv-dis", [(0, 9, 4), (0, 10, 5)])
            self.assertEqual([(row["spirvModule"], row["function"], row["samples"])
                              for row in result["windows"][0]["functions"]], [(4, "first", 3), (5, "second", 2)])


if __name__ == "__main__":
    unittest.main()
