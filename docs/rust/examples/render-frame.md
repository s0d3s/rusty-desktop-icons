# Render an Off-Screen Frame

This complete program uses desktop artwork but does not move or hide icons.
It renders at an exact timestamp and reports the resulting buffer. Install the
[two library crates](../getting-started/installation.md) and run it in an
interactive Windows session with at least one desktop icon.

```rust
use rdi_core::{Canvas, Curve, DesktopController, Duration, IconAnimationSpec,
    OverlayRenderOptions, Point, Scene, SceneIcon};
use rdi_platform_windows::WindowsBackend;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let controller = DesktopController::new(WindowsBackend::new())?;
    let icon = controller.list_icons()?.into_iter().next()
        .ok_or("At least one desktop icon is needed for artwork")?;
    let scene = Scene {
        canvas: Canvas { width: 640, height: 240, dpi_scale: 1.0, icon_size: 48 },
        icons: vec![SceneIcon {
            origin: Point::new(60, 80),
            animation: IconAnimationSpec::new(
                icon.id, Point::new(500, 80),
                Duration::Fixed(std::time::Duration::from_secs(2)),
                Curve::ease_in_out(),
            ),
        }],
        render_options: OverlayRenderOptions::default(),
    };
    let session = controller.prepare_scene(scene)?;
    let frame = session.render_at(1.0)?;
    session.close()?;
    assert_eq!(frame.pixels.len(), frame.width as usize * frame.height as usize * 4);
    println!("{}x{} at {}s: {} bytes", frame.width, frame.height,
        frame.seconds, frame.pixels.len());
    Ok(())
}
```

For a sequence, retain the session and render at `frame_index / fps`, bounded by
`session.duration`. Earlier timestamps reproduce earlier states without sleeping.
Attach a [shader effect](../guides/shaders.md) before preparing the scene.

The bytes are premultiplied BGRA8, not PNG data. Before giving them to a
straight-alpha RGBA encoder, reorder channels and unpremultiply nonzero-alpha
RGB (set RGB to zero for zero alpha). The [rendering contract](../../concepts/rendering.md)
describes limits and coordinates; the [Python PNG example](../../python/examples/showcases.md)
shows conversion through an existing decoder.