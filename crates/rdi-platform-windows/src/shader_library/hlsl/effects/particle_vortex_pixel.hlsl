
float4 pixel(VertexOutput input) : SV_Target {
    float2 local = input.tile.xy + lerp(input.uv, float2(0.5, 0.5), input.dust.x) * input.tile.zw;
    float2 pixel = clamp(input.region.xy + local,
        input.region.xy + 0.5, input.region.xy + input.region.zw - 0.5);
    float4 color = artwork.SampleLevel(artwork_sampler, pixel / dimensions.zw, 0);
    float radius = length(input.uv * 2.0 - 1.0);
    float coverage = lerp(1.0, 1.0 - smoothstep(0.55, 1.0, radius), input.dust.x);
    return color * coverage;
}
