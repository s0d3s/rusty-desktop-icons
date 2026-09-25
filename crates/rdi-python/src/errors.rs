//! Python error hierarchy and `DesktopError`/`CurveError` conversions.
//!
//! The Python side sees a small tree of custom exception types rooted at
//! [`RustyDesktopError`]. Every Rust-side [`rdi_core::DesktopError`] is
//! mapped to exactly one of these classes so Python callers can catch
//! them individually or `except RustyDesktopError` as a broad net.

use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;

use rdi_core::{CurveError, DesktopError};

// -- Base + subclass hierarchy ---------------------------------------------

create_exception!(
    rusty_desktop_icons,
    RustyDesktopError,
    PyException,
    "Base class for all errors raised by rusty-desktop-icons."
);

create_exception!(
    rusty_desktop_icons,
    IconNotFound,
    RustyDesktopError,
    "Raised when a requested icon id is not present on the desktop."
);

create_exception!(
    rusty_desktop_icons,
    InvalidCurve,
    RustyDesktopError,
    "Raised when a curve fails validation or a sampled Python callable \
     returned an unusable value."
);

create_exception!(
    rusty_desktop_icons,
    InvalidDuration,
    RustyDesktopError,
    "Raised when a Duration policy cannot be resolved."
);

create_exception!(
    rusty_desktop_icons,
    UnsupportedPlatform,
    RustyDesktopError,
    "Raised when no compiled backend supports the current OS."
);

create_exception!(
    rusty_desktop_icons,
    BackendUnavailable,
    RustyDesktopError,
    "Raised when the desktop backend could not be initialised or acquired."
);

create_exception!(
    rusty_desktop_icons,
    AnimationBusy,
    RustyDesktopError,
    "Raised when a caller asks for exclusive animation access but another \
     animation is already running."
);

create_exception!(
    rusty_desktop_icons,
    WorkerCrashed,
    RustyDesktopError,
    "Raised when the animation worker thread has stopped unexpectedly."
);

create_exception!(
    rusty_desktop_icons,
    ComError,
    RustyDesktopError,
    "Raised when a COM / Win32 call returned a failure HRESULT."
);

// -- Conversion ------------------------------------------------------------

create_exception!(
    rusty_desktop_icons,
    InvalidEffect,
    RustyDesktopError,
    "Invalid shader source or effect configuration."
);
create_exception!(
    rusty_desktop_icons,
    InvalidGrid,
    RustyDesktopError,
    "Unavailable grid geometry or no collision-free destination."
);

/// Convert a [`rdi_core::DesktopError`] into a Python exception with the
/// most specific matching class.
pub fn map_desktop_error(err: DesktopError) -> PyErr {
    let msg = err.to_string();
    match err {
        DesktopError::BackendUnavailable(_) => BackendUnavailable::new_err(msg),
        DesktopError::Com { .. } => ComError::new_err(msg),
        DesktopError::IconNotFound(_) => IconNotFound::new_err(msg),
        DesktopError::InvalidCurve(_) => InvalidCurve::new_err(msg),
        DesktopError::InvalidDuration(_) => InvalidDuration::new_err(msg),
        DesktopError::InvalidGrid(_) => InvalidGrid::new_err(msg),
        DesktopError::InvalidEffect(_) => InvalidEffect::new_err(msg),
        DesktopError::UnsupportedPlatform => UnsupportedPlatform::new_err(msg),
        DesktopError::WorkerCrashed(_) => WorkerCrashed::new_err(msg),
        DesktopError::AnimationBusy => AnimationBusy::new_err(msg),
        // `#[non_exhaustive]` — future variants become the generic base class.
        _ => RustyDesktopError::new_err(msg),
    }
}

/// Convert a [`rdi_core::CurveError`] into a Python exception.
pub fn map_curve_error(err: CurveError) -> PyErr {
    InvalidCurve::new_err(err.to_string())
}
