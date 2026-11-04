# SPDX-FileCopyrightText: Copyright (c) 2026 linlin
# SPDX-License-Identifier: GPL-3.0-only
"""离线诊断；不接入 target、回放、GPU 或网络。"""

OUTPUT_SCHEMA_VERSION = 1
TOOL_VERSION = "1.2"


class FormatError(ValueError):
    """输入损坏，或超出已明确支持的格式范围。"""
