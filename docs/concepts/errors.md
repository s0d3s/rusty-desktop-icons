# Errors, Enums and Logging

Rust APIs return `Result` with `DesktopError` (or `CurveError` for curve
construction). Python raises typed subclasses of `RustyDesktopError`.
Do not turn every failure into a silent fallback: invalid input, missing artwork
and a disrupted Shell require different decisions.

| Python exception | Typical action |
| --- | --- |
| `InvalidCurve`, `InvalidDuration`, `InvalidEffect` | Correct input/source before retrying |
| `InvalidGrid` | Inspect grid origin/capacity and revise destinations |
| `IconNotFound` | Re-enumerate, remove stale IDs from the request |
| `AnimationBusy` | Finish/cancel the current reservation before writing |
| `BackendUnavailable`, `ComError` | Preserve diagnostics; check interactive desktop/Shell state |
| `UnsupportedPlatform` | Select a supported platform/backend |
| `WorkerCrashed` | Treat the controller as unavailable and report the failure |

Errors during asynchronous playback arrive through `FinishReason`, not just
the call that started it. Check both startup errors and completion results.
An enumeration failure can include the failing item name, available path and
HRESULT; redact personal names/paths before publishing logs. Native exceptions
preserve their public Python type and message through pickle/process transport.

## Enum Values

Python `CurveKind`, `DurationKind`, `FolderFlagOpKind`, `FinishReasonKind`,
`KeyframeInterp`, `ShaderPipeline`, `BuiltinShader` and `LogTarget` are
string-backed enums. Prefer their members; legacy string inputs remain accepted.
They compare/serialize as strings, but exact `type(value) is str` checks are
inappropriate. Folder flags are a Windows-only `IntFlag`. Stop/timeline enums
are native binding types; do not assume every enum shares string-enum behavior.

## Diagnostics

Rust library crates emit `tracing` events without installing a global subscriber.
Applications own that subscriber. Python routes native events through loguru;
`init_logging` can select a level/`LogTarget`, and `RDI_LOG` controls filtering.
Configure sinks in the application, not in an animation callback. Avoid verbose
per-frame logs when measuring performance. Logging must not pollute stdout when
it carries a protocol such as MCP JSON-RPC.

## Put It to Work

- [Python recipes](../python/guides/basic-usage.md)
- [Rust lifecycle recipes](../rust/guides/animation-lifecycle.md)
- [Completion and observers](execution.md)