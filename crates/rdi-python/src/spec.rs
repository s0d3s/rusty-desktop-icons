//! Python wrappers for [`IconAnimationSpec`], [`AnimationOptions`] and
//! [`FolderFlagOp`].
//!
//! Specs can be constructed programmatically in Python or built from a
//! plain `dict` at [`DesktopController.animate`] time — the latter is
//! important for the MCP layer (see PLAN §6.1).

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyType};

use rdi_core::{
    AnimationOptions as CoreAnimationOptions, FolderFlagOp as CoreFolderFlagOp,
    IconAnimationSpec as CoreSpec, OverlayRenderOptions as CoreOverlayRenderOptions,
};

use crate::curve::PyCurve;
use crate::duration::PyDuration;
use crate::geometry::{icon_id_from_any, point_from_any};

/// Platform-independent movement, strength envelope and duration values.
///
/// Defaults are ease-in-out movement, smooth 15 percent strength fades and
/// two seconds. Pass fields separately to IconAnimationSpec and Effect;
/// presets never change constructor defaults or compile shaders.
#[pyclass(frozen, from_py_object, module = "rusty_desktop_icons", name = "AnimationPreset")]
#[derive(Clone, Debug)]
pub struct PyAnimationPreset {
    inner: rdi_core::AnimationPreset,
}

#[pymethods]
impl PyAnimationPreset {
    #[new]
    #[pyo3(signature = (*, movement=None, envelope=None, duration=None))]
    fn new(movement: Option<PyCurve>, envelope: Option<PyCurve>, duration: Option<PyDuration>) -> Self {
        let defaults = rdi_core::AnimationPreset::default();
        Self {
            inner: rdi_core::AnimationPreset {
                movement: movement.map_or(defaults.movement, |curve| curve.inner),
                envelope: envelope.map_or(defaults.envelope, |curve| curve.inner),
                duration: duration.map_or(defaults.duration, |duration| duration.inner),
            },
        }
    }

    /// Look up recommendations from the Windows shader catalog without compilation.
    /// Raises UnsupportedPlatform on other systems; AnimationPreset() is portable.
    #[classmethod]
    #[pyo3(signature = (name: "BuiltinShader | str"))]
    fn builtin(_cls: &Bound<'_, PyType>, name: &str) -> PyResult<Self> {
        #[cfg(windows)]
        {
            let shader = name.parse::<rdi_platform_windows::shader::BuiltinShader>()
                .map_err(crate::errors::map_desktop_error)?;
            Ok(Self { inner: shader.default_preset() })
        }
        #[cfg(not(windows))]
        {
            let _ = name;
            Err(crate::errors::map_desktop_error(rdi_core::DesktopError::UnsupportedPlatform))
        }
    }

    #[getter]
    fn movement(&self) -> PyCurve {
        PyCurve::from_inner(self.inner.movement.clone())
    }

    #[getter]
    fn envelope(&self) -> PyCurve {
        PyCurve::from_inner(self.inner.envelope.clone())
    }

    #[getter]
    fn duration(&self) -> PyDuration {
        PyDuration::from_inner(self.inner.duration)
    }
}

// ---------------------------------------------------------------------------
// FolderFlagOp
// ---------------------------------------------------------------------------

/// Folder-flag mutation mode used in
/// [`AnimationOptions.before_flags`] / `after_flags`.
///
/// Construct via [`FolderFlagOp.set`] (OR-set) or
/// [`FolderFlagOp.exactly`] (mask-set).
#[pyclass(frozen, from_py_object, module = "rusty_desktop_icons", name = "FolderFlagOp")]
#[derive(Clone, Copy, Debug)]
pub struct PyFolderFlagOp {
    pub(crate) inner: CoreFolderFlagOp,
}

impl PyFolderFlagOp {
    pub(crate) fn to_core(self) -> CoreFolderFlagOp {
        self.inner
    }
}

#[pymethods]
impl PyFolderFlagOp {
    /// OR-set semantics — matches the legacy `set_desktop_flags`.
    #[classmethod]
    fn set(_cls: &Bound<'_, PyType>, flags: u32) -> Self {
        Self {
            inner: CoreFolderFlagOp::Set(flags),
        }
    }

    /// Exactly-set semantics — matches the legacy
    /// `exactly_set_desktop_flags`.
    #[classmethod]
    fn exactly(_cls: &Bound<'_, PyType>, flags: u32) -> Self {
        Self {
            inner: CoreFolderFlagOp::Exactly(flags),
        }
    }

    /// Operation kind, also accepted in dict and tuple flag operations.
    #[getter]
    fn kind(&self) -> crate::enums::FolderFlagOpKind {
        let value = match self.inner {
            CoreFolderFlagOp::Set(_) => "set",
            CoreFolderFlagOp::Exactly(_) => "exactly",
        };
        crate::enums::FolderFlagOpKind(value)
    }

    /// Flag word.
    #[getter]
    fn flags(&self) -> u32 {
        match self.inner {
            CoreFolderFlagOp::Set(f) | CoreFolderFlagOp::Exactly(f) => f,
        }
    }

    fn __repr__(&self) -> String {
        match self.inner {
            CoreFolderFlagOp::Set(f) => format!("FolderFlagOp.set(0x{f:08x})"),
            CoreFolderFlagOp::Exactly(f) => format!("FolderFlagOp.exactly(0x{f:08x})"),
        }
    }
}

/// Accept `PyFolderFlagOp`, a `(kind, flags)` tuple, or a
/// `{"kind": ..., "flags": ...}` dict.
pub(crate) fn folder_flag_op_from_any(
    obj: &Bound<'_, PyAny>,
) -> PyResult<CoreFolderFlagOp> {
    if let Ok(op) = obj.extract::<PyFolderFlagOp>() {
        return Ok(op.to_core());
    }
    if let Ok(dict) = obj.cast::<PyDict>() {
        let kind: String = dict
            .get_item("kind")?
            .ok_or_else(|| PyValueError::new_err("folder flag op dict missing 'kind'"))?
            .extract()?;
        let flags: u32 = dict
            .get_item("flags")?
            .ok_or_else(|| PyValueError::new_err("folder flag op dict missing 'flags'"))?
            .extract()?;
        return match kind.as_str() {
            "set" => Ok(CoreFolderFlagOp::Set(flags)),
            "exactly" => Ok(CoreFolderFlagOp::Exactly(flags)),
            other => Err(PyValueError::new_err(format!(
                "unknown folder flag op kind: {other:?} (expected 'set' or 'exactly')"
            ))),
        };
    }
    if let Ok((kind, flags)) = obj.extract::<(String, u32)>() {
        return match kind.as_str() {
            "set" => Ok(CoreFolderFlagOp::Set(flags)),
            "exactly" => Ok(CoreFolderFlagOp::Exactly(flags)),
            other => Err(PyValueError::new_err(format!(
                "unknown folder flag op kind: {other:?} (expected 'set' or 'exactly')"
            ))),
        };
    }
    Err(PyTypeError::new_err(
        "expected FolderFlagOp, (kind, flags) tuple, or {'kind':..,'flags':..} dict",
    ))
}

// ---------------------------------------------------------------------------
// IconAnimationSpec
// ---------------------------------------------------------------------------

/// Per-icon animation description.
///
/// May be constructed programmatically or, when passed to
/// [`DesktopController.animate`], as a plain dict with keys `id`,
/// `target`, `duration`, `curve_x` / `curve_y` (or `curve` for both axes).
#[pyclass(frozen, from_py_object, module = "rusty_desktop_icons", name = "IconAnimationSpec")]
#[derive(Clone, Debug)]
pub struct PyIconAnimationSpec {
    pub(crate) inner: CoreSpec,
}

impl PyIconAnimationSpec {
    pub(crate) fn to_core(&self) -> CoreSpec {
        self.inner.clone()
    }
}

#[pymethods]
impl PyIconAnimationSpec {
    /// Build a spec with a single curve applied to both axes.
    #[new]
    #[pyo3(signature = (id: "str", target: "typing.Sequence[int]", duration, curve, curve_y=None, *, effect=None))]
    fn new(
        id: &Bound<'_, PyAny>,
        target: &Bound<'_, PyAny>,
        duration: PyDuration,
        curve: PyCurve,
        curve_y: Option<PyCurve>,
        effect: Option<crate::effect::PyEffect>,
    ) -> PyResult<Self> {
        let id_ = icon_id_from_any(id)?;
        let tgt = point_from_any(target)?;
        let mut inner = if let Some(cy) = curve_y {
            CoreSpec::with_axes(id_, tgt, duration.inner, curve.inner, cy.inner)
        } else {
            CoreSpec::new(id_, tgt, duration.inner, curve.inner)
        };
        inner.effect = effect.map(|effect| effect.inner);
        Ok(Self { inner })
    }

    #[getter]
    fn id(&self) -> &str {
        self.inner.id.as_str()
    }

    #[getter]
    fn target(&self) -> (i32, i32) {
        (self.inner.target.x, self.inner.target.y)
    }

    fn __repr__(&self) -> String {
        format!(
            "IconAnimationSpec(id={:?}, target=({}, {}))",
            self.inner.id.as_str(),
            self.inner.target.x,
            self.inner.target.y
        )
    }
}

/// Accept `PyIconAnimationSpec` or a dict; convert to
/// [`CoreSpec`](rdi_core::IconAnimationSpec).
pub(crate) fn spec_from_any(obj: &Bound<'_, PyAny>) -> PyResult<CoreSpec> {
    if let Ok(spec) = obj.extract::<PyIconAnimationSpec>() {
        return Ok(spec.to_core());
    }
    let dict: Bound<'_, PyDict> = obj.extract().map_err(|_| {
        PyTypeError::new_err(
            "expected IconAnimationSpec or a dict with keys id/target/duration/curve[/curve_y]",
        )
    })?;

    let id_obj = dict
        .get_item("id")?
        .ok_or_else(|| PyValueError::new_err("spec dict missing 'id'"))?;
    let target_obj = dict
        .get_item("target")?
        .ok_or_else(|| PyValueError::new_err("spec dict missing 'target'"))?;
    let duration_obj = dict
        .get_item("duration")?
        .ok_or_else(|| PyValueError::new_err("spec dict missing 'duration'"))?;

    let curve_x_obj = dict.get_item("curve_x")?;
    let curve_y_obj = dict.get_item("curve_y")?;
    let curve_obj = dict.get_item("curve")?;

    let id = icon_id_from_any(&id_obj)?;
    let target = point_from_any(&target_obj)?;
    let duration: PyDuration = duration_obj.extract().map_err(|_| {
        PyTypeError::new_err("spec 'duration' must be a Duration instance")
    })?;

    let (cx, cy) = match (curve_x_obj, curve_y_obj, curve_obj) {
        (Some(x), Some(y), _) => {
            let cx: PyCurve = x.extract()?;
            let cy: PyCurve = y.extract()?;
            (cx.inner, cy.inner)
        }
        (Some(x), None, _) => {
            let cx: PyCurve = x.extract()?;
            let clone = cx.inner.clone();
            (cx.inner, clone)
        }
        (None, Some(y), _) => {
            let cy: PyCurve = y.extract()?;
            let clone = cy.inner.clone();
            (clone, cy.inner)
        }
        (None, None, Some(c)) => {
            let c: PyCurve = c.extract()?;
            let clone = c.inner.clone();
            (c.inner, clone)
        }
        (None, None, None) => {
            return Err(PyValueError::new_err(
                "spec dict must contain 'curve' or 'curve_x'/'curve_y'",
            ));
        }
    };

    let mut spec = CoreSpec::with_axes(id, target, duration.inner, cx, cy);
    if let Some(effect) = dict.get_item("effect")? {
        if !effect.is_none() { spec.effect = Some(effect.extract::<crate::effect::PyEffect>()?.inner); }
    }
    Ok(spec)
}

/// Turn any Python iterable (list, tuple, generator) of specs into a
/// `Vec<CoreSpec>`.
pub(crate) fn specs_from_any(obj: &Bound<'_, PyAny>) -> PyResult<Vec<CoreSpec>> {
    let mut out = Vec::new();
    for item in obj.try_iter()? {
        out.push(spec_from_any(&item?)?);
    }
    if out.is_empty() {
        return Err(PyValueError::new_err(
            "animate(): specs iterable must not be empty",
        ));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// AnimationOptions
// ---------------------------------------------------------------------------

/// Global animation options.
///
/// All fields are optional; pass `None` (or omit) to keep the engine
/// default. Also accepted as a plain dict at [`DesktopController.animate`].
#[pyclass(frozen, from_py_object, module = "rusty_desktop_icons", name = "AnimationOptions")]
#[derive(Clone, Debug, Default)]
pub struct PyAnimationOptions {
    pub(crate) inner: CoreAnimationOptions,
}

impl PyAnimationOptions {
    pub(crate) fn to_core(&self) -> CoreAnimationOptions {
        self.inner
    }
}

#[pymethods]
impl PyAnimationOptions {
    #[new]
    #[pyo3(signature = (
        tick_hz=None,
        position_tolerance_px=None,
        before_flags: "FolderFlagOp | tuple[str, int] | dict[str, object] | None"=None,
        after_flags: "FolderFlagOp | tuple[str, int] | dict[str, object] | None"=None,
        force_fallback=false,
        draw_labels=true,
        draw_shortcut_overlay=true,
        draw_shield_overlay=true,
        *,
        snap_to_grid=false,
    ))]
    fn new(
        tick_hz: Option<u32>,
        position_tolerance_px: Option<i32>,
        before_flags: Option<&Bound<'_, PyAny>>,
        after_flags: Option<&Bound<'_, PyAny>>,
        force_fallback: bool,
        draw_labels: bool,
        draw_shortcut_overlay: bool,
        draw_shield_overlay: bool,
        snap_to_grid: bool,
    ) -> PyResult<Self> {
        let before = before_flags
            .map(folder_flag_op_from_any)
            .transpose()?;
        let after = after_flags.map(folder_flag_op_from_any).transpose()?;
        Ok(Self {
            inner: CoreAnimationOptions {
                snap_to_grid,
                tick_hz,
                position_tolerance_px,
                before_flags: before,
                after_flags: after,
                force_fallback,
                render_options: CoreOverlayRenderOptions {
                    draw_labels,
                    draw_shortcut_overlay,
                    draw_shield_overlay,
                },
            },
        })
    }

    #[getter]
    fn snap_to_grid(&self) -> bool {
        self.inner.snap_to_grid
    }
    #[getter]
    fn tick_hz(&self) -> Option<u32> {
        self.inner.tick_hz
    }
    #[getter]
    fn position_tolerance_px(&self) -> Option<i32> {
        self.inner.position_tolerance_px
    }
    #[getter]
    fn force_fallback(&self) -> bool {
        self.inner.force_fallback
    }
    #[getter]
    fn draw_labels(&self) -> bool {
        self.inner.render_options.draw_labels
    }
    #[getter]
    fn draw_shortcut_overlay(&self) -> bool {
        self.inner.render_options.draw_shortcut_overlay
    }
    #[getter]
    fn draw_shield_overlay(&self) -> bool {
        self.inner.render_options.draw_shield_overlay
    }

    fn __repr__(&self) -> String {
        format!(
            "AnimationOptions(tick_hz={:?}, position_tolerance_px={:?}, \
             force_fallback={}, draw_labels={}, \
             draw_shortcut_overlay={}, draw_shield_overlay={}, snap_to_grid={})",
            self.inner.tick_hz,
            self.inner.position_tolerance_px,
            self.inner.force_fallback,
            self.inner.render_options.draw_labels,
            self.inner.render_options.draw_shortcut_overlay,
            self.inner.render_options.draw_shield_overlay,
            self.inner.snap_to_grid,
        )
    }
}

/// Accept `None`, `PyAnimationOptions`, or a dict.
pub(crate) fn options_from_any(
    obj: Option<&Bound<'_, PyAny>>,
) -> PyResult<CoreAnimationOptions> {
    let Some(obj) = obj else {
        return Ok(CoreAnimationOptions::default());
    };
    if obj.is_none() {
        return Ok(CoreAnimationOptions::default());
    }
    if let Ok(opts) = obj.extract::<PyAnimationOptions>() {
        return Ok(opts.to_core());
    }
    let dict: Bound<'_, PyDict> = obj.extract().map_err(|_| {
        PyTypeError::new_err("expected AnimationOptions, None, or a dict")
    })?;
    let tick_hz: Option<u32> = dict
        .get_item("tick_hz")?
        .map(|v| v.extract())
        .transpose()?;
    let position_tolerance_px: Option<i32> = dict
        .get_item("position_tolerance_px")?
        .map(|v| v.extract())
        .transpose()?;
    let before_flags = dict
        .get_item("before_flags")?
        .map(|v| folder_flag_op_from_any(&v))
        .transpose()?;
    let after_flags = dict
        .get_item("after_flags")?
        .map(|v| folder_flag_op_from_any(&v))
        .transpose()?;
    let force_fallback: bool = dict
        .get_item("force_fallback")?
        .map(|v| v.extract())
        .transpose()?
        .unwrap_or(false);
    let default_render = CoreOverlayRenderOptions::default();
    let draw_labels: bool = dict
        .get_item("draw_labels")?
        .map(|v| v.extract())
        .transpose()?
        .unwrap_or(default_render.draw_labels);
    let draw_shortcut_overlay: bool = dict
        .get_item("draw_shortcut_overlay")?
        .map(|v| v.extract())
        .transpose()?
        .unwrap_or(default_render.draw_shortcut_overlay);
    let draw_shield_overlay: bool = dict
        .get_item("draw_shield_overlay")?
        .map(|v| v.extract())
        .transpose()?
        .unwrap_or(default_render.draw_shield_overlay);
    Ok(CoreAnimationOptions {
        snap_to_grid: dict.get_item("snap_to_grid")?.map(|value| value.extract()).transpose()?.unwrap_or(false),
        tick_hz,
        position_tolerance_px,
        before_flags,
        after_flags,
        force_fallback,
        render_options: CoreOverlayRenderOptions {
            draw_labels,
            draw_shortcut_overlay,
            draw_shield_overlay,
        },
    })
}

/// Extract just an [`OverlayRenderOptions`](CoreOverlayRenderOptions)
/// from an optional Python argument. Accepts `None`, an
/// `AnimationOptions` instance (uses its `render_options` field), or a
/// dict with any of `draw_labels`, `draw_shortcut_overlay`,
/// `draw_shield_overlay`. Missing keys default to
/// `CoreOverlayRenderOptions::default()`.
pub(crate) fn render_options_from_any(
    obj: Option<&Bound<'_, PyAny>>,
) -> PyResult<CoreOverlayRenderOptions> {
    let Some(obj) = obj else {
        return Ok(CoreOverlayRenderOptions::default());
    };
    if obj.is_none() {
        return Ok(CoreOverlayRenderOptions::default());
    }
    if let Ok(opts) = obj.extract::<PyAnimationOptions>() {
        return Ok(opts.to_core().render_options);
    }
    let dict: Bound<'_, PyDict> = obj.extract().map_err(|_| {
        PyTypeError::new_err(
            "expected AnimationOptions, None, or a dict with draw_labels / \
             draw_shortcut_overlay / draw_shield_overlay",
        )
    })?;
    let default = CoreOverlayRenderOptions::default();
    let draw_labels: bool = dict
        .get_item("draw_labels")?
        .map(|v| v.extract())
        .transpose()?
        .unwrap_or(default.draw_labels);
    let draw_shortcut_overlay: bool = dict
        .get_item("draw_shortcut_overlay")?
        .map(|v| v.extract())
        .transpose()?
        .unwrap_or(default.draw_shortcut_overlay);
    let draw_shield_overlay: bool = dict
        .get_item("draw_shield_overlay")?
        .map(|v| v.extract())
        .transpose()?
        .unwrap_or(default.draw_shield_overlay);
    Ok(CoreOverlayRenderOptions {
        draw_labels,
        draw_shortcut_overlay,
        draw_shield_overlay,
    })
}