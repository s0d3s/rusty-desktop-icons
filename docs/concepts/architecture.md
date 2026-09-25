# Architecture

Rusty Desktop Icons separates **desktop state** from **rendered motion**.
Applications describe an operation; a native worker coordinates Explorer and
draws prepared icon copies. Python and Rust use the same engine, not separate
implementations of animation math.

## Layers and Ownership

| Layer | Responsibility |
| --- | --- |
| `rdi-core` | Safe, platform-independent data, curves, controller, worker and backend contracts |
| `rdi-platform-windows` | Shell/COM, artwork extraction, Direct2D, D3D11 and DirectComposition |
| `rdi-platform-stub` | Unsupported-platform behavior, not emulated desktop control |
| `rdi-python` | PyO3 objects, exceptions, argument conversion and GIL-aware waits |
| Application | Layout policy, UI, cancellation, diagnostics and image encoding |

`DesktopController` sends work to one worker. On Windows that worker initializes
COM as STA and owns the Shell/rendering objects for their entire lifecycle.
Changing callers or using `asyncio.to_thread` does not move those resources.
Observers also run there; see [non-blocking execution](execution.md).

## Choose the Operation

| Need | Path |
| --- | --- |
| Read/save/restore actual desktop state | [Desktop management](desktop.md) |
| One transition that finalizes automatically | [Animation](animation.md) |
| Interactive preview, scrub and reverse | [Timeline](timelines.md) |
| Exact-time pixels without desktop writes | [Render session](rendering.md) |

Preparation captures artwork and builds resources once. Live overlay playback
temporarily moves/hides real icons while drawing copies. Off-screen rendering
shares motion, layout and shaders, but never enters that Shell visibility
lifecycle. Neither capture API is a screenshot of wallpaper or other windows.

## Backend Integration Types

Most applications need only the controller. Rust backend authors and renderer
tests additionally use these contracts:

- `DesktopBackend` supplies desktop queries/writes and optional overlay/rendering
  hooks. `WindowsBackend` implements them; `fake::FakeDesktop` supports deterministic
  engine tests without Explorer. A fake does not prove GPU/display behavior.
- `IconBitmap` owns captured pixels and dimensions. `IconLabel` carries label
  content/layout inputs. `IconRenderPlan` combines artwork, movement endpoints
  and an optional effect for preparation.
- `OverlayRenderOptions` chooses labels, shortcut overlays and shield overlays.
  These choices affect prepared artwork, not just a final image filter.
- `IconFrame` is evaluated per-icon position/progress/time sent to the renderer.
  `SceneRenderer` consumes frames and returns owned `CapturedFrame` data.
- `SnapshotFrame` and `SnapshotIconGeometry` expose rendered geometry for tests
  and diagnostics; they are not a second animation engine.

See the [entity map](entities.md) for the rest of the public surface and the
[Rust reference](../rust/index.md) for backend method signatures. Device/COM
implementation details remain in the Windows crate; applications should not
transport those objects onto another thread.

## Documentation Sources

Concepts define contracts; language guides show recipes. The
[Python reference](../python/index.md) comes from generated PyO3 stubs and the
[Rust reference](../rust/index.md) is native rustdoc. Neither requires a second
hand-maintained signature catalog. See [contributing](../contributing.md) for
generation and validation commands.