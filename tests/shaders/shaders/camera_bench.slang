/// 4K 主射线生成和相机运动下的世界点重投影。
module camera_bench;
import camera.pinhole;

[[vk::binding(0, 0)]]
var inputs : StructuredBuffer<float4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<float4>;

static const let EXTENT = uint2(3840, 2160);

func load_camera(start: uint)->camera::Camera {
    return { inputs[start], inputs[start + 1], inputs[start + 2], inputs[start + 3] };
}

[shader("compute")]
[numthreads(8, 8, 1)]
func ray_main(id: uint3: SV_DispatchThreadID)->void {
    let current = load_camera(0);
    let sampling = inputs[8];
    let uv = camera::camera_sample_uv_offset(id.xy, sampling.xy, sampling.zw);
    let direction = camera::camera_direction(current, uv);
    outputs[id.y * EXTENT.x + id.x] = float4(direction, 1.0);
}

[shader("compute")]
[numthreads(8, 8, 1)]
func project_main(id: uint3: SV_DispatchThreadID)->void {
    let current = load_camera(0);
    let previous = load_camera(4);
    let sampling = inputs[8];
    let uv = camera::camera_sample_uv_offset(id.xy, sampling.xy, sampling.zw);
    let direction = camera::camera_direction(current, uv);
    var result = float4(0.0);
    [loop]
    for (var sample = 0u; sample < 32; ++sample) {
        let distance = float(sample) + 0.5;
        let point_value = current.position.xyz + direction * distance;
        let projected = camera::camera_project(previous, point_value, EXTENT);
        result += float4(projected.pixel, projected.view_z, float(projected.valid));
    }
    outputs[id.y * EXTENT.x + id.x] = result;
}
