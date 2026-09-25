# Scenes, Canvases and Captured Frames

Off-screen rendering uses real icon artwork without moving or hiding desktop
icons. It creates no overlay window. This is useful for thumbnails, exported
animations, deterministic shader tests and previews in another application.

## Canvas and Scene

`Canvas` declares output width/height in pixels, DPI scale and baseline icon
size in DIPs. It is independent of physical monitor dimensions. Each dimension
is 1..8192, total area is at most 33,554,432 pixels, DPI scale is 0.5..4, and
icon size is 16..256. Valid dimensions can still exceed practical GPU/memory
budgets once artwork and intermediate render targets are included.

Rust's `Scene` contains a canvas, `SceneIcon` values (explicit origin plus
animation spec) and `OverlayRenderOptions`. Python supplies the same inputs as
`prepare_scene(canvas, specs, positions, options=...)`. IDs must identify current
desktop artwork and be unique; custom coordinates do not create new icon IDs.

## RenderSession

Preparation reads artwork and builds reusable GPU resources on the worker.
`render_at(seconds)` accepts exact finite seconds from zero through
`session.duration`, with no sleeping or wall-clock pacing. Rendering a later
frame and then an earlier frame evaluates the earlier state deterministically.
The scene duration is the longest icon duration.

A render session reserves its controller: reads remain available, but competing
writes and sessions do not. Close it explicitly, use Python's context manager,
or let Rust Drop request release. Resources are not shared by moving the native
renderer to a different thread; only owned output pixels should cross workers.

## CapturedFrame and Pixel Conversion

`CapturedFrame` owns width, height, timestamp and tightly packed **premultiplied
BGRA8** bytes. Python additionally exposes `stride` and `pixel_format`.
The stride is width times four. Alpha multiplies the color channels; simply
swapping BGRA to RGBA does not produce straight-alpha PNG pixels.

When using Pillow, decode with raw mode `BGRa` (premultiplied BGRA) into RGBA,
which performs the unpremultiplication. Transparent pixels must remain
transparent; composite onto a background before converting to RGB. The native
library returns raw pixels, not PNG/APNG/GIF files. Encoding is application work.

`SnapshotFrame` and `SnapshotIconGeometry` in Rust, or the diagnostic dictionary
from Python `render_overlay_snapshot`, add per-icon rectangles for comparing
layout/artwork geometry. They are diagnostic snapshots, not live playback.

## Put It to Work

- [Python off-screen example](../python/examples/showcases.md)
- [Rust off-screen example](../rust/examples/render-frame.md)
- [Native-generated showcase assets](showcases.md)
- [Timeline capture](timelines.md)