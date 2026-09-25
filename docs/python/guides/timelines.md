# Scrub, Reverse and Capture

Use a [timeline](../../concepts/timelines.md) when your UI must seek, pause or
reverse a prepared batch. This helper opens a **live overlay**: real icons are
temporarily moved and hidden. Use [off-screen rendering](../examples/showcases.md)
for write-free previews.

## Preview a Reviewed Batch

Call with specs you built using the [batch recipe](basic-usage.md). The helper
samples a halfway frame, travels to the target, then reverses to the origin.
Context exit restores the original desktop positions even if a Python exception
interrupts the body.

```python
import rusty_desktop_icons as rdi

def preview_round_trip(
    controller: rdi.DesktopController, specs: list[rdi.IconAnimationSpec],
) -> rdi.CapturedFrame:
    prepared = controller.prepare(specs)
    with prepared.open_timeline() as timeline:
        halfway = timeline.seek_and_capture(0.5)
        outward = timeline.play_to(1.0, speed=1.0).wait()
        if outward != rdi.PlaybackOutcome.Reached:
            raise RuntimeError(f"Preview interrupted: {outward}")
        returning = timeline.play_to(0.0, speed=1.5).wait()
        if returning != rdi.PlaybackOutcome.Reached:
            raise RuntimeError(f"Return interrupted: {returning}")
        return halfway
```

The returned frame owns its pixels after the session closes. Encode it using
the [PNG recipe](../examples/showcases.md).

## Connect to UI Controls

Keep the session alive in your UI model. Invoke `seek(slider_value)` with a
normalized slider value, `pause()` for a pause button, and `play_to(0.0)` for
reverse. Keep the returned `PlaybackHandle` to inspect that particular traversal.
Do not call `wait()` on the GUI event thread; use a worker/executor as in the
[non-blocking recipe](basic-usage.md).

On an explicit Apply action, use
`timeline.close(rdi.TimelineCloseMode.LeaveInPlace)`; on Cancel, use
`timeline.close(rdi.TimelineCloseMode.RestoreOrigins)`. Closing with
`TeleportToTarget` commits the original requested endpoints. Inspect the returned
`FinishReason` instead of assuming cleanup succeeded.

Attach [effects](shaders.md) before preparation. Seeking drives their virtual
clock together with movement, so a saved slider position can reproduce a frame.