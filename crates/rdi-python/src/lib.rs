//! PyO3 bindings for `rusty-desktop-icons`.
//!
//! The compiled extension lives at `rusty_desktop_icons._rusty_desktop_icons`.
//! The pure-Python re-export package (`python/rusty_desktop_icons/__init__.py`)
//! surfaces the same names one level up.

mod controller;
mod curve;
mod duration;
mod effect;
mod enums;
mod errors;
mod geometry;
mod handle;
mod logging;
mod procedural;
mod scene;
mod spec;
mod timeline;

#[cfg(windows)]
#[pyo3::pyfunction]
fn _folder_flag_catalog() -> Vec<(u32, &'static str, &'static str, &'static str)> {
    rdi_platform_windows::FolderFlag::ALL
        .iter()
        .map(|flag| {
            (
                flag.bits(),
                flag.name().unwrap(),
                flag.title().unwrap(),
                flag.description().unwrap(),
            )
        })
        .collect()
}

#[cfg(windows)]
#[pyo3::pyfunction]
fn _builtin_shader_catalog() -> Vec<(String, &'static str)> {
    rdi_platform_windows::shader::BuiltinShader::ALL
        .iter()
        .map(|shader| (format!("{shader:?}"), shader.name()))
        .collect()
}

/// Rust-powered Windows desktop icon controller and animator.
#[pyo3::pymodule]
mod _rusty_desktop_icons {
    #[cfg(windows)]
    #[pymodule_export]
    use super::_builtin_shader_catalog;
    #[cfg(windows)]
    #[pymodule_export]
    use super::_folder_flag_catalog;
    #[pymodule_export]
    use super::controller::PyDesktopController;
    #[pymodule_export]
    use super::curve::{PyCurve, PyMotionContext};
    #[pymodule_export]
    use super::duration::PyDuration;
    #[pymodule_export]
    use super::effect::{PyEffect, PyPreparedAnimation, PyShader, PyShaderSource};
    #[pymodule_export]
    use super::procedural::{PyEffectParameter, PyRenderTarget, PyRenderPass, PyProceduralSource};
    #[pymodule_export]
    use super::errors::{
        AnimationBusy, BackendUnavailable, ComError, IconNotFound, InvalidCurve,
        InvalidDuration, InvalidEffect, InvalidGrid, RustyDesktopError,
        UnsupportedPlatform, WorkerCrashed,
    };
    #[pymodule_export]
    use super::geometry::{
        PyDesktopInfo, PyFinalCommitOutcome, PyFinishReason, PyIconAnimationState,
        PyIconGrid, PyIconSnapshot, PyMonitorInfo, PyRect, PyStartContext,
        PyStopMode, PyTickContext,
    };
    #[pymodule_export]
    use super::handle::PyAnimationHandle;
    #[pymodule_export]
    use super::scene::{PyCanvas, PyCapturedFrame, PyRenderSession};
    #[pymodule_export]
    use super::logging::init_logging;
    #[pymodule_export]
    use super::spec::{PyAnimationOptions, PyAnimationPreset, PyFolderFlagOp, PyIconAnimationSpec};
    #[pymodule_export]
    use super::timeline::{
        PyPlaybackHandle, PyPlaybackOutcome, PyTimelineCloseMode, PyTimelineSession,
        PyTimelineState,
    };

    #[pymodule_export]
    #[allow(non_upper_case_globals)]
    const __version__: &str = env!("CARGO_PKG_VERSION");
    #[pymodule_export]
    #[allow(non_upper_case_globals)]
    const __author__: &str = "s0d3s";
}
