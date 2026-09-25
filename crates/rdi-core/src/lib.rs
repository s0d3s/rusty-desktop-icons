//! Platform-agnostic animation core for `rusty-desktop-icons`.
//!
//! This crate defines the pure-Rust, cross-platform surface of the project:
//!
//! * Geometry primitives ([`Point`]) and icon identifiers ([`IconId`]).
//! * The [`Curve`] system with two variants ([`BuiltinEasing`] and
//!   [`KeyframeCurve`]) that share a single [`AnimationCurve`] contract.
//! * A [`Duration`] policy that resolves to a concrete `std::time::Duration`
//!   either from a fixed value or from movement distance and speed.
//! * The animation specification types ([`IconAnimationSpec`],
//!   [`AnimationOptions`], [`FolderFlagOp`]).
//! * Error types shared by all backends.
//!
//! The Windows Shell/COM backend, the worker thread, and the Python
//! bindings live in sibling crates and depend on this one.
//!
//! Everything in this crate is `#![forbid(unsafe_code)]` — the only place
//! `unsafe` is allowed is inside `rdi-platform-windows`.
//!
//! ## Quick tour of curves
//!
//! ```
//! use rdi_core::{AnimationCurve, Curve, KeyframeInterp};
//!
//! // Predefined easing.
//! let ease = Curve::ease_in_out();
//! assert!((ease.eval(0.5) - 0.5).abs() < 1e-3);
//!
//! // Data-driven keyframes.
//! let keys = Curve::keyframes(
//!     vec![
//!         rdi_core::Keyframe::new(0.0, 0.0),
//!         rdi_core::Keyframe::new(0.5, 0.9),
//!         rdi_core::Keyframe::new(1.0, 1.0),
//!     ],
//!     KeyframeInterp::Linear,
//! ).unwrap();
//! assert!(keys.eval(0.5) > 0.5);
//!
//! // Function-generated curve — sampled once, then pure data at runtime.
//! let damped = Curve::from_curve_fn(
//!     |t| 1.0 - (1.0 - t).powi(3),
//!     32,
//!     KeyframeInterp::Linear,
//! ).unwrap();
//! assert!(damped.eval(1.0) > 0.9);
//! ```

pub mod curve;
pub mod desktop;
pub mod duration;
pub mod effect;
pub mod error;
pub mod geometry;
pub mod id;
pub mod monitor;
pub mod overlay_plan;
pub mod procedural;
pub mod snapshot;
pub mod scene;
pub mod spec;

pub mod backend;
pub mod controller;
pub mod events;
pub mod fake;
pub mod handle;
pub mod timeline;

mod engine;

pub use backend::DesktopBackend;
pub use controller::{DesktopController, PreparedAnimation};
pub use curve::{
    AnimationCurve, BuiltinEasing, Curve, Keyframe, KeyframeCurve, KeyframeInterp, MotionContext,
};
pub use duration::Duration;
pub use desktop::{DesktopInfo, IconGrid, resolve_grid_targets};
pub use effect::{Effect, IconFrame, ShaderPipeline, ShaderProgram};
pub mod effect_graph;
pub use effect_graph::{DrawSpec, DrawTopology, EffectExecution, EffectParameter, EffectPass, EffectTarget, ParameterKind, PassBlend};
pub use error::{CurveError, DesktopError};
pub use events::{
    FinishReason, IconAnimationState, StartContext, StopMode, TickContext,
};
pub use geometry::Point;
pub use handle::{AnimationHandle, PreObservers};
pub use id::IconId;
pub use monitor::{MonitorInfo, Rect};
pub use overlay_plan::{
    FinalCommitOutcome, IconBitmap, IconLabel, IconRenderPlan, OverlayRenderOptions,
    SnapshotFrame, SnapshotIconGeometry,
};
pub use snapshot::IconSnapshot;
pub use scene::{Canvas, CapturedFrame, RenderSession, Scene, SceneIcon, SceneRenderer};
pub use spec::{AnimationOptions, AnimationPreset, FolderFlagOp, IconAnimationSpec};
pub use timeline::{PlaybackHandle, PlaybackOutcome, TimelineCloseMode, TimelineSession, TimelineState};
