"""Motion-aware curve example — projectile gravity.

Demonstrates ``Curve.from_motion_function``. Unlike ``from_function``
the callable receives a ``MotionContext`` giving it access to the
animation's origin, target, distance, duration and a user-supplied
parameter dict. This lets the callable shape the curve in units that
depend on the physical animation being run — for example a gravity
term measured in pixels-per-second^2.

The curve is still sampled once at spec-build time and stored as
keyframes; the tick loop never calls Python.

Requirements
------------
* ``maturin develop -m crates/rdi-python/Cargo.toml`` has been run.
* You are on Windows (real desktop). On other platforms the animation
  step raises ``UnsupportedPlatform``.

Run::

    .venv/Scripts/python.exe crates/rdi-python/examples/curve_from_motion.py
"""

from __future__ import annotations

import sys

from rusty_desktop_icons import (
    Curve,
    DesktopController,
    Duration,
    UnsupportedPlatform,
)


def gravity(ctx, t: float) -> float:
    """Parabolic drop scaled by user param ``g`` (px/s^2).

    We normalise the analytic ``t + 0.5 * g_norm * t^2`` so the curve
    still ends at ``1.0`` at ``t = 1.0``; without this the curve would
    overshoot the target when ``g`` is large.
    """

    g = ctx.param("g", 0.0)
    if g <= 0.0:
        return t
    numerator = t + 0.5 * g * t * t
    denominator = 1.0 + 0.5 * g
    return numerator / denominator


def anticipation_then_snap(ctx, t: float) -> float:
    """Pull back a little, then rush toward the target.

    Uses ``ctx.distance_px`` to scale the pull-back — long journeys get
    a bigger anticipation so short ones do not look jittery.
    """

    strength = min(0.1, ctx.distance_px / 4000.0)
    if t < 0.25:
        # Pull back by `strength` over the first quarter.
        return -strength * (t / 0.25)
    # Then accelerate toward 1.0 over the remaining three quarters.
    tp = (t - 0.25) / 0.75
    return -strength + (1.0 + strength) * (tp * tp * (3.0 - 2.0 * tp))


def print_curve(name: str, curve: Curve) -> None:
    print(f"\n{name}  (kind={curve.kind})")
    print("   t    |    v")
    print("--------+---------")
    for i in range(0, 11):
        t = i / 10.0
        print(f"  {t:>4.2f}  |  {curve.eval(t):+.4f}")


def main() -> int:
    # Build a gravity curve for a 400-pixel vertical drop over 1 second.
    grav_curve = Curve.from_motion_function(
        gravity,
        origin=(0, 0),
        target=(0, 400),
        duration_seconds=1.0,
        params={"g": 6.0},
        samples=64,
    )
    print_curve("gravity(g=6.0, distance=400px, dur=1s)", grav_curve)

    # Anticipation curve for the same journey.
    anticip = Curve.from_motion_function(
        anticipation_then_snap,
        origin=(0, 0),
        target=(0, 400),
        duration_seconds=0.5,
        samples=64,
    )
    print_curve("anticipation_then_snap(distance=400px, dur=0.5s)", anticip)

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
        f"\nAnimating {icon.display_name!r} in place with the gravity curve "
        f"(pos={icon.position})"
    )

    # Rebuild the gravity curve for this icon's actual position so
    # `ctx.distance_px` is meaningful in the sampled range.
    curve = Curve.from_motion_function(
        gravity,
        origin=icon.position,
        target=icon.position,  # no-op, keeps the layout undisturbed
        duration_seconds=0.4,
        params={"g": 6.0},
        samples=64,
    )
    spec = {
        "id": icon.id,
        "target": icon.position,
        "duration": Duration.fixed(seconds=0.4),
        "curve": curve,
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
