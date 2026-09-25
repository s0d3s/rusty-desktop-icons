"""Function-generated animation curve example.

Demonstrates ``Curve.from_function`` — pass any Python callable
``f(t: float) -> float`` and the extension will sample it once at
spec-build time (under the GIL), store the samples as a keyframe
curve, and then interpolate them on the worker thread. The Python
callable is dropped after construction; the tick loop never re-enters
Python.

Requirements
------------
* ``maturin develop -m crates/rdi-python/Cargo.toml`` has been run.
* You are on Windows (real desktop). On other platforms the animation
  step raises ``UnsupportedPlatform``.

Run::

    .venv/Scripts/python.exe crates/rdi-python/examples/curve_from_function.py
"""

from __future__ import annotations

import math
import sys

from rusty_desktop_icons import (
    Curve,
    DesktopController,
    Duration,
    UnsupportedPlatform,
)


def damped_cosine(t: float) -> float:
    """A curve that overshoots then settles: 1 - e^{-4t} * cos(6πt)."""

    return 1.0 - math.exp(-4.0 * t) * math.cos(6.0 * math.pi * t)


def linear_ramp_plus_wobble(t: float) -> float:
    """Mostly-linear ramp with a small sinusoidal wobble at the end."""

    return t + 0.05 * math.sin(t * math.pi * 4.0) * (1.0 - t)


def print_curve(name: str, curve: Curve) -> None:
    print(f"\n{name}  (kind={curve.kind})")
    print("   t    |    v")
    print("--------+---------")
    for i in range(0, 11):
        t = i / 10.0
        print(f"  {t:>4.2f}  |  {curve.eval(t):+.4f}")


def main() -> int:
    damped = Curve.from_function(damped_cosine, samples=128)
    wobble = Curve.from_function(linear_ramp_plus_wobble, samples=128)

    print_curve("damped_cosine (128 samples)", damped)
    print_curve("linear_ramp_plus_wobble (128 samples)", wobble)

    try:
        ctrl = DesktopController()
    except UnsupportedPlatform as exc:
        print(f"\n[non-Windows] skipping desktop animation: {exc}")
        return 0

    icons = ctrl.list_icons()
    if not icons:
        print("\n[no icons] skipping desktop animation")
        return 0

    icon = icons[0]
    print(
        f"\nAnimating {icon.display_name!r} in place with the damped cosine curve "
        f"(pos={icon.position})"
    )

    spec = {
        "id": icon.id,
        "target": icon.position,  # no-op move so we do not disturb the layout
        "duration": Duration.fixed(seconds=0.4),
        "curve": damped,
    }
    handle = ctrl.animate([spec])
    reason = handle.wait_timeout(5.0)
    if reason is None:
        print("  animation timed out (unexpected)")
        return 1
    print(f"  done: reason={reason.kind} progress={handle.progress():.3f}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
