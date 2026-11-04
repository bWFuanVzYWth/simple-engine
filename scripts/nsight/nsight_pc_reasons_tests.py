# SPDX-FileCopyrightText: Copyright (c) 2026 linlin
# SPDX-License-Identifier: GPL-3.0-only
"""版本身份门与已审查 wire 字段；不包含 NVIDIA 二进制。"""

from hashlib import sha256
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

from nsight_diagnostics.pc_reasons import reason_name, reviewed_profile


_REVIEWED_SHA = "e6ea6618cbf9be0c5ee143a1213159a9130f94b34347243275db8afb0437aa12"


class PcReasonTest(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.path = Path(directory.name) / "plugin.dll"
        self.path.write_bytes(b"independent identity fixture; not NVIDIA code")

    def approved_profile(self, arch=400, impl=2, version=1):
        # 只注入文件身份；真实 DLL 的 SHA 与代码链由现场独立审查。
        digest = SimpleNamespace(hexdigest=lambda: _REVIEWED_SHA)
        with mock.patch("nsight_diagnostics.pc_reasons.sha256", return_value=digest):
            return reviewed_profile(self.path, arch, impl, version)

    def test_all_reviewed_identity_components_are_required(self):
        self.assertEqual(self.approved_profile()["status"], "decoded")
        for arch, impl, version in ((0x1a0, 2, 1), (400, 3, 1), (400, 2, 2)):
            with self.subTest(arch=arch, impl=impl, version=version):
                profile = self.approved_profile(arch, impl, version)
                self.assertEqual(profile["status"], "unavailable")
                self.assertIsNone(reason_name(0x0001080100000000, profile))

    def test_unreviewed_file_keeps_actual_digest_without_decoding(self):
        profile = reviewed_profile(self.path, 400, 2, 1)
        self.assertEqual(profile["pluginSha256"], sha256(self.path.read_bytes()).hexdigest())
        self.assertEqual(profile["status"], "unavailable")
        self.assertIsNone(reason_name(0x0001080100000000, profile))

    def test_missing_plugin_keeps_the_io_failure(self):
        profile = reviewed_profile(self.path.with_name("absent.dll"), 400, 2, 1)
        self.assertEqual(profile["status"], "unavailable")
        self.assertEqual(profile["errorType"], "FileNotFoundError")
        self.assertIn("absent.dll", profile["pluginPath"])

    def test_reason_reads_only_the_reviewed_byte(self):
        profile = self.approved_profile()
        self.assertEqual(reason_name(0xffff070102030405, profile), "smsp__pcsamp_warps_issue_stalled_wait")
        self.assertEqual(reason_name(0x0001080100000000, profile),
                         "smsp__pcsamp_warps_issue_stalled_long_scoreboard")
        self.assertEqual(reason_name(0x00010b0100000000, profile),
                         "smsp__pcsamp_warps_issue_stalled_no_instructions")
        self.assertIsNone(reason_name(0x0001ff0100000000, profile))

    def test_shared_names_do_not_erase_raw_code_identity(self):
        profile = self.approved_profile()
        self.assertEqual(profile["reasons"][8], profile["reasons"][9])
        self.assertEqual(profile["reasons"][14], profile["reasons"][16])
        self.assertEqual(profile["reasons"][0], profile["reasons"][20])


if __name__ == "__main__":
    unittest.main()
