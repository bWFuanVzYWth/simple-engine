# SPDX-FileCopyrightText: Copyright (c) 2026 linlin
# SPDX-License-Identifier: GPL-3.0-only
"""独立 ELF64 夹具与 PC 半开范围行为，不包含 NVIDIA 代码。"""

import json
from pathlib import Path
import struct
import tempfile
from types import SimpleNamespace
import unittest

from google.protobuf import descriptor_pb2, descriptor_pool, message_factory

from nsight_diagnostics.internal_rt import attribute_internal_pcs, extract_internal_ranges


class Message(SimpleNamespace):
    def HasField(self, name):
        return hasattr(self, name)


def redacted_elf(symbols):
    names = b"\0.shstrtab\0.strtab\0.symtab\0"
    strings, entries = bytearray(b"\0"), bytearray(24)
    for index, (symbol, value, size) in enumerate(symbols):
        name = len(strings)
        strings += symbol.encode() + b"\0"
        entries += struct.pack("<IBBHQQ", name, 0x12, 8, index + 4, value, size)
    data = bytearray(64)
    data[:7] = b"\x7fELF\x02\x01\x01"
    struct.pack_into("<HHI", data, 16, 2, 190, 1)
    sections = [bytes(64)]
    for name, kind, content, link, stride in ((1, 3, names, 0, 0), (11, 3, strings, 0, 0),
                                             (19, 2, entries, 2, 24)):
        sections.append(struct.pack("<IIQQQQIIQQ", name, kind, 0, 0, len(data), len(content), link, 0, 1, stride))
        data += content
    for _ in symbols:
        sections.append(struct.pack("<IIQQQQIIQQ", 0, 8, 0, 0, 0, 0, 0, 0, 1, 0))
    struct.pack_into("<Q", data, 40, len(data))
    struct.pack_into("<HHHHHH", data, 52, 64, 0, 0, 64, len(sections), 1)
    return bytes(data) + b"".join(sections)


def trace_fixture(data, copies=1):
    blob = Message(data=data, compression=1, obfuscation=1, uncompressedSize=len(data))
    modules = [Message(rtCoreInternalRedactedCubin=Message(cubin=Message(ref=Message(idx=0))))
               for _ in range(copies)]
    report = Message(blobTable=Message(blobs=[blob]),
                     pcCountersInfo=Message(pcSamplingSession=Message(modules=modules)))
    return Message(devices=[Message(ShaderProfilerReport=report)], gpus=[Message()])


def optional_ref_fixture(missing):
    descriptor = descriptor_pb2.FileDescriptorProto(name="optional-ref.proto", package="fixture", syntax="proto2")
    for name, field, target in (("Ref", "idx", None), ("Cubin", "ref", "Ref"),
                                 ("Internal", "cubin", "Cubin"), ("Module", "rtCoreInternalRedactedCubin", "Internal")):
        message = descriptor.message_type.add(name=name)
        item = message.field.add(name=field, number=1, type=11 if target else 13, label=1)
        if target:
            item.type_name = ".fixture." + target
    pool = descriptor_pool.DescriptorPool()
    pool.Add(descriptor)
    cls = message_factory.GetMessageClass(pool.FindMessageTypeByName("fixture.Module"))
    module = cls()
    current = module.rtCoreInternalRedactedCubin
    current.SetInParent()
    for field in ("cubin", "ref"):
        if field == missing:
            break
        current = getattr(current, field)
        current.SetInParent()
    return module


def range_fixture(start, size, symbol, blob=0):
    return {"device": 0, "gpuVA": start, "bytes": size, "symbol": symbol,
            "sources": [{"blob": blob, "modules": [blob], "symbol": symbol}]}


def metadata_fixture(ranges):
    return {"status": "decoded", "deviceCount": 1, "gpuCount": 1, "ranges": ranges}


def shaders_fixture(total, pcs=None):
    window = {"windowId": "frame", "totalSamples": total, "unmappedSamples": total}
    if pcs is not None:
        window["pcHistogram"] = {"topPcs": [{"pc": hex(pc), "samples": samples} for pc, samples in pcs]}
    return {"functions": [], "pcWindows": [window]}


class InternalRtTest(unittest.TestCase):
    def test_redacted_symbol_range_does_not_require_or_invent_code(self):
        data = redacted_elf([("Ray Tracing Traversal", 0x1000, 0x80)])
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            result = extract_internal_ranges(trace_fixture(data, copies=2), out, None)
            self.assertEqual(result["status"], "decoded")
            self.assertEqual(len(result["objects"]), 1)
            self.assertEqual(result["objects"][0]["modules"], [0, 1])
            self.assertEqual((out / result["objects"][0]["path"]).read_bytes(), data)
            row = result["ranges"][0]
            self.assertEqual((row["symbol"], row["gpuVA"], row["bytes"]), ("Ray Tracing Traversal", 0x1000, 0x80))
            self.assertFalse(row["codeAvailable"])
            self.assertEqual(row["sources"][0]["sectionBytes"], 0)

    def test_bad_elf_preserves_exported_bytes_and_failure(self):
        data = b"truncated ELF fixture"
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            result = extract_internal_ranges(trace_fixture(data), out, None)
            self.assertEqual(result["status"], "partial")
            self.assertEqual(result["failures"][0]["errorType"], "FormatError")
            self.assertEqual((out / result["objects"][0]["path"]).read_bytes(), data)
            self.assertEqual(result["ranges"], [])

    def test_absent_optional_reference_never_aliases_blob_zero(self):
        data = redacted_elf([("Ray Tracing Traversal", 0x1000, 0x80)])
        for missing in ("cubin", "ref", "idx"):
            with self.subTest(missing=missing), tempfile.TemporaryDirectory() as directory:
                trace = trace_fixture(data)
                trace.devices[0].ShaderProfilerReport.pcCountersInfo.pcSamplingSession.modules = [optional_ref_fixture(missing)]
                result = extract_internal_ranges(trace, directory, None)
                self.assertEqual(result["status"], "partial")
                self.assertEqual(result["ranges"], [])
                self.assertIsNone(result["objects"][0]["blob"])

    def test_exact_aliases_merge_sources_without_changing_existing_attribution(self):
        metadata = metadata_fixture([range_fixture(0x1000, 0x100, "captured category", 1),
                                     range_fixture(0x1000, 0x100, "captured category", 2)])
        shaders = shaders_fixture(10, [(0x1000, 10)])
        before = json.dumps(shaders, sort_keys=True)
        result = attribute_internal_pcs(shaders, metadata)
        self.assertEqual(json.dumps(shaders, sort_keys=True), before)
        self.assertEqual(len(result["metadata"]["ranges"]), 1)
        self.assertEqual(len(result["metadata"]["ranges"][0]["sources"]), 2)
        self.assertEqual(result["windows"][0]["categorySamples"], {"captured category": 10})

    def test_partial_top_list_is_never_extrapolated(self):
        metadata = metadata_fixture([range_fixture(0x1000, 0x100, "captured category")])
        window = attribute_internal_pcs(shaders_fixture(100, [(0x1000, 30), (0x1200, 5)]), metadata)["windows"][0]
        self.assertFalse(window["allPcsExported"])
        self.assertEqual(window["exportedSamples"], 35)
        self.assertEqual(window["sampleCounts"], {"internal": 30, "unattributed": 5})

    def test_function_range_end_is_exclusive(self):
        metadata = metadata_fixture([range_fixture(0x1000, 0x100, "captured category")])
        window = attribute_internal_pcs(shaders_fixture(7, [(0x10ff, 3), (0x1100, 4)]), metadata)["windows"][0]
        self.assertEqual(window["sampleCounts"], {"internal": 3, "unattributed": 4})

    def test_full_pc_export_does_not_imply_complete_identity_attribution(self):
        metadata = metadata_fixture([range_fixture(0x1000, 0x100, "captured category")])
        window = attribute_internal_pcs(shaders_fixture(4, [(0x1200, 4)]), metadata)["windows"][0]
        self.assertTrue(window["allPcsExported"])
        self.assertEqual(window["attributionStatus"], "partial")

    def test_nonidentical_overlapping_ranges_are_ambiguous(self):
        metadata = metadata_fixture([range_fixture(0x1000, 0x100, "A"), range_fixture(0x1080, 0x100, "B")])
        window = attribute_internal_pcs(shaders_fixture(10, [(0x10a0, 10)]), metadata)["windows"][0]
        self.assertTrue(window["allPcsExported"])
        self.assertEqual(window["sampleCounts"], {"ambiguous": 10})
        self.assertEqual(window["categorySamples"], {})
        self.assertEqual(window["pcs"][0]["rangeIndices"], [0, 1])

    def test_conflicting_names_for_identical_range_are_preserved(self):
        metadata = metadata_fixture([range_fixture(0x1000, 0x100, "A"), range_fixture(0x1000, 0x100, "B")])
        result = attribute_internal_pcs(shaders_fixture(10, [(0x1000, 10)]), metadata)
        self.assertEqual(len(result["metadata"]["ranges"]), 1)
        self.assertEqual(result["metadata"]["ranges"][0]["symbols"], ["A", "B"])
        self.assertEqual(result["windows"][0]["sampleCounts"], {"ambiguous": 10})

    def test_no_pc_histogram_keeps_metadata_but_not_complete_attribution(self):
        metadata = metadata_fixture([range_fixture(0x1000, 0x100, "captured category")])
        result = attribute_internal_pcs(shaders_fixture(100), metadata)
        self.assertEqual(len(result["metadata"]["ranges"]), 1)
        self.assertFalse(result["windows"][0]["allPcsExported"])
        self.assertEqual(result["windows"][0]["pcs"], [])

    def test_existing_function_overlap_is_not_overwritten(self):
        metadata = metadata_fixture([range_fixture(0x1000, 0x100, "captured category")])
        shaders = shaders_fixture(10, [(0x1000, 10)])
        shaders["functions"] = [{"gpuVA": 0x1000, "bytes": 0x100}]
        window = attribute_internal_pcs(shaders, metadata)["windows"][0]
        self.assertEqual(window["sampleCounts"], {"ambiguous": 10})
        self.assertEqual(window["pcs"][0]["existingFunctionIndices"], [0])

    def test_multiple_device_address_namespace_is_unavailable(self):
        metadata = metadata_fixture([range_fixture(0x1000, 0x100, "captured category")])
        metadata["deviceCount"] = 2
        window = attribute_internal_pcs(shaders_fixture(10, [(0x1000, 10)]), metadata)["windows"][0]
        self.assertEqual(window["sampleCounts"], {"unavailable": 10})
        self.assertEqual(window["categorySamples"], {})


if __name__ == "__main__":
    unittest.main()
