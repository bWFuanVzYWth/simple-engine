module prepare;

struct Parameters {
    var frame : uint;
};

[[vk::binding(0, 0)]]
var input : RWTexture2D<float4>;
[[vk::binding(1, 0)]]
var output : RWTexture2D<float4>;
[[vk::binding(2, 0)]]
var parameters : ConstantBuffer<Parameters>;

[numthreads(8, 8, 1)]
func prepare_main(uint3 id: SV_DispatchThreadID)->void {
    var width : uint;
    var height : uint;
    input.GetDimensions(width, height);
    if (id.x >= width || id.y >= height)
        return;
    input[id.xy] = float4(id.x + 1, id.y + 1, parameters.frame, 1);
    output[id.xy] = -1;
}
