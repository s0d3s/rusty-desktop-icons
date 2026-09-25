
VertexOutput vertex(Instance instance, uint vertex_id : SV_VertexID) {
    float2 corners[6] = {float2(0,0), float2(1,0), float2(0,1),
                         float2(0,1), float2(1,0), float2(1,1)};
    uint particle = vertex_id / 6;
    uint2 grid = (uint2)clamp(ceil(instance.region.zw / instance.params.x), 1, 64);
    float2 cell = instance.region.zw / grid;
    float2 tile = float2(particle % grid.x, particle / grid.x) * cell;
    float2 source = tile + cell * 0.5;
    float progress = instance.timing.y;
    float phase = smoothstep(0.0, 0.2, progress) * (1.0 - smoothstep(0.7, 1.0, progress));
    float amount = min(phase, instance.timing.z);
    float random = particle_hash(particle + instance.timing.w * 17.0);
    float second = particle_hash(particle * 3.0 + instance.timing.w * 29.0);
    float angle = random * 6.2831853 + progress * instance.params.z * 6.2831853
        * (0.7 + second * 0.6);
    float radius = sqrt(second) * instance.geometry.z * instance.params.y;
    radius *= 0.85 + 0.15 * sin(progress * 12.566371 + random * 6.2831853);
    float2 orbit = instance.geometry.xy + float2(cos(angle), sin(angle) * 0.75) * radius;
    float2 center = lerp(source, orbit, amount);
    float diameter = max(1.0, min(cell.x, cell.y) * instance.params.w);
    float2 extent = lerp(cell, float2(diameter, diameter), amount);
    VertexOutput output;
    output.uv = corners[vertex_id % 6];
    float2 pixel = instance.rect.xy + center + (output.uv - 0.5) * extent + offset.xy;
    output.position = float4(pixel.x / dimensions.x * 2 - 1,
                            1 - pixel.y / dimensions.y * 2, 0, 1);
    output.region = instance.region;
    output.tile = float4(tile, cell);
    output.dust = float2(amount, 0);
    output.timing = instance.timing;
    output.params = instance.params;
    return output;
}
