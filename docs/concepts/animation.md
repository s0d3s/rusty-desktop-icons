# Animation Specifications and Preparation

`IconAnimationSpec` describes one existing icon's destination, `Duration`, X/Y
curves and optional `Effect`. The origin comes from the desktop at preparation;
an [off-screen scene](rendering.md) supplies its own origin instead. Use one
specification per icon and keep IDs unique.

## Options

`AnimationOptions` controls the session rather than a single icon:

| Setting | Purpose |
| --- | --- |
| `tick_hz` | Requested engine cadence, not guaranteed displayed FPS |
| `position_tolerance_px` | Shell position-confirmation tolerance |
| `snap_to_grid` | Reserve available destination cells during preparation |
| `draw_labels` | Include filename artwork |
| `draw_shortcut_overlay` / `draw_shield_overlay` | Include enabled badges |
| `before_flags` / `after_flags` | Explicit folder-flag operations |
| `force_fallback` | Exercise direct-placement fallback |

Artwork choices affect both plain rendering and shader inputs. Destination
planning is independent of [Explorer flags](desktop.md).

## PreparedAnimation

`prepare` validates the request, resolves destinations/durations and loads
artwork and GPU resources. It is a blocking preparation step, not animation
time. It reserves access but does not display an overlay, apply before flags,
or move real icons. Read queries remain available; competing writes/sessions
return an [error](errors.md).

`PreparedAnimation` is one-shot: start it, open a timeline, or cancel it.
Cancelling/dropping an unused preparation releases resources without committing
positions. Python supports a context manager. Rust consumes the prepared value
on start/open. `animate` is the convenience path when no preparation boundary
is needed.

## Choosing a Session

| Need | API |
| --- | --- |
| One transition to completion | `start` -> `AnimationHandle` |
| Seek, pause and reverse repeatedly | `open_timeline` -> `TimelineSession` |
| Export exact frames without desktop writes | `prepare_scene` -> `RenderSession` |

An ordinary animation's icon set, targets, resources and effects are fixed.
There is no in-flight add/remove/retarget API. Stop and wait for cleanup before
preparing a different request. Only one live overlay session can run per process.

## Real Desktop Effects

On Windows, the overlay draws icon copies while real participants are moved
behind it. On ordinary completion, the engine commits final positions and
restores visibility. `StopMode.LeaveInPlace` commits the current interpolated
positions; `TeleportToTarget` chooses the final targets. Neither means restore
origins. Save a layout explicitly or use a timeline's restore close mode.

If an ordinary animation cannot obtain an overlay, it may warn and fall back
to a direct move. An off-screen render is not a substitute for a live overlay
and never enters that write/fallback path. Process termination can bypass
cleanup; do not force-kill live demos.

## Put It to Work

- [Python lifecycle recipes](../python/guides/basic-usage.md)
- [Rust lifecycle recipes](../rust/guides/animation-lifecycle.md)
- [Non-blocking execution](execution.md), [timelines](timelines.md),
  [render sessions](rendering.md)