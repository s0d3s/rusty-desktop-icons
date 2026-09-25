# Build and Reuse Effects

These recipes construct effects without moving icons. Attach the returned effect
to an `IconAnimationSpec`, then use the [off-screen example](../examples/showcases.md)
before live playback. See [shader concepts](../../concepts/shaders/index.md) and
the [custom HLSL contract](../../concepts/shaders/custom.md) for pipeline details.

## Compile a Built-in Once

```python
import rusty_desktop_icons as rdi

shader = rdi.Shader.compile(rdi.ShaderSource.builtin(rdi.BuiltinShader.SilkFlow))
effect = rdi.Effect(
    shader, parameters={"strands": 8, "spread": 1.0, "folds": 2.0, "density": 0.8},
    seed=7.0,
)
```

Reuse `shader` across effects with different seeds/parameters. The catalog also
provides Identity, Glitch, ParticleVortex and DustTransfer.

## Tint the Artwork

```python
import rusty_desktop_icons as rdi

source = rdi.ShaderSource(pixel="""
float4 pixel(VertexOutput input) : SV_Target {
    float4 color = default_pixel(input);
    color.rgb *= float3(0.4, 1.0, 0.7);
    return color;
}
""")
effect = rdi.Effect(rdi.Shader.compile(source), padding_px=16)
```

The Sprite wrapper fades the pixel effect using its envelope. To displace the
geometry too, supply a `vertex` string with the signature in the custom contract.

## Add a Named Procedural Parameter

```python
import rusty_desktop_icons as rdi

recipe = rdi.ProceduralSource(
    parameters=[rdi.EffectParameter("green", 0.8, min=0.0, max=1.0)],
    passes=[rdi.RenderPass(pixel="""
float4 pixel(VertexOutput input) : SV_Target {
    float4 color = default_pixel(input);
    color.rgb *= float3(0.5, parameter(0), 0.8);
    return color;
}
""")],
)
shader = rdi.Shader.compile(rdi.ShaderSource(
    pipeline=rdi.ShaderPipeline.Procedural, execution=recipe,
))
effect = rdi.Effect(shader, parameters={"green": 1.0})
```

## Render Through an Intermediate Target

```python
import rusty_desktop_icons as rdi

fullscreen = """
VertexOutput vertex(Instance instance, uint vertex_id : SV_VertexID) {
    VertexOutput output = default_vertex(instance, vertex_id);
    output.position = float4(output.uv.x * 2 - 1, 1 - output.uv.y * 2, 0, 1);
    return output;
}
"""
composite = """
float4 pixel(VertexOutput input) : SV_Target {
    return pass_inputs[0].Sample(artwork_sampler, input.uv);
}
"""
recipe = rdi.ProceduralSource(
    targets=[rdi.RenderTarget(128, 128)],
    passes=[
        rdi.RenderPass(vertex=fullscreen, output=0, blend="replace"),
        rdi.RenderPass(pixel=composite, inputs=[0]),
    ],
)
shader = rdi.Shader.compile(rdi.ShaderSource(
    pipeline=rdi.ShaderPipeline.Procedural, execution=recipe,
))
effect = rdi.Effect(shader)
```

The first pass maps the complete artwork onto the target; the second maps that
target onto the icon's scene rectangle. This is a resampling example, not a
blur or feedback simulation. Add processing in the first/second pixel stage
within the [pass-graph limits](../../concepts/shaders/custom.md).