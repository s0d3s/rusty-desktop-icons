# Shaders and Effects

A shader changes rendered artwork, not the real Shell icon. Compile a reusable
program, wrap it in an `Effect`, and attach it to an `IconAnimationSpec`.
Effects work in live animations, seekable timelines and off-screen scenes.

## Sources, Programs and Effects

`ShaderSource` holds optional vertex/pixel HLSL, a `ShaderPipeline` and optional
procedural execution recipe. Python `Shader.compile` returns a `Shader`; Rust
`shader::compile` returns a `ShaderProgram`. Compilation validates and compiles
source, while animation preparation creates device resources and artwork.
Reuse the compiled program across icons rather than compiling every frame.

`Effect` adds values, a seed, transparent padding and a strength envelope.
Python accepts a four-float `params` tuple or named `parameters` for a declared
recipe. Rust uses the program's four-value `default_params()` array and
`with_parameters` for named recipe values; procedural execution expands these
to 16 slots. Values must match the pipeline/schema; padding is 0..256 physical
pixels. Seeds must be finite; particle/procedural seeds also require absolute
value at most 65535.

The envelope is independent of X/Y motion. Sprite/particle Python defaults fade
in and out; procedural defaults use constant strength. Strength is clamped to
`[0, 1]`. Endpoints and zero-strength frames use the original artwork. A movement
hold does not pause the effect, and a stationary target can still have an effect
for the entire nonzero fixed duration.

## Built-ins

| `BuiltinShader` | Use |
| --- | --- |
| Identity | Baseline artwork rendering |
| Glitch | Displaced horizontal bands and RGB separation |
| ParticleVortex | Artwork fragments orbit and reform |
| DustTransfer | Side-first dissolution and destination assembly |
| SilkFlow | Artwork-colored translucent sheets with label-last reveal |

The built-in catalog is shared across Python and Rust. Dust Transfer and Silk
Flow control transport internally; constant-strength envelopes avoid blending
their motion back toward the ordinary moving sprite. They are stateless effects,
not fluid solvers or persistent particle simulations.

Silk Flow declares four named parameters:

| Name | Default | Valid range |
| --- | --- | --- |
| `strands` | 8 | Integer 2..16 |
| `spread` | 1 | 0..3 icon-width units |
| `folds` | 2 | 0.25..6 |
| `density` | 0.8 | 0.1..1 |

Particle Vortex and Dust Transfer use the four particle slots described in
[custom pipelines](custom.md). Sprite custom parameters are untyped finite
floats, so do not apply a built-in's visual meaning to an unrelated shader.

Enabled labels and badges are part of the input artwork. Procedural recipes
can request body and label regions separately. Destination grid reservations
do not confine shader pixels: effects can overlap neighbors and clip at edges.

(animation-presets)=
## Animation Presets

`AnimationPreset` is a platform-independent value type (`rdi_core::AnimationPreset`
in Rust). It contains `movement: Curve`, `envelope: Curve` and `duration: Duration`.
`AnimationPreset::default()` / Python `AnimationPreset()` returns ease-in-out
movement, smooth strength fades over the first/last 15%, and a fixed two seconds.
Python accepts optional keyword overrides for each field and exposes read-only
properties; Rust exposes owned public fields. Every lookup returns an independent
value, without compiling a shader or creating desktop resources.

On Windows, obtain shader recommendations with Rust
`BuiltinShader::DustTransfer.default_preset()` or Python
`AnimationPreset.builtin(BuiltinShader.DustTransfer)`. Python also accepts the
catalog name string. Built-in lookup raises `UnsupportedPlatform` elsewhere;
constructing and using the common preset value itself remains portable.

| Shader | Movement | Envelope | Duration |
| --- | --- | --- | --- |
| Identity | Ease-in-out | Smooth 15% fades | 2 seconds |
| Glitch | Smooth travel between 15% and 85%, holds outside | Smooth 15% fades | 2 seconds |
| ParticleVortex | Ease-in-out | Smooth 15% fades | 4 seconds |
| DustTransfer | Ease-in-out | Constant one | 4 seconds |
| SilkFlow | Ease-in-out | Constant one | 5 seconds |

Pass the values separately through the existing APIs. This Windows example
constructs a spec but does not start playback or move real icons:

```python
import rusty_desktop_icons as rdi

preset = rdi.AnimationPreset.builtin(rdi.BuiltinShader.DustTransfer)
shader = rdi.Shader.compile(rdi.ShaderSource.builtin(rdi.BuiltinShader.DustTransfer))
effect = rdi.Effect(shader, envelope=preset.envelope)
spec = rdi.IconAnimationSpec(
	id="example", target=(600, 300), duration=preset.duration,
	curve=preset.movement, effect=effect,
)
```

Reuse `shader` across icons. Substitute any field with your own curve/duration,
or pass the movement curve independently as `curve_x` and `curve_y`. Presets are
opt-in: existing constructors and showcases retain their defaults. The interactive
timeline editor applies them when selecting built-in shaders in Default Preset
mode; its duration slider and Custom curves remain independent afterward.
Movement does not retime shader progress or pause an effect during a hold.
Dust Transfer/Silk Flow own visual travel at full strength, but the movement
curve still determines the engine position used for stop-in-place. Constant
envelopes do not disable the engine's clean endpoint override.

Catalog contributors may add `default_preset: <expression>` after `pixel` in
`builtin_shaders!`. Omitting it returns `AnimationPreset::default()`. Keep
shader-specific recommendations in this catalog, not in the core renderer.

## Trusted Source Only

This is a proof-of-concept HLSL API, not a security sandbox. Expensive or malicious
code can stall the GPU, freeze the display or trigger a driver reset. Filesystem
includes are disabled, but that does not establish compiler/driver safety.
Review third-party or AI-generated source before compiling it. Shader interfaces
are not promised stable; the implemented backend is D3D11/HLSL, not WGSL.

## Put It to Work

- [Python shader recipes](../../python/guides/shaders.md)
- [Rust shader recipes](../../rust/guides/shaders.md)
- [Off-screen rendering](../rendering.md)

```{toctree}
:maxdepth: 1

custom
```