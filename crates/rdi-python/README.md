# Rusty Desktop Icons for Python

[![PyPI version](https://img.shields.io/pypi/v/rusty-desktop-icons)](https://pypi.org/project/rusty-desktop-icons/)
[![Latest GitHub release](https://img.shields.io/github/v/release/s0d3s/rusty-desktop-icons)](https://github.com/s0d3s/rusty-desktop-icons/releases/latest)
[![Documentation build status](https://img.shields.io/github/actions/workflow/status/s0d3s/rusty-desktop-icons/docs.yml?label=docs)](https://s0d3s.github.io/rusty-desktop-icons/)
![Windows](https://img.shields.io/badge/platform-Windows-0078D4)
![Python 3.9+](https://img.shields.io/badge/Python-3.9%2B-3776AB?logo=python&logoColor=white)
![Proof of concept](https://img.shields.io/badge/status-proof%20of%20concept-yellow)
[![Apache 2.0 license](https://img.shields.io/badge/license-Apache%202.0-blue)](https://github.com/s0d3s/rusty-desktop-icons/blob/main/LICENSE)

A Rust-powered desktop icon controller and GPU-backed animator, with Python
bindings built using [PyO3](https://pyo3.rs/) and [maturin](https://maturin.rs/).
Inspect and restore desktop layouts, animate transitions, and apply built-in
or custom shader effects.

**[Main GitHub repository](https://github.com/s0d3s/rusty-desktop-icons)** |
**[Documentation](https://s0d3s.github.io/rusty-desktop-icons/)** |
**[Python guide](https://s0d3s.github.io/rusty-desktop-icons/python/)** |
**[Report an issue](https://github.com/s0d3s/rusty-desktop-icons/issues)**

<p align="center">
  <a href="https://s0d3s.github.io/rusty-desktop-icons/"><img src="https://raw.githubusercontent.com/s0d3s/rusty-desktop-icons/assets_storage/assets_storage/optimized/silk_flow_solo.webp" alt="A desktop icon flowing into translucent silk and reforming" height="140" width="960"></a>
</p>

## Features

- Enumerate desktop icons, positions, monitors, DPI and grid information.
- Move icons directly and read or change Windows desktop folder flags.
- Animate with configurable curves, independent axis motion and reversible timelines.
- Apply Glitch, Particle Vortex, Dust Transfer and Silk Flow effects, or trusted custom HLSL.
- Render scenes off-screen and export frames without moving real icons.

## Showcases

These looping previews are off-screen renders over a desktop mockup, not
screen recordings or FPS benchmarks. Click a preview to open its full-size version.

<table>
  <tr>
    <td width="50%" align="center"><strong>Movement</strong><br><a href="https://raw.githubusercontent.com/s0d3s/rusty-desktop-icons/assets_storage/assets_storage/optimized/movement.gif"><img src="https://raw.githubusercontent.com/s0d3s/rusty-desktop-icons/assets_storage/assets_storage/optimized/movement_small.gif" alt="Curve-driven icon movement without shaders" width="404"></a></td>
    <td width="50%" align="center"><strong>Glitch</strong><br><a href="https://raw.githubusercontent.com/s0d3s/rusty-desktop-icons/assets_storage/assets_storage/optimized/glitch.gif"><img src="https://raw.githubusercontent.com/s0d3s/rusty-desktop-icons/assets_storage/assets_storage/optimized/glitch_small.gif" alt="Icons moving with the built-in glitch effect" width="404"></a></td>
  </tr>
  <tr>
    <td width="50%" align="center"><strong>Particle Vortex</strong><br><a href="https://raw.githubusercontent.com/s0d3s/rusty-desktop-icons/assets_storage/assets_storage/optimized/particle_vortex.gif"><img src="https://raw.githubusercontent.com/s0d3s/rusty-desktop-icons/assets_storage/assets_storage/optimized/particle_vortex_small.gif" alt="Icon artwork dispersing into a particle vortex and reforming" width="404"></a></td>
    <td width="50%" align="center"><strong>Dust Transfer</strong><br><a href="https://raw.githubusercontent.com/s0d3s/rusty-desktop-icons/assets_storage/assets_storage/optimized/dust_transfer.gif"><img src="https://raw.githubusercontent.com/s0d3s/rusty-desktop-icons/assets_storage/assets_storage/optimized/dust_transfer_small.gif" alt="Icons dissolving from one side and reassembling at their destinations" width="404"></a></td>
  </tr>
  <tr>
    <td colspan="2" align="center"><strong>Silk Flow</strong><br><a href="https://raw.githubusercontent.com/s0d3s/rusty-desktop-icons/assets_storage/assets_storage/optimized/silk_flow.gif"><img src="https://raw.githubusercontent.com/s0d3s/rusty-desktop-icons/assets_storage/assets_storage/optimized/silk_flow_small.gif" alt="Icon colors flowing into translucent sheets and reforming" width="404"></a></td>
  </tr>
</table>

<p align="center">
  <a href="https://raw.githubusercontent.com/s0d3s/rusty-desktop-icons/assets_storage/assets_storage/optimized/effects_comparison.webp"><img src="https://raw.githubusercontent.com/s0d3s/rusty-desktop-icons/assets_storage/assets_storage/optimized/effects_comparison.webp" alt="Movement, glitch, dust transfer and particle vortex side by side" width="616"></a>
</p>

See the [showcase export guide](https://s0d3s.github.io/rusty-desktop-icons/python/examples/showcases/)
to render your own scenes.

## Install (from PyPI)

Use **CPython 3.9+ on Windows 10/11 x64** with a compatible wheel.
Prebuilt wheels do not require Rust.

```powershell
python -m pip install rusty-desktop-icons
```

## Install (from source)

Install [Rust 1.88+](https://rustup.rs/) and Visual Studio C++ Build Tools with
the Windows SDK, then clone the repository:

```powershell
git clone https://github.com/s0d3s/rusty-desktop-icons.git
cd rusty-desktop-icons
python -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install maturin==1.15.0
python scripts/generate-stubs.py
maturin develop --release -m crates/rdi-python/Cargo.toml
```

See the [Python installation guide](https://s0d3s.github.io/rusty-desktop-icons/python/getting-started/installation/)
for build and troubleshooting details.

## Example: Dust Transfer Round Trip

Turn two icons into drifting particles, swap their positions, then animate
them home. Change `DustTransfer` to `SilkFlow`, `ParticleVortex` or `Glitch`
to try another effect with its recommended movement and timing.

**This changes your real desktop.** Disable **View > Auto arrange icons** and
start with a non-overlapping layout. The example selects the first two icons;
use two on the same monitor, and do not rearrange icons or change display
settings while it runs. Cleanup restores saved positions and the snap setting
after normal completion or ordinary errors, but cannot survive force-killing
the process.

```python
from __future__ import annotations

import rusty_desktop_icons as rdi


def main() -> None:
  controller = rdi.DesktopController()
  original_flags = controller.get_flags()
  if original_flags & rdi.FolderFlag.FWF_AUTOARRANGE:
    raise RuntimeError("Disable desktop Auto arrange before running")
  icons = controller.list_icons()[:2]
  if len(icons) < 2:
    print("This example needs at least two desktop icons")
    return

  origins = [(icon.id, icon.position) for icon in icons]
  targets = [(icons[0].id, icons[1].position),
         (icons[1].id, icons[0].position)]
  name = rdi.BuiltinShader.DustTransfer
  preset = rdi.AnimationPreset.builtin(name)
  shader = rdi.Shader.compile(rdi.ShaderSource.builtin(name))
  effect = rdi.Effect(shader, envelope=preset.envelope)

  def play(positions: list[tuple[str, tuple[int, int]]]) -> None:
    specs = [
      rdi.IconAnimationSpec(
        icon_id, target, preset.duration, preset.movement, effect=effect
      )
      for icon_id, target in positions
    ]
    handle = controller.prepare(specs).start()
    try:
      reason = handle.wait()
      if reason.kind != rdi.FinishReasonKind.Completed:
        raise RuntimeError(str(reason))
      commit = handle.final_commit()
      if commit is not None and commit.missing_ids:
        raise RuntimeError(f"Unresolved icons: {commit.missing_ids}")
    finally:
      handle.stop()
      handle.wait()

  snap = rdi.FolderFlag.FWF_SNAPTOGRID
  try:
    controller.unset_flags(snap)
    play(targets)
    play(origins)
  finally:
    try:
      missing = controller.set_positions(origins)
      if missing:
        raise RuntimeError(f"Could not restore icons: {missing}")
    finally:
      controller.apply_flags(snap, original_flags)


if __name__ == "__main__":
  main()
```

## Demo App

**`animate_timeline`** is a separate Rust terminal app for exploring animation
without writing Python. Its **Main**, **Curves** and **Shader Editor** tabs
provide reversible playback, timeline seeking, movement/envelope editing and
built-in or custom HLSL shaders. It is not installed by `pip`.

[![Demo App Main tab with playback and duration controls](https://raw.githubusercontent.com/s0d3s/rusty-desktop-icons/assets_storage/assets_storage/animate_timeline_screenshots/animate_timeline_0_main.png)](https://s0d3s.github.io/rusty-desktop-icons/demo-app/)

Download `animate_timeline.exe` from the **Assets** section of the
[latest GitHub release](https://github.com/s0d3s/rusty-desktop-icons/releases/latest).
From PowerShell, in the download directory:

```powershell
.\animate_timeline.exe --shader dust-transfer
```

Or build it with the Rust/Windows prerequisites listed above:

```powershell
cargo install rusty-desktop-icons --bin animate_timeline
animate_timeline
```

Disable **View > Auto arrange icons** before launching. The demo moves real
icons; press `q` to quit normally and restore their original positions. Avoid
force-killing it. See the [Demo App guide](https://s0d3s.github.io/rusty-desktop-icons/demo-app/)
for controls, installation details and limitations.

## Examples in this crate

- [Smoke example](https://github.com/s0d3s/rusty-desktop-icons/blob/main/crates/rdi-python/examples/smoke.py) — end-to-end sanity check.
- [Function-sampled curves](https://github.com/s0d3s/rusty-desktop-icons/blob/main/crates/rdi-python/examples/curve_from_function.py) —
  demonstrates `Curve.from_function` with a damped-cosine curve.
- [Motion-aware curves](https://github.com/s0d3s/rusty-desktop-icons/blob/main/crates/rdi-python/examples/curve_from_motion.py) —
  demonstrates `Curve.from_motion_function` with a parametric gravity
  curve.

More recipes: [layout and playback](https://s0d3s.github.io/rusty-desktop-icons/python/guides/basic-usage/),
[timelines](https://s0d3s.github.io/rusty-desktop-icons/python/guides/timelines/),
and [shaders](https://s0d3s.github.io/rusty-desktop-icons/python/guides/shaders/).

## Status and limitations

This is a **proof of concept**, with no stable API promise. Windows is the
only working desktop backend; multi-monitor and mixed-DPI behavior remain
work in progress. Thumbnail, shortcut-arrow and label rendering can differ
from Explorer. Only one overlay session can run per process.

**Use trusted shaders only.** Custom HLSL is not sandboxed: expensive or
malicious shaders can freeze the display or reset the GPU driver. Review
third-party and AI-generated shader sources before compiling them.

## Tests

```powershell
.venv\Scripts\python.exe -m pytest crates\rdi-python\tests -v
```

## License

Licensed under the [Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0).
The license text is included in the distribution.
