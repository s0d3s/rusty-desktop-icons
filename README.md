<div align="center">

<img src="https://raw.githubusercontent.com/s0d3s/rusty-desktop-icons/assets_storage/assets_storage/optimized/project_intro.gif" alt="Desktop icons gather into the Rusty Desktop Icons project name" height="450" width="800">

<h1>Rusty Desktop Icons</h1>

<p>
  <a href="https://github.com/s0d3s/rusty-desktop-icons/releases/latest"><img src="https://img.shields.io/github/v/release/s0d3s/rusty-desktop-icons" alt="Latest GitHub release"></a>
  <a href="https://pypi.org/project/rusty-desktop-icons/"><img src="https://img.shields.io/pypi/v/rusty-desktop-icons" alt="PyPI version"></a>
  <a href="https://crates.io/crates/rusty-desktop-icons"><img src="https://img.shields.io/crates/v/rusty-desktop-icons" alt="crates.io version"></a>
  <a href="https://s0d3s.github.io/rusty-desktop-icons/"><img src="https://img.shields.io/github/actions/workflow/status/s0d3s/rusty-desktop-icons/docs.yml?label=docs" alt="Documentation build status"></a>
</p>

<p>
  <img src="https://img.shields.io/badge/status-proof%20of%20concept-yellow" alt="Status: proof of concept">
  <img src="https://img.shields.io/badge/platform-Windows-0078D4" alt="Platform: Windows">
  <img src="https://img.shields.io/badge/Rust-1.88%2B-black?logo=rust" alt="Rust 1.88+">
  <img src="https://img.shields.io/badge/Python-3.9%2B-3776AB?logo=python&amp;logoColor=white" alt="Python 3.9+">
  <a href="#license"><img src="https://img.shields.io/badge/license-Apache%202.0-blue" alt="License: Apache 2.0"></a>
</p>

<p>A Rust-powered desktop icon controller and GPU-backed animator, with Python bindings.</p>

<p>Keep your carefully arranged desktop intact, and make moving between layouts worth watching.</p>

</div>

## Table of Contents

- [Prologue](#prologue)
- [Demo App](#demo-app)
- [Getting Started (Python)](#getting-started-python)
- [Getting Started (Rust)](#getting-started-rust)
- [Features & Showcases](#features--showcases)     <-------\[LOOK AT ME]
- [Status & Limitations](#status--limitations)
- [About Curves and Animation](#about-curves-and-animation)
- [Positioner & MCP Server](#positioner--mcp-server)
- [License](#license)

<div align="center">
  <a href="https://s0d3s.github.io/rusty-desktop-icons/">[DOCUMENTATION]</a>
  <br/>
  <img src="https://raw.githubusercontent.com/s0d3s/rusty-desktop-icons/assets_storage/assets_storage/optimized/silk_flow_solo.webp" height="140" width="960">
</div>

## Prologue

<details>
<summary><strong>Some history & project background</strong></summary>
This started as a small C library I wrote for
[Positioner](https://github.com/s0d3s/Positioner): manage desktop icon positions
and animate transitions between layouts. Originally, every animation frame
simply moved the real desktop icons through the Windows API.

Then I moved to Windows 11. In my testing, Explorer would only reflect those
updates about **300 ms after the last change**, not the first. Continuous
updates could keep postponing the repaint. On Windows 10, it had been around
10 ms. Apparently, "Windows 10" was also the latency specification. Marketing
forgot to mention that part.

Even a charitable three frames per second is not much of an animation. The
solution was to separate **real desktop manipulation** from **visual
emulation**: let the Shell own the actual icon positions, and animate their
copies in a transparent overlay. That split also gave the Rust rewrite a
platform-independent core and room for future backends.

My main reason for building all this remains quite ordinary: I want my perfect
icon layout to survive a Windows update or restart. The effects are a pleasant
bonus. I hope someone else finds it useful too.
</details>

## Demo App

The Rust `animate_timeline` app is an interactive terminal playground for
movement, curves, and built-in shaders. It can be installed from source or downloaded as binary from release page.

| Main | Curves | Shader Editor |
| :---: | :---: | :---: |
| [![animate_timeline Main tab with playback and duration controls][timeline-main]][timeline-main] | [![animate_timeline Curves tab with keyframe graph and JSON editor][timeline-curves]][timeline-curves] | [![animate_timeline Shader Editor tab with HLSL source][timeline-shader-editor]][timeline-shader-editor] |

[timeline-main]: https://raw.githubusercontent.com/s0d3s/rusty-desktop-icons/assets_storage/assets_storage/animate_timeline_screenshots/animate_timeline_0_main.png
[timeline-curves]: https://raw.githubusercontent.com/s0d3s/rusty-desktop-icons/assets_storage/assets_storage/animate_timeline_screenshots/animate_timeline_1_curves.png
[timeline-shader-editor]: https://raw.githubusercontent.com/s0d3s/rusty-desktop-icons/assets_storage/assets_storage/animate_timeline_screenshots/animate_timeline_2_shader_editor.png

> [!NOTE]
> To better test result -> Disable **View > Auto arrange icons** in the desktop context menu first(Right Mouse Click on the Desktop)

<details>
<summary><strong>Via crates.io or build</strong></summary>

Option A:
- build directly from crate.io
  ```bash
  cargo install rusty-desktop-icons --bin animate_timeline
  animate_timeline
  ```


Option B:
- download and build from github source and launch(run from PowerShell of CMD):
  ```powershell
  git clone https://github.com/s0d3s/rusty-desktop-icons.git
  cd rusty-desktop-icons
  cargo install --path . --bin animate_timeline
  animate_timeline
  ```


Build requirenments:
- **Windows 10/11 x64**(with an
interactive desktop)
- [Rust 1.88+](https://rustup.rs/)
- Visual Studio C++
- Build Tools with the Windows SDK


</details>

<details>
<summary><strong>Via binary from release</strong></summary>

1. Open the [latest release](https://github.com/s0d3s/rusty-desktop-icons/releases/latest).
2. Under **Assets**, download `animate_timeline.exe` to your **Downloads**(for example) folder.
3. Open PowerShell and run:

  ```powershell
  cd "$HOME\Downloads"
  .\animate_timeline.exe
  ```

</details>

Playback starts at zero and runs to halfway. Use the timeline slider to seek,
`f` to play forward, `r` to reverse, `s` to choose a shader, and `q` to quit
and restore the original positions. The duration slider controls traversal
time. Try `--no-shader`, `--shader glitch`, `--shader particle-vortex`, or
`--shader dust-transfer`.

Use `Tab`, `Shift+Tab`, or the mouse to switch between **Main**, **Curves**
and **Shader Editor** (minimum terminal size: 42 columns by 24 rows).
Curves has one Movement/Envelope track at a time, grouped graph markers and
editable keyframe JSON. Shader Editor accepts trusted HLSL and full procedural
recipes. There is one custom curve pair and one custom shader per run; edits
replace them. Built-in shader changes apply their preset duration, after which
the duration slider remains independent. See the
[editor controls](https://s0d3s.github.io/rusty-desktop-icons/demo-app/).

This is a live desktop demo, not a read-only preview: the real icons are moved
while hidden behind the overlay(but in default mode they are not shown). Normal exit restores their origins; avoid
force-killing the process.

## Getting Started (Python)

### Installation

Use **CPython 3.9+ on Windows x64** with a compatible wheel:

```powershell
python -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install rusty-desktop-icons
```

No Rust installation is needed for a prebuilt wheel. If your Python version
has no published wheel, install the Rust/Windows build prerequisites above,
clone this repository, and run these commands from its root in an activated
virtual environment:

```powershell
python -m pip install maturin==1.15.0
python scripts/generate-stubs.py
maturin develop --release -m crates/rdi-python/Cargo.toml
```

See the [Python installation guide](docs/python/getting-started/installation.md).

### Example

<a id="walkthrough-sequence"></a>

The Python and Rust walkthroughs below perform the same sequence:

1. Print icons, desktop bounds, monitors, DPI, and grid information.
2. Move up to three icons one column right, directly, without an overlay.
3. Unset Explorer's `FWF_SNAPTOGRID` flag.
4. Animate back without shaders, then right with the built-in glitch shader.
5. Build keyframe and function-sampled curves; animate back with the latter.
6. Animate right with a small runtime HLSL shader.
7. Animate back with tick/finish observers, then set `FWF_SNAPTOGRID`.
8. Restore the saved positions and the original snap setting on exit.

**These examples change your desktop.** Disable Auto arrange first and leave
some free space to the right of your icons. The conservative selection skips
occupied destinations and screen edges; it may find nothing on a full desktop.
It assumes an already non-overlapping layout on the selected monitor. Do not
rearrange icons or change display settings while it runs. Cleanup covers normal
completion and ordinary errors, not process termination.

<details>
<summary><strong>Expand the complete Python walkthrough</strong></summary>

```python
from __future__ import annotations

import rusty_desktop_icons as rdi


def main() -> None:
  controller = rdi.DesktopController()
  # 1. Inspect icons, desktop bounds, monitors, DPI and grids.
  icons = controller.list_icons()
  for icon in icons:
    print(icon.id, icon.display_name, icon.path, icon.is_virtual, icon.position)
  desktop = controller.desktop_info()
  print("Desktop / monitors / grids:", desktop.to_dict())
  for monitor in controller.list_monitors():
    print("Monitor:", monitor)

  snap = rdi.FolderFlag.FWF_SNAPTOGRID
  original_flags = controller.get_flags()
  if original_flags & rdi.FolderFlag.FWF_AUTOARRANGE:
    raise RuntimeError("Disable desktop Auto arrange before running this example")
  grid = next((item for item in desktop.grids if item.origin is not None), None)
  if grid is None:
    raise RuntimeError("No usable desktop grid")
  width = max(grid.cell_size[0], grid.icon_size[0])
  height = max(grid.cell_size[1], grid.icon_size[1])
  area = grid.work_area

  def has_room(icon: rdi.IconSnapshot) -> bool:
    horizontal, vertical = icon.position
    target_x = horizontal + grid.cell_size[0]
    return (
      area.left <= horizontal
      and target_x + width <= area.right
      and area.top <= vertical
      and vertical + height <= area.bottom
      and all(
        other.id == icon.id
        or abs(other.position[0] - target_x) >= width
        or abs(other.position[1] - vertical) >= height
        for other in icons
      )
    )

  selected = [icon for icon in icons if has_room(icon)][:3]
  if not selected:
    print("No icons have a free column to their right")
    return
  origins = [(icon.id, icon.position) for icon in selected]
  targets = [
    (icon.id, (icon.position[0] + grid.cell_size[0], icon.position[1]))
    for icon in selected
  ]

  # Prepare the effects and curves for stages 4-6 before moving any icons.
  envelope = rdi.Curve.keyframes([(0.0, 0.0), (0.5, 1.0), (1.0, 0.0)])
  glitch = rdi.Effect(
    rdi.Shader.compile(rdi.ShaderSource.builtin(rdi.BuiltinShader.Glitch)), envelope=envelope
  )
  tint_source = """
  float4 pixel(VertexOutput input) : SV_Target {
    float4 color = default_pixel(input);
    color.rgb *= lerp(float3(1, 1, 1), float3(0.4, 1, 0.7), input.timing.z);
    return color;
  }
  """
  tint = rdi.Effect(rdi.Shader.compile(tint_source), envelope=envelope)
  sampled = rdi.Curve.from_function(lambda progress: progress ** 2, samples=64)
  keyframed = rdi.Curve.keyframes([(0.0, 0.0), (0.5, 0.8), (1.0, 1.0)])
  print("Curve midpoint:", keyframed.eval(0.5), sampled.eval(0.5))

  def play(
    positions: list[tuple[str, tuple[int, int]]],
    curve: rdi.Curve,
    effect: rdi.Effect | None = None,
    observe: bool = False,
  ) -> None:
    specs = [
      rdi.IconAnimationSpec(
        icon_id, position, rdi.Duration.fixed(0.8), curve, effect=effect
      )
      for icon_id, position in positions
    ]
    handle = controller.prepare(specs).start()
    ticks: list[float] = []
    finishes: list[str] = []

    def on_tick(context: rdi.TickContext) -> None:
      ticks.append(context.progress)

    def on_finish(reason: rdi.FinishReason) -> None:
      finishes.append(reason.kind)

    if observe:
      handle.on_tick(on_tick)
      handle.on_finish(on_finish)
    try:
      reason = handle.wait()
      if reason.kind != rdi.FinishReasonKind.Completed:
        raise RuntimeError(str(reason))
      commit = handle.final_commit()
      if commit is not None and commit.missing_ids:
        raise RuntimeError(f"Unresolved icons: {commit.missing_ids}")
      if observe:
        print("Observed ticks:", len(ticks), "finish:", finishes)
    finally:
      handle.stop()
      handle.wait()

  try:
    # 2. Move up to three icons one column right without an overlay.
    print("Direct move; unresolved IDs:", controller.set_positions(targets))
    # 3. Unset Explorer's snap-to-grid flag.
    controller.unset_flags(snap)
    # 4. Animate back without a shader, then right with glitch.
    play(origins, rdi.Curve.ease_in_out())
    play(targets, rdi.Curve.ease_in_out(), glitch)
    # 5. Animate back with the function-sampled curve prepared above.
    play(origins, sampled)
    # 6. Animate right with the runtime HLSL tint shader.
    play(targets, rdi.Curve.ease_in_out(), tint)
    # 7. Observe the return animation, then enable snap-to-grid.
    play(origins, rdi.Curve.ease_in_out(), observe=True)
    controller.set_flags(snap)
  finally:
    # 8. Restore saved positions and the original snap setting, even on error.
    try:
      controller.unset_flags(snap)
      print("Restore; unresolved IDs:", controller.set_positions(origins))
    finally:
      controller.apply_flags(snap, original_flags)


if __name__ == "__main__":
  main()
```

</details>

### Folder Flags

`FolderFlag` is Windows-only and includes all 33 named Microsoft
[FOLDERFLAGS](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/ne-shobjidl_core-folderflags)
entries. Presence in the enum does not mean modern Explorer honors a flag. Some of them may not work or will be useless. But using them is possible to disable snap-to-grid or hide desktop icons, and something more.


<details>
<summary><strong>Open to read about flags</strong></summary>

| Intent | Python | Rust |
| --- | --- | --- |
| Read flags | `controller.get_flags()` | `controller.get_flags()?` |
| Set selected bits; preserve the rest | `controller.set_flags(bits)` | `controller.set_flags(bits)?` |
| Unset selected bits; preserve the rest | `controller.unset_flags(bits)` | `controller.unset_flags(bits)?` |
| Toggle selected bits | `controller.toggle_flags(bits)` | `controller.toggle_flags(bits)?` |
| Set and unset in one masked update | `controller.apply_flags(mask, values)` | `controller.apply_flags(mask, values)?` |
| Legacy exactly-set operation | `controller.set_flags_exactly(bits)` | `controller.set_flags_exactly(bits)?` |

Combine flags with `|`. Python accepts the `IntFlag` directly; Rust converts
named flags with `.bits()`, for example
`(FolderFlag::SnapToGrid | FolderFlag::AutoArrange).bits()`.

For **every bit** in `apply_flags(mask, values)`:

| Mask bit | Value bit | Result |
| --- | --- | --- |
| 0 | 0 | Unchanged |
| 0 | 1 | Unchanged |
| 1 | 0 | Unset |
| 1 | 1 | Set |

To set `enable` and unset `disable` together, pass `mask = enable | disable`
and `values = enable`; set wins if a bit appears in both. The full rule is
`new = (old & ~mask) | (values & mask)` within a 32-bit word.

**Important:** `set_flags_exactly(bits)` clears only bits up to and including
the highest set bit in `bits`, preserving higher bits; passing zero does
nothing. It is not a whole-word replacement. `apply_flags(0xFFFFFFFF, values)`
requests a full replacement, but can change unrelated Shell settings.

Explorer's placement flags and the engine's grid option are separate:

| Auto arrange | Snap to grid | Explorer placement behavior |
| --- | --- | --- |
| Unset | Unset | Free positioning |
| Unset | Set | Positions may be snapped to Explorer's grid |
| Set | Unset | Explorer controls arrangement; manual targets may not persist |
| Set | Set | Explorer controls arrangement with grid alignment |

`AnimationOptions(snap_to_grid=True)` in Python, or
`AnimationOptions { snap_to_grid: true, ..Default::default() }` in Rust,
reserves available destination cells without changing either flag. It does
not override Auto arrange or prevent icons from crossing during transit.
General flag/position writes are blocked during preparation and animation.
`before_flags` / `after_flags` support `FolderFlagOp.set` / `.exactly` in Python
and `FolderFlagOp::Set` / `::Exactly` in Rust; there is no `Unset` hook variant.
For explicit clearing, wait for the previous session to end and call
`unset_flags` before preparing the next one, as above.

</details>

## Getting Started (Rust)

### Installation

Install Rust 1.88+ and the Windows build prerequisites from [Demo App](#demo-app).
The library consists of `rdi-core` and a platform backend; the root
`rusty-desktop-icons` package contains demos, not a facade library.

For a new application beside this checkout:

```powershell
cargo new desktop-example
cd desktop-example
cargo add rdi-core --path ../rusty-desktop-icons/crates/rdi-core
cargo add rdi-platform-windows --path ../rusty-desktop-icons/crates/rdi-platform-windows
```

Adjust the paths to your checkout. Local dependencies avoid assuming a
crates.io release exists. For reproducible builds using Git dependencies,
pin both crates to the same reviewed revision. See the
[Rust installation guide](docs/rust/getting-started/installation.md).

### Example

Use this as the new application's `main` module, then run `cargo run --release`.
It follows the same logic and numbered stages as the
[Python walkthrough sequence](#walkthrough-sequence), including its precautions;
the [folder flag reference](#folder-flags) applies to both languages.

<details>
<summary><strong>Expand the equivalent Rust walkthrough</strong></summary>

```rust
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use rdi_core::{
  AnimationCurve, AnimationOptions, Curve, DesktopController, Duration, Effect,
  FinishReason, IconAnimationSpec, IconId, Keyframe, KeyframeInterp, Point,
};
use rdi_platform_windows::{FolderFlag, WindowsBackend, shader};

type DemoResult = Result<(), Box<dyn std::error::Error>>;

fn play(
  controller: &DesktopController,
  positions: &[(IconId, Point)],
  curve: Curve,
  effect: Option<&Effect>,
  observe: bool,
) -> DemoResult {
  let specs = positions.iter().map(|(icon_id, position)| {
    let mut spec = IconAnimationSpec::new(
      icon_id.clone(), *position,
      Duration::fixed(StdDuration::from_millis(800)), curve.clone(),
    );
    spec.effect = effect.cloned();
    spec
  }).collect();
  let handle = controller.prepare(specs, AnimationOptions::default())?.start()?;
  let ticks = Arc::new(Mutex::new(Vec::new()));
  let finishes = Arc::new(Mutex::new(Vec::new()));
  if observe {
    let tick_log = Arc::clone(&ticks);
    handle.on_tick(move |context| {
      tick_log.lock().unwrap().push(context.progress);
    });
    let finish_log = Arc::clone(&finishes);
    handle.on_finish(move |reason| {
      finish_log.lock().unwrap().push(reason.clone());
    });
  }
  let reason = handle.wait();
  if !matches!(reason, FinishReason::Completed) {
    return Err(format!("Animation ended: {reason:?}").into());
  }
  if let Some(commit) = handle.final_commit() {
    if !commit.missing_ids.is_empty() {
      return Err(format!("Unresolved icons: {:?}", commit.missing_ids).into());
    }
  }
  if observe {
    println!("Observed ticks: {}; finish: {:?}",
      ticks.lock().unwrap().len(), finishes.lock().unwrap());
  }
  Ok(())
}

fn main() -> DemoResult {
  let controller = DesktopController::new(WindowsBackend::new())?;
  // 1. Inspect icons, desktop bounds, monitors, DPI and grids.
  let icons = controller.list_icons()?;
  for icon in &icons {
    println!("{icon:?}");
  }
  let desktop = controller.desktop_info()?;
  println!("Desktop / monitors / grids: {desktop:#?}");
  println!("Monitors: {:#?}", controller.list_monitors()?);

  let snap = FolderFlag::SnapToGrid.bits();
  let original_flags = controller.get_flags()?;
  if original_flags & FolderFlag::AutoArrange.bits() != 0 {
    return Err("Disable desktop Auto arrange before running this example".into());
  }
  let grid = desktop.grids.iter().find(|grid| grid.origin.is_some())
    .ok_or("No usable desktop grid")?;
  let width = grid.cell_size.x.max(grid.icon_size.x);
  let height = grid.cell_size.y.max(grid.icon_size.y);
  let area = grid.work_area;
  let selected: Vec<_> = icons.iter().filter(|icon| {
    let target_x = icon.position.x + grid.cell_size.x;
    area.left <= icon.position.x && target_x + width <= area.right
      && area.top <= icon.position.y && icon.position.y + height <= area.bottom
      && icons.iter().all(|other| {
        other.id == icon.id
          || (other.position.x - target_x).abs() >= width
          || (other.position.y - icon.position.y).abs() >= height
      })
  }).take(3).collect();
  if selected.is_empty() {
    println!("No icons have a free column to their right");
    return Ok(());
  }
  let origins: Vec<_> = selected.iter()
    .map(|icon| (icon.id.clone(), icon.position)).collect();
  let targets: Vec<_> = selected.iter().map(|icon| (
    icon.id.clone(), Point::new(icon.position.x + grid.cell_size.x, icon.position.y),
  )).collect();

  // Prepare the effects and curves for stages 4-6 before moving any icons.
  let envelope = Curve::keyframes(vec![
    Keyframe::new(0.0, 0.0), Keyframe::new(0.5, 1.0), Keyframe::new(1.0, 0.0),
  ], KeyframeInterp::Linear)?;
  let program = shader::compile(shader::BuiltinShader::Glitch)?;
  let glitch = Effect {
    params: program.default_params(), shader: program, padding_px: 16,
    envelope: envelope.clone(), seed: 0.0,
  };
  let tint_source = r#"
  float4 pixel(VertexOutput input) : SV_Target {
    float4 color = default_pixel(input);
    color.rgb *= lerp(float3(1, 1, 1), float3(0.4, 1, 0.7), input.timing.z);
    return color;
  }
  "#;
  let program = shader::compile(tint_source)?;
  let tint = Effect {
    params: program.default_params(), shader: program, padding_px: 16,
    envelope, seed: 0.0,
  };
  let sampled = Curve::from_curve_fn(
    |progress| progress * progress, 64, KeyframeInterp::Linear,
  )?;
  let keyframed = Curve::keyframes(vec![
    Keyframe::new(0.0, 0.0), Keyframe::new(0.5, 0.8), Keyframe::new(1.0, 1.0),
  ], KeyframeInterp::Linear)?;
  println!("Curve midpoint: {}, {}", keyframed.eval(0.5), sampled.eval(0.5));

  let result = (|| -> DemoResult {
    // 2. Move up to three icons one column right without an overlay.
    println!("Direct move; unresolved IDs: {:?}", controller.set_positions(targets.clone())?);
    // 3. Unset Explorer's snap-to-grid flag.
    controller.unset_flags(snap)?;
    // 4. Animate back without a shader, then right with glitch.
    play(&controller, &origins, Curve::ease_in_out(), None, false)?;
    play(&controller, &targets, Curve::ease_in_out(), Some(&glitch), false)?;
    // 5. Animate back with the function-sampled curve prepared above.
    play(&controller, &origins, sampled, None, false)?;
    // 6. Animate right with the runtime HLSL tint shader.
    play(&controller, &targets, Curve::ease_in_out(), Some(&tint), false)?;
    // 7. Observe the return animation, then enable snap-to-grid.
    play(&controller, &origins, Curve::ease_in_out(), None, true)?;
    controller.set_flags(snap)?;
    Ok(())
  })();
  // 8. Attempt both position and flag restoration before propagating errors.
  let clear_result = controller.unset_flags(snap);
  let restore_result = controller.set_positions(origins);
  let flags_result = controller.apply_flags(snap, original_flags);
  clear_result?;
  println!("Restore; unresolved IDs: {:?}", restore_result?);
  flags_result?;
  result
}
```

</details>

Observers run on the worker thread. Keep callbacks short: enqueue data for
your UI rather than rendering, doing slow I/O, or calling blocking controller
methods from them. The examples collect a few events and print after `wait()`.
Observers attached after `start()` may miss early ticks. A successful finish
or Shell call alone is not proof of correct on-screen appearance.

## Features & Showcases

- **Desktop surface control:** enumerate icon IDs, names, paths and positions;
  move icons directly; inspect desktop bounds, monitors, DPI and live grid
  spacing; read and change desktop folder flags.
- **Rich icon animation:** GPU-backed overlay rendering, configurable curves,
  independent axis motion, reversible timelines, and per-icon effects.
- **Runtime shaders:** use built-in effects or compile your own HLSL strings,
  including custom vertex and pixel stages. Effects can include labels and badges.
- **Multi-monitor support, work in progress:** monitor-aware coordinates and
  destination planning exist; mixed-DPI and cross-monitor behavior need more testing.
- **An [MCP server](#mcp-server)** for AI-assisted desktop organization.
- **Python bindings via [PyO3](https://pyo3.rs/):** the animation engine stays in Rust.
- **Off-screen rendering:** build scenes and export frames without moving real icons.

**Performance:** the aim is smooth animation across a full desktop with 200+
icons. Recorded GPU tests cover 256 icons at 3840 x 2400, but off-screen render
times are not displayed FPS. Stable **60+ FPS at 8K with 200+ icons verified performance claim**, but not a guarantee, especially for custom shaders.

**Trusted shaders only.** This shader API is a proof of concept. Expensive or
malicious shaders can overload the GPU, freeze the display, or cause a driver
reset. The exposed HLSL interface has no filesystem API or host-code execution
facility, and filesystem includes are disabled; that does **not** make it a
security sandbox or a guarantee against compiler/driver vulnerabilities. Review
shader sources, including AI-generated ones, before compiling them.

### Showcases

These looping previews are off-screen renders composed over a desktop mockup,
not screen recordings or FPS benchmarks. **Click any small preview to open
its full-size version.**

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

The project-intro animation is the banner at the top. All README media lives on
the dedicated [assets_storage branch](https://github.com/s0d3s/rusty-desktop-icons/tree/assets_storage/assets_storage).
See [showcase export](docs/python/examples/showcases.md)
to render your own scenes.

## Status & Limitations

This is a **proof of concept**, built quickly to prove that separating Shell
manipulation from visual animation works, and to replace Positioner's desktop
backend after the Windows 11 repaint changes described in the prologue.

The architecture was designed with cross-platform backends in mind. There is
not yet a clear audience driving those implementations, so only Windows has a
working desktop backend today.

| Platform | Status |
| --- | --- |
| Windows 10 / 11, x64 | ![Supported](https://img.shields.io/badge/%E2%9C%93-supported-brightgreen) |
| macOS | ![Planned](https://img.shields.io/badge/%E2%88%92-planned-lightgrey) |
| Linux / other platforms | ![Planned](https://img.shields.io/badge/%E2%88%92-planned-lightgrey) |

Non-Windows builds use a stub backend; successful compilation does not mean
desktop control is available.

The renderer and shaders are a **PoC inside a PoC**. Apparently one layer of
experimental software was not enough :) Today it uses DirectX: Direct2D,
DirectComposition, and D3D11 with HLSL. A move to **wgpu and WGSL** is planned,
not implemented. There is **no stable API promise**, particularly for shaders.

### Overlay Limitations

- **Thumbnails and shortcut arrows:** thumbnail sizing and arrows on shortcuts
  to thumbnail-bearing objects can look different from Explorer. Labels also
  retain some wrapping differences.
- **Unusual desktop flags:** ordinary user-facing settings have been exercised,
  but combinations of less-visible Shell flags, such as label suppression or
  checkbox styles, are not comprehensively verified. Not every named flag is
  supported by current Windows versions.
- **Resolution and DPI combinations:** my testing found sizing
  problems in roughly 5 of 60 combinations, mainly around 2K resolutions
  at 300% scaling and above. Most tested combinations looked closer to Explorer;
  unusual display configurations still need checking.
- **Multi-monitor behavior:** mixed DPI and movement between monitors remain WIP.
- **Lifecycle constraints:** only one overlay session can run per process;
  animation participants and targets are fixed at preparation. Stop and prepare
  a new session to change them. If an ordinary animation cannot create an
  overlay, it can fall back to a warned direct move rather than animate.

## About Curves and Animation

A curve maps normalized time to movement progress. For each axis:

```text
time = clamp(elapsed / duration, 0, 1)
position = origin + curve(time) * (target - origin)
```

X and Y can use different curves while sharing a clock. Curves may overshoot
the target, pause on a plateau, or accelerate differently on each axis.
Duration can be fixed, distance-based, or distance-based with minimum/maximum
bounds. An effect's strength envelope is another curve, independent of motion.

| Build a curve from | Useful for |
| --- | --- |
| Built-in easing functions | Linear moves, ease-in/out, sine, cubic and quadratic easing |
| Configurable helpers | Custom cubic Bezier shapes, spring and bounce motion |
| Explicit keyframes | Holds, staged movement, overshoot and authored timing |
| A sampled function | Procedural curves from your own mathematics |
| A motion-aware function | Curves based on origin, target, distance, duration and parameters |

Keyframes support linear, step, and smoothstep interpolation. Supply them
directly, generate them from data, or sample a function: Python uses
`Curve.from_function` / `Curve.from_motion_function`; Rust uses
`Curve::from_curve_fn` / `Curve::from_motion_fn`.

Functions are sampled **once when building the curve**, not on each frame.
The resulting animation is plain data evaluated by Rust. In particular,
Python movement callbacks do not run in the tick loop. Observer callbacks are
different: they run during playback and must remain lightweight.

Shared contracts live in [Concepts](docs/concepts/index.md):
[desktop management](docs/concepts/desktop.md),
[curves and durations](docs/concepts/curves.md),
[animation](docs/concepts/animation.md), and
[non-blocking execution](docs/concepts/execution.md).

For working code, see [Python layout/playback recipes](docs/python/guides/basic-usage.md),
[Rust curve recipes](docs/rust/guides/curves.md),
[reversible timelines](docs/python/guides/timelines.md), and
[native frame export](docs/python/examples/showcases.md).
[Shader concepts](docs/concepts/shaders/index.md) lead to the full
[custom HLSL and multi-pass API](docs/concepts/shaders/custom.md), with
[Python](docs/python/guides/shaders.md) and [Rust](docs/rust/guides/shaders.md) recipes.

## Positioner & MCP Server

### Positioner

[Positioner](https://github.com/s0d3s/Positioner) is the reason this library
exists: a desktop app for saving, restoring, and switching icon layouts. Its
named snapshots, quick slots, desktop-view settings, and startup restoration
turn "I finally arranged everything" into something you can keep.

Rusty Desktop Icons was designed as its new desktop backend. Positioner is
the app to look at when you want layout management with a GUI; this project
is the engine for building that behavior into your own tools. That is the
integration goal, not a claim that existing Positioner releases already use
this backend.

### MCP Server

The second goal is an MCP server so an AI agent can inspect the desktop,
help organize icons, and animate the resulting layouts through structured
tools. The runtime shader API also opens the door to genuinely custom effects,
including shader source proposed by an agent and reviewed by you.

**[MCP server repository (placeholder; publication pending)](https://github.com/s0d3s/rusty-desktop-icons-mcp)**

The companion server is a separate project, not something the library's
`pip install` starts. AI-authored shaders deserve the same scrutiny as any
other third-party GPU code; custom-effect workflows are a goal, not a promise
that every library API is already exposed as an MCP tool.

## License

Licensed under the [Apache License 2.0](LICENSE).

Third-party dependencies and artwork retain their respective licenses and
trademark rights. Positioner and the companion MCP server are separate projects
with their own licenses.
