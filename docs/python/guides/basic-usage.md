# Manage Layouts and Playback

Recipes for [desktop management](../../concepts/desktop.md),
[animation requests](../../concepts/animation.md) and
[non-blocking execution](../../concepts/execution.md). Functions below are
reusable building blocks; defining them does not change the desktop.

## Save and Restore a Layout

Save before changing a layout. Restore only after reviewing the saved positions
against the current monitor arrangement. IDs are opaque, not portable identities;
renamed/deleted icons can disappear between saving and restoring.

```python
from pathlib import Path
import json
import rusty_desktop_icons as rdi

def save_layout(controller: rdi.DesktopController, path: Path) -> None:
    layout = {icon.id: list(icon.position) for icon in controller.list_icons()}
    path.write_text(json.dumps(layout, indent=2), encoding="utf-8")

def restore_layout(controller: rdi.DesktopController, path: Path) -> list[str]:
    saved = json.loads(path.read_text(encoding="utf-8"))
    current = {icon.id for icon in controller.list_icons()}
    positions = [(identity, (int(point[0]), int(point[1])))
                 for identity, point in saved.items() if identity in current]
    absent = [identity for identity in saved if identity not in current]
    return absent + controller.set_positions(positions)
```

This format is for your own trusted snapshots, not arbitrary uploaded JSON.
`save_layout(controller, Path("layout.json"))` only reads the desktop;
`restore_layout(...)` writes it. Inspect its returned missing IDs. Explorer's
Auto arrange can prevent a requested layout from sticking.

## Change Only the Relevant Flags

This context manager temporarily clears Auto arrange and Snap to grid, then
restores only those bits. Use it around direct positioning, not around an active
animation reservation; stop and wait before leaving the block.

```python
from contextlib import contextmanager
from typing import Iterator
import rusty_desktop_icons as rdi

@contextmanager
def free_positioning(controller: rdi.DesktopController) -> Iterator[None]:
    mask = rdi.FolderFlag.FWF_AUTOARRANGE | rdi.FolderFlag.FWF_SNAPTOGRID
    saved = controller.get_flags() & int(mask)
    try:
        controller.unset_flags(mask)
        yield
    finally:
        controller.apply_flags(mask, saved)
```

For animation-scoped changes, supply `FolderFlagOp` values in
`AnimationOptions.before_flags` and `after_flags` instead. See the
[flag-operation contract](../../concepts/desktop.md).

## Start a Planned Batch

Pass reviewed icon IDs and targets; this function starts a **live desktop
animation** and returns immediately after startup/preparation. Grid snapping
can adjust destinations, so inspect the prepared context when exact planning
matters.

```python
from typing import Mapping
import rusty_desktop_icons as rdi

def start_layout(
    controller: rdi.DesktopController,
    targets: Mapping[str, tuple[int, int]],
) -> rdi.AnimationHandle:
    if not targets:
        raise ValueError("Select at least one icon")
    specs = [rdi.IconAnimationSpec(
        identity, point, rdi.Duration.distance_clamped(500, 0.25, 1.5),
        rdi.Curve.ease_in_out(),
    ) for identity, point in targets.items()]
    return controller.animate(specs, rdi.AnimationOptions(snap_to_grid=True))
```

In a GUI, poll `handle.progress()` and `handle.is_running()` from its timer.
After completion, inspect `handle.finish_reason()` and `handle.final_commit()`.
To cancel, call `handle.stop()` followed by `handle.wait()` before restoring
positions or starting another write. The [execution contract](../../concepts/execution.md)
explains stop policies and why a timeout is not cancellation.

## Wait Without Blocking an Async Event Loop

Preparation can block too, so move both startup and waiting to executor threads.
The controller still performs native work on its own STA worker. Cancellation
waits for in-flight startup before requesting cleanup, avoiding an orphaned
animation. Call this helper from your existing asyncio application; it starts
real playback with the supplied specs.

```python
import asyncio
import rusty_desktop_icons as rdi

async def play(
    controller: rdi.DesktopController, specs: list[rdi.IconAnimationSpec],
) -> rdi.FinishReason:
    startup = asyncio.create_task(asyncio.to_thread(controller.animate, specs))
    handle = None
    try:
        handle = await asyncio.shield(startup)
        return await asyncio.to_thread(handle.wait)
    finally:
        if handle is None:
            handle = await startup
        handle.stop()
        await asyncio.to_thread(handle.wait)
```

Keep the cleanup task alive during application shutdown. For reversible UI
scrubbing, use the [timeline recipe](timelines.md). For failures and diagnostic
routing, see [errors and logging](../../concepts/errors.md).