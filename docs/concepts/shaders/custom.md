# Custom HLSL and Render Pipelines

Custom shaders run in the same renderer as built-ins. You supply stage functions
and, optionally, a bounded procedural recipe. You do not create a device, upload
textures, bind arbitrary resources, or install a new backend.

Start with [shader/effect ownership and safety](index.md). For executable source
construction, see [Python recipes](../../python/guides/shaders.md) and
[Rust recipes](../../rust/guides/shaders.md).

## Source Contract

The library supplies resource declarations, `Instance`, `VertexOutput`,
`default_vertex` and `default_pixel`. Define only the stage functions you replace:

```hlsl
VertexOutput vertex(Instance instance, uint vertex_id : SV_VertexID) {
    VertexOutput output = default_vertex(instance, vertex_id);
    output.position.x += 12.0 * instance.timing.z * 2.0 / dimensions.x;
    return output;
}

float4 pixel(VertexOutput input) : SV_Target {
    float4 color = default_pixel(input);
    color.rgb *= float3(0.5, 1.0, 0.8);
    return color;
}
```

Vertex and pixel sources are compiled separately as Shader Model 5.0 (`vs_5_0`
and `ps_5_0`). A helper defined only in vertex source is not visible in pixel
source. Both stages always execute; omitting either source selects that
pipeline's default. An empty string is not the same as an omitted stage.
Do not redeclare library structs/resources or use your own `main` entry point.
Typed wrapper declarations reject incompatible function signatures.

A string passed directly to `compile` is Sprite pixel-only shorthand. To use
Particles or Procedural, set the pipeline explicitly. When modifying a built-in,
retain its pipeline and execution recipe, not only one HLSL string.

## Shared Coordinates and Data

| Field/resource | Meaning |
| --- | --- |
| `instance.rect` | Moving padded cell origin and size in physical pixels |
| `instance.region` | Complete artwork atlas origin and cell size |
| `timing.x` | Elapsed/virtual seconds |
| `timing.y` | Normalized movement time |
| `timing.z` | Clamped effect strength |
| `timing.w` | Per-icon seed |
| `params` | Four caller-supplied values |
| `dimensions.xy` | Current render dimensions |
| `dimensions.zw` | Artwork atlas dimensions |
| `offset.xy` | Translation applied when mapping scene coordinates |

All listed instance fields are `float4`. Sprite has `rect`, `region`, `timing`
and `params`. Particles adds `geometry` and `travel`; Procedural also adds
`body` and `label`. Regions contain atlas X/Y and width/height in pixels, not UVs.

`VertexOutput` is pipeline-specific. Do not redeclare it; initialize it through
`default_vertex`, then modify the fields your stages need:

| Pipeline | Available output fields |
| --- | --- |
| All | `float4 position`, `float2 uv`, flat `float4 region`, `timing`, `params` |
| Particles | Flat `float4 tile` (local origin/size), flat `float2 dust` |
| Procedural | Flat `float4 material`, interpolated `float4 color`, flat `float4 geometry` |

Flat means HLSL `nointerpolation`. Particle defaults initialize `dust` to zero;
procedural defaults initialize `material` to zero and `color` to one. Custom
stages can use these varyings to communicate, but default pixels do not
automatically apply a custom `color`/`material`. `body`, `label` and `travel` are
vertex inputs, not automatically forwarded pixel inputs.

Output `position` is clip space, not desktop pixels. Start from `default_vertex`
to initialize the pipeline-specific varyings. Pixel coordinates convert to
clip coordinates with the current dimensions; positive pixel Y points downward.
Custom vertices must implement their own strength-dependent displacement.

Artwork samples and shader output are **premultiplied RGBA** in HLSL. Multiply
RGB and alpha together when changing opacity. Returning bright RGB at alpha
zero is invalid and produces compositing artifacts. Readback bytes use BGRA
channel order; that is a separate [frame conversion concern](../rendering.md).

## Sprite

Sprite draws six vertices (two triangles) per artwork cell. It is suitable for
tint, color separation, UV distortion, scaling, rotation and simple geometric
deformation. Custom vertex code cannot increase the vertex count in this pipeline.

`input.uv` is normalized over the padded cell. `sample_icon(uv, input)` reads
premultiplied artwork and returns transparent outside that cell. `default_pixel`
samples without distortion. The pixel wrapper blends original/effect pixels
using strength; it does not undo custom vertex displacement. Increase
`padding_px` for local UV displacement, but keep atlas and allocation limits in
mind. Geometry/effect pixels can clip at the canvas edge.

Sprite accepts four finite parameters without a fixed schema. Its default tuple
`(8, 3, 6, 30)` serves the Glitch preset; custom Sprite source defines its own
meaning. Neither `parameter(index)` nor intermediate pass inputs exist here.

## Particles

Particles generates a bounded grid of quads from vertex IDs and artwork samples.
The default geometry can break an icon into colored pieces and gather them
again; custom stages can choose different trajectories and fragment shading.
The cap is 64x64 particles per padded cell, so large artwork can use coarser tiles.

The four parameters are cell size (1..32 pixels), radius (0..3 icon widths),
turns (-8..8), and dust size (0.1..2); defaults are `(3, 1.5, 2, 0.65)`.
`geometry.xy` locates the icon center within its padded cell; `.zw` is icon size.
`travel.xy` is source minus current position and `.zw` is target minus current,
so a custom vertex can construct endpoint-relative transport. These inputs are
also present in the procedural pipeline.

The particle pixel input carries interpolated UV/color and pipeline-defined
shape data. Use its `default_pixel` helper as the starting point; do not copy
the Sprite `VertexOutput` declaration into particle source. Pause and reverse
must be functions of virtual time/seed, not accumulated simulation state.

## Procedural Recipes

Procedural is the extension point for custom topology, named parameters,
separate artwork layers and multi-pass compositing. The descriptor types map as
follows; these are source descriptions until compilation/preparation.

| Python | Rust | Responsibility |
| --- | --- | --- |
| `ProceduralSource` | `shader::ExecutionSource` | Parameters, targets, ordered passes, artwork requests |
| `EffectParameter` | `EffectParameter` / `ParameterKind` | Name, kind, finite min/max/default |
| `RenderTarget` | `EffectTarget` | Intermediate width/height |
| `RenderPass` | `shader::PassSource` | Stage source, draw, input/output targets, blend |
| `RenderPass` draw fields | `DrawSpec` / `DrawTopology` | Vertex-count formula and primitive topology |
| Blend string | `PassBlend` | Over, Add or Replace |
| Compiled recipe | `EffectExecution` / `EffectPass` | Validated stages and resource plan |

Python descriptor arguments and defaults:

| Constructor | Arguments |
| --- | --- |
| `EffectParameter` | `name, default, min=..., max=..., kind="float"`; bounds are required keywords |
| `RenderTarget` | `width, height` in pixels |
| `RenderPass` | Keyword-only `vertex=None`, `pixel=None`, `vertices=6`, `count_parameter=None`, `multiplier=0`, `topology="triangles"`, `inputs=None`, `output=None`, `blend="over"` |
| `ProceduralSource` | `passes`, keyword-only `parameters=None`, `targets=None`, `body_artwork=False`, `label_artwork=False` |

Parameter kinds are `"float"`, `"integer"`, `"boolean"`; topologies are
`"triangles"`, `"triangle-strip"`, `"lines"`; blends are `"over"`, `"add"`,
`"replace"`. Python's `count_parameter` is a declared parameter **name**;
Rust's `DrawSpec.parameter` is its zero-based **index**. `inputs` and `output`
are zero-based target indices in both languages. Omitted inputs mean none;
omitted output means the scene. Pass-local stages override top-level stages,
which override pipeline defaults. Omitting the entire execution recipe creates
one default procedural pass, not the Silk Flow preset.

Parameters are float, integer or boolean (numeric 0/1). Names are unique,
nonempty and contain ASCII letters, digits or underscores. They occupy slots in
declaration order. HLSL `parameter(index)` reads one of up to 16 values from
`b1`; no uniform named after the parameter is generated. Python named overrides
use `Effect(parameters=...)`; Rust uses `ShaderProgram::with_parameters` before
constructing the effect. Unknown names and invalid values are rejected.

`vertices + multiplier * count_parameter` determines a draw's count. A count
parameter must have nonnegative integer bounds. Counts must be 1..65536; triangles
need multiples of three, lines multiples of two, triangle strips at least three.
Validation checks the permitted count range, not only the default value.

Request `body_artwork` / `label_artwork` only when needed. Complete artwork is
always available; optional `instance.body` and `instance.label` identify separate
atlas regions. Use `sample_artwork(local_pixels, region)` to read a region.
In vertex code, explicit-LOD sampling is required; the helper uses `SampleLevel`.
This enables independent body breakup and later label reveal without reuploading
artwork each frame. Badge content remains part of body artwork.

## Pass Graph and Resource Bindings

Declare intermediate targets, then passes in execution order. Each target has
exactly one producer; inputs may reference only targets written by earlier
passes. A pass cannot read its own output. Every declared target must be
produced, and only the last pass may write to the scene (`output=None`).
Targets are cleared per icon/frame: there is no persistent feedback buffer,
compute stage, cross-frame simulation state or access to the desktop behind it.

| Binding | Ownership |
| --- | --- |
| `t0` | Prepared artwork atlas |
| `t1`..`t4` | `pass_inputs[0]`..`pass_inputs[3]` in the pass's declared input order |
| `s0` | `artwork_sampler` |
| `b0` | Canvas dimensions and offset |
| `b1` | Procedural parameter values |

For example, `inputs=[2]` binds target 2 at `pass_inputs[0]`, not slot 2.
Intermediate passes render to their target dimensions; the final pass composites
to the scene. Over uses premultiplied alpha compositing; Add accumulates color;
Replace overwrites. Choose source coordinates appropriate to the current pass,
and use normalized UVs when sampling a full intermediate texture.

All resources/programs are prepared before playback. Limits are 16 parameters,
four intermediate targets, eight passes, 64 MiB of target storage per recipe,
and 256 MiB of prepared intermediate targets per session. Target axes are at
most 8192. Combined top-level stage source and combined pass source each have
a 1 MiB limit. Artwork
packing also has 8192-pixel atlas-axis limits. These are validation bounds, not
promises of a usable frame rate at maximum settings.

## Validation Workflow

Compile trusted source before moving icons. Render an off-screen scene at zero,
several intermediate times, the endpoint, then an earlier time again. Check
clean endpoints, premultiplied alpha, padding/edge clipping and repeatable
rewind. Test with labels/badges and more than one icon. Then test live playback
separately: export throughput is not displayed FPS.