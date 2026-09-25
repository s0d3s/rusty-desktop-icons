"""rusty_desktop_icons — Rust-powered Windows desktop icon controller.

The compiled extension lives at ``rusty_desktop_icons._rusty_desktop_icons``
and is built with `maturin <https://maturin.rs>`_. This package re-exports
its public surface so callers can simply::

    from rusty_desktop_icons import DesktopController, Curve, Duration

    ctrl = DesktopController()
    icons = ctrl.list_icons()
    spec = [
        {
            "id": ic.id,
            "target": (ic.position[0] + 200, ic.position[1]),
            "duration": Duration.fixed(seconds=1.0),
            "curve": Curve.ease_in_out(),
        }
        for ic in icons
    ]
    handle = ctrl.animate(spec)
    handle.wait()

All errors raised by the extension subclass :class:`RustyDesktopError`.

Logging: importing this package routes the Rust side's ``tracing`` events
into `loguru <https://github.com/Delgan/loguru>`_, so configuring a
loguru sink is all that is normally needed. To get these records into
stdlib :mod:`logging` instead, add loguru's ``PropagateHandler``::

    import logging
    from loguru import logger
    from rusty_desktop_icons import LOGGER_NAME

    class PropagateHandler(logging.Handler):
        def emit(self, record):
            logging.getLogger(record.name).handle(record)

    logger.add(PropagateHandler(), format="{message}")

See :func:`init_logging`.
"""

import sys

from ._builtin_shaders import BuiltinShader
from ._enums import (
    CurveKind,
    DurationKind,
    FinishReasonKind,
    FolderFlagOpKind,
    KeyframeInterp,
    LogTarget,
    ShaderPipeline,
)

if sys.platform == "win32":
    from ._flags import FolderFlag

from ._logging import LOGGER_NAME  # noqa: F401 -- re-export
from ._rusty_desktop_icons import (  # noqa: F401 -- re-exports
    __author__,
    __version__,
    # Logging
    init_logging,
    # Value types
    AnimationOptions,
    AnimationPreset,
    Canvas,
    CapturedFrame,
    RenderSession,
    Curve,
    Duration,
    Shader,
    ShaderSource,
    EffectParameter,
    RenderTarget,
    RenderPass,
    ProceduralSource,
    Effect,
    PreparedAnimation,
    TimelineSession,
    TimelineState,
    TimelineCloseMode,
    PlaybackHandle,
    PlaybackOutcome,
    FinalCommitOutcome,
    FinishReason,
    FolderFlagOp,
    IconAnimationSpec,
    IconAnimationState,
    IconSnapshot,
    IconGrid,
    DesktopInfo,
    MonitorInfo,
    MotionContext,
    Rect,
    StartContext,
    StopMode,
    TickContext,
    # Live objects
    AnimationHandle,
    DesktopController,
    # Error hierarchy
    AnimationBusy,
    BackendUnavailable,
    ComError,
    IconNotFound,
    InvalidCurve,
    InvalidDuration,
    InvalidEffect,
    InvalidGrid,
    RustyDesktopError,
    UnsupportedPlatform,
    WorkerCrashed,
)

init_logging(target=LogTarget.Loguru)

__all__ = [
    "BuiltinShader",
    "CurveKind",
    "DurationKind",
    "FinishReasonKind",
    "FolderFlagOpKind",
    "KeyframeInterp",
    "LogTarget",
    "ShaderPipeline",
    "__version__",
    "__author__",
    # Logging
    "init_logging",
    "LOGGER_NAME",
    # Value types
    "AnimationOptions",
    "AnimationPreset",
    "Canvas",
    "CapturedFrame",
    "RenderSession",
    "Curve",
    "Duration",
    "Shader",
    "ShaderSource",
    "EffectParameter",
    "RenderTarget",
    "RenderPass",
    "ProceduralSource",
    "Effect",
    "PreparedAnimation",
    "TimelineSession",
    "TimelineState",
    "TimelineCloseMode",
    "PlaybackHandle",
    "PlaybackOutcome",
    "FinalCommitOutcome",
    "FinishReason",
    "FolderFlagOp",
    "IconAnimationSpec",
    "IconAnimationState",
    "IconSnapshot",
    "IconGrid",
    "DesktopInfo",
    "MonitorInfo",
    "MotionContext",
    "Rect",
    "StartContext",
    "StopMode",
    "TickContext",
    # Live objects
    "AnimationHandle",
    "DesktopController",
    # Error hierarchy
    "AnimationBusy",
    "BackendUnavailable",
    "ComError",
    "IconNotFound",
    "InvalidCurve",
    "InvalidDuration",
    "InvalidEffect",
    "InvalidGrid",
    "RustyDesktopError",
    "UnsupportedPlatform",
    "WorkerCrashed",
]

if sys.platform == "win32":
    __all__.append("FolderFlag")
