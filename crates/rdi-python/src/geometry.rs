//! Python wrappers for geometry, snapshot, event and mode types.
//!
//! Rust structs are exposed directly as `#[pyclass(frozen)]` wherever
//! possible, so Python code can read fields without a round-trip.
//!
//! [`Point`](rdi_core::Point) is **not** wrapped — it round-trips as a
//! plain `(x, y)` tuple to keep the surface ergonomic (see [`point_from_any`]).

use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};

use rdi_core::{
    FinalCommitOutcome, FinishReason, IconAnimationState, IconId, IconSnapshot, MonitorInfo,
    Point, Rect, StartContext, StopMode, TickContext,
};

// ---------------------------------------------------------------------------
// Point <-> (i32, i32) helpers
// ---------------------------------------------------------------------------

/// Extract a [`Point`] from any Python object that yields two integers.
///
/// Accepts `(x, y)` tuples, lists of length 2, or any 2-length iterable of
/// integer-convertible values. Raises `TypeError` otherwise.
pub fn point_from_any(obj: &Bound<'_, PyAny>) -> PyResult<Point> {
    if let Ok(tup) = obj.cast::<PyTuple>() {
        if tup.len() == 2 {
            let x: i32 = tup.get_item(0)?.extract()?;
            let y: i32 = tup.get_item(1)?.extract()?;
            return Ok(Point::new(x, y));
        }
    }
    if let Ok(list) = obj.cast::<PyList>() {
        if list.len() == 2 {
            let x: i32 = list.get_item(0)?.extract()?;
            let y: i32 = list.get_item(1)?.extract()?;
            return Ok(Point::new(x, y));
        }
    }
    // Fallback: any 2-element iterable.
    let items: Vec<i32> = obj.extract().map_err(|_| {
        PyTypeError::new_err(
            "expected a 2-element (x, y) tuple of integers for a Point",
        )
    })?;
    if items.len() != 2 {
        return Err(PyTypeError::new_err(format!(
            "expected a 2-element (x, y) tuple for a Point, got {} elements",
            items.len()
        )));
    }
    Ok(Point::new(items[0], items[1]))
}

// ---------------------------------------------------------------------------
// IconSnapshot
// ---------------------------------------------------------------------------

/// Read-only snapshot of a single desktop icon.
#[pyclass(frozen, skip_from_py_object, module = "rusty_desktop_icons", name = "IconSnapshot")]
#[derive(Clone, Debug)]
pub struct PyIconSnapshot {
    pub(crate) inner: IconSnapshot,
}

impl PyIconSnapshot {
    pub(crate) fn from_inner(inner: IconSnapshot) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyIconSnapshot {
    /// Opaque stable identifier for this icon (safe to JSON-round-trip).
    #[getter]
    fn id(&self) -> &str {
        self.inner.id.as_str()
    }

    /// Human-readable name as shown on the desktop.
    #[getter]
    fn display_name(&self) -> &str {
        &self.inner.display_name
    }

    /// Absolute filesystem path, or `None` for virtual items
    /// (This PC, Recycle Bin, ...).
    #[getter]
    fn path(&self) -> Option<String> {
        self.inner
            .path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
    }

    /// `True` for virtual items that have no filesystem path.
    #[getter]
    fn is_virtual(&self) -> bool {
        self.inner.is_virtual
    }

    /// Current position as a `(x, y)` tuple.
    #[getter]
    fn position(&self) -> (i32, i32) {
        (self.inner.position.x, self.inner.position.y)
    }

    fn __repr__(&self) -> String {
        format!(
            "IconSnapshot(id={:?}, display_name={:?}, position=({}, {}), is_virtual={})",
            self.inner.id.as_str(),
            self.inner.display_name,
            self.inner.position.x,
            self.inner.position.y,
            self.inner.is_virtual
        )
    }
}

// ---------------------------------------------------------------------------
// IconAnimationState
// ---------------------------------------------------------------------------

/// Per-icon animation state captured by [`AnimationHandle.snapshot`].
#[pyclass(frozen, skip_from_py_object, module = "rusty_desktop_icons", name = "IconAnimationState")]
#[derive(Clone, Debug)]
pub struct PyIconAnimationState {
    pub(crate) inner: IconAnimationState,
}

impl PyIconAnimationState {
    pub(crate) fn from_inner(inner: IconAnimationState) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyIconAnimationState {
    #[getter]
    fn id(&self) -> &str {
        self.inner.id.as_str()
    }

    #[getter]
    fn origin(&self) -> (i32, i32) {
        (self.inner.origin.x, self.inner.origin.y)
    }

    #[getter]
    fn current(&self) -> (i32, i32) {
        (self.inner.current.x, self.inner.current.y)
    }

    #[getter]
    fn target(&self) -> (i32, i32) {
        (self.inner.target.x, self.inner.target.y)
    }

    /// Normalized time in `[0, 1]` for this icon.
    #[getter]
    fn t(&self) -> f32 {
        self.inner.t
    }

    /// `True` after the engine's final commit for this icon.
    #[getter]
    fn is_final(&self) -> bool {
        self.inner.is_final
    }

    fn __repr__(&self) -> String {
        format!(
            "IconAnimationState(id={:?}, current=({}, {}), t={:.3}, final={})",
            self.inner.id.as_str(),
            self.inner.current.x,
            self.inner.current.y,
            self.inner.t,
            self.inner.is_final
        )
    }
}

// ---------------------------------------------------------------------------
// StartContext / TickContext
// ---------------------------------------------------------------------------

/// Argument passed to `on_start` callbacks.
#[pyclass(frozen, skip_from_py_object, module = "rusty_desktop_icons", name = "StartContext")]
#[derive(Clone, Debug)]
pub struct PyStartContext {
    #[pyo3(get)]
    pub total_icons: usize,
    #[pyo3(get)]
    pub missing_icons: usize,
}

impl PyStartContext {
    pub(crate) fn from_inner(ctx: &StartContext) -> Self {
        Self {
            total_icons: ctx.total_icons,
            missing_icons: ctx.missing_icons,
        }
    }
}

#[pymethods]
impl PyStartContext {
    fn __repr__(&self) -> String {
        format!(
            "StartContext(total_icons={}, missing_icons={})",
            self.total_icons, self.missing_icons
        )
    }
}

/// Argument passed to `on_tick` callbacks.
#[pyclass(frozen, skip_from_py_object, module = "rusty_desktop_icons", name = "TickContext")]
#[derive(Clone, Debug)]
pub struct PyTickContext {
    /// Wall-clock seconds since the animation started.
    #[pyo3(get)]
    pub elapsed_seconds: f64,
    /// Global progress in `[0, 1]`.
    #[pyo3(get)]
    pub progress: f32,
    /// Number of icons still moving.
    #[pyo3(get)]
    pub active_icons: usize,
    /// Number of icons that have reached their target.
    #[pyo3(get)]
    pub finalized_icons: usize,
}

impl PyTickContext {
    pub(crate) fn from_inner(ctx: &TickContext) -> Self {
        Self {
            elapsed_seconds: ctx.elapsed.as_secs_f64(),
            progress: ctx.progress,
            active_icons: ctx.active_icons,
            finalized_icons: ctx.finalized_icons,
        }
    }
}

#[pymethods]
impl PyTickContext {
    fn __repr__(&self) -> String {
        format!(
            "TickContext(elapsed_seconds={:.3}, progress={:.3}, active_icons={}, finalized_icons={})",
            self.elapsed_seconds, self.progress, self.active_icons, self.finalized_icons
        )
    }
}

// ---------------------------------------------------------------------------
// Modes and finish reason
// ---------------------------------------------------------------------------

/// How an animation should leave the desktop when
/// [`AnimationHandle.stop`] is invoked.
#[pyclass(eq, eq_int, from_py_object, module = "rusty_desktop_icons", name = "StopMode")]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PyStopMode {
    LeaveInPlace,
    TeleportToTarget,
}

impl PyStopMode {
    pub(crate) fn to_core(self) -> StopMode {
        match self {
            Self::LeaveInPlace => StopMode::LeaveInPlace,
            Self::TeleportToTarget => StopMode::TeleportToTarget,
        }
    }

    pub(crate) fn from_core(m: StopMode) -> Self {
        match m {
            StopMode::LeaveInPlace => Self::LeaveInPlace,
            StopMode::TeleportToTarget => Self::TeleportToTarget,
        }
    }
}

/// Why an animation ended.
///
/// `kind` is a string-compatible `FinishReasonKind` member.
/// For `"stopped"`, `stop_mode` carries the requested [`StopMode`].
/// For `"error"`, `message` carries a human-readable description.
#[pyclass(frozen, skip_from_py_object, module = "rusty_desktop_icons", name = "FinishReason")]
#[derive(Clone, Debug)]
pub struct PyFinishReason {
    pub kind: &'static str,
    #[pyo3(get)]
    pub stop_mode: Option<PyStopMode>,
    #[pyo3(get)]
    pub message: Option<String>,
}

impl PyFinishReason {
    pub(crate) fn from_core(reason: FinishReason) -> Self {
        match reason {
            FinishReason::Completed => Self {
                kind: "completed",
                stop_mode: None,
                message: None,
            },
            FinishReason::Stopped(mode) => Self {
                kind: "stopped",
                stop_mode: Some(PyStopMode::from_core(mode)),
                message: None,
            },
            FinishReason::Error(msg) => Self {
                kind: "error",
                stop_mode: None,
                message: Some(msg),
            },
        }
    }
}

#[pymethods]
impl PyFinishReason {
    #[getter]
    fn kind(&self) -> crate::enums::FinishReasonKind {
        crate::enums::FinishReasonKind(self.kind)
    }

    fn __repr__(&self) -> String {
        match self.kind {
            "stopped" => format!(
                "FinishReason(kind='stopped', stop_mode={:?})",
                self.stop_mode
            ),
            "error" => format!(
                "FinishReason(kind='error', message={:?})",
                self.message.as_deref().unwrap_or("")
            ),
            _ => "FinishReason(kind='completed')".into(),
        }
    }
}

// ---------------------------------------------------------------------------
// FinalCommitOutcome
// ---------------------------------------------------------------------------

/// Result of the Shell-side final commit at the end of an animation.
///
/// Returned by [`AnimationHandle.final_commit`]; `None` there means the
/// outcome is unknown (still running, or finalization itself failed).
///
/// `moved_ids` is a **confirmation signal, not a census** — the Windows
/// backend stops polling the Shell at the first icon that lands, so a
/// successful 200-icon commit reports a single id. Non-empty means "the
/// commit reached the Shell".
///
/// `missing_ids` **is** complete: every id listed there could not be
/// resolved and was left wherever the Shell last had it.
#[pyclass(
    frozen,
    skip_from_py_object,
    module = "rusty_desktop_icons",
    name = "FinalCommitOutcome"
)]
#[derive(Clone, Debug)]
pub struct PyFinalCommitOutcome {
    #[pyo3(get)]
    pub moved_ids: Vec<String>,
    #[pyo3(get)]
    pub missing_ids: Vec<String>,
}

impl PyFinalCommitOutcome {
    pub(crate) fn from_core(outcome: FinalCommitOutcome) -> Self {
        Self {
            moved_ids: outcome
                .moved_ids
                .into_iter()
                .map(|id| id.into_string())
                .collect(),
            missing_ids: outcome
                .missing_ids
                .into_iter()
                .map(|id| id.into_string())
                .collect(),
        }
    }
}

#[pymethods]
impl PyFinalCommitOutcome {
    fn __repr__(&self) -> String {
        format!(
            "FinalCommitOutcome(moved_ids={} confirmed, missing_ids={})",
            self.moved_ids.len(),
            self.missing_ids.len()
        )
    }
}

// ---------------------------------------------------------------------------
// IconId conversion helpers
// ---------------------------------------------------------------------------

/// Extract an [`IconId`] from a Python string.
pub fn icon_id_from_any(obj: &Bound<'_, PyAny>) -> PyResult<IconId> {
    let s: String = obj.extract().map_err(|_| {
        PyTypeError::new_err("expected an icon id string (as returned by list_icons)")
    })?;
    Ok(IconId::from(s))
}

// ---------------------------------------------------------------------------
// Rect / MonitorInfo
// ---------------------------------------------------------------------------

/// Axis-aligned pixel rectangle in virtual-screen coordinates.
///
/// Uses inclusive-left / exclusive-right / inclusive-top / exclusive-
/// bottom (matches Windows `RECT`).
#[pyclass(frozen, skip_from_py_object, module = "rusty_desktop_icons", name = "Rect")]
#[derive(Clone, Debug)]
pub struct PyRect {
    pub(crate) inner: Rect,
}

impl PyRect {
    pub(crate) fn from_inner(inner: Rect) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyRect {
    #[getter]
    fn left(&self) -> i32 {
        self.inner.left
    }
    #[getter]
    fn top(&self) -> i32 {
        self.inner.top
    }
    #[getter]
    fn right(&self) -> i32 {
        self.inner.right
    }
    #[getter]
    fn bottom(&self) -> i32 {
        self.inner.bottom
    }
    #[getter]
    fn width(&self) -> i32 {
        self.inner.width()
    }
    #[getter]
    fn height(&self) -> i32 {
        self.inner.height()
    }

    /// `True` if the given `(x, y)` point lies inside this rectangle
    /// (inclusive of `left`/`top`, exclusive of `right`/`bottom`).
    #[pyo3(signature = (point: "typing.Sequence[int]"))]
    fn contains(&self, point: &Bound<'_, PyAny>) -> PyResult<bool> {
        let p = point_from_any(point)?;
        Ok(self.inner.contains(p))
    }

    /// `(left, top, right, bottom)` — matches Windows `RECT` layout.
    fn as_tuple(&self) -> (i32, i32, i32, i32) {
        (self.inner.left, self.inner.top, self.inner.right, self.inner.bottom)
    }

    fn __repr__(&self) -> String {
        format!(
            "Rect(left={}, top={}, right={}, bottom={})",
            self.inner.left, self.inner.top, self.inner.right, self.inner.bottom,
        )
    }
}

/// A single connected display in the virtual-screen coordinate system.
///
/// See [`DesktopController.list_monitors`]. Bounds share the same
/// coordinate space as icon positions.
#[pyclass(frozen, skip_from_py_object, module = "rusty_desktop_icons", name = "MonitorInfo")]
#[derive(Clone, Debug)]
pub struct PyMonitorInfo {
    pub(crate) inner: MonitorInfo,
}

impl PyMonitorInfo {
    pub(crate) fn from_inner(inner: MonitorInfo) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyMonitorInfo {
    /// Stable-for-session identifier for the Windows display device.
    #[getter]
    fn id(&self) -> &str {
        &self.inner.id
    }

    /// Human-readable label.
    #[getter]
    fn name(&self) -> &str {
        &self.inner.name
    }

    /// Full monitor bounds in virtual-screen coordinates.
    #[getter]
    fn bounds(&self) -> PyRect {
        PyRect::from_inner(self.inner.bounds)
    }

    /// Bounds excluding the taskbar and other reserved-space widgets.
    #[getter]
    fn work_area(&self) -> PyRect {
        PyRect::from_inner(self.inner.work_area)
    }

    /// `True` for the monitor that owns virtual coordinate `(0, 0)`.
    #[getter]
    fn is_primary(&self) -> bool {
        self.inner.is_primary
    }

    /// Effective DPI scale factor (`1.0` = 96 DPI).
    #[getter]
    fn scale_factor(&self) -> f32 {
        self.inner.scale_factor
    }

    #[getter]
    fn resolution(&self) -> (i32, i32) {
        let size = self.inner.resolution();
        (size.x, size.y)
    }

    #[getter]
    fn dpi(&self) -> f32 { self.inner.dpi() }

    #[pyo3(signature = () -> "dict[str, object]")]
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let result = PyDict::new(py);
        result.set_item("id", self.id())?;
        result.set_item("name", self.name())?;
        result.set_item("bounds", self.bounds().as_tuple())?;
        result.set_item("work_area", self.work_area().as_tuple())?;
        result.set_item("is_primary", self.is_primary())?;
        result.set_item("scale_factor", self.scale_factor())?;
        result.set_item("resolution", self.resolution())?;
        result.set_item("dpi", self.dpi())?;
        Ok(result)
    }

    /// `True` if the given `(x, y)` point lies inside this monitor's
    /// full bounds.
    #[pyo3(signature = (point: "typing.Sequence[int]"))]
    fn contains(&self, point: &Bound<'_, PyAny>) -> PyResult<bool> {
        let p = point_from_any(point)?;
        Ok(self.inner.contains(p))
    }

    fn __repr__(&self) -> String {
        format!(
            "MonitorInfo(id={:?}, primary={}, bounds=({}, {}, {}, {}), scale={:.2})",
            self.inner.id,
            self.inner.is_primary,
            self.inner.bounds.left,
            self.inner.bounds.top,
            self.inner.bounds.right,
            self.inner.bounds.bottom,
            self.inner.scale_factor,
        )
    }
}

#[pyclass(frozen, skip_from_py_object, module = "rusty_desktop_icons", name = "IconGrid")]
#[derive(Clone, Debug)]
pub struct PyIconGrid {
    pub(crate) inner: rdi_core::IconGrid,
}

#[pymethods]
impl PyIconGrid {
    #[getter]
    fn monitor_id(&self) -> &str { &self.inner.monitor_id }
    #[getter]
    fn work_area(&self) -> PyRect { PyRect::from_inner(self.inner.work_area) }
    #[getter]
    fn icon_size(&self) -> (i32, i32) { (self.inner.icon_size.x, self.inner.icon_size.y) }
    #[getter]
    fn cell_size(&self) -> (i32, i32) { (self.inner.cell_size.x, self.inner.cell_size.y) }
    #[getter]
    fn origin(&self) -> Option<(i32, i32)> { self.inner.origin.map(|point| (point.x, point.y)) }
    #[getter]
    fn origin_inferred(&self) -> bool { self.inner.origin_inferred }
    #[getter]
    fn columns(&self) -> u32 { self.inner.columns }
    #[getter]
    fn rows(&self) -> u32 { self.inner.rows }
    #[getter]
    fn capacity(&self) -> u64 { u64::from(self.inner.columns) * u64::from(self.inner.rows) }

    #[pyo3(signature = () -> "dict[str, object]")]
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let result = PyDict::new(py);
        result.set_item("monitor_id", self.monitor_id())?;
        result.set_item("work_area", self.work_area().as_tuple())?;
        result.set_item("icon_size", self.icon_size())?;
        result.set_item("cell_size", self.cell_size())?;
        result.set_item("origin", self.origin())?;
        result.set_item("origin_inferred", self.origin_inferred())?;
        result.set_item("columns", self.columns())?;
        result.set_item("rows", self.rows())?;
        result.set_item("capacity", self.capacity())?;
        Ok(result)
    }

    fn __repr__(&self) -> String {
        format!("IconGrid(monitor_id={:?}, cell_size={:?}, origin={:?}, columns={}, rows={})",
            self.monitor_id(), self.cell_size(), self.origin(), self.columns(), self.rows())
    }
}

#[pyclass(frozen, skip_from_py_object, module = "rusty_desktop_icons", name = "DesktopInfo")]
#[derive(Clone, Debug)]
pub struct PyDesktopInfo {
    pub(crate) inner: rdi_core::DesktopInfo,
}

#[pymethods]
impl PyDesktopInfo {
    #[getter]
    fn bounds(&self) -> PyRect { PyRect::from_inner(self.inner.bounds) }
    #[getter]
    fn monitors(&self) -> Vec<PyMonitorInfo> {
        self.inner.monitors.iter().cloned().map(PyMonitorInfo::from_inner).collect()
    }
    #[getter]
    fn grids(&self) -> Vec<PyIconGrid> {
        self.inner.grids.iter().cloned().map(|inner| PyIconGrid { inner }).collect()
    }

    #[pyo3(signature = () -> "dict[str, object]")]
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let result = PyDict::new(py);
        result.set_item("bounds", self.bounds().as_tuple())?;
        result.set_item("monitors", self.monitors().iter().map(|monitor| monitor.to_dict(py)).collect::<PyResult<Vec<_>>>()?)?;
        result.set_item("grids", self.grids().iter().map(|grid| grid.to_dict(py)).collect::<PyResult<Vec<_>>>()?)?;
        Ok(result)
    }

    fn __repr__(&self) -> String {
        format!("DesktopInfo(monitors={}, grids={})", self.inner.monitors.len(), self.inner.grids.len())
    }
}
