//! Routing for the `tracing` events emitted by `rdi-core` and
//! `rdi-platform-windows`.
//!
//! This crate is a **leaf artifact**, so unlike the library crates it is
//! allowed to install a global `tracing` subscriber. Two sinks are
//! offered:
//!
//! * [`LogTarget::Loguru`] (default) — hands each event to
//!   `rusty_desktop_icons._logging.emit`, which does the loguru record
//!   surgery in Python where it is readable and testable. This is what
//!   makes the diagnostics visible to whatever sinks the application has
//!   configured; a raw `eprintln!` writes to the process stderr *file
//!   descriptor* and bypasses every one of them.
//! * [`LogTarget::Stderr`] — a plain `tracing_subscriber` fmt writer.
//!   Cheaper (no GIL round-trip per event) and survives an interpreter
//!   crash, which makes it the right pick when chasing a hard fault.
//!
//! There is deliberately **no stdout option**: stdout carries the
//! JSON-RPC stream for stdio MCP servers, and writing anything else
//! there breaks every client.

use std::sync::OnceLock;

use pyo3::prelude::*;
use pyo3::types::PyDict;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, registry};

/// Environment variable read for the filter directive, in
/// `EnvFilter` syntax (e.g. `RDI_LOG=rdi_platform_windows=debug,warn`).
const ENV_VAR: &str = "RDI_LOG";

/// Pure-Python module holding the loguru emitter.
const EMITTER_MODULE: &str = "rusty_desktop_icons._logging";

/// Set once the global subscriber has been installed. `tracing` only
/// permits one global subscriber per process, so every entry point here
/// is idempotent and later calls only adjust the reloadable filter.
static INSTALLED: OnceLock<()> = OnceLock::new();

/// Handle to the installed filter so `init_logging` can retune the level
/// after the subscriber is already in place.
static RELOAD: OnceLock<tracing_subscriber::reload::Handle<EnvFilter, Registry>> =
    OnceLock::new();

type Registry = tracing_subscriber::Registry;

/// Where the installed subscriber writes.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum LogTarget {
    /// loguru, via the Python emitter module (default).
    Loguru,
    /// Process stderr, via `tracing_subscriber`'s fmt layer.
    Stderr,
}

// ---------------------------------------------------------------------------
// tracing -> loguru layer
// ---------------------------------------------------------------------------

/// Splits an `Event`'s fields into the message and the structured rest.
#[derive(Default)]
struct FieldCollector {
    message: String,
    extras: Vec<(String, String)>,
}

impl Visit for FieldCollector {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            self.extras
                .push((field.name().to_string(), format!("{value:?}")));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.extras
                .push((field.name().to_string(), value.to_string()));
        }
    }
}

/// `tracing` level names as the Python emitter understands them.
///
/// loguru has a real `TRACE` level, so nothing is folded.
fn level_name(level: &Level) -> &'static str {
    match *level {
        Level::ERROR => "ERROR",
        Level::WARN => "WARN",
        Level::INFO => "INFO",
        Level::DEBUG => "DEBUG",
        Level::TRACE => "TRACE",
    }
}

struct LoguruLayer;

impl<S> Layer<S> for LoguruLayer
where
    S: Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut collector = FieldCollector::default();
        event.record(&mut collector);

        // Enclosing span names, so an overlay warning stays attributable
        // to the animation that produced it.
        let mut scope = Vec::new();
        if let Some(span) = ctx.event_span(event) {
            for s in span.scope().from_root() {
                scope.push(s.name().to_string());
            }
        }

        let meta = event.metadata();

        // Acquiring the GIL per event is why nothing in the tick loop may
        // log above `trace!`.
        Python::attach(|py| {
            let result = emit_to_loguru(
                py,
                level_name(meta.level()),
                meta.target(),
                &collector.message,
                &scope.join(">"),
                meta.file(),
                meta.line(),
                &collector.extras,
            );
            if let Err(e) = result {
                // Never propagate: a logging failure must not take down
                // an in-flight animation.
                e.write_unraisable(py, None);
            }
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_to_loguru(
    py: Python<'_>,
    level: &str,
    target: &str,
    message: &str,
    scope: &str,
    file: Option<&str>,
    line: Option<u32>,
    extras: &[(String, String)],
) -> PyResult<()> {
    // Hits `sys.modules` after the first call.
    let module = py.import(EMITTER_MODULE)?;

    let extras_dict = PyDict::new(py);
    for (k, v) in extras {
        extras_dict.set_item(k.as_str(), v.as_str())?;
    }

    let kwargs = PyDict::new(py);
    kwargs.set_item("level", level)?;
    kwargs.set_item("target", target)?;
    kwargs.set_item("message", message)?;
    kwargs.set_item("scope", scope)?;
    kwargs.set_item("file", file)?;
    kwargs.set_item("line", line)?;
    kwargs.set_item("extras", &extras_dict)?;

    module.call_method("emit", (), Some(&kwargs))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Installation
// ---------------------------------------------------------------------------

fn base_filter(level: Option<&str>) -> EnvFilter {
    match level {
        // Explicit level always wins over the environment.
        Some(l) => EnvFilter::new(l),
        None => EnvFilter::try_from_env(ENV_VAR).unwrap_or_else(|_| EnvFilter::new("warn")),
    }
}

/// Install the global subscriber. Idempotent: the first call wins and
/// later calls only retune the filter.
pub(crate) fn install(target: LogTarget, level: Option<&str>) -> bool {
    if INSTALLED.get().is_some() {
        if let Some(handle) = RELOAD.get() {
            let _ = handle.reload(base_filter(level));
        }
        return false;
    }

    let (filter, handle) = tracing_subscriber::reload::Layer::new(base_filter(level));

    let result = match target {
        LogTarget::Loguru => registry().with(filter).with(LoguruLayer).try_init().is_ok(),
        LogTarget::Stderr => registry()
            .with(filter)
            .with(
                tracing_subscriber::fmt::layer()
                    // NEVER stdout — see module docs.
                    .with_writer(std::io::stderr)
                    .with_target(true),
            )
            .try_init()
            .is_ok(),
    };

    if result {
        let _ = INSTALLED.set(());
        let _ = RELOAD.set(handle);
    }
    result
}

// ---------------------------------------------------------------------------
// Python surface
// ---------------------------------------------------------------------------

/// Route this library's diagnostics somewhere useful.
///
/// Called automatically at import time with the defaults, so configuring
/// a loguru sink is usually all you need. Call this explicitly to change
/// the level or switch sinks.
#[pyfunction]
#[pyo3(signature = (level=None, target: "LogTarget | str"="loguru"))]
pub(crate) fn init_logging(level: Option<&str>, target: &str) -> PyResult<bool> {
    let target = match target {
        // "python" is a legacy alias for the loguru sink.
        "loguru" | "python" => LogTarget::Loguru,
        "stderr" => LogTarget::Stderr,
        "stdout" => {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "logging to stdout is not supported: stdout carries the JSON-RPC \
                 stream for stdio MCP servers. Use 'stderr' or 'loguru'.",
            ));
        }
        other => {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "unknown log target {other:?}; expected 'loguru' or 'stderr'"
            )));
        }
    };
    Ok(install(target, level))
}
