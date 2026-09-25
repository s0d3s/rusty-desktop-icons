# Non-blocking Execution and Observers

Starting an animation returns an `AnimationHandle`; the caller can continue
working while the controller's worker advances the animation. This is
thread-based execution, not an asyncio coroutine or Rust `Future`.
Preparation and acknowledged controller commands can still block.

## AnimationHandle

Use `is_running`, `progress` and `snapshot` to inspect state. Each
`IconAnimationState` records an ID, origin, target, current position, normalized
time and whether it is final. A snapshot is a copy, not a mutable control object.

`wait` blocks until completion and cleanup; `wait_timeout` returns no result if
the timeout expires. A timeout does **not** stop playback. `stop` requests a
stop policy; follow it with `wait` before issuing another write. Python releases
the GIL around blocking native waits, but that still blocks the calling thread.
Use a UI timer for polling, or an executor/`asyncio.to_thread` for a blocking wait.

## Events

| Callback | Payload |
| --- | --- |
| `on_start` | `StartContext`: total and missing icon counts |
| `on_tick` | `TickContext`: elapsed time, progress, active/finalized counts |
| `on_icon_complete` | Completed icon ID |
| `on_finish` | `FinishReason` |

Callbacks execute on the animation worker. Keep them short: enqueue a message
for the UI or store a lightweight value. Do not perform slow I/O, render a GUI,
or call a blocking controller/timeline method from them. Python callbacks acquire
the GIL; movement curves do not call Python at tick time.

Observers registered after start can miss earlier events. Rust's `PreObservers`
supports registration before startup through `DesktopController::animate_with_observers`.
Python exposes handle registration after start, so do not depend on its
`on_start` observer to initialize required application state.

## Completion Is More Than a Boolean

`FinishReason` distinguishes completed, stopped (with a `StopMode`), and error
(with a message). Python's `FinishReasonKind` is a string enum; use its members
instead of assuming an exact built-in `str` type. Missing icons may be reported
separately on the handle; treat them according to your layout policy.

`FinalCommitOutcome` reports Shell confirmation: `missing_ids` identifies IDs
still unresolved. `moved_ids` is **not a count of every moved icon**; Windows
confirmation can stop once an icon lands. Successful completion does not prove
physical display cadence or pixel-perfect Explorer matching.

## Put It to Work

- [Python lifecycle recipes](../python/guides/basic-usage.md)
- [Rust lifecycle recipes](../rust/guides/animation-lifecycle.md)
- [Errors and logging](errors.md)