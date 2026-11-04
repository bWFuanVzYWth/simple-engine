# SPDX-FileCopyrightText: Copyright (c) 2026 linlin
# SPDX-License-Identifier: GPL-3.0-only
"""按已审查安装版的身份解码 PC reason；不执行插件，不推断其余 packed 字段。"""

from hashlib import sha256
from pathlib import Path


_PLUGIN_SHA256 = "e6ea6618cbf9be0c5ee143a1213159a9130f94b34347243275db8afb0437aa12"
_PROFILE_ID = "ngfx-2026.1.0-arch400-impl2-persm1"
_REASONS = tuple("smsp__pcsamp_warps_issue_stalled_" + name for name in (
    "misc", "drain", "imc_miss", "barrier", "membar", "sleeping", "branch_resolving", "wait",
    "long_scoreboard", "long_scoreboard", "short_scoreboard", "no_instructions", "math_pipe_throttle",
    "tex_throttle", "mio_throttle", "lg_throttle", "mio_throttle", "dispatch_stall", "selected",
    "not_selected", "misc",
))


def reviewed_profile(plugin_path, arch, impl, version):
    """验证插件 SHA、捕获 Arch/Impl 与 PerSM version，返回具名映射或 unavailable。

    输入须来自已审查的 WRPV v10 descriptor profile；架构不由产品名称推断。
    仅审查 Arch=400、Impl=2、version=1。调用方每个捕获／组合准备一次并复用返回值；
    本函数读取 DLL 文件计算 SHA，不加载或调用 NVIDIA 代码。失败保留实际身份及原因。
    映射记录公开指标名称与代码审查位置，不携带 NVIDIA 二进制或反汇编字节。
    """
    result = {"status": "unavailable", "arch": arch, "impl": impl, "recordVersion": version}
    if (arch, impl, version) != (400, 2, 1):
        result["reason"] = "unreviewed architecture/implementation/PC record version"
        return result
    if plugin_path is None:
        result["reason"] = "explicit WarpVizPlugin.dll path required for versioned reason mapping"
        return result
    path = Path(plugin_path).resolve()
    result["pluginPath"] = str(path)
    try:
        digest = sha256(path.read_bytes()).hexdigest()
    except OSError as error:
        result.update(reason=str(error), errorType=type(error).__name__)
        return result
    result["pluginSha256"] = digest
    if digest != _PLUGIN_SHA256:
        result["reason"] = "unreviewed WarpVizPlugin.dll SHA256; packed reasons remain unknown"
        return result
    result.update(status="decoded", profileId=_PROFILE_ID, reasons=list(_REASONS),
        rawReason={"recordBytes": 16, "recordByteOffset": 13, "secondU64Shift": 40, "mask": 255},
        evidence={
            "kind": "reviewedInstalledPluginCode", "pluginFileVersion": "2026.1.0.0",
            "installationRelease": "2026.1.0",
            "descriptorProfile": "wrpv10-reviewed-wire-v1",
            "rvaBase": "preferred image base excluded",
            "wireFields": [
                {"rva": "0x088f80", "field": 2, "name": "version", "memberOffset": "0x30"},
                {"rva": "0x088fc2", "field": 3, "name": "timestampsStorage", "memberOffset": "0x18"},
                {"rva": "0x088fe3", "field": 4, "name": "pcSamplesStorage", "memberOffset": "0x20"},
            ],
            "copyChain": [
                {"rva": "0x16f84e", "operation": "require PerSM version 1"},
                {"rva": "0x16fa7f..0x16faa0", "operation": "copy pcSamplesStorage bytes into per-SM records"},
                {"rva": "0x0f0b70..0x0f0b9e", "operation": "select records with unchanged 16-byte copies"},
                {"rva": "0x0f56b7..0x0f5771", "operation": "read byte 13, remap by architecture table, advance 16"},
                {"rva": "0x0efdb4..0x0efdd4", "operation": "count remapped reason in per-PC sample-type columns"},
            ],
            "architectureDispatchRva": "0x0f4c7c..0x0f4cc7",
            "rawCodeNameTableRva": "0xb0a9e0",
        },
        limitations=[
            "This is a hash-gated reviewed private layout, not a public NVIDIA file-format specification.",
            "Raw codes remain distinct: 8/9 share Long Scoreboard; 14/16 share MIO Throttle; 0/20 share Misc.",
            "No RTCORE/L1TEX split is inferred from duplicate metric names.",
            "Other bits of rawSecondU64 remain uninterpreted; sample frequency is not elapsed time.",
        ])
    return result


def reason_name(raw_second, profile):
    """返回已审查记录的完整指标名；profile 不可用或 code 越界时返回 None。

    raw_second 为原始 little-endian 记录的第二个 u64；调用方继续保存原值及独立 code，
    不以合并后的名称覆盖记录。profile 须为 reviewed_profile 的未修改返回值。
    """
    if profile["status"] != "decoded":
        return None
    code = (raw_second >> 40) & 255
    return profile["reasons"][code] if code < len(profile["reasons"]) else None
