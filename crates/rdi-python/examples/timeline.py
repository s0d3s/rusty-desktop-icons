"""Interactive timeline showcase; close or Ctrl+C restores saved origins."""

from __future__ import annotations

import argparse

import rusty_desktop_icons as rdi


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--offset-x", type=int, default=0)
    parser.add_argument("--seconds", type=float, default=2.0)
    args = parser.parse_args()
    controller = rdi.DesktopController()
    icons = controller.list_icons()
    shader = rdi.Shader.compile(rdi.ShaderSource.builtin(rdi.BuiltinShader.Glitch))
    specs = [
        rdi.IconAnimationSpec(
            icon.id,
            (icon.position[0] + args.offset_x, icon.position[1]),
            rdi.Duration.fixed(args.seconds),
            rdi.Curve.ease_in_out(),
            effect=rdi.Effect(shader, seed=float(index)),
        )
        for index, icon in enumerate(icons)
    ]
    print("play <0..1> [speed], seek <0..1>, speed <multiplier>, pause, status, close")
    with controller.prepare(specs, rdi.AnimationOptions(tick_hz=144)).open_timeline() as timeline:
        try:
            while timeline.state() != rdi.TimelineState.Closed:
                parts = input("timeline> ").split()
                if not parts:
                    continue
                command, *values = parts
                try:
                    if command == "close":
                        break
                    if command == "play" and 1 <= len(values) <= 2:
                        timeline.play_to(float(values[0]), float(values[1]) if len(values) == 2 else timeline.speed())
                    elif command == "seek" and len(values) == 1:
                        timeline.seek(float(values[0]))
                    elif command == "speed" and len(values) == 1:
                        timeline.set_speed(float(values[0]))
                    elif command == "pause" and not values:
                        timeline.pause()
                    elif command != "status" or values:
                        print("Invalid command")
                        continue
                    print(f"{timeline.state()}: {timeline.position():.4f}, {timeline.speed():g}x")
                except (ValueError, rdi.RustyDesktopError) as error:
                    print(error)
        except (KeyboardInterrupt, EOFError):
            print()
    reason = timeline.finish_reason()
    if reason is not None and reason.kind == rdi.FinishReasonKind.Error:
        raise RuntimeError(reason.message)
    restored = {icon.id: icon.position for icon in controller.list_icons()}
    if any(restored.get(icon.id) != icon.position for icon in icons):
        raise RuntimeError("Shell did not restore every original position; check snapping or external changes")


if __name__ == "__main__":
    main()