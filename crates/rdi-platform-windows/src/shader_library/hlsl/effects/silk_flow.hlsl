float4 ribbon_sample(float2 local, float4 region) {
    float2 pixel = clamp(region.xy + local, region.xy + 0.5, region.xy + region.zw - 0.5);
    return artwork.SampleLevel(artwork_sampler, pixel / dimensions.zw, 0);
}
float ribbon_hash(float value) {
    return frac(sin(value * 12.9898 + 78.233) * 43758.5453);
}
float2 ribbon_center(float along, float strand, Instance instance) {
    float progress = instance.timing.y;
    float2 displacement = instance.travel.zw - instance.travel.xy;
    float distance = length(displacement);
    float2 forward = distance > 0.001 ? displacement / distance : float2(1, 0);
    float2 lateral = float2(-forward.y, forward.x);
    float seed = instance.timing.w * 0.17;
    float phase = along * 6.2831853 * instance.params.z - progress * 5.0 + seed + strand * 0.24;
    float fold = sin(phase + sin(phase * 0.61 + seed) * 1.4);
    float secondary = sin(phase * 1.83 + strand * 0.43 + seed);
    float taper = pow(max(0.0, sin(along * 3.14159265)), 0.7);
    float separation = (strand / max(1.0, instance.params.x - 1.0) - 0.5);
    float spread = instance.geometry.z * instance.params.y;
    float2 bend = lateral * (fold * 0.55 + secondary * 0.16 + separation * 0.85)
        + forward * (cos(phase * 0.92 + separation) * 0.55);
    precise float2 origin = instance.rect.xy + instance.travel.xy;
    return origin + instance.geometry.xy + displacement * along + bend * spread * taper;
}
VertexOutput vertex(Instance instance, uint vertex_id : SV_VertexID) {
    float2 corners[6] = {float2(0,0), float2(1,0), float2(0,1),
                         float2(0,1), float2(1,0), float2(1,1)};
    VertexOutput output = (VertexOutput)0;
    float2 corner = corners[vertex_id % 6];
    float progress = instance.timing.y;
    float strength = saturate(instance.timing.z);
    output.timing = instance.timing;
    output.params = instance.params;
    output.geometry = instance.geometry;
    output.color = 1.0;
    float2 pixel;
    if (vertex_id < 24) {
        uint layer = vertex_id / 6;
        bool destination = layer >= 2;
        bool label = (layer % 2) == 1;
        output.region = label ? instance.label : instance.body;
        output.uv = corner;
        float visibility = destination
            ? smoothstep(label ? 0.91 : 0.70, label ? 1.0 : 0.96, progress)
            : 1.0 - smoothstep(label ? 0.01 : 0.06, label ? 0.16 : 0.34, progress);
        output.material = float4(0, visibility * strength + (destination ? 0 : 1 - strength), destination ? 1 : -1, label ? 1 : 0);
        float2 endpoint = destination ? instance.travel.zw : instance.travel.xy;
        pixel = instance.rect.xy + endpoint * strength + corner * instance.rect.zw;
    } else {
        uint quad = (vertex_id - 24) / 6;
        uint segment = quad % 128;
        uint strand = (quad / 128) % (uint)instance.params.x;
        bool halo = quad / (128 * (uint)instance.params.x) == 0;
        float coordinate = (segment + corner.x) / 128.0;
        float tail = smoothstep(0.45, 0.98, progress);
        float head = smoothstep(0.04, 0.60, progress);
        float along = lerp(tail, head, coordinate);
        float2 center = ribbon_center(along, strand, instance);
        float2 previous = ribbon_center(max(0.0, along - 0.003), strand, instance);
        float2 next = ribbon_center(min(1.0, along + 0.003), strand, instance);
        float2 tangent = next - previous;
        float2 normal = length(tangent) > 0.00001 ? normalize(float2(-tangent.y, tangent.x)) : float2(0,1);
        float flutter = 0.72 + 0.28 * sin(along * 19.0 + strand * 1.7 - progress * 8.0);
        float taper = pow(max(0.0, sin(coordinate * 3.14159265)), 0.55);
        float width = instance.geometry.z * (0.22 + 0.055 * sin(strand * 2.7)) * flutter;
        width *= taper * (halo ? 2.0 : 1.0);
        float2 incoming = center - previous;
        float2 outgoing = next - center;
        float turn = abs(incoming.x * outgoing.y - incoming.y * outgoing.x)
            / max(0.000001, length(incoming) * length(outgoing));
        float bend_radius = length(tangent) / max(0.0001, 2.0 * turn);
        width = min(width, max(0.75, bend_radius * 0.55));
        pixel = center + normal * (corner.y * 2.0 - 1.0) * width;
        output.uv = float2(along, corner.y);
        float phase = smoothstep(0.04, 0.25, progress) * (1 - smoothstep(0.77, 0.99, progress));
        output.material = float4(halo ? 2 : 1, phase * strength * taper, strand, 0);
        float4 pigment = 0;
        float angle = strand * 2.399963;
        float radius = strand == 0 ? 0 : 0.30;
        float2 pigment_uv = 0.5 + float2(cos(angle), sin(angle)) * radius;
        [unroll] for (uint sample_index = 0; sample_index < 8; ++sample_index) {
            float sample_angle = sample_index * 2.399963;
            float2 sample_uv = pigment_uv + float2(cos(sample_angle), sin(sample_angle)) * 0.035;
            pigment += ribbon_sample(instance.geometry.xy + (sample_uv - 0.5) * instance.geometry.zw, instance.body);
        }
        float3 color = pigment.a > 0.0001 ? pigment.rgb / pigment.a : float3(0.3, 0.55, 0.9);
        float maximum = max(color.r, max(color.g, color.b));
        color = lerp(color, color / max(0.35, maximum), 0.35);
        output.color = float4(color, saturate(pigment.a * 0.5));
    }
    pixel += offset.xy;
    output.position = float4(pixel.x / dimensions.x * 2 - 1, 1 - pixel.y / dimensions.y * 2, 0, 1);
    return output;
}
float4 pixel(VertexOutput input) : SV_Target {
    if (input.material.x < 0.5) {
        float progress = input.timing.y;
        float fade = input.material.y;
        float2 local = input.uv * input.region.zw;
        float2 uv = (local - input.geometry.xy) / input.geometry.zw + 0.5;
        if (input.material.w < 0.5) {
            float deformation = sin(saturate(fade) * 3.14159265) * input.timing.z;
            local += float2(sin(uv.y * 23 + progress * 9), cos(uv.x * 19 - progress * 7)) * deformation * input.geometry.zw * 0.06;
            float noise = 0.5 + 0.25 * sin(uv.x * 12 + sin(uv.y * 9) * 2) + 0.25 * sin(uv.y * 15);
            fade = smoothstep(noise * 0.65, noise * 0.65 + 0.35, fade);
        }
        return ribbon_sample(local, input.region) * fade;
    }
    float across = input.uv.y * 2.0 - 1.0;
    float along = input.uv.x;
    float strand = input.material.z;
    float phase = along * 36.0 - input.timing.y * 8 + strand;
    float warp = across + 0.07 * sin(phase + sin(across * 7.0 + along * 21.0));
    float edge = saturate(1.0 - abs(warp));
    float grain = 0.5 + 0.5 * sin(warp * 8.0 + sin(phase * 0.7) * 2.0 + along * 13.0);
    float filament_phase = warp * 24.0 + sin(phase) * 2.0 + along * 11.0;
    float filament = pow(0.5 + 0.5 * sin(filament_phase), 7.0)
        / (1.0 + fwidth(filament_phase) * 2.0);
    float density = pow(edge, 0.75) * (0.58 + 0.24 * grain) + filament * pow(edge, 0.4) * 0.18;
    bool halo = input.material.x > 1.5;
    float alpha = (halo ? pow(edge, 3.0) * 0.13 : density * 0.90)
        * input.material.y * input.params.w * input.color.a;
    float rim = pow(saturate(1.0 - abs(abs(warp) - 0.70) * 9.0), 3.0);
    float3 color = input.color.rgb * (0.80 + grain * 0.20);
    color = lerp(color, sqrt(saturate(input.color.rgb)), saturate(rim * 0.8 + filament * 0.45));
    return float4(saturate(color) * saturate(alpha), saturate(alpha));
}