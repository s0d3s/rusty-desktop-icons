
struct Instance {
    float4 rect : POSITION0;
    float4 region : TEXCOORD0;
    float4 timing : TEXCOORD1;
    float4 params : TEXCOORD2;
};
struct VertexOutput {
    float4 position : SV_POSITION;
    float2 uv : TEXCOORD0;
    nointerpolation float4 region : TEXCOORD1;
    nointerpolation float4 timing : TEXCOORD2;
    nointerpolation float4 params : TEXCOORD3;
};
float4 sample_icon(float2 uv, VertexOutput input) {
    if (any(uv < 0.0) || any(uv > 1.0)) return 0.0;
    float2 pixel = clamp(input.region.xy + uv * input.region.zw,
        input.region.xy + 0.5, input.region.xy + input.region.zw - 0.5);
    return artwork.SampleLevel(artwork_sampler, pixel / dimensions.zw, 0);
}
float4 default_pixel(VertexOutput input) {
    return sample_icon(input.uv, input);
}
VertexOutput default_vertex(Instance instance, uint vertex_id) {
    float2 corners[6] = {float2(0,0), float2(1,0), float2(0,1),
                         float2(0,1), float2(1,0), float2(1,1)};
    VertexOutput output;
    output.uv = corners[vertex_id];
    float2 pixel = instance.rect.xy + output.uv * instance.rect.zw + offset.xy;
    output.position = float4(pixel.x / dimensions.x * 2 - 1,
                            1 - pixel.y / dimensions.y * 2, 0, 1);
    output.region = instance.region;
    output.timing = instance.timing;
    output.params = instance.params;
    return output;
}
