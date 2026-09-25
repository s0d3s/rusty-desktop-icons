//! Python wrapper for [`rdi_core::DesktopController`].
//!
//! Instantiation picks the compiled backend at compile time — the real
//! Windows Shell backend on Windows, or the always-fails stub elsewhere.

use pyo3::prelude::*;
use pyo3::types::PyDict;

use rdi_core::{DesktopController as CoreController, IconId, Point};

use crate::errors::map_desktop_error;
use crate::geometry::{icon_id_from_any, point_from_any, PyIconSnapshot, PyMonitorInfo};
use crate::handle::PyAnimationHandle;
use crate::spec::{options_from_any, render_options_from_any, specs_from_any};

// ---------------------------------------------------------------------------
// Backend selection
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn build_controller() -> Result<CoreController, rdi_core::DesktopError> {
    use rdi_platform_windows::WindowsBackend;
    CoreController::new(WindowsBackend::new())
}

#[cfg(not(windows))]
fn build_controller() -> Result<CoreController, rdi_core::DesktopError> {
    use rdi_platform_stub::StubBackend;
    CoreController::new(StubBackend::new())
}

// ---------------------------------------------------------------------------
// Wrapper
// ---------------------------------------------------------------------------

/// Top-level entry point for animating the current user's desktop.
///
/// On Windows this drives the real `IFolderView2` COM interface. On other
/// platforms every method raises `UnsupportedPlatform`.
#[pyclass(module = "rusty_desktop_icons", name = "DesktopController")]
pub struct PyDesktopController {
    pub(crate) inner: CoreController,
}

impl Drop for PyDesktopController {
    /// PyO3 deallocates `#[pyclass]` values with the GIL held, and
    /// joining the worker blocks. If that worker is inside an observer
    /// callback it is waiting on the very GIL this thread holds, so the
    /// join has to run detached. `shutdown` leaves the implicit
    /// `CoreController::drop` below with nothing to join.
    fn drop(&mut self) {
        Python::attach(|py| py.detach(|| self.inner.shutdown()));
    }
}

#[pymethods]
impl PyDesktopController {
    /// Spawn the worker thread and connect to the desktop backend.
    #[new]
    fn new() -> PyResult<Self> {
        let inner = build_controller().map_err(map_desktop_error)?;
        Ok(Self { inner })
    }

    /// Snapshot every icon currently on the desktop.
    fn list_icons(&self, py: Python<'_>) -> PyResult<Vec<PyIconSnapshot>> {
        let icons = py
            .detach(|| self.inner.list_icons())
            .map_err(map_desktop_error)?;
        Ok(icons.into_iter().map(PyIconSnapshot::from_inner).collect())
    }

    /// Read the raw desktop folder-view flag word.
    fn get_flags(&self, py: Python<'_>) -> PyResult<u32> {
        py.detach(|| self.inner.get_flags())
            .map_err(map_desktop_error)
    }

    /// Perform a masked update of the desktop folder-view flags.
    ///
    /// `new_flags = (old_flags & !mask) | (values & mask)`
    ///
    /// Raises `AnimationBusy` while an animation is running.
    fn apply_flags(&self, py: Python<'_>, mask: u32, values: u32) -> PyResult<()> {
        py.detach(|| self.inner.apply_flags(mask, values))
            .map_err(map_desktop_error)
    }

    /// OR-set semantics.
    fn set_flags(&self, py: Python<'_>, flags: u32) -> PyResult<()> {
        py.detach(|| self.inner.set_flags(flags))
            .map_err(map_desktop_error)
    }

    /// AND-clear semantics.
    fn unset_flags(&self, py: Python<'_>, flags: u32) -> PyResult<()> {
        py.detach(|| self.inner.unset_flags(flags))
            .map_err(map_desktop_error)
    }

    /// XOR-toggle semantics.
    fn toggle_flags(&self, py: Python<'_>, flags: u32) -> PyResult<()> {
        py.detach(|| self.inner.toggle_flags(flags))
            .map_err(map_desktop_error)
    }

    /// Exactly-set semantics — expands `flags` via `build_true_mask`.
    fn set_flags_exactly(&self, py: Python<'_>, flags: u32) -> PyResult<()> {
        py.detach(|| self.inner.set_flags_exactly(flags))
            .map_err(map_desktop_error)
    }

    /// Instantly move a batch of icons.
    ///
    /// `moves` is an iterable of `(icon_id, (x, y))` pairs (or dicts with
    /// `id` / `position` keys). Returns the list of icon ids the backend
    /// could not resolve. Raises `AnimationBusy` while an animation is
    /// running.
    #[pyo3(signature = (moves: "typing.Iterable[tuple[str, typing.Sequence[int]] | dict[str, object]]"))]
    fn set_positions(
        &self,
        py: Python<'_>,
        moves: &Bound<'_, PyAny>,
    ) -> PyResult<Vec<String>> {
        let mut pairs: Vec<(IconId, Point)> = Vec::new();
        for item in moves.try_iter()? {
            let item = item?;
            pairs.push(extract_move(&item)?);
        }
        let missing = py
            .detach(|| self.inner.set_positions(pairs))
            .map_err(map_desktop_error)?;
        Ok(missing.into_iter().map(|id| id.into_string()).collect())
    }

    /// Prepare artwork and GPU resources without starting the visual clock.
    ///
    /// Scene positions are independent of real desktop coordinates. Every spec must
    /// have exactly one origin; absent desktop icons/artwork fail preparation.
    #[pyo3(signature = (canvas, specs: "typing.Iterable[IconAnimationSpec | dict[str, object]]", positions: "typing.Iterable[tuple[str, typing.Sequence[int]] | dict[str, object]]", options: "AnimationOptions | dict[str, object] | None"=None))]
    fn prepare_scene(slf: Py<Self>, py: Python<'_>, canvas: crate::scene::PyCanvas, specs: &Bound<'_, PyAny>, positions: &Bound<'_, PyAny>, options: Option<&Bound<'_, PyAny>>) -> PyResult<crate::scene::PyRenderSession> {
        let specs = specs_from_any(specs)?;
        let mut origins = std::collections::HashMap::new();
        for item in positions.try_iter()? {
            let (id, point) = extract_move(&item?)?;
            if origins.insert(id, point).is_some() { return Err(pyo3::exceptions::PyValueError::new_err("duplicate origin id")); }
        }
        let mut icons = Vec::with_capacity(specs.len());
        for animation in specs {
            let origin = origins.remove(&animation.id).ok_or_else(|| pyo3::exceptions::PyValueError::new_err("each animation requires exactly one scene origin"))?;
            icons.push(rdi_core::SceneIcon { origin, animation });
        }
        if !origins.is_empty() { return Err(pyo3::exceptions::PyValueError::new_err("origin without an animation spec")); }
        let scene = rdi_core::Scene { canvas: canvas.inner, icons, render_options: render_options_from_any(options)? };
        let owner = slf.borrow(py);
        let controller = &owner.inner;
        let inner = py.detach(|| controller.prepare_scene(scene)).map_err(map_desktop_error)?;
        drop(owner);
        Ok(crate::scene::PyRenderSession { inner, _owner: slf })
    }

    /// Prepare artwork and GPU resources without starting the visual clock.
    #[pyo3(signature = (specs: "typing.Iterable[IconAnimationSpec | dict[str, object]]", options: "AnimationOptions | dict[str, object] | None"=None))]
    fn prepare(slf: Py<Self>, py: Python<'_>, specs: &Bound<'_, PyAny>, options: Option<&Bound<'_, PyAny>>) -> PyResult<crate::effect::PyPreparedAnimation> {
        let specs = specs_from_any(specs)?;
        let options = options_from_any(options)?;
        let owner = slf.borrow(py);
        let controller = &owner.inner;
        let prepared = py.detach(|| controller.prepare(specs, options)).map_err(map_desktop_error)?;
        drop(owner);
        Ok(crate::effect::PyPreparedAnimation { inner: std::sync::Mutex::new(Some(prepared)), _owner: slf })
    }

    /// Start a fixed set of icon animations and return immediately with a handle.
    #[pyo3(signature = (specs: "typing.Iterable[IconAnimationSpec | dict[str, object]]", options: "AnimationOptions | dict[str, object] | None"=None))]
    fn animate(
        &self,
        py: Python<'_>,
        specs: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyAnimationHandle> {
        let specs = specs_from_any(specs)?;
        let opts = options_from_any(options)?;
        let handle = py
            .detach(|| self.inner.animate(specs, opts))
            .map_err(map_desktop_error)?;
        Ok(PyAnimationHandle::from_core(handle))
    }

    /// Enumerate every connected display.
    ///
    /// Returns a list of [`MonitorInfo`] objects whose `bounds` share
    /// the same virtual-screen coordinate space as icon positions.
    /// Callers can pick a monitor and build target positions within
    /// its `work_area` to place icons intelligently on multi-monitor
    /// setups.
    fn list_monitors(&self, py: Python<'_>) -> PyResult<Vec<PyMonitorInfo>> {
        let monitors = py
            .detach(|| self.inner.list_monitors())
            .map_err(map_desktop_error)?;
        Ok(monitors.into_iter().map(PyMonitorInfo::from_inner).collect())
    }

    fn desktop_info(&self, py: Python<'_>) -> PyResult<crate::geometry::PyDesktopInfo> {
        let inner = py.detach(|| self.inner.desktop_info()).map_err(map_desktop_error)?;
        Ok(crate::geometry::PyDesktopInfo { inner })
    }

    /// Look up which monitor contains a given `(x, y)` point.
    ///
    /// Returns `None` if the point falls outside every monitor's
    /// bounds — which can happen for points in the "dead" space
    /// between mismatched-resolution monitors.
    #[pyo3(signature = (point: "typing.Sequence[int]"))]
    fn monitor_for_point(
        &self,
        py: Python<'_>,
        point: &Bound<'_, PyAny>,
    ) -> PyResult<Option<PyMonitorInfo>> {
        let p = point_from_any(point)?;
        let monitors = py
            .detach(|| self.inner.list_monitors())
            .map_err(map_desktop_error)?;
        Ok(monitors
            .into_iter()
            .find(|m| m.contains(p))
            .map(PyMonitorInfo::from_inner))
    }

    /// Render one frame of the overlay off-screen with pixel and geometry data.
    ///
    /// `positions` is an iterable of `(icon_id, (x, y))` pairs (or
    /// dicts with `id` / `position` keys), where `(x, y)` are in
    /// **overlay-local pixel coordinates** — i.e. the top-left of the
    /// snapshot buffer is `(0, 0)`. `dpi_scale` picks the render DPI
    /// (`1.0` → 96 DPI, `2.5` → 240 DPI). `options` accepts the same
    /// three feature toggles as `AnimationOptions` (`draw_labels`,
    /// `draw_shortcut_overlay`, `draw_shield_overlay`).
    ///
    /// Returns a dictionary with ``pixels`` (premultiplied BGRA bytes),
    /// ``width``, ``height`` and ``geometry``. Each geometry entry contains
    /// ``id``, ``icon_rect_px``, ``label_rect_px`` and ``arrow_rect_px``.
    /// The pixel buffer has length ``width * height * 4``. Wrap it with Pillow
    /// using ``Image.frombuffer("RGBA", (width, height), result["pixels"], "raw", "BGRA", 0, 1)``.
    ///
    /// Never touches the display mode or an on-screen window. Call
    /// `list_icons()` first so the backend's PIDL/name caches are
    /// populated; unknown ids render as placeholder tiles.
    #[pyo3(signature = (width, height, dpi_scale, positions: "typing.Iterable[tuple[str, typing.Sequence[int]] | dict[str, object]]", options: "AnimationOptions | dict[str, object] | None"=None) -> "dict[str, object]")]
    fn render_overlay_snapshot(
        &self,
        py: Python<'_>,
        width: u32,
        height: u32,
        dpi_scale: f32,
        positions: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Py<pyo3::types::PyDict>> {
        let mut pairs: Vec<(IconId, Point)> = Vec::new();
        for item in positions.try_iter()? {
            let item = item?;
            pairs.push(extract_move(&item)?);
        }
        let render_options = render_options_from_any(options)?;
        let frame = py
            .detach(|| {
                self.inner
                    .render_overlay_snapshot(width, height, dpi_scale, pairs, render_options)
            })
            .map_err(map_desktop_error)?;

        // Build the geometry list as a Python list-of-dicts. Each
        // element carries the icon id, the D2D icon-bitmap rect,
        // and (when the icon had a label) the DirectWrite-metrics
        // label rect — all in overlay-canvas physical pixels.
        let geometry = pyo3::types::PyList::empty(py);
        for g in &frame.geometry {
            let entry = pyo3::types::PyDict::new(py);
            entry.set_item("id", g.id.as_str())?;
            entry.set_item("icon_rect_px", g.icon_rect_px)?;
            entry.set_item("label_rect_px", g.label_rect_px)?;
            entry.set_item("arrow_rect_px", g.arrow_rect_px)?;
            geometry.append(entry)?;
        }

        let out = pyo3::types::PyDict::new(py);
        out.set_item("pixels", pyo3::types::PyBytes::new(py, &frame.pixels))?;
        out.set_item("width", frame.width)?;
        out.set_item("height", frame.height)?;
        out.set_item("geometry", geometry)?;
        Ok(out.unbind())
    }

    fn __repr__(&self) -> &'static str {
        "DesktopController()"
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn extract_move(item: &Bound<'_, PyAny>) -> PyResult<(IconId, Point)> {
    // Dict form: {"id": ..., "position": (x, y)}
    if let Ok(dict) = item.cast::<PyDict>() {
        let id_obj = dict.get_item("id")?.ok_or_else(|| {
            pyo3::exceptions::PyValueError::new_err(
                "move dict missing 'id'",
            )
        })?;
        let pos_obj = dict.get_item("position")?.ok_or_else(|| {
            pyo3::exceptions::PyValueError::new_err(
                "move dict missing 'position'",
            )
        })?;
        return Ok((icon_id_from_any(&id_obj)?, point_from_any(&pos_obj)?));
    }
    // Tuple form: (id, (x, y))
    let (id_obj, pos_obj): (Bound<'_, PyAny>, Bound<'_, PyAny>) = item.extract().map_err(|_| {
        pyo3::exceptions::PyTypeError::new_err(
            "expected (icon_id, (x, y)) tuple or {'id': ..., 'position': ...} dict",
        )
    })?;
    Ok((icon_id_from_any(&id_obj)?, point_from_any(&pos_obj)?))
}
