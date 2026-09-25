
float4 pixel(VertexOutput input) : SV_Target {
    float row = floor(input.uv.y * input.region.w / max(1.0, input.params.z));
    float moment = floor(input.timing.x * max(1.0, input.params.w));
    float noise = frac(sin(row * 12.9898 + moment * 78.233 + input.timing.w) * 43758.5453);
    float displacement = (noise * 2 - 1) * input.params.x * step(0.65, noise);
    float2 uv = input.uv + float2(displacement / input.region.z, 0);
    float2 split = float2(input.params.y / input.region.z, 0);
    float4 red = sample_icon(uv + split, input);
    float4 green = sample_icon(uv, input);
    float4 blue = sample_icon(uv - split, input);
    return float4(red.r, green.g, blue.b, max(red.a, max(green.a, blue.a)));
}
