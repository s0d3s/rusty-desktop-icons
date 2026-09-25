"""Smoke test / demo for `rusty_desktop_icons`.

Run with the Python interpreter that has the extension installed:

    maturin develop -m crates/rdi-python/Cargo.toml
    python crates/rdi-python/examples/smoke.py

The script is read-only: it lists desktop icons, prints folder flags,
constructs a variety of curves (including a Python-defined one) and
runs a no-op animation that moves every icon back to its current
position.

On non-Windows platforms it verifies that `DesktopController()` raises
`UnsupportedPlatform` cleanly.
"""

from __future__ import annotations

import math
import sys

import rusty_desktop_icons as rdi
from rusty_desktop_icons import (
    Curve,
    DesktopController,
    Duration,
    UnsupportedPlatform,
)


def show_curve(name: str, c: Curve) -> None:
    samples = [round(c.eval(t / 4), 4) for t in range(5)]
    print(f"  {name:<24} kind={c.kind:<8} samples(0..1)={samples}")


def build_curves() -> None:
    print("=== Curves ===")
    show_curve("linear", Curve.linear())
    show_curve("ease_in_out", Curve.ease_in_out())
    show_curve(
        "cubic_bezier",
        Curve.cubic_bezier(0.42, 0.0, 0.58, 1.0),
    )
    show_curve(
        "keyframes",
        Curve.keyframes(
            [(0.0, 0.0), (0.5, 0.9), (1.0, 1.0)], interp=rdi.KeyframeInterp.SmoothStep
        ),
    )
    show_curve("spring", Curve.spring())
    show_curve("bounce", Curve.bounce())
    show_curve(
        "from_function",
        Curve.from_function(
            lambda t: 1.0 - math.exp(-4.0 * t) * math.cos(6 * math.pi * t),
            samples=64,
        ),
    )
    show_curve(
        "from_motion_function",
        Curve.from_motion_function(
            lambda ctx, t: t + 0.5 * ctx.param("g", 0.0) * t * t,
            origin=(0, 0),
            target=(0, 400),
            duration_seconds=1.0,
            params={"g": 0.5},
            samples=48,
        ),
    )


def main() -> int:
    print(f"rusty_desktop_icons {rdi.__version__} (by {rdi.__author__})")
    build_curves()

    try:
        ctrl = DesktopController()
    except UnsupportedPlatform as exc:
        print(f"\n[non-Windows] DesktopController() raised UnsupportedPlatform: {exc}")
        return 0

    print("\n=== Desktop state ===")
    flags = ctrl.get_flags()
    print(f"folder view flags = 0x{flags:08x}")

    icons = ctrl.list_icons()
    print(f"{len(icons)} icons on the desktop")
    for icon in icons[:5]:
        print(f"  {icon.display_name!r:<30} pos={icon.position} id={icon.id[:16]}...")

    if not icons:
        print("(no icons: skipping animation)")
        return 0

    print("\n=== No-op animation (move each icon to its current position) ===")
    specs = [
        {
            "id": icon.id,
            "target": icon.position,
            "duration": Duration.fixed(seconds=0.1),
            "curve": Curve.linear(),
        }
        for icon in icons
    ]

    ticks_seen = 0

    def on_tick(ctx):
        nonlocal ticks_seen
        ticks_seen += 1

    def on_finish(reason):
        print(
            f"  on_finish: kind={reason.kind} "
            f"stop_mode={reason.stop_mode} message={reason.message!r}"
        )

    handle = ctrl.animate(specs)
    handle.on_tick(on_tick)
    handle.on_finish(on_finish)

    reason = handle.wait_timeout(5.0)
    if reason is None:
        print("  animation timed out (bug?)")
        return 1
    print(
        f"  reason={reason.kind} progress={handle.progress():.3f} "
        f"missing={len(handle.missing_icons())} ticks_observed={ticks_seen}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
