"""Multi-monitor enumeration and placement example.

Shows how to:

* Enumerate every attached display via
  ``DesktopController.list_monitors()``.
* Look up which monitor a point lives on via
  ``DesktopController.monitor_for_point()``.
* Read icon coordinates in the shared virtual-screen space (which can
  be **negative** on monitors arranged to the left of or above the
  primary).
* Run a no-op animation that keeps every icon exactly where it
  already is — proving that the pipeline round-trips virtual-screen
  coordinates cleanly regardless of monitor layout.

Requirements
------------
* ``maturin develop -m crates/rdi-python/Cargo.toml`` has been run.
* You are on Windows. On other platforms the script prints an
  ``UnsupportedPlatform`` message and exits.

Run::

    .venv/Scripts/python.exe crates/rdi-python/examples/multi_monitor.py
"""

from __future__ import annotations

import sys
from collections import Counter

from rusty_desktop_icons import (
    Curve,
    DesktopController,
    Duration,
    UnsupportedPlatform,
)


def print_monitor_table(monitors) -> None:
    print(f"\n=== {len(monitors)} monitor(s) ===")
    print(f"{'id':<24} {'primary':<8} {'bounds':<32} "
          f"{'work_area':<32} {'scale':>6}")
    print("-" * 108)
    for m in monitors:
        b = m.bounds
        w = m.work_area
        print(
            f"{m.id:<24} {str(m.is_primary):<8} "
            f"({b.left:>6},{b.top:>6},{b.right:>6},{b.bottom:>6})  "
            f"({w.left:>6},{w.top:>6},{w.right:>6},{w.bottom:>6})  "
            f"{m.scale_factor:>6.2f}"
        )


def main() -> int:
    try:
        ctrl = DesktopController()
    except UnsupportedPlatform as exc:
        print(f"[non-Windows] cannot enumerate monitors: {exc}")
        return 0

    monitors = ctrl.list_monitors()
    print_monitor_table(monitors)

    if not monitors:
        print("no monitors reported — nothing to do")
        return 0

    # Group icons by monitor.
    icons = ctrl.list_icons()
    counts: Counter[str] = Counter()
    off_screen: list[str] = []
    for icon in icons:
        m = ctrl.monitor_for_point(icon.position)
        if m is None:
            off_screen.append(f"{icon.display_name!r} at {icon.position}")
        else:
            counts[m.id] += 1

    print(f"\n=== {len(icons)} icon(s) grouped by monitor ===")
    for m in monitors:
        print(f"  {m.id:<24} {counts.get(m.id, 0):>4} icon(s)")
    if off_screen:
        print("\nIcons whose recorded position is not on any monitor:")
        for line in off_screen:
            print(f"  - {line}")
        print(
            "  (these can occur when a previously-connected monitor was\n"
            "  disconnected without the shell re-arranging the icons)"
        )

    # Demonstrate that the animation pipeline handles the coordinate
    # space transparently: pick any monitor and animate each icon
    # currently on it back to its current position (a no-op).
    if not icons:
        print("\nNo icons on the desktop — skipping demo animation.")
        return 0

    primary = next(m for m in monitors if m.is_primary)
    on_primary = [i for i in icons if primary.contains(i.position)]
    if not on_primary:
        print("\nNo icons on the primary monitor — skipping demo animation.")
        return 0

    print(
        f"\nRunning a no-op animation on {len(on_primary)} icon(s) that live on "
        f"the primary monitor ({primary.id})…"
    )
    specs = [
        {
            "id": icon.id,
            "target": icon.position,
            "duration": Duration.fixed(seconds=0.1),
            "curve": Curve.linear(),
        }
        for icon in on_primary
    ]
    handle = ctrl.animate(specs)
    reason = handle.wait_timeout(5.0)
    if reason is None:
        print("  animation timed out (unexpected)")
        return 1
    print(
        f"  done: reason={reason.kind} "
        f"progress={handle.progress():.3f} "
        f"missing={len(handle.missing_icons())}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
