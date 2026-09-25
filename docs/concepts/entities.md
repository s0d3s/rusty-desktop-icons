# Public Entity Map

Related value types share a concept page so their contracts are explained
together. Follow the generated [Python reference](../python/index.md) or
[Rust reference](../rust/index.md) for every field, method and module path.

| Entity or family | Concept and use |
| --- | --- |
| `DesktopController` | [Desktop management](desktop.md): query and coordinate operations |
| `IconSnapshot`, Rust `IconId` | [Desktop management](desktop.md): observed identity and layout |
| Rust `Point`, `Rect` | [Desktop management](desktop.md): coordinates and bounds |
| `DesktopInfo`, `MonitorInfo`, `IconGrid`, Rust `resolve_grid_targets` | [Desktop management](desktop.md): geometry and destination planning |
| `FolderFlag`, `FolderFlagOp`, `FolderFlagOpKind` | [Desktop management](desktop.md): masked Shell preferences and hooks |
| `Curve`, `CurveKind`, Rust `AnimationCurve`, `BuiltinEasing` | [Curves](curves.md): reusable progress functions |
| `KeyframeInterp`, Rust `Keyframe`, `KeyframeCurve` | [Curves](curves.md): authored interpolation |
| `MotionContext` | [Curves](curves.md): build-time motion-aware sampling |
| `Duration`, `DurationKind` | [Durations](curves.md): fixed or distance-based timing |
| `IconAnimationSpec`, `AnimationOptions` | [Animation](animation.md): requests and session choices |
| `AnimationPreset` | [Presets](animation-presets): portable movement, envelope and duration recommendations |
| `PreparedAnimation` | [Preparation](animation.md): reserve and build before starting |
| `AnimationHandle`, `IconAnimationState` | [Execution](execution.md): polling, stopping and waiting |
| `StartContext`, `TickContext`, Rust `PreObservers` | [Execution](execution.md): lightweight worker observers |
| `FinishReason`, `FinishReasonKind`, `StopMode`, `FinalCommitOutcome` | [Execution](execution.md): completion and Shell confirmation |
| `TimelineSession`, `TimelineState`, `TimelineCloseMode` | [Timelines](timelines.md): interactive lifecycle |
| `PlaybackHandle`, `PlaybackOutcome` | [Timelines](timelines.md): one acknowledged traversal |
| `Canvas`, Rust `Scene`, `SceneIcon` | [Rendering](rendering.md): explicit off-screen scene data |
| `RenderSession`, `CapturedFrame` | [Rendering](rendering.md): exact-time pixels |
| Rust `SnapshotFrame`, `SnapshotIconGeometry` | [Rendering](rendering.md): geometry diagnostics |
| `ShaderSource`, `BuiltinShader`, `ShaderPipeline` | [Shaders](shaders/index.md): source and pipeline selection |
| Python `Shader`, Rust `ShaderProgram`, `Effect` | [Shaders](shaders/index.md): compiled code and per-icon values |
| `ProceduralSource`, Rust `ExecutionSource`, `EffectExecution` | [Custom shaders](shaders/custom.md): source/compiled recipes |
| `EffectParameter`, Rust `ParameterKind` | [Custom shaders](shaders/custom.md): typed named values |
| `RenderTarget`, Rust `EffectTarget` | [Custom shaders](shaders/custom.md): intermediate storage |
| `RenderPass`, Rust `PassSource`, `EffectPass`, `DrawSpec`, `DrawTopology`, `PassBlend` | [Custom shaders](shaders/custom.md): pass graphs and geometry |
| Rust `DesktopError`, `CurveError`, Python exception family | [Errors](errors.md): validation and runtime failures |
| `init_logging`, `LogTarget`, `LOGGER_NAME` | [Logging](errors.md): diagnostic routing |
| `__version__`, `__author__` | [Python setup](../python/getting-started/installation.md): package metadata |
| Rust `DesktopBackend`, `WindowsBackend`, `fake::FakeDesktop`, `SceneRenderer` | [Architecture](architecture.md): platform and test integration |
| Rust `IconBitmap`, `IconLabel`, `IconRenderPlan`, `IconFrame`, `OverlayRenderOptions` | [Architecture](architecture.md): prepared artwork and frame delivery |

Python does not mirror every Rust backend extension type. Its scene constructor
takes specs/positions instead of exposing `Scene` and `SceneIcon`, and snapshot
diagnostics return dictionaries instead of native planning structs.