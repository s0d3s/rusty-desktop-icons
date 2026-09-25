//! Windows Shell/COM backend for `rusty-desktop-icons`.
//!
//! This crate provides [`WindowsBackend`], the concrete
//! [`rdi_core::DesktopBackend`] driving the real Windows desktop through
//! `IFolderView2`. Everything unsafe lives here — `rdi-core` itself
//! `forbid`s `unsafe_code`.
//!
//! # Usage
//!
//! ```no_run
//! use rdi_core::DesktopController;
//! use rdi_platform_windows::WindowsBackend;
//!
//! let ctrl = DesktopController::new(WindowsBackend::new())?;
//! let icons = ctrl.list_icons()?;
//! println!("{} icons on the desktop", icons.len());
//! # Ok::<(), rdi_core::DesktopError>(())
//! ```
//!
//! # Threading
//!
//! `WindowsBackend` initialises COM (STA) lazily on the thread that
//! first calls a `DesktopBackend` method, which — under normal use — is
//! the engine's dedicated worker thread. Do not construct two backends
//! on the same thread.
//!
//! # Modules
//!
//! * [`ids`] — hex-encoded UTF-16 identifier scheme (compatible with
//!   the legacy Python extension).
//! * `com` — `IFolderView2` acquisition and caching (crate-private).
//! * `pidl` — RAII wrappers for shell-allocated memory (crate-private).
//! * `backend` — [`WindowsBackend`] itself (crate-private module, type re-exported).

#![cfg(windows)]
#![deny(unsafe_op_in_unsafe_fn)]

mod backend;
mod com;
mod folder_flags;
pub mod ids;
mod imaging;
mod layout;
mod monitors;
mod overlay;
mod pidl;
pub mod shader;
pub mod shader_library;
mod sprites;
mod text;

pub use backend::WindowsBackend;
pub use folder_flags::FolderFlag;
