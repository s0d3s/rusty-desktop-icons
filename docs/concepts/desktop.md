# Desktop Management

Use `DesktopController` to inspect the desktop, save layouts, apply positions,
or prepare an animation. It owns a worker and platform backend; keep it alive
while its sessions run. Python constructs the Windows backend automatically.
Rust supplies `WindowsBackend` explicitly, or a test/backend implementation.

## Icons and Identity

`IconSnapshot` is a read-only observation: ID, display name, optional filesystem
path, virtual-item flag and position. Python exposes IDs as strings; Rust uses
`IconId`. Treat IDs as opaque keys, not names or paths. Names can collide,
virtual icons may have no path, and snapshots do not update themselves.

Store positions keyed by ID for layout restoration. Re-enumerate before applying
a saved layout; skip/report IDs no longer present. Persistence across deletion,
recreation or a different machine is not guaranteed. The library manages
existing icons; it does not create shortcuts or rename/delete desktop items.

## Geometry and Monitors

`Point` in Rust and `(x, y)` in Python represent physical desktop coordinates.
They may be negative on monitors left of or above the primary display. `Rect`
uses left/top-inclusive, right/bottom-exclusive bounds.

`MonitorInfo` supplies monitor identity, bounds, work area, primary status,
resolution and scale factor. Use the work area when planning destinations so
the taskbar is excluded. `DesktopInfo` combines virtual desktop bounds, monitors
and per-monitor `IconGrid` measurements. Python offers `to_dict()` for reports.

`IconGrid` contains live cell/icon sizes, row/column counts, capacity and an
optional origin. The origin is inferred from occupied positions; an empty or
ambiguous layout can make it unavailable. DPI scale is not a replacement for
live grid spacing. Mixed-DPI and cross-monitor placement remain less tested.

## Direct Placement Versus Animation

`set_positions` performs a batch Shell move and returns unresolved IDs. It is
appropriate for restoring a layout immediately. Check that result and, when
placement matters, re-enumerate to confirm coordinates: Explorer may snap or
rearrange a request. A successful API call is not proof of visible placement.

For a transition, create [animation specifications](animation.md). With
`AnimationOptions.snap_to_grid`, preparation reserves available destination
cells while protecting stationary icons. Rust also exposes
`resolve_grid_targets` for explicit planning. This does not change Explorer's
flags, prevent transit overlap, or silently move an overflowing layout onto a
different monitor.

Position and flag writes are rejected while a preparation, animation, timeline
or render session reserves the controller. Read queries remain available.
Do not use a second controller to evade this coordination.

## Folder Flags

`FolderFlag` names all 33 Microsoft `FOLDERFLAGS` values. Python uses Windows-only
`IntFlag` members such as `FWF_SNAPTOGRID`; Rust uses `FolderFlag::SnapToGrid`
and `.bits()`. Enum presence does not guarantee modern Explorer honors a flag.

| Operation | Effect |
| --- | --- |
| `get_flags` | Read the current unsigned 32-bit word |
| `set_flags` | Set selected bits, preserve others |
| `unset_flags` | Clear selected bits, preserve others |
| `toggle_flags` | Invert selected bits |
| `apply_flags(mask, values)` | Set/clear only masked bits |
| `set_flags_exactly` | Legacy operation: clears only through the highest set bit |

The masked rule is `new = (old & ~mask) | (values & mask)`. A zero mask changes
nothing. `set_flags_exactly(0)` also changes nothing; it is **not** whole-word
replacement. Avoid replacing the entire word just to change one preference.

Auto arrange gives Explorer control of placement. Snap to grid can round manual
positions. Neither is overridden by the engine's destination planner. When
temporarily changing a flag, capture its original state and restore only that
mask in cleanup; do not overwrite unrelated settings changed by the user.

`FolderFlagOp` describes before/after-animation hooks (`Set` and `Exactly` in
Rust, `set` and `exactly` in Python). There is no unset hook. Explicit clearing
must happen before preparation. An after hook is not a substitute for an
application's error/cancellation cleanup policy.

## Put It to Work

- [Python desktop recipes](../python/guides/basic-usage.md)
- [Rust first desktop query](../rust/getting-started/quickstart.md)
- [Python API](../python/index.md) and [Rust API](../rust/index.md)