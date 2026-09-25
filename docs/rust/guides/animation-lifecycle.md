# Run and Observe a Batch

Use [animation requests](../../concepts/animation.md) to describe a batch and
[execution handles](../../concepts/execution.md) to control it. These functions
accept reviewed specs; calling them starts **live desktop playback**.

## Register Observers Before Startup

Send progress through a bounded channel rather than touching a GUI from the
worker callback. A full channel drops an obsolete update, not a render frame.
This standalone program defines the integration helper but does not call it.

```rust
use std::sync::mpsc::{sync_channel, Receiver};
use rdi_core::{AnimationHandle, AnimationOptions, DesktopController, DesktopError,
    IconAnimationSpec, PreObservers};

fn start_observed(
    controller: &DesktopController,
    specs: Vec<IconAnimationSpec>,
) -> Result<(AnimationHandle, Receiver<f32>), DesktopError> {
    let (sender, receiver) = sync_channel(1);
    let observers = PreObservers::new().on_tick(move |context| {
        let _ = sender.try_send(context.progress);
    });
    let handle = controller.animate_with_observers(
        specs, AnimationOptions::default(), observers,
    )?;
    Ok((handle, receiver))
}

fn main() {}
```

The UI can call `receiver.try_recv()` in its event loop and check
`handle.is_running()`. Keep blocking waits off that thread. To cancel, request
`handle.stop(StopMode::LeaveInPlace)` and then `handle.wait()` before a new write.
Read `FinishReason` and `final_commit()` after completion; see
[completion semantics](../../concepts/execution.md).

## Preview and Reverse

Use this helper with reviewed specs. It waits synchronously, making it suitable
for a CLI or application worker. Explicit close acknowledges restoration; Drop
remains a cleanup backstop when a preceding operation fails.

```rust
use rdi_core::{AnimationOptions, DesktopController, DesktopError, FinishReason,
    IconAnimationSpec, PlaybackOutcome, TimelineCloseMode};

fn preview(
    controller: &DesktopController, specs: Vec<IconAnimationSpec>,
) -> Result<FinishReason, DesktopError> {
    let timeline = controller.prepare(specs, AnimationOptions::default())?
        .open_timeline()?;
    timeline.seek(0.5)?;
    let outward = timeline.play_to(1.0, 1.0)?.wait()?;
    if outward == PlaybackOutcome::Reached {
        let returning = timeline.play_to(0.0, 1.5)?.wait()?;
        println!("Return: {returning:?}");
    }
    timeline.close(TimelineCloseMode::RestoreOrigins)
}

fn main() {}
```

For Apply/Cancel controls, choose `LeaveInPlace` or `RestoreOrigins` at close.
See [timeline contracts](../../concepts/timelines.md),
[shader attachment](shaders.md) and the [installed CLI example](../examples/index.md).
Use an [off-screen scene](../examples/render-frame.md) when no desktop writes
are acceptable.