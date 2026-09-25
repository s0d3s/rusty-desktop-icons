
VertexOutput vertex(Instance instance, uint vertex_id : SV_VertexID) {
    VertexOutput output = default_vertex(instance, vertex_id);
    uint particle = vertex_id / 6;
    float2 cell = output.tile.zw;
    float2 source = output.tile.xy + cell * 0.5;
    float seed = instance.timing.w;
    float random = particle_hash(particle + seed * 17.0);
    float second = particle_hash(particle * 3.0 + seed * 29.0);
    uint side = (uint)(particle_hash(seed * 7.0 + 13.0) * 4.0);
    float2 direction = side == 0 ? float2(1, 0) : side == 1 ? float2(-1, 0)
        : side == 2 ? float2(0, 1) : float2(0, -1);
    float order = saturate(0.5 + dot(source / instance.region.zw - 0.5, direction)
        + (random - 0.5) * 0.08);
    float release = 0.02 + order * 0.26;
    float arrival = 0.72 + order * 0.26;
    float progress = instance.timing.y;
    float flight = smoothstep(release, arrival, progress);
    float dust = smoothstep(release, release + 0.10, progress)
        * (1.0 - smoothstep(arrival - 0.10, arrival, progress));
    float2 displacement = instance.travel.zw - instance.travel.xy;
    float distance = length(displacement);
    float2 forward = distance > 0.001 ? displacement / distance : direction;
    float2 lateral = float2(-forward.y, forward.x);
    float arch = sin(flight * 3.14159265);
    arch *= arch;
    float phase = random * 6.2831853;
    float wave = flight * instance.params.z * 6.2831853;
    float spread = instance.geometry.z * instance.params.y * 0.35;
    float2 turbulence = lateral * ((second - 0.5) * 1.5
        + 0.35 * sin(wave + phase) + 0.15 * sin(wave * 1.7 + phase * 2.0))
        + forward * (0.25 * sin(wave * 0.7 + phase));
    float strength = saturate(instance.timing.z);
    precise float2 source_origin = instance.rect.xy + instance.travel.xy;
    float2 transport = displacement * flight + turbulence * spread * arch;
    float amount = dust * strength;
    float diameter = max(1.0, min(cell.x, cell.y) * instance.params.w * (0.65 + second * 0.35));
    float2 extent = lerp(cell, float2(diameter, diameter), amount);
    float2 pixel = lerp(instance.rect.xy, source_origin, strength) + source + transport * strength
        + (output.uv - 0.5) * extent + offset.xy;
    output.position = float4(pixel.x / dimensions.x * 2 - 1,
                            1 - pixel.y / dimensions.y * 2, 0, 1);
    output.dust = float2(amount, 0);
    return output;
}
