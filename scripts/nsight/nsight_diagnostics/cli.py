# SPDX-FileCopyrightText: Copyright (c) 2026 linlin
# SPDX-License-Identifier: GPL-3.0-only
"""离线 CLI 与隔离的 NVPerf worker；不启动捕获或回放。"""

import argparse
import ctypes
import importlib.metadata
import json
import platform
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

from google.protobuf.message import DecodeError

from . import FormatError, OUTPUT_SCHEMA_VERSION, TOOL_VERSION
from .analysis import DEFAULT_METRICS, gpu_windows
from .correlation import correlate_sources, source_pair
from .format import schema_class, sha256, unpack_chunks, write_json
from .internal_rt import attribute_internal_pcs, extract_internal_ranges
from .pc_reasons import reviewed_profile
from .report import markdown_report
from .shaders import DEFAULT_MAX_PC_KEYS, known_mapping, shader_objects, unpack_blob


def file_evidence(path):
    data = path.read_bytes()
    result = {"path": str(path), "bytes": len(data), "sha256": sha256(data), "fileVersion": None}
    if sys.platform == "win32":
        try:
            api = ctypes.WinDLL("version")
            api.GetFileVersionInfoSizeW.argtypes = [ctypes.c_wchar_p, ctypes.c_void_p]
            api.GetFileVersionInfoSizeW.restype = ctypes.c_uint32
            api.GetFileVersionInfoW.argtypes = [ctypes.c_wchar_p, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_void_p]
            api.VerQueryValueW.argtypes = [ctypes.c_void_p, ctypes.c_wchar_p,
                                          ctypes.POINTER(ctypes.c_void_p), ctypes.POINTER(ctypes.c_uint32)]
            size = api.GetFileVersionInfoSizeW(str(path), None)
            if size:
                buffer = ctypes.create_string_buffer(size)
                pointer, length = ctypes.c_void_p(), ctypes.c_uint32()
                if api.GetFileVersionInfoW(str(path), 0, size, buffer) and api.VerQueryValueW(
                        buffer, "\\", ctypes.byref(pointer), ctypes.byref(length)) and length.value >= 52:
                    fixed = ctypes.cast(pointer, ctypes.POINTER(ctypes.c_uint32))
                    if fixed[0] == 0xfeef04bd:
                        result["fileVersion"] = ".".join(map(str, (fixed[2] >> 16, fixed[2] & 65535,
                                                                   fixed[3] >> 16, fixed[3] & 65535)))
        except (OSError, ValueError) as error:
            result["fileVersionUnavailableReason"] = str(error)
    return result


def load_trace(path, cls, max_bytes):
    data = path.read_bytes()
    chunks = unpack_chunks(data, max_chunk_bytes=max_bytes)
    trace_bytes = next((decoded for row, decoded in chunks if row["id"] == 0), None)
    if trace_bytes is None:
        raise FormatError("required metadata chunk 0 missing")
    trace = cls.FromString(trace_bytes)
    if not trace.IsInitialized():
        raise FormatError("protobuf required fields missing: " + ", ".join(trace.FindInitializationErrors()[:10]))
    return trace, sha256(data)


def worker(host, out):
    from .nvperf import evaluate_counter_image
    out = Path(out)
    config = json.loads((out / "counter-request.json").read_text(encoding="utf-8"))
    try:
        result = evaluate_counter_image(Path(host), out / "chunks/chunk-1.bin", config["windows"], config["metrics"], out)
        write_json(out / "counters.json", result)
        return 0
    except (FormatError, OSError, AttributeError, ValueError) as error:
        write_json(out / "counters.json", {"status": "unavailable", "reason": str(error)})
        return 2


def main(argv=None):
    parser = argparse.ArgumentParser(add_help=False,
        description="仅用 CPU 读取已有 WRPV v10 GPU Trace；不启动 Nsight、回放或 GPU。")
    parser.add_argument("-h", "--help", action="help", help="显示帮助后退出")
    parser.add_argument("trace", type=Path, nargs="?", help="已有 .ngfx-gputrace 文件")
    parser.add_argument("--out", type=Path, help="新建或空的输出目录，不覆盖已有证据")
    parser.add_argument("--nsight-host", type=Path, help="含 Plugins/ 和 nvperf_grfx_host.dll 的 Nsight host 目录")
    parser.add_argument("--schema-dir", type=Path, help="此前导出的五份 *.proto.pb；优先于插件提取")
    parser.add_argument("--metadata-only", action="store_true", help="只导出 chunks/schema/GPU窗口，跳过counter与shader")
    parser.add_argument("--metric", action="append", help="只求值指定 NVPerf 指标，可重复；指标原名不翻译")
    parser.add_argument("--expected-spv-dir", type=Path, help="独立冻结的 SPV 目录，用全长 SHA256 比较身份")
    parser.add_argument("--xor-reference-trace", type=Path, help="独立识别的参考 capture，用于推导有限长度 XOR 映射")
    parser.add_argument("--xor-reference-spv", type=Path, help="参考 blob 的独立原始 SPV，不能用待验证目标")
    parser.add_argument("--xor-reference-blob", type=int, help="参考 capture 中的明确 blob-table 索引")
    parser.add_argument("--xor-reference-device", type=int, default=0, help="参考 capture 的 device 索引，默认0")
    parser.add_argument("--cuda-bin", type=Path, help="可选的 CPU cuobjdump/nvdisasm 目录")
    parser.add_argument("--spirv-dis", type=Path, help="可选的 CPU spirv-dis 可执行文件；配合 --source-pair")
    parser.add_argument("--source-pair", action="append", type=source_pair, default=[],
                        help="显式源码配对 device:cubinModule=spirvModule，可重复；PC覆盖取决于 --pc-top")
    parser.add_argument("--max-chunk-mib", type=int, default=512, help="单块解压上限，默认 512 MiB")
    parser.add_argument("--pc-top", type=int, default=0, help="每GPU窗口导出前N个 PC/raw第二u64 频次，默认0关闭")
    parser.add_argument("--pc-reasons", action="store_true", help="按已审查插件 SHA/架构/record version 解码 reason；需 --nsight-host")
    parser.add_argument("--internal-rt", action="store_true", help="导出 redacted RT ELF 具名范围，独立关联已导出 PC；不推断内部代码")
    parser.add_argument("--max-pc-keys", type=int, default=DEFAULT_MAX_PC_KEYS,
                        help="所有窗口的唯一 PC/raw pair 总项数上限，默认1000000；超限明确unavailable")
    parser.add_argument("--counter-worker", nargs=2, metavar=("HOST", "OUT"), help=argparse.SUPPRESS)
    args = parser.parse_args(argv)
    if args.counter_worker:
        return worker(*args.counter_worker)
    if args.trace is None or args.out is None:
        parser.error("须提供 trace 和 --out")
    references = [args.xor_reference_trace, args.xor_reference_spv, args.xor_reference_blob]
    if any(value is not None for value in references) and not all(value is not None for value in references):
        parser.error("须同时提供 --xor-reference-{trace,spv,blob} 三项")
    if args.max_chunk_mib <= 0:
        parser.error("--max-chunk-mib 须为正数")
    if args.pc_top < 0 or args.max_pc_keys <= 0:
        parser.error("--pc-top 须非负，--max-pc-keys 须为正数")
    if bool(args.spirv_dis) != bool(args.source_pair):
        parser.error("须同时提供 --spirv-dis 和 --source-pair")
    if args.pc_reasons and args.nsight_host is None:
        parser.error("--pc-reasons 须提供 --nsight-host，以验证实际插件身份")
    args.trace, args.out = args.trace.resolve(), args.out.resolve()
    if args.out.exists() and (not args.out.is_dir() or any(args.out.iterdir())):
        parser.error("--out 须为新建或空目录，不覆盖已有证据")
    if args.trace == args.out or args.trace in args.out.parents:
        parser.error("输出与原始 trace 路径重叠")
    args.out.mkdir(parents=True, exist_ok=True)
    summary = {"outputSchemaVersion": OUTPUT_SCHEMA_VERSION, "toolVersion": TOOL_VERSION,
               "createdUtc": datetime.now(timezone.utc).isoformat(), "status": "partial",
               "provenance": {"python": sys.version, "platform": platform.platform(),
                              "dependencies": {name: importlib.metadata.version(name) for name in ("protobuf", "lz4")}},
               "errors": [], "unknown": {"actualSpecializations": None, "spp": None, "bounceBudget": None,
               "seed": None, "rtContinuationStackBytes": None, "dynamicSpillBytes": None},
               "limitations": ["Only the reviewed WRPV v10 descriptor profile is supported. This is not a public NVIDIA format specification.",
               "Parsing is CPU-only; original files are never modified. No network, target, replay, or GPU API is used.",
               "Reports may contain private paths/source/debug data. Publication/redaction is the caller's responsibility."]}
    fatal, original = False, None
    counters = shaders = None
    try:
        original = file_evidence(args.trace)
        summary["provenance"]["input"] = original
        chunks = unpack_chunks(args.trace.read_bytes(), max_chunk_bytes=args.max_chunk_mib * 1024**2)
        chunk_dir = args.out / "chunks"
        chunk_dir.mkdir()
        for row, decoded in chunks:
            row["path"] = f"chunks/chunk-{row['id']}.bin"
            (args.out / row["path"]).write_bytes(decoded)
        summary["chunks"] = [row for row, _ in chunks]
        write_json(args.out / "chunks.json", summary["chunks"])
        schemas = args.out / "schemas"
        schemas.mkdir()
        trace = None
        try:
            cls, evidence = schema_class(args.nsight_host, args.schema_dir, schemas)
            summary["provenance"]["schema"] = evidence
            write_json(schemas / "manifest.json", evidence)
        except (FormatError, OSError, DecodeError, ValueError) as error:
            summary["errors"].append({"component": "metadata/schema", "reason": str(error)})
        else:
            trace_bytes = next((decoded for row, decoded in chunks if row["id"] == 0), None)
            if trace_bytes is None:
                raise FormatError("metadata chunk 0 missing")
            trace = cls.FromString(trace_bytes)
            if not trace.IsInitialized():
                raise FormatError("protobuf required fields missing: " + ", ".join(trace.FindInitializationErrors()[:10]))
            clean = cls()
            clean.CopyFrom(trace)
            clean.DiscardUnknownFields()
            has_unknown = clean.ByteSize() != trace.ByteSize()
            del clean
            if has_unknown:
                summary["errors"].append({"component": "metadata/schema",
                    "reason": "trace contains unreviewed protobuf fields; metadata decoding stopped"})
                trace = None
            else:
                tables = [{"name": table.TableName, "entries": [{"name": item.Name, "value": item.Value}
                           for item in table.TableEntries]} for table in trace.InfoTables]
                write_json(args.out / "metadata.json", tables)
                summary["metadata"] = {"path": "metadata.json", "capturedProducerVersion": next(
                    (item.Value for table in trace.InfoTables if table.TableName == "Session"
                     for item in table.TableEntries if item.Name == "Product Version"), None)}
                windows = gpu_windows(trace)
                write_json(args.out / "gpu-windows.json", windows)
                summary["gpuWindows"] = windows
        if trace is not None and not args.metadata_only:
            mapping, reference_evidence = None, None
            if args.xor_reference_trace:
                try:
                    reference, digest = load_trace(args.xor_reference_trace, cls, args.max_chunk_mib * 1024**2)
                    source = args.xor_reference_spv.read_bytes()
                    mapping = known_mapping(reference, args.xor_reference_device, args.xor_reference_blob, source)
                    encoded = unpack_blob(reference.devices[args.xor_reference_device].ShaderProfilerReport.blobTable.blobs[args.xor_reference_blob])
                    reference_evidence = {"sourceSha256": sha256(source), "encodedSha256": sha256(encoded)}
                    summary["provenance"]["xorReference"] = {"capture": str(args.xor_reference_trace),
                        "captureSha256": digest, "device": args.xor_reference_device, "blob": args.xor_reference_blob,
                        "source": file_evidence(args.xor_reference_spv), "mappingBytes": len(mapping),
                        "mappingSha256": sha256(mapping), "encodedSha256": sha256(encoded),
                        "referenceIdentityStatus": "suppliedKnownPlaintext/identityNotIndependentlyVerified",
                        "method": "explicit known plaintext supplied by caller",
                        "limitation": "Derivation from the same target/source is circular and is not independent validation."}
                except (FormatError, OSError, DecodeError, ValueError) as error:
                    summary["errors"].append({"component": "xorReference", "reason": str(error)})
            shader_dir = args.out / "shaders"
            shader_dir.mkdir()
            reason_profile = None
            if args.pc_reasons:
                if len(trace.gpus) != 1 or not trace.gpus[0].HasField("gpuType"):
                    reason_profile = {"status": "unavailable", "reason": "single captured GPU identity required"}
                else:
                    gpu = trace.gpus[0]
                    versions = {sm.version if sm.HasField("version") else None
                                for sm in gpu.SamplingData.PcSamplingPerSM if sm.pcSamplesStorage or sm.timestampsStorage}
                    version = next(iter(versions)) if len(versions) == 1 else None
                    reason_profile = reviewed_profile(args.nsight_host / "Plugins/WarpVizPlugin/WarpVizPlugin.dll",
                                                      gpu.gpuType.Arch, gpu.gpuType.Impl, version)
            shaders = shader_objects(trace, windows, shader_dir, mapping, args.expected_spv_dir, args.cuda_bin,
                                     reference_evidence, args.pc_top, args.max_pc_keys, reason_profile)
            if args.internal_rt:
                internal = attribute_internal_pcs(shaders, extract_internal_ranges(trace, args.out, mapping))
                write_json(args.out / "internal-rt.json", internal)
                shaders["internalRt"] = {"path": "internal-rt.json", "status": internal["status"]}
                if internal["status"] != "decoded":
                    shaders["status"] = "partial"
            if args.spirv_dis:
                correlation = correlate_sources(shaders, args.out, args.spirv_dis, args.source_pair)
                shaders["sourceCorrelation"] = {"path": "source-analysis.json", "status": correlation["status"]}
                if correlation["status"] != "correlated":
                    shaders["status"] = "partial"
            write_json(args.out / "shaders.json", shaders)
            summary["shaders"] = {"path": "shaders.json", "status": shaders["status"],
                                  "fullShaSourceMatchCount": len(shaders["sourceMatches"])}
            counters = {"status": "unavailable", "reason": "no --nsight-host; raw counter image is exported"}
            if args.nsight_host and any(row["id"] == 1 for row, _ in chunks):
                if len(trace.devices) != 1 or len(trace.gpus) != 1:
                    counters["reason"] = "counter-image GPU association not reviewed for multiple GPUs/devices"
                else:
                    metrics = list(dict.fromkeys(args.metric or DEFAULT_METRICS))
                    write_json(args.out / "counter-request.json", {"windows": windows, "metrics": metrics})
                    command = [sys.executable, str(Path(__file__).resolve().parents[1] / "nsight-trace.py"),
                               "--counter-worker", str(args.nsight_host.resolve()), str(args.out)]
                    try:
                        process = subprocess.run(command, capture_output=True, timeout=300)
                    except (OSError, subprocess.SubprocessError) as error:
                        for name in ("stdout", "stderr"):
                            value = getattr(error, name, None) or b""
                            if isinstance(value, str):
                                value = value.encode("utf-8")
                            (args.out / ("counter-worker." + name + ".txt")).write_bytes(value)
                        counters = {"status": "unavailable", "reason": str(error),
                                    "workerFailureType": type(error).__name__,
                                    "workerExitCode": getattr(error, "returncode", None)}
                    else:
                        (args.out / "counter-worker.stdout.txt").write_bytes(process.stdout)
                        (args.out / "counter-worker.stderr.txt").write_bytes(process.stderr)
                        if (args.out / "counters.json").exists():
                            counters = json.loads((args.out / "counters.json").read_text(encoding="utf-8"))
                        else:
                            counters = {"status": "unavailable", "reason": "NVPerf child failed without a result"}
                        counters["workerExitCode"] = process.returncode
                        if process.returncode != 0 and counters["status"] == "decoded":
                            counters["status"] = "partial"
                            counters["workerFailureReason"] = "NVPerf child exited unsuccessfully after writing a result"
            elif args.nsight_host:
                counters["reason"] = "counter image chunk 1 absent"
            write_json(args.out / "counters.json", counters)
            summary["counters"] = {"path": "counters.json", "status": counters["status"]}
            if args.nsight_host:
                dll = args.nsight_host / "nvperf_grfx_host.dll"
                if dll.exists():
                    summary["provenance"]["nvperfHostLibrary"] = file_evidence(dll)
            if shaders["status"] == "decoded" and counters["status"] == "decoded" and not summary["errors"]:
                summary["status"] = "decoded"
        elif args.metadata_only:
            summary["limitations"].append("--metadata-only intentionally omitted shader objects and counter evaluation.")
    except (OSError, FormatError, DecodeError, ValueError, subprocess.SubprocessError) as error:
        fatal = True
        summary["errors"].append({"component": "input/analysis", "reason": str(error)})
    finally:
        if original:
            try:
                after = file_evidence(args.trace)
                summary["provenance"]["inputSha256After"] = after["sha256"]
                summary["provenance"]["inputUnchanged"] = original["sha256"] == after["sha256"]
                if not summary["provenance"]["inputUnchanged"]:
                    fatal = True
                    summary["errors"].append({"component": "integrity", "reason": "input changed during analysis"})
            except OSError as error:
                fatal = True
                summary["errors"].append({"component": "integrity", "reason": str(error)})
        if fatal:
            summary["status"] = "failed"
        summary["report"] = markdown_report(summary, counters, shaders, args.out)
        write_json(args.out / "summary.json", summary)
    print(json.dumps({"status": summary["status"], "summary": str(args.out / "summary.json"),
                      "errors": summary["errors"]}, ensure_ascii=False))
    return 2 if fatal else (0 if summary["status"] == "decoded" else 3)
