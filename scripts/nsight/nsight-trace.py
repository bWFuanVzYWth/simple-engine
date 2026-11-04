# SPDX-FileCopyrightText: Copyright (c) 2026 linlin
# SPDX-License-Identifier: GPL-3.0-only
"""只读已有 Nsight GPU Trace 的 CPU 工具；操作见 manual/nsight.md。"""

import sys

sys.stdout.reconfigure(encoding="utf-8")
sys.stderr.reconfigure(encoding="utf-8")

try:
    from nsight_diagnostics.cli import main
except ModuleNotFoundError as error:
    print("离线读取依赖缺失：" + str(error) +
          "。请在独立环境执行 python -m pip install -r scripts/nsight-requirements.txt", file=sys.stderr)
    raise SystemExit(2)


if __name__ == "__main__":
    raise SystemExit(main())
