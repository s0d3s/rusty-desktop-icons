
float4 rdi_pixel(VertexOutput input) : SV_Target {
    float4 original = default_pixel(input);
    if (input.timing.z <= 0.0) return original;
    float4 changed = saturate(pixel(input));
    changed.rgb = min(changed.rgb, changed.aaa);
    return lerp(original, changed, saturate(input.timing.z));
}
