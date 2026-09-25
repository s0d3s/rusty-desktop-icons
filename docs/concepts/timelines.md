# Timelines and Playback

`TimelineSession` is a persistent, seekable animation opened from a prepared
request. It starts paused at zero. Reaching either endpoint pauses; it does not
close the overlay or release resources. Use it for an editor, preview scrubber,
reversible transition or interactive demonstration.

## Position and Time

`seek` accepts a finite normalized position in `[0, 1]` and pauses there.
`play_to` chooses a destination and positive finite speed; reverse playback is
selected by a destination below the current position, not by a negative speed.
`set_speed` changes traversal speed without replacing the session; `pause`
retains the current position. Commands wait for acknowledgement from the worker.

Movement and shader time share the virtual clock. Seeking backward evaluates
the earlier state, rather than undoing accumulated physics. Each icon retains
its own duration while the session spans the longest resolved duration.

`TimelineState` is Paused, Playing or Closed. `PlaybackHandle` belongs to one
`play_to` request; waiting on it returns `PlaybackOutcome.Reached`, Interrupted
or Closed. Reached means the requested position was reached, not that the
timeline was finalized. Replacing a playback request can interrupt the old one.

## Closing and Capturing

`TimelineCloseMode` selects RestoreOrigins, LeaveInPlace or TeleportToTarget.
Python's context manager and default `close()` restore origins. Rust uses an
explicit mode for `close`; Drop requests restoration, but explicit close gives
you the completion result before continuing. Stop all UI requests before close.

This is a **live desktop session**: real icons can be teleported and hidden
during playback even when you only seek. `set_real_icons_visible` is a guarded
timeline operation for comparing Explorer with the overlay, not a general
permission to write flags during playback.

`capture` reads the last submitted overlay frame. `seek_and_capture` performs
pause, seek and capture as one worker operation. Both return `CapturedFrame`,
not a screenshot of other windows or the wallpaper. Use an
[off-screen render session](rendering.md) for exports with no desktop writes.

## Put It to Work

- [Python timeline recipes](../python/guides/timelines.md)
- [Rust lifecycle recipes](../rust/guides/animation-lifecycle.md)