Texture2D pass_inputs[4] : register(t1);
cbuffer EffectValues : register(b1) { float4 effect_values[4]; };
float parameter(uint index) { return effect_values[index / 4][index % 4]; }
struct Instance {
    float4 rect : POSITION0;
    float4 region : TEXCOORD0;
    float4 timing : TEXCOORD1;
    float4 params : TEXCOORD2;
    float4 geometry : TEXCOORD3;
    float4 travel : TEXCOORD4;
    float4 body : TEXCOORD5;
    float4 label : TEXCOORD6;
};
struct VertexOutput {
    float4 position : SV_POSITION;
    float2 uv : TEXCOORD0;
    nointerpolation float4 region : TEXCOORD1;
    nointerpolation float4 timing : TEXCOORD2;
    nointerpolation float4 params : TEXCOORD3;
    nointerpolation float4 material : TEXCOORD4;
    float4 color : TEXCOORD5;
    nointerpolation float4 geometry : TEXCOORD6;
};
float4 sample_artwork(float2 local, float4 region) {
    if (any(local < 0) || any(local >= region.zw)) return 0;
    float2 pixel = clamp(region.xy + local, region.xy + 0.5, region.xy + region.zw - 0.5);
    return artwork.SampleLevel(artwork_sampler, pixel / dimensions.zw, 0);
}
VertexOutput default_vertex(Instance instance, uint vertex_id) {
    float2 corners[6] = {float2(0,0), float2(1,0), float2(0,1), float2(0,1), float2(1,0), float2(1,1)};
    VertexOutput output = (VertexOutput)0;
    output.uv = corners[vertex_id % 6];
    float2 pixel = instance.rect.xy + output.uv * instance.rect.zw + offset.xy;
    output.position = float4(pixel.x / dimensions.x * 2 - 1, 1 - pixel.y / dimensions.y * 2, 0, 1);
    output.region = instance.region;
    output.timing = instance.timing;
    output.params = instance.params;
    output.geometry = instance.geometry;
    output.color = 1;
    return output;
}
float4 default_pixel(VertexOutput input) { return sample_artwork(input.uv * input.region.zw, input.region); }