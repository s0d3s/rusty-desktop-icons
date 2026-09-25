"""loguru sink for the Rust side's ``tracing`` events.

The compiled extension calls :func:`emit` once per event. Everything
loguru-specific lives here rather than in PyO3 because building
``logger.patch()`` closures through FFI would be unreadable and
impossible to test without a live desktop.

Why ``patch`` at all: loguru derives ``{name}``, ``{function}`` and
``{line}`` from the Python call stack. For an event raised in Rust there
is no meaningful Python frame — without the patch every record would be
attributed to this module instead of to the ``.rs`` file that produced
it.
"""

from __future__ import annotations

from typing import Any, Mapping

from loguru import logger

#: `tracing` level names mapped onto loguru's. loguru has a real TRACE
#: level (5), so unlike stdlib `logging` nothing has to be folded.
_LEVELS = {
    "ERROR": "ERROR",
    "WARN": "WARNING",
    "INFO": "INFO",
    "DEBUG": "DEBUG",
    "TRACE": "TRACE",
}

#: Exposed so callers can route these records into stdlib `logging`:
#:
#:     from rusty_desktop_icons import LOGGER_NAME
#:     logger.add(PropagateHandler(), filter=LOGGER_NAME)
LOGGER_NAME = "rusty_desktop_icons"


def _patcher(target: str, scope: str, line: int | None):
    def patch(record: dict) -> None:
        # `name` is what `{name}` renders; point it at the Rust module
        # path (e.g. "rdi_platform_windows::backend").
        record["name"] = target
        # `function` carries the enclosing tracing span, so an overlay
        # warning stays attributable to the animation that caused it.
        record["function"] = scope or "-"
        if line is not None:
            record["line"] = line

    return patch


def emit(
    level: str,
    target: str,
    message: str,
    scope: str = "",
    file: str | None = None,
    line: int | None = None,
    extras: Mapping[str, Any] | None = None,
) -> None:
    """Forward one `tracing` event to loguru.

    Called from Rust; must never raise, because a logging failure has to
    not abort an in-flight animation.
    """
    try:
        loguru_level = _LEVELS.get(level, "INFO")
        bound = {"rdi_target": target, "rdi_file": file, **(extras or {})}
        logger.patch(_patcher(target, scope, line)).bind(**bound).log(
            loguru_level, message
        )
    except Exception:  # pragma: no cover - sink misconfiguration
        pass
