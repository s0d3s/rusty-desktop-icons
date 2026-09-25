---
myst: {
substitutions: {
showcase_gallery: '<table>
<tr>
<td width="50%" align="center"><strong>Movement</strong><br><a href="{{ github_assets_url }}/optimized/movement.gif"><img src="{{ github_assets_url }}/optimized/movement_small.gif" alt="Curve-driven icon movement without shaders" width="404"></a></td>
<td width="50%" align="center"><strong>Glitch</strong><br><a href="{{ github_assets_url }}/optimized/glitch.gif"><img src="{{ github_assets_url }}/optimized/glitch_small.gif" alt="Icons moving with the built-in glitch effect" width="404"></a></td>
</tr>
<tr>
<td width="50%" align="center"><strong>Particle Vortex</strong><br><a href="{{ github_assets_url }}/optimized/particle_vortex.gif"><img src="{{ github_assets_url }}/optimized/particle_vortex_small.gif" alt="Icon artwork dispersing into a particle vortex and reforming" width="404"></a></td>
<td width="50%" align="center"><strong>Dust Transfer</strong><br><a href="{{ github_assets_url }}/optimized/dust_transfer.gif"><img src="{{ github_assets_url }}/optimized/dust_transfer_small.gif" alt="Icons dissolving from one side and reassembling at their destinations" width="404"></a></td>
</tr>
<tr>
<td colspan="2" align="center"><strong>Silk Flow</strong><br><a href="{{ github_assets_url }}/optimized/silk_flow.gif"><img src="{{ github_assets_url }}/optimized/silk_flow_small.gif" alt="Icon colors flowing into translucent sheets and reforming" width="404"></a></td>
</tr>
</table>'
}
}
---

# Rusty Desktop Icons

Control and animate Windows desktop icons from Python or Rust. Read icon
positions and monitor geometry, move icons in batches, or prepare reversible
animations with curves and GPU effects.

Choose [Python](python/getting-started/installation.md) or
[Rust](rust/getting-started/installation.md) to install from the package registry.
Both use the same native engine.

| Task | Start Here |
| --- | --- |
| Save layouts or inspect monitors | [Desktop management](concepts/desktop.md) |
| Design timing and motion | [Curves](concepts/curves.md) and [animation](concepts/animation.md) |
| Keep a UI responsive | [Non-blocking execution](concepts/execution.md) |
| Scrub/reverse a preview | [Timelines](concepts/timelines.md) |
| Create images without moving icons | [Off-screen rendering](concepts/rendering.md) |
| Attach or author GPU effects | [Shaders](concepts/shaders/index.md) |

The [entity map](concepts/entities.md) is the index to public types. Language
guides contain recipes; concepts contain shared contracts and limitations.
This is a proof-of-concept library: test live writes on a saved layout, and use
off-screen rendering when a preview must leave desktop state untouched.

## Showcases

These looping previews are off-screen renders composed over a desktop mockup,
not screen recordings or FPS benchmarks.

{{ showcase_gallery | replace("{" ~ "{ github_assets_url }" ~ "}", github_assets_url) }}

```{toctree}
:maxdepth: 2
:caption: Library Documentation

concepts/index
demo-app
python/index
rust/index
contributing
GitHub Repository <https://github.com/s0d3s/rusty-desktop-icons>
```