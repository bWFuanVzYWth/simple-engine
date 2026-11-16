/// 直接消费生产准入、存储和合并入口；不另复制它们的布局或算式。
module restir_test;
import restir_pt.storage;
import restir_pt.admission;
import restir_pt.merge;
import restir_pt.types;
import restir_pt.reservoir;
import restir_pt.resampling;
import geometry.ray;

using namespace restir_pt;

[[vk::binding(0, 0)]]
var source : RWByteAddressBuffer;
[[vk::binding(1, 0)]]
var output : RWByteAddressBuffer;

[shader("compute")]
[numthreads(64, 1, 1)]
func storage_roundtrip(uint3 id: SV_DispatchThreadID)->void
{
    let index = id.x;
    if (index >= 55u)
        return;
    store_reservoir(output, index, load_reservoir(source, index));
    // 以同一buffer内的独立尾段分别检查60B与20B记录，末条记录后无额外padding。
    var base = 55u * RESERVOIR_STRIDE;
    var record = base / RECONNECTION_STRIDE + index;
    store_reconnection(output, record, load_reconnection(source, record));
    base = (base / RECONNECTION_STRIDE + 55u) * RECONNECTION_STRIDE;
    record = base / PRECOMPUTED_STRIDE + index;
    store_precomputed(output, record, load_precomputed(source, record));
}

[shader("compute")]
[numthreads(1, 1, 1)]
func admission_cases()->void
{
    let a : AdmissionSurface = { float3(0, 0, 1), 10.0, false };
    var b : AdmissionSurface = { float3(0, 0, -1), 10.0, true };
    output.Store(0, uint(spatial_candidate(a, b)));
    output.Store(4, uint(spatial_candidate(b, a)));
    b.both_sides = false;
    output.Store(8, uint(spatial_candidate(a, b)));
    let front : geometry::SurfacePoint = { float3(0, 0, 10), float3(0, 0, 1), 0.0 };
    let back : geometry::SurfacePoint = { float3(0, 0, 10), float3(0, 0, -1), 0.0 };
    output.Store(12, uint(spatial_candidate(admission_surface(front, 0.0, false),
                                            admission_surface(back, 0.0, false))));
    b = { float3(0, 0, 1), 11.2, true };
    output.Store(16, uint(spatial_candidate(a, b)));
    var donor : uint2;
    let accepted =
        temporal_candidate(float2(12.75, 9.25), true, uint2(32), float2(0.25, -0.25), donor);
    output.Store3(20, uint3(donor, accepted));
    output.Store(
        32, uint(temporal_candidate(float2(0.1, 0.5), true, uint2(32), float2(0.25, 0), donor)));
    output.Store(36, uint(temporal_candidate(float2(1), false, uint2(32), 0.0, donor)));
}

[shader("compute")]
[numthreads(64, 1, 1)]
func selection_equivalence(uint3 id: SV_DispatchThreadID)->void
{
    let index = id.x;
    let canonical = load_reservoir(source, 0);
    let neighbor = load_reservoir(source, 1);
    let u = (float(index) + 0.5) / 64.0;
    let forward : PrecomputedShiftData = { float3(0.5, 0.25, 0.75), 0.8, 0.6 };
    let reverse : PrecomputedShiftData = { float3(0.2, 0.4, 0.1), 1.2, 0.9 };
    var expected =
        temporal_resample(canonical, neighbor, reverse, forward, float2(u, 1.0 - u), 20.0);
    let choice = temporal_selection(load_merge_source(source, 0), load_merge_source(source, 1),
                                    reverse, forward, 0, 1, float2(u, 1.0 - u), 20.0);
    var actual = load_selection(source, choice);
    store_reservoir(output, 2u * index, expected);
    store_reservoir(output, 2u * index + 1u, actual);
    let factor
        : PairwiseFactor = { canonical.sample_mass + neighbor.sample_mass, canonical.sample_mass };
    expected = empty_reservoir();
    var canonical_weight = spatial_begin(factor);
    spatial_merge_neighbor(expected, canonical, neighbor, forward, reverse, factor,
                           canonical_weight, u);
    spatial_finish(expected, canonical, canonical_weight, 1.0 - u);
    var spatial : SpatialSelection;
    spatial.init(load_merge_source(source, 0), factor.c_sum);
    spatial.add(load_merge_source(source, 1), 1, forward, reverse, u);
    spatial.finish(0, 1.0 - u);
    actual = load_selection(source, spatial.selected);
    store_reservoir(output, 128u + 2u * index, expected);
    store_reservoir(output, 128u + 2u * index + 1u, actual);
}
