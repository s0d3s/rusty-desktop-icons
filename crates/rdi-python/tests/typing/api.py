"""Static-only API examples. Do not execute against a desktop."""

import sys
from typing import assert_type

import rusty_desktop_icons as rdi


def check_api(controller: rdi.DesktopController, timeline: rdi.TimelineSession) -> None:
    curve = rdi.Curve.from_function(lambda progress: progress * progress)
    assert_type(curve, rdi.Curve)
    assert_type(controller.list_icons(), list[rdi.IconSnapshot])
    assert_type(timeline.play_to(1.0), rdi.PlaybackHandle)
    assert_type(timeline.close(), rdi.FinishReason)
    spec = rdi.IconAnimationSpec("id", (10, 20), rdi.Duration.fixed(1.0), curve)
    assert_type(spec.target, tuple[int, int])
    assert_type(controller.prepare([spec]), rdi.PreparedAnimation)
    handle = controller.animate([spec], rdi.AnimationOptions(snap_to_grid=True))
    def observe(context: rdi.TickContext) -> None:
        assert_type(context.progress, float)

    handle.on_tick(observe)
    assert_type(handle.wait(), rdi.FinishReason)
    assert_type(rdi.InvalidCurve("bad curve"), rdi.InvalidCurve)


def check_animation_presets() -> None:
    preset = rdi.AnimationPreset()
    assert_type(preset.movement, rdi.Curve)
    assert_type(preset.envelope, rdi.Curve)
    assert_type(preset.duration, rdi.Duration)
    assert_type(rdi.AnimationPreset.builtin(rdi.BuiltinShader.DustTransfer), rdi.AnimationPreset)
    assert_type(rdi.AnimationPreset.builtin("glitch"), rdi.AnimationPreset)
    assert_type(rdi.AnimationPreset(movement=preset.movement, envelope=preset.envelope, duration=preset.duration), rdi.AnimationPreset)
    rdi.IconAnimationSpec("id", (10, 20), preset.duration, preset.movement)


def check_string_enums(reason: rdi.FinishReason) -> None:
    curve = rdi.Curve.keyframes([(0.0, 0.0), (1.0, 1.0)], interp=rdi.KeyframeInterp.Step)
    assert_type(curve.kind, rdi.CurveKind)
    assert_type(rdi.Duration.fixed(1.0).kind, rdi.DurationKind)
    assert_type(rdi.FolderFlagOp.set(0).kind, rdi.FolderFlagOpKind)
    assert_type(reason.kind, rdi.FinishReasonKind)
    source = rdi.ShaderSource(pipeline=rdi.ShaderPipeline.Particles)
    assert_type(source.pipeline, rdi.ShaderPipeline)
    assert_type(rdi.ShaderSource.builtin(rdi.BuiltinShader.Glitch), rdi.ShaderSource)
    assert_type(rdi.BuiltinShader("glitch"), rdi.BuiltinShader)
    assert_type(list(rdi.BuiltinShader), list[rdi.BuiltinShader])
    rdi.Curve.from_function(lambda progress: progress, interp=rdi.KeyframeInterp.Linear)
    rdi.Curve.from_motion_function(
        lambda context, progress: progress, (0, 0), (10, 10), 1.0,
        interp=rdi.KeyframeInterp.SmoothStep,
    )
    rdi.AnimationOptions(before_flags=(rdi.FolderFlagOpKind.Set, 0))
    rdi.init_logging(target=rdi.LogTarget.Loguru)
    rdi.Curve.keyframes([(0.0, 0.0), (1.0, 1.0)], interp="smooth_step")
    rdi.ShaderSource(pipeline="sprite")
    rdi.ShaderSource.builtin("glitch")
    rdi.init_logging(target="python")


if sys.platform == "win32":
    def check_folder_flags(controller: rdi.DesktopController) -> None:
        flags = rdi.FolderFlag.FWF_AUTOARRANGE | rdi.FolderFlag.FWF_SNAPTOGRID
        assert_type(flags, rdi.FolderFlag)
        assert_type(flags.title, str | None)
        assert_type(flags.description, str | None)
        controller.set_flags(flags)
        controller.apply_flags(flags, rdi.FolderFlag.FWF_NONE)
        assert_type(rdi.FolderFlag(controller.get_flags()), rdi.FolderFlag)
        assert_type(rdi.FolderFlagOp.set(flags), rdi.FolderFlagOp)