# Export Native Frames

Render into a [canvas](../../concepts/rendering.md), then encode its pixels with
Pillow. This is a complete package-use example, not a screen recording: no real
icon positions, visibility or folder flags are changed. It needs one desktop
icon as an artwork source and an interactive Windows session.

```powershell
python -m pip install rusty-desktop-icons Pillow
```

## Write a Transparent PNG

```python
from pathlib import Path
from PIL import Image
import rusty_desktop_icons as rdi

def export_preview(output: Path) -> None:
    controller = rdi.DesktopController()
    icons = controller.list_icons()
    if not icons:
        raise RuntimeError("At least one desktop icon is needed for artwork")
    identity = icons[0].id
    canvas = rdi.Canvas(640, 240, dpi_scale=1.0, icon_size=48)
    spec = rdi.IconAnimationSpec(
        identity, (500, 80), rdi.Duration.fixed(2.0), rdi.Curve.ease_in_out(),
    )
    with controller.prepare_scene(canvas, [spec], [(identity, (60, 80))]) as scene:
        frame = scene.render_at(1.0)
    image = Image.frombytes(
        "RGBA", (frame.width, frame.height), frame.pixels, "raw", "BGRa",
    )
    image.save(output)

if __name__ == "__main__":
    export_preview(Path("preview.png"))
```

Pillow's `BGRa` decoder converts premultiplied BGRA into straight-alpha RGBA.
Treating the bytes as ordinary `BGRA` produces dark transparent fringes.
Add an [effect](../guides/shaders.md) to `spec` before preparing the scene.

## Export a Sequence

Inside the same prepared session, call `render_at(frame_index / fps)` for each
output frame and encode the result as above. At 30 FPS over two seconds, render
indices `0..60` when both endpoints are needed. An endlessly looping animation
normally omits the repeated final frame. For reverse travel, decrease the same
timestamp; do not rebuild a new animation with swapped endpoints.

Use `scene.duration` to derive the range, record frame delays explicitly, and
stream numbered PNGs to disk rather than keeping an entire movie in memory.
Export throughput does not determine playback timing. Review filenames and
artwork before sharing images from your desktop.

## Reproduce the Repository Gallery

The [showcase overview](../../concepts/showcases.md) links the native-generated
assets and scene sources. These commands require a repository checkout and the
[contributor tooling environment](../../contributing.md) with a fresh native
wheel installed; they are not modules shipped in the PyPI package.

```powershell
python scripts/showcases/render-showcases.py --showcase silk_flow_small --output target/my-showcases
python scripts/showcases/assemble-showcase.py --input target/my-showcases
python scripts/showcases/convert-showcase-gif.py --input target/my-showcases --output target/my-showcases-compressed
```

Omit `--showcase` for all 13 scenes. Full-grid scenes use up to 30 desktop icons,
compact scenes use six, and the intro requires at least 96. Scene declarations
set canvas size, DPI, icon size, duration, grid and background. Raw PNGs retain
alpha; assembly uses the manifest's background/alpha policy. Transparent strips
convert to WebP and opaque previews to GIF.

Existing outputs are skipped. Add `--overwrite` to each stage you deliberately
regenerate; rerendering replaces that scene's frame directory. Use `--help` for
worker/encoder controls. The native renderer stays serial on each controller;
encoding can run on separate workers after pixel ownership transfers to Python.