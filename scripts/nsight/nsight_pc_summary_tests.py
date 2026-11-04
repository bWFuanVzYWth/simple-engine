# SPDX-FileCopyrightText: Copyright (c) 2026 linlin
# SPDX-License-Identifier: GPL-3.0-only
"""窗口 reason 统计独立于可截断的热点 PC；未知与别名保留。"""

import struct
from types import SimpleNamespace
import unittest

from nsight_diagnostics.shaders import pc_samples


class PcSummaryTests(unittest.TestCase):
    def test_reason_totals_survive_top_truncation_with_raw_aliases(self):
        records = [(100, 8), (100, 9), (116, 11), (116, 255)]
        sm = SimpleNamespace(version=1, pcSamplesStorage=b"".join(struct.pack("<QQ", pc, code << 40)
                             for pc, code in records), timestampsStorage=struct.pack("<QQ", 10, 4),
                             HasField=lambda field: field == "version")
        names = [None] * 21
        names[8] = names[9] = "long_scoreboard"
        names[11] = "no_instructions"
        profile = {"status": "decoded", "profileId": "fixture", "reasons": names}
        windows = {"frames": [{"id": "frame", "startNs": 10, "endNs": 20, "durationNs": 10}], "dispatches": []}
        row = pc_samples(SimpleNamespace(PcSamplingPerSM=[sm]), [{"gpuVA": 100, "bytes": 32}],
                         windows, pc_top=1, reason_profile=profile)[0]
        self.assertEqual(sum(r["samples"] for r in row["pcReasons"]["counts"]), 4)
        self.assertEqual(row["pcReasons"]["unknownSamples"], 1)
        self.assertEqual(row["pcReasons"]["status"], "partial")
        top = row["pcHistogram"]["topPcs"][0]
        self.assertEqual(top["samples"], 2)
        self.assertEqual([r["rawReasonCode"] for r in top["reasonSamples"]], [8, 9])


if __name__ == "__main__":
    unittest.main()
