
struct Instance {
    float4 rect : POSITION0;
    float4 region : TEXCOORD0;
    float4 timing : TEXCOORD1;
    float4 params : TEXCOORD2;
    float4 geometry : TEXCOORD3;
    float4 travel : TEXCOORD4;
};
struct VertexOutput {
    float4 position : SV_POSITION;
    float2 uv : TEXCOORD0;
    nointerpolation float4 region : TEXCOORD1;
    nointerpolation float4 tile : TEXCOORD2;
    nointerpolation float2 dust : TEXCOORD3;
    nointerpolation float4 timing : TEXCOORD4;
    nointerpolation float4 params : TEXCOORD5;
};
VertexOutput default_vertex(Instance instance, uint vertex_id) {
    float2 corners[6] = {float2(0,0), float2(1,0), float2(0,1),
                         float2(0,1), float2(1,0), float2(1,1)};
    uint particle = vertex_id / 6;
    uint2 grid = (uint2)clamp(ceil(instance.region.zw / instance.params.x), 1, 64);
    float2 cell = instance.region.zw / grid;
    float2 tile = float2(particle % grid.x, particle / grid.x) * cell;
    VertexOutput output;
    output.uv = corners[vertex_id % 6];
    float2 pixel = instance.rect.xy + tile + output.uv * cell + offset.xy;
    output.position = float4(pixel.x / dimensions.x * 2 - 1,
                            1 - pixel.y / dimensions.y * 2, 0, 1);
    output.region = instance.region;
    output.tile = float4(tile, cell);
    output.dust = float2(0, 0);
    output.timing = instance.timing;
    output.params = instance.params;
    return output;
}
float4 default_pixel(VertexOutput input) {
    float2 local = input.tile.xy + input.uv * input.tile.zw;
    float2 pixel = clamp(input.region.xy + local,
        input.region.xy + 0.5, input.region.xy + input.region.zw - 0.5);
    return artwork.SampleLevel(artwork_sampler, pixel / dimensions.zw, 0);
}
float particle_hash(float value) {
    return frac(sin(value * 12.9898 + 78.233) * 43758.5453);
}
