//! Shared `tracing` setup for the demo binaries.
//!
//! The library crates only depend on the `tracing` facade; somebody has
//! to install a subscriber or every event is silently dropped. For the
//! wheel that is `rdi-python`; for these binaries it is this file.
//!
//! Included via `#[path = "../tracing_init.rs"] mod tracing_init;`
//! because Cargo binaries can't share a module tree any other way.

use tracing_subscriber::EnvFilter;

/// Install a stderr subscriber honouring `RDI_LOG`.
///
/// Defaults to `info` so the demos narrate what they're doing; set
/// `RDI_LOG=debug` or `RDI_LOG=rdi_platform_windows=trace` for detail.
/// Never writes to stdout — that is reserved for the demos' own
/// `println!` output.
pub fn init() {
    let filter = EnvFilter::try_from_env("RDI_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(true)
        .try_init();
}
