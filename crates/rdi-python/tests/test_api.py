"""Pure-Python API tests for `rusty_desktop_icons`.

These tests do not touch the real Windows desktop — they exercise:

* Curve construction (all builtin + procedural + function-generated).
* Duration factories and validation.
* Spec / AnimationOptions dict interop.
* Error hierarchy and `map_desktop_error` classification.

Run with:

    .venv/Scripts/python.exe -m pytest crates/rdi-python/tests/test_api.py -v
"""

from __future__ import annotations

import math
import json
import os
import sys
import time
import ast
import importlib.util
import pickle
from enum import Enum
from pathlib import Path
from types import ModuleType

import pytest

import rusty_desktop_icons as rdi
from rusty_desktop_icons import (
    AnimationOptions,
    Curve,
    Duration,
    FolderFlagOp,
    IconAnimationSpec,
    InvalidCurve,
    InvalidDuration,
    MonitorInfo,
    Rect,
    RustyDesktopError,
    StopMode,
    UnsupportedPlatform,
)


NATIVE_ERROR_NAMES = [
    "RustyDesktopError", "IconNotFound", "InvalidCurve", "InvalidDuration",
    "UnsupportedPlatform", "BackendUnavailable", "AnimationBusy", "WorkerCrashed",
    "ComError", "InvalidEffect", "InvalidGrid",
]


def _raise_native_error(name: str, message: str) -> None:
    raise getattr(rdi, name)(message)


@pytest.mark.parametrize("name", NATIVE_ERROR_NAMES)
def test_native_exception_pickle_roundtrip(name: str) -> None:
    error_type = getattr(rdi, name)
    message = "IFolderView2::GetItemPosition failed: Element not found. (0x80070490)"
    error = error_type(message)
    error.add_note("showcase worker")
    assert error_type.__module__ == "rusty_desktop_icons"
    for protocol in range(pickle.HIGHEST_PROTOCOL + 1):
        restored = pickle.loads(pickle.dumps(error, protocol=protocol))
        assert type(restored) is error_type
        assert isinstance(restored, RustyDesktopError)
        assert restored.args == (message,)
        assert restored.__notes__ == ["showcase worker"]


def test_native_exceptions_cross_spawned_process_pool() -> None:
    from concurrent.futures import ProcessPoolExecutor
    import multiprocessing

    message = "IFolderView2::GetItemPosition failed: Element not found. (0x80070490)"
    with ProcessPoolExecutor(max_workers=1, mp_context=multiprocessing.get_context("spawn")) as pool:
        for name in NATIVE_ERROR_NAMES:
            future = pool.submit(_raise_native_error, name, message)
            with pytest.raises(getattr(rdi, name)) as caught:
                future.result(timeout=30)
            assert type(caught.value) is getattr(rdi, name)
            assert caught.value.args == (message,)
            assert "_raise_native_error" in str(caught.value.__cause__)
            assert message in str(caught.value.__cause__)
        assert pool.submit(int, "42").result(timeout=30) == 42


# ---------------------------------------------------------------------------
# Curve
# ---------------------------------------------------------------------------

def test_packaged_stubs_cover_public_api() -> None:
    package = Path(rdi.__file__).parent
    assert (package / "py.typed").is_file()
    public = ast.parse((package / "__init__.pyi").read_text(encoding="utf-8"))
    native = ast.parse((package / "_rusty_desktop_icons.pyi").read_text(encoding="utf-8"))
    public_nodes = list(public.body)
    if sys.platform == "win32":
        for node in public.body:
            if isinstance(node, ast.If) and ast.unparse(node.test) == "sys.platform == 'win32'":
                public_nodes.extend(node.body)
    exports = {
        alias.asname or alias.name for node in public_nodes
        if isinstance(node, ast.ImportFrom) for alias in node.names
    } | {
        node.target.id for node in public.body
        if isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name)
    }
    assert exports == set(rdi.__all__)
    assert not any(isinstance(node, ast.FunctionDef) and node.name == "__getattr__"
                   for node in native.body)
    for node in native.body:
        if not isinstance(node, ast.ClassDef):
            continue
        runtime = getattr(rdi, node.name)
        declared = {
            member.name for member in node.body if isinstance(member, ast.FunctionDef)
        } | {
            member.target.id for member in node.body
            if isinstance(member, ast.AnnAssign) and isinstance(member.target, ast.Name)
        }
        assert {name for name in vars(runtime) if not name.startswith("_")} <= declared
        assert all(hasattr(runtime, name) for name in declared)
    for node in ast.walk(native):
        if isinstance(node, ast.FunctionDef):
            assert node.returns is not None, node.name
            assert ast.unparse(node.returns) != "Any", node.name
            for arg in node.args.posonlyargs + node.args.args + node.args.kwonlyargs:
                if arg.arg not in {"self", "cls"}:
                    assert arg.annotation is not None, (node.name, arg.arg)
                    assert ast.unparse(arg.annotation) != "Any", (node.name, arg.arg)


@pytest.mark.parametrize("enum_type", [
    rdi.KeyframeInterp, rdi.ShaderPipeline, rdi.BuiltinShader, rdi.CurveKind,
    rdi.DurationKind, rdi.FolderFlagOpKind, rdi.FinishReasonKind, rdi.LogTarget,
])
def test_string_enum_protocol(enum_type: type[Enum]) -> None:
    assert issubclass(enum_type, str)
    assert enum_type.__name__ in rdi.__all__
    for member in enum_type:
        assert isinstance(member, Enum)
        assert enum_type(member.value) is member
        assert member == member.value
        assert str(member) == member.value
        assert json.loads(json.dumps(member)) == member.value
        assert pickle.loads(pickle.dumps(member)) is member
    with pytest.raises(ValueError):
        enum_type("unknown")


@pytest.mark.parametrize("mode", list(rdi.KeyframeInterp))
def test_keyframe_enum_and_string_inputs(mode: rdi.KeyframeInterp) -> None:
    for interp in (mode, mode.value):
        explicit = Curve.keyframes([(0.0, 0.0), (1.0, 1.0)], interp=interp)
        sampled = Curve.from_function(lambda time: time, samples=2, interp=interp)
        motion = Curve.from_motion_function(
            lambda context, time: time, (0, 0), (10, 10), 1.0, samples=2, interp=interp,
        )
        for curve in (explicit, sampled, motion):
            assert curve.kind is rdi.CurveKind.Keyframe
            assert curve.eval(0.25) == explicit.eval(0.25)
    assert Curve.keyframes([(0.0, 0.0), (1.0, 1.0)], interp="SMOOTH_STEP").kind is rdi.CurveKind.Keyframe
    assert Curve.linear().kind is rdi.CurveKind.Builtin


def test_enum_getters_and_flag_payloads() -> None:
    assert Duration.fixed(1.0).kind is rdi.DurationKind.Fixed
    assert Duration.distance(100.0).kind is rdi.DurationKind.Distance
    assert FolderFlagOp.set(0).kind is rdi.FolderFlagOpKind.Set
    assert FolderFlagOp.exactly(0).kind is rdi.FolderFlagOpKind.Exactly
    for mode in rdi.FolderFlagOpKind:
        for kind in (mode, mode.value):
            AnimationOptions(before_flags=(kind, 0), after_flags={"kind": kind, "flags": 0})
    for pipeline in rdi.ShaderPipeline:
        assert rdi.ShaderSource(pipeline=pipeline).pipeline is pipeline
        assert rdi.ShaderSource(pipeline=pipeline.value).pipeline is pipeline


@pytest.mark.skipif(sys.platform != "win32", reason="Windows HLSL compiler")
def test_builtin_shader_enum_matches_native_catalog() -> None:
    from rusty_desktop_icons import _rusty_desktop_icons as native

    assert [(member.name, member.value) for member in rdi.BuiltinShader] == native._builtin_shader_catalog()
    for member in rdi.BuiltinShader:
        source = rdi.ShaderSource.builtin(member)
        legacy = rdi.ShaderSource.builtin(member.value)
        assert (source.pixel, source.vertex, source.pipeline) == (legacy.pixel, legacy.vertex, legacy.pipeline)
        assert isinstance(source.pipeline, rdi.ShaderPipeline)
        assert rdi.Shader.compile(source).bytecode_size > 0


def test_animation_preset_defaults_and_overrides() -> None:
    preset = rdi.AnimationPreset()
    assert "AnimationPreset" in rdi.__all__
    assert repr(preset.duration) == "Duration.fixed(seconds=2.000)"
    assert preset.movement.eval(0.25) == Curve.ease_in_out().eval(0.25)
    assert [preset.envelope.eval(progress) for progress in (0.0, 0.15, 0.5, 0.85, 1.0)] == [0.0, 1.0, 1.0, 1.0, 0.0]
    custom = rdi.AnimationPreset(movement=Curve.linear(), envelope=Curve.linear(), duration=Duration.distance(100.0))
    assert custom.movement.eval(0.25) == 0.25
    assert custom.envelope.eval(0.25) == 0.25
    assert custom.duration.kind is rdi.DurationKind.Distance
    assert rdi.AnimationPreset().envelope.eval(0.15) == 1.0
    IconAnimationSpec("preset", (100, 100), custom.duration, custom.movement)
    with pytest.raises(AttributeError):
        setattr(preset, "movement", Curve.linear())


@pytest.mark.skipif(sys.platform != "win32", reason="Windows shader catalog")
def test_animation_preset_builtin_values() -> None:
    durations = {"identity": 2, "glitch": 2, "particle-vortex": 4, "dust-transfer": 4, "silk-flow": 5}
    for member in rdi.BuiltinShader:
        preset = rdi.AnimationPreset.builtin(member)
        legacy = rdi.AnimationPreset.builtin(member.value)
        assert repr(preset.duration) == f"Duration.fixed(seconds={durations[member.value]:.3f})"
        assert repr(legacy.duration) == repr(preset.duration)
        for progress in (0.0, 0.05, 0.15, 0.5, 0.85, 0.95, 1.0):
            assert preset.movement.eval(progress) == legacy.movement.eval(progress)
            assert preset.envelope.eval(progress) == legacy.envelope.eval(progress)
        shader = rdi.Shader.compile(rdi.ShaderSource.builtin(member))
        effect = rdi.Effect(shader, envelope=preset.envelope)
        IconAnimationSpec("preset", (100, 100), preset.duration, preset.movement, effect=effect)
    glitch = rdi.AnimationPreset.builtin(rdi.BuiltinShader.Glitch)
    assert glitch.movement.eval(0.15) == 0.0
    assert glitch.movement.eval(0.85) == 1.0
    for member in (rdi.BuiltinShader.DustTransfer, rdi.BuiltinShader.SilkFlow):
        assert rdi.AnimationPreset.builtin(member).envelope.eval(0.05) == 1.0
    with pytest.raises(rdi.InvalidEffect, match="unknown built-in shader"):
        rdi.AnimationPreset.builtin("not-a-shader")


@pytest.mark.skipif(sys.platform == "win32", reason="Non-Windows catalog boundary")
def test_animation_preset_builtin_requires_windows() -> None:
    with pytest.raises(rdi.UnsupportedPlatform):
        rdi.AnimationPreset.builtin(rdi.BuiltinShader.Glitch)


def test_log_target_enum_inputs() -> None:
    for target in rdi.LogTarget:
        assert isinstance(rdi.init_logging(target=target), bool)
        assert isinstance(rdi.init_logging(target=target.value), bool)
    rdi.init_logging(target=rdi.LogTarget.Loguru)


def test_enum_stub_flattens_memberless_base() -> None:
    script = Path(__file__).resolve().parents[3] / "scripts/generate-stubs.py"
    spec = importlib.util.spec_from_file_location("stub_generator", script)
    assert spec is not None and spec.loader is not None
    generator = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(generator)
    for source in (
        "from enum import Enum\nclass _StringEnum(str, Enum):\n    pass\n"
        "class Choice(_StringEnum):\n    Value = 'value'\n",
        "from ._enums import _StringEnum\nclass Choice(_StringEnum):\n    Value = 'value'\n",
    ):
        result = generator.enum_stub(source)
        assert "_StringEnum" not in result
        assert "from enum import Enum" in result
        assert "class Choice(str, Enum):" in result
        assert "Value = 'value'" in result


@pytest.mark.parametrize("separator", ["", "\n", "\n\n"])
def test_stub_completion_rejects_unknown_exports(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path, separator: str,
) -> None:
    script = Path(__file__).resolve().parents[3] / "scripts/generate-stubs.py"
    spec = importlib.util.spec_from_file_location("stub_generator", script)
    assert spec is not None and spec.loader is not None
    generator = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(generator)
    native = ModuleType("test_native")
    native.Unhandled = 42
    with pytest.raises(RuntimeError, match="Unhandled"):
        generator.complete_native("", native)
    del native.Unhandled
    native.BaseFailure = type("BaseFailure", (Exception,), {"__doc__": "Base failure."})
    native.ChildFailure = type("ChildFailure", (native.BaseFailure,), {"__doc__": "Child failure."})
    prefix = (
        "from _typeshed import Incomplete\n"
        "def __getattr__(name: str) -> Incomplete: ...\n" + separator
        + 'class Public:\n    """First paragraph.\n\n\n    Second paragraph."""\n'
        + separator
    )
    suffix = "def init_logging() -> bool: ...\n"
    source = prefix + suffix
    completed = generator.complete_native(source, native)
    assert "class ChildFailure(BaseFailure):" in completed
    assert "__getattr__" not in completed
    assert "First paragraph.\n\n\n    Second paragraph." in completed
    assert generator.complete_native(source, native) == completed
    monkeypatch.setattr(generator, "sys", ModuleType("platform_for_test"))
    generator.sys.platform = "win32"
    windows_source = (
        prefix + "def _folder_flag_catalog() -> list[tuple[int, str, str, str]]: ...\n"
        + separator + "def _builtin_shader_catalog() -> list[tuple[str, str]]: ...\n"
        + separator + suffix
    )
    windows_completed = generator.complete_native(windows_source, native)
    (tmp_path / "_rusty_desktop_icons.pyi").write_text(windows_completed, encoding="utf-8")
    monkeypatch.setattr(generator, "PACKAGE", tmp_path)
    generator.sys.platform = "linux"
    assert generator.complete_native(source, native) == windows_completed


@pytest.mark.skipif(sys.platform != "win32", reason="Windows HLSL compiler")
def test_shader_compilation_and_effect_validation() -> None:
    shader = rdi.Shader.compile("float4 pixel(VertexOutput input) : SV_Target { return default_pixel(input); }")
    assert shader.bytecode_size > 0
    glitch = rdi.Shader.compile(rdi.ShaderSource.builtin("glitch"))
    effect = rdi.Effect(glitch, params=(8.0, 3.0, 6.0, 30.0), padding_px=16)
    assert effect.params == (8.0, 3.0, 6.0, 30.0)
    assert effect.padding_px == 16
    assert rdi.Effect(glitch).params == effect.params
    assert rdi.Effect(glitch, params=None).params == effect.params
    IconAnimationSpec("test", (100, 100), Duration.fixed(1.0), Curve.linear(), effect=effect)
    with pytest.raises(rdi.InvalidEffect, match="pixel.hlsl"):
        rdi.Shader.compile("invalid hlsl")
    with pytest.raises(rdi.InvalidEffect):
        rdi.Shader.compile('#include "external.hlsl"')
    with pytest.raises(rdi.InvalidEffect):
        rdi.Effect(shader, params=(math.nan, 0.0, 0.0, 0.0))
    with pytest.raises(rdi.InvalidEffect):
        rdi.Effect(shader, padding_px=257)


@pytest.mark.skipif(sys.platform != "win32", reason="Windows HLSL compiler")
def test_particle_vortex_defaults_and_validation() -> None:
    shader = rdi.Shader.compile(rdi.ShaderSource.builtin("particle-vortex"))
    assert shader.bytecode_size > 0
    effect = rdi.Effect(shader)
    assert effect.params == pytest.approx((3.0, 1.5, 2.0, 0.65))
    IconAnimationSpec("dust", (100, 100), Duration.fixed(4.0), Curve.linear(), effect=effect)
    for params in (
        (0.0, 1.5, 2.0, 0.65), (33.0, 1.5, 2.0, 0.65),
        (3.0, -1.0, 2.0, 0.65), (3.0, 3.1, 2.0, 0.65),
        (3.0, 1.5, 9.0, 0.65), (3.0, 1.5, -9.0, 0.65),
        (3.0, 1.5, 2.0, 0.0), (3.0, 1.5, 2.0, 2.1),
        (math.nan, 1.5, 2.0, 0.65),
    ):
        with pytest.raises(rdi.InvalidEffect):
            rdi.Effect(shader, params=params)
    with pytest.raises(rdi.InvalidEffect):
        rdi.Effect(shader, seed=65536.0)
    assert rdi.Effect(shader, params=(1.0, 3.0, -8.0, 2.0), seed=-65535.0).params == (1.0, 3.0, -8.0, 2.0)


@pytest.mark.skipif(sys.platform != "win32", reason="Windows HLSL compiler")
def test_common_shader_compilation_interface() -> None:
    assert "ShaderSource" in rdi.__all__
    assert not hasattr(rdi.Shader, "particle_vortex")
    assert not hasattr(rdi.Shader, "glitch")
    for name in ("identity", "glitch", "particle-vortex", "dust-transfer", "silk-flow"):
        source = rdi.ShaderSource.builtin(name)
        copied = rdi.ShaderSource(source.pixel, vertex=source.vertex, pipeline=source.pipeline, execution=source.execution)
        assert rdi.Shader.compile(source).bytecode_size == rdi.Shader.compile(copied).bytecode_size
        particles = name in ("particle-vortex", "dust-transfer")
        assert (source.vertex is not None) == (particles or name == "silk-flow")
        assert source.pipeline == ("procedural" if name == "silk-flow" else "particles" if particles else "sprite")
        assert "struct " not in (source.vertex or "")
        assert "struct " not in (source.pixel or "")
    identity = rdi.ShaderSource.builtin("identity")
    assert identity.vertex is None and identity.pixel is None
    assert rdi.Shader.compile(identity).bytecode_size == rdi.Shader.compile(rdi.ShaderSource()).bytecode_size
    pixel = "float4 pixel(VertexOutput input) : SV_Target { return default_pixel(input); }"
    assert rdi.Shader.compile(pixel).bytecode_size == rdi.Shader.compile(rdi.ShaderSource(pixel)).bytecode_size
    source = rdi.ShaderSource.builtin("particle-vortex")
    assert source.vertex is not None
    custom = rdi.ShaderSource(source.pixel, vertex=source.vertex.replace("0.75", "0.5"), pipeline=source.pipeline)
    assert rdi.Shader.compile(custom).bytecode_size > 0
    with pytest.raises(rdi.InvalidEffect, match="vertex.hlsl"):
        rdi.Shader.compile(rdi.ShaderSource(source.pixel, vertex="invalid vertex", pipeline=source.pipeline))
    with pytest.raises(rdi.InvalidEffect, match="pixel.hlsl"):
        rdi.Shader.compile(rdi.ShaderSource("invalid pixel", vertex=source.vertex, pipeline=source.pipeline))
    with pytest.raises(rdi.InvalidEffect, match="1 MiB"):
        rdi.Shader.compile(rdi.ShaderSource(source.pixel, vertex=" " * (1024 * 1024), pipeline=source.pipeline))
    with pytest.raises(rdi.InvalidEffect, match="unknown built-in"):
        rdi.ShaderSource.builtin("unknown")


@pytest.mark.skipif(sys.platform != "win32", reason="Windows HLSL compiler")
@pytest.mark.parametrize("pipeline", ["sprite", "particles", "procedural"])
def test_shader_rejects_incompatible_stage_entries(pipeline: str) -> None:
    for vertex in (
        "float4 vertex(Instance instance, uint vertex_id : SV_VertexID) : SV_Position { return default_vertex(instance, vertex_id).position; }",
        "VertexOutput vertex(uint vertex_id : SV_VertexID) { return (VertexOutput)0; }",
    ):
        with pytest.raises(rdi.InvalidEffect):
            rdi.Shader.compile(rdi.ShaderSource(vertex=vertex, pipeline=pipeline))
    for pixel in (
        "float pixel(VertexOutput input) : SV_Target { return 1; }",
        "float4 pixel(float2 uv : TEXCOORD0) : SV_Target { return float4(uv, 0, 1); }",
    ):
        with pytest.raises(rdi.InvalidEffect):
            rdi.Shader.compile(rdi.ShaderSource(pixel, pipeline=pipeline))


def test_shader_source_pipeline_validation() -> None:
    source = rdi.ShaderSource()
    assert source.pixel is None and source.vertex is None
    assert source.pipeline == "sprite"
    with pytest.raises(rdi.InvalidEffect, match="unknown shader pipeline"):
        rdi.ShaderSource(pipeline="unknown")
    with pytest.raises(AttributeError):
        setattr(source, "pipeline", "particles")


@pytest.mark.skipif(sys.platform != "win32", reason="Windows HLSL compiler")
@pytest.mark.parametrize("pipeline", ["sprite", "particles", "procedural"])
@pytest.mark.parametrize("custom_vertex", [False, True])
@pytest.mark.parametrize("custom_pixel", [False, True])
def test_optional_shader_stages(pipeline: str, custom_vertex: bool, custom_pixel: bool) -> None:
    vertex = """
VertexOutput vertex(Instance instance, uint vertex_id : SV_VertexID) {
    VertexOutput output = default_vertex(instance, vertex_id);
    output.position.x += instance.timing.z * 0.1;
    return output;
}
""" if custom_vertex else None
    pixel = "float4 pixel(VertexOutput input) : SV_Target { return default_pixel(input); }" if custom_pixel else None
    source = rdi.ShaderSource(pixel, vertex=vertex, pipeline=pipeline)
    shader = rdi.Shader.compile(source)
    assert shader.bytecode_size > 0
    expected = {"sprite": (8.0, 3.0, 6.0, 30.0), "particles": (3.0, 1.5, 2.0, 0.65),
                "procedural": (0.0, 0.0, 0.0, 0.0)}[pipeline]
    assert rdi.Effect(shader).params == pytest.approx(expected)
    with pytest.raises(rdi.InvalidEffect):
        rdi.Shader.compile(rdi.ShaderSource("", vertex=vertex, pipeline=pipeline))
    with pytest.raises(rdi.InvalidEffect):
        rdi.Shader.compile(rdi.ShaderSource(pixel, vertex="", pipeline=pipeline))
    with pytest.raises(rdi.InvalidEffect):
        rdi.Shader.compile(rdi.ShaderSource("float4 main(VertexOutput input) : SV_Target { return 0; }", pipeline=pipeline))


@pytest.mark.skipif(sys.platform != "win32", reason="Windows HLSL compiler")
def test_silk_flow_parameters() -> None:
    shader = rdi.Shader.compile(rdi.ShaderSource.builtin(rdi.BuiltinShader.SilkFlow))
    assert rdi.ShaderSource.builtin(rdi.BuiltinShader.SilkFlow).pipeline is rdi.ShaderPipeline.Procedural
    assert rdi.Effect(shader).params == pytest.approx((8, 1, 2, 0.8))
    for index, value in [(0, 1), (0, 17), (0, 2.5), (1, -1), (1, 4),
                         (2, 0), (2, 7), (3, 0), (3, 2)]:
        params = [8.0, 1.0, 2.0, 0.8]
        params[index] = value
        with pytest.raises(rdi.InvalidEffect):
            rdi.Effect(shader, params=tuple(params))
    with pytest.raises(rdi.InvalidEffect):
        rdi.Effect(shader, seed=65536)
    effect = rdi.Effect(shader, parameters={"strands": 12, "density": 0.5})
    assert effect.parameters == pytest.approx({"strands": 12, "spread": 1, "folds": 2, "density": 0.5})
    assert rdi.Effect(shader).parameters["strands"] == 8
    for parameters in ({"strands": 2.5}, {"strands": 20}, {"missing": 1}, {"density": float("nan")}):
        with pytest.raises(rdi.InvalidEffect):
            rdi.Effect(shader, parameters=parameters)
    with pytest.raises(rdi.InvalidEffect, match="not both"):
        rdi.Effect(shader, params=(8, 1, 2, 0.8), parameters={"strands": 8})


@pytest.mark.skipif(sys.platform != "win32", reason="Windows HLSL compiler")
def test_procedural_descriptor_interface() -> None:
    parameters = [rdi.EffectParameter(f"value{index}", 0.5, min=0, max=1) for index in range(5)]
    recipe = rdi.ProceduralSource([
        rdi.RenderPass(pixel="float4 pixel(VertexOutput input) : SV_Target { return parameter(4).xxxx; }", output=0),
        rdi.RenderPass(pixel="float4 pixel(VertexOutput input) : SV_Target { return pass_inputs[0].SampleLevel(artwork_sampler, input.uv, 0); }", inputs=[0]),
    ], parameters=parameters, targets=[rdi.RenderTarget(32, 32)])
    source = rdi.ShaderSource(pipeline=rdi.ShaderPipeline.Procedural, execution=recipe)
    assert source.execution is not None
    assert source.execution.passes[1].inputs == [0]
    shader = rdi.Shader.compile(source)
    effect = rdi.Effect(shader, parameters={"value4": 0.75})
    assert effect.parameters["value4"] == 0.75
    assert effect.params == (0.5, 0.5, 0.5, 0.5)
    assert rdi.Effect(shader).parameters["value4"] == 0.5
    silk = rdi.ShaderSource.builtin(rdi.BuiltinShader.SilkFlow)
    assert silk.execution is not None
    assert silk.execution.body_artwork and silk.execution.label_artwork
    assert silk.execution.passes[0].count_parameter == "strands"
    assert silk.execution.parameters[0].kind == "integer"
    copied = rdi.ShaderSource(silk.pixel, vertex=silk.vertex, pipeline=silk.pipeline, execution=silk.execution)
    assert rdi.Effect(rdi.Shader.compile(copied)).parameters == rdi.Effect(rdi.Shader.compile(silk)).parameters
    for invalid in [
        rdi.ProceduralSource([]),
        rdi.ProceduralSource([rdi.RenderPass(inputs=[0])]),
        rdi.ProceduralSource([rdi.RenderPass(output=0)], targets=[rdi.RenderTarget(32, 32)]),
        rdi.ProceduralSource([rdi.RenderPass(output=0), rdi.RenderPass(inputs=[0], output=0)], targets=[rdi.RenderTarget(32, 32)]),
        rdi.ProceduralSource([rdi.RenderPass()], targets=[rdi.RenderTarget(8192, 8192)]),
        rdi.ProceduralSource([rdi.RenderPass(vertices=65538)]),
        rdi.ProceduralSource([rdi.RenderPass(count_parameter="missing")]),
        rdi.ProceduralSource([rdi.RenderPass()], parameters=[parameters[0], parameters[0]]),
    ]:
        with pytest.raises(rdi.InvalidEffect):
            rdi.Shader.compile(rdi.ShaderSource(pipeline="procedural", execution=invalid))
    with pytest.raises(rdi.InvalidEffect, match="procedural"):
        rdi.Shader.compile(rdi.ShaderSource(execution=recipe))
    with pytest.raises(rdi.InvalidEffect):
        rdi.RenderPass(topology="points")
    with pytest.raises(rdi.InvalidEffect):
        rdi.RenderPass(blend="unknown")
    with pytest.raises(rdi.InvalidEffect):
        rdi.EffectParameter("count", 0.5, min=0, max=4, kind="integer")


def test_desktop_grid_api_and_option_validation() -> None:
    for name in ("DesktopInfo", "IconGrid", "InvalidGrid"):
        assert name in rdi.__all__
        assert getattr(rdi, name) is not None
    assert issubclass(rdi.InvalidGrid, RustyDesktopError)
    assert not AnimationOptions().snap_to_grid
    assert AnimationOptions(snap_to_grid=True).snap_to_grid
    assert "snap_to_grid=true" in repr(AnimationOptions(snap_to_grid=True))
    controller = rdi.DesktopController()
    specs = [IconAnimationSpec("missing", (0, 0), Duration.fixed(0.1), Curve.linear())]
    with pytest.raises(TypeError):
        controller.prepare(specs, {"snap_to_grid": "not a bool"})
    if sys.platform != "win32":
        with pytest.raises(UnsupportedPlatform):
            controller.desktop_info()


@pytest.mark.skipif(sys.platform != "win32" or os.environ.get("RDI_DESKTOP_TESTS") != "1",
                    reason="opt-in read-only desktop grid preparation")
def test_desktop_grid_metrics_and_cancelled_preparation() -> None:
    controller = rdi.DesktopController()
    icons = controller.list_icons()
    before = {icon.id: icon.position for icon in icons}
    flags = controller.get_flags()
    info = controller.desktop_info()
    data = json.loads(json.dumps(info.to_dict()))
    assert len(data["monitors"]) == len(info.monitors) > 0
    assert len(data["grids"]) == len(info.grids)
    for monitor in info.monitors:
        assert monitor.resolution == (monitor.bounds.width, monitor.bounds.height)
        assert monitor.dpi == pytest.approx(monitor.scale_factor * 96.0)
    for grid in info.grids:
        assert grid.cell_size[0] > 0 and grid.cell_size[1] > 0
        assert grid.icon_size[0] > 0 and grid.icon_size[1] > 0
        assert grid.capacity == grid.columns * grid.rows
        assert grid.to_dict()["origin"] == grid.origin
    usable = next((grid for grid in info.grids if grid.origin is not None and grid.capacity >= len(icons)), None)
    if usable is None or not icons:
        pytest.skip("no known grid with capacity for this desktop")
    assert usable.origin is not None
    specs = [{"id": icon.id, "target": (usable.origin[0] + 1, usable.origin[1] + 1),
              "duration": Duration.fixed(0.1), "curve": Curve.linear()} for icon in icons]
    for options in (AnimationOptions(snap_to_grid=True), {"snap_to_grid": True}):
        with controller.prepare(specs, options):
            assert controller.desktop_info().grids[0].cell_size == info.grids[0].cell_size
            assert {icon.id: icon.position for icon in controller.list_icons()} == before
        assert {icon.id: icon.position for icon in controller.list_icons()} == before
        assert controller.get_flags() == flags
    with pytest.raises(rdi.BackendUnavailable, match="duplicate animation icon"):
        controller.prepare([specs[0], specs[0]], {"snap_to_grid": True})
    assert {icon.id: icon.position for icon in controller.list_icons()} == before
    assert controller.get_flags() == flags


def test_movement_curve_can_hold_before_and_after_motion() -> None:
    movement = Curve.keyframes([(0.0, 0.0), (0.2, 0.0), (0.8, 1.0), (1.0, 1.0)])
    assert movement.eval(0.1) == 0.0
    assert movement.eval(0.5) == pytest.approx(0.5)
    assert movement.eval(0.9) == 1.0


def test_timeline_api_exports() -> None:
    for name in ("TimelineSession", "TimelineState", "TimelineCloseMode", "PlaybackHandle", "PlaybackOutcome"):
        assert name in rdi.__all__
        assert getattr(rdi, name) is not None
    assert rdi.TimelineState.Paused != rdi.TimelineState.Closed
    assert rdi.PlaybackOutcome.Reached != rdi.PlaybackOutcome.Interrupted
    assert rdi.TimelineCloseMode.RestoreOrigins != rdi.TimelineCloseMode.LeaveInPlace
    assert callable(rdi.PreparedAnimation.open_timeline)
    assert callable(rdi.TimelineSession.set_real_icons_visible)
    assert callable(rdi.TimelineSession.real_icons_visible)


@pytest.mark.skipif(os.environ.get("RDI_DESKTOP_TESTS") != "1", reason="opt-in real desktop timeline lifecycle")
@pytest.mark.parametrize("particles", [False, True], ids=["glitch", "particle-vortex"])
def test_timeline_desktop_lifecycle(particles: bool) -> None:
    controller = rdi.DesktopController()
    icons = controller.list_icons()
    if not icons:
        pytest.skip("desktop has no icons")
    before = {icon.id: icon.position for icon in icons}
    flags = controller.get_flags()
    shader = rdi.Shader.compile(rdi.ShaderSource.builtin("particle-vortex" if particles else "glitch"))
    specs = [IconAnimationSpec(icon.id, icon.position, Duration.fixed(0.4), Curve.linear(),
                              effect=rdi.Effect(shader, seed=float(index)))
             for index, icon in enumerate(icons)]
    prepared = controller.prepare(specs, AnimationOptions(tick_hz=144))
    with prepared.open_timeline() as timeline:
        with pytest.raises(RuntimeError, match="consumed"):
            prepared.open_timeline()
        assert timeline.state() == rdi.TimelineState.Paused
        assert timeline.position() == 0.0
        assert not timeline.real_icons_visible()
        for visible in (True, False):
            timeline.set_real_icons_visible(visible)
            assert timeline.real_icons_visible() == visible
            assert (controller.get_flags() & 0x200 == 0) == visible
            assert timeline.position() == 0.0
        time.sleep(0.05)
        assert timeline.position() == 0.0
        for position in (0.5, 0.0, 1.0, 0.25):
            playback = timeline.play_to(position, speed=2.0)
            assert playback.wait_timeout(5.0) == rdi.PlaybackOutcome.Reached
            assert playback.wait() == rdi.PlaybackOutcome.Reached
            assert timeline.position() == position
            assert timeline.state() == rdi.TimelineState.Paused
            assert timeline.final_commit() is None
        playback = timeline.play_to(1.0, speed=0.001)
        assert playback.wait_timeout(0.0) is None
        timeline.set_speed(0.002)
        assert timeline.speed() == 0.002
        timeline.pause()
        assert playback.wait() == rdi.PlaybackOutcome.Interrupted
        timeline.seek(0.5)
        assert all(state.t == pytest.approx(0.5) for state in timeline.snapshot())
        captured = timeline.capture()
        assert captured.seconds == pytest.approx(0.2)
        assert any(captured.pixels)
        timeline.seek_and_capture(1.0)
        assert timeline.seek_and_capture(0.5).pixels == captured.pixels
        assert timeline.state() == rdi.TimelineState.Paused
        assert not timeline.missing_icons()
        with pytest.raises(rdi.AnimationBusy):
            controller.set_positions([])
        for position in (math.nan, math.inf, -0.1, 1.1):
            with pytest.raises(InvalidDuration):
                timeline.seek(position)
        for speed in (math.nan, math.inf, -1.0, 0.0):
            with pytest.raises(InvalidDuration):
                timeline.set_speed(speed)
        with pytest.raises(ValueError):
            playback.wait_timeout(-1.0)
    assert timeline.state() == rdi.TimelineState.Closed
    assert timeline.finish_reason() is not None
    outcome = timeline.final_commit()
    assert outcome is not None and not outcome.missing_ids
    with pytest.raises(rdi.BackendUnavailable):
        timeline.seek(0.0)
    timeline.close()
    assert {icon.id: icon.position for icon in controller.list_icons()} == before
    assert controller.get_flags() == flags

    with pytest.raises(ValueError, match="context exit"):
        with controller.prepare(specs).open_timeline() as timeline:
            timeline.seek(0.8)
            raise ValueError("context exit")
    assert timeline.state() == rdi.TimelineState.Closed
    assert {icon.id: icon.position for icon in controller.list_icons()} == before
    assert controller.get_flags() == flags


@pytest.mark.skipif(os.environ.get("RDI_DESKTOP_TESTS") != "1", reason="opt-in real desktop shader lifecycle")
@pytest.mark.parametrize("particles", [False, True], ids=["glitch", "particle-vortex"])
def test_prepared_shader_desktop_lifecycle(particles: bool) -> None:
    controller = rdi.DesktopController()
    icons = controller.list_icons()
    if not icons:
        pytest.skip("desktop has no icons")
    before = {icon.id: icon.position for icon in icons}
    flags = controller.get_flags()
    shader = rdi.Shader.compile(rdi.ShaderSource.builtin("particle-vortex" if particles else "glitch"))
    specs = [IconAnimationSpec(icon.id, icon.position, Duration.fixed(0.8), Curve.linear(),
                              effect=rdi.Effect(shader, seed=float(index)))
             for index, icon in enumerate(icons)]
    options = AnimationOptions(tick_hz=144)
    prepared = controller.prepare(specs, options)
    assert {icon.id: icon.position for icon in controller.list_icons()} == before
    assert controller.get_flags() == flags
    with pytest.raises(rdi.AnimationBusy):
        controller.set_positions([])
    prepared.cancel()
    prepared.cancel()
    with pytest.raises(RuntimeError, match="consumed"):
        prepared.start()
    with controller.prepare(specs, options) as prepared:
        clock = time.perf_counter()
        ticks: list[float] = []
        handle = prepared.start()
        handle.on_tick(lambda _context: ticks.append(time.perf_counter()))
        with pytest.raises(RuntimeError, match="consumed"):
            prepared.start()
        reason = handle.wait_timeout(10.0)
        assert reason is not None and reason.kind == "completed", str(reason)
        assert time.perf_counter() - clock >= 0.8
        assert len(ticks) > 1
        print(f"desktop icons={len(icons)}, callbacks={len(ticks)}, callback rate={(len(ticks)-1)/(ticks[-1]-ticks[0]):.1f}Hz")
    assert {icon.id: icon.position for icon in controller.list_icons()} == before
    assert controller.get_flags() == flags

def test_builtin_curves_eval_endpoints():
    for factory in (
        Curve.linear,
        Curve.ease_in,
        Curve.ease_out,
        Curve.ease_in_out,
        Curve.quad_in,
        Curve.quad_out,
        Curve.quad_in_out,
        Curve.cubic_in,
        Curve.cubic_out,
        Curve.cubic_in_out,
        Curve.sine_in,
        Curve.sine_out,
        Curve.sine_in_out,
    ):
        c = factory()
        assert c.kind == "builtin"
        assert abs(c.eval(0.0)) < 1e-3
        assert abs(c.eval(1.0) - 1.0) < 1e-3


def test_cubic_bezier_validates_x_range():
    Curve.cubic_bezier(0.42, 0.0, 0.58, 1.0)  # valid
    with pytest.raises(InvalidCurve):
        Curve.cubic_bezier(1.5, 0.0, 0.5, 1.0)


def test_keyframes_dict_and_tuple_forms():
    c1 = Curve.keyframes([(0.0, 0.0), (1.0, 1.0)])
    c2 = Curve.keyframes(
        [{"t": 0.0, "v": 0.0}, {"t": 0.5, "v": 0.7}, {"t": 1.0, "v": 1.0}],
        interp="smoothstep",
    )
    assert c1.kind == "keyframe"
    assert c2.kind == "keyframe"
    assert c2.eval(0.5) == pytest.approx(0.7, abs=0.01)


def test_from_function_samples_python_callable_once():
    calls = 0

    def f(t):
        nonlocal calls
        calls += 1
        return t * t

    c = Curve.from_function(f, samples=17)
    assert c.kind == "keyframe"
    assert calls == 17
    # After construction the extension holds no reference to `f`.
    # Evaluating the curve must not increment `calls`.
    for i in range(11):
        c.eval(i / 10.0)
    assert calls == 17


def test_from_function_bubbles_up_python_exceptions_as_invalid_curve():
    def broken(t):
        if t > 0.4:
            raise RuntimeError("nope")
        return t

    with pytest.raises(RuntimeError, match="nope"):
        Curve.from_function(broken, samples=16)


def test_from_function_rejects_non_finite():
    with pytest.raises(InvalidCurve):
        Curve.from_function(lambda t: float("nan"), samples=8)


def test_from_motion_function_receives_ctx():
    seen = []

    def f(ctx, t):
        seen.append((ctx.distance_px, ctx.duration_seconds, ctx.param("g")))
        return t

    Curve.from_motion_function(
        f,
        origin=(0, 0),
        target=(0, 400),
        duration_seconds=1.5,
        params={"g": 9.8},
        samples=4,
    )
    assert len(seen) == 4
    for dist, dur, g in seen:
        assert dist == pytest.approx(400.0)
        assert dur == pytest.approx(1.5)
        assert g == pytest.approx(9.8)


def test_procedural_curves_construct():
    for c in (
        Curve.spring(),
        Curve.spring(damping=0.7, stiffness=200.0, mass=1.2),
        Curve.bounce(),
        Curve.bounce(bounces=5, decay=0.4),
        Curve.elastic(),
        Curve.overshoot(),
    ):
        assert c.kind == "keyframe"


# ---------------------------------------------------------------------------
# Duration
# ---------------------------------------------------------------------------

def test_duration_fixed():
    d = Duration.fixed(seconds=0.75)
    assert d.kind == "fixed"


def test_duration_fixed_rejects_zero_or_negative():
    with pytest.raises(ValueError):
        Duration.fixed(seconds=0.0)
    with pytest.raises(ValueError):
        Duration.fixed(seconds=-1.0)


def test_duration_distance_variants():
    d = Duration.distance(speed_px_per_sec=300.0)
    assert d.kind == "distance"
    d2 = Duration.distance_clamped(
        speed_px_per_sec=300.0, min_seconds=0.1, max_seconds=1.0
    )
    assert d2.kind == "distance"


def test_duration_distance_rejects_bad_speed():
    with pytest.raises(ValueError):
        Duration.distance(speed_px_per_sec=0.0)
    with pytest.raises(ValueError):
        Duration.distance_clamped(
            speed_px_per_sec=100.0, min_seconds=1.0, max_seconds=0.5
        )


# ---------------------------------------------------------------------------
# Spec / options / flag ops
# ---------------------------------------------------------------------------

def test_spec_from_object_and_dict():
    s = IconAnimationSpec(
        id="abc",
        target=(100, 200),
        duration=Duration.fixed(seconds=0.5),
        curve=Curve.linear(),
    )
    assert s.id == "abc"
    assert s.target == (100, 200)


@pytest.mark.skipif(sys.platform != "win32", reason="Windows folder flags")
def test_folder_flag_catalog() -> None:
    from enum import IntFlag

    assert issubclass(rdi.FolderFlag, IntFlag)
    expected = {
        "FWF_AUTOARRANGE": (0x1, "Auto Arrange", "Automatically arrange the icons"),
        "FWF_SNAPTOGRID": (0x4, "Snap to grid", "Snap icon positions to grid"),
        "FWF_SINGLESEL": (0x40, "Single Select", "Prevents selection of multiple icons"),
        "FWF_NOICONS": (0x1000, "Hide Icons", "Don't show icons"),
        "FWF_SINGLECLICKACTIVATE": (0x8000, "One Click Activate", "Icons open with one click"),
        "FWF_HIDEFILENAMES": (0x20000, "Hide Filenames", "Don't show filenames"),
        "FWF_CHECKSELECT": (0x40000, "Checkbox Select #0", "Rudiment. Check for fun"),
        "FWF_TRICHECKSELECT": (0x4000000, "Checkbox Select #1", "Rudiment. Check for fun"),
        "FWF_AUTOCHECKSELECT": (0x8000000, "Checkbox Select #2", "Rudiment. Check for fun"),
    }
    actual = {name: (flag.value, flag.title, flag.description)
              for name, flag in rdi.FolderFlag.__members__.items()}
    assert {name: actual[name] for name in expected} == expected
    assert len(actual) == 33
    assert {flag.value for flag in rdi.FolderFlag.__members__.values()} == {0, *(1 << shift for shift in range(32))}
    assert rdi.FolderFlag.FWF_NONE.value == 0
    assert rdi.FolderFlag.FWF_ALLOWRTLREADING.value == 0x80000000
    assert all(flag.title and flag.description for flag in rdi.FolderFlag.__members__.values())
    stub = ast.parse((Path(rdi.__file__).parent / "_flags.pyi").read_text(encoding="utf-8"))
    definition = next(node for node in stub.body if isinstance(node, ast.ClassDef))
    declared = {node.targets[0].id: ast.literal_eval(node.value)
                for node in definition.body if isinstance(node, ast.Assign)}
    assert declared == {name: flag.value for name, flag in rdi.FolderFlag.__members__.items()}


@pytest.mark.skipif(sys.platform != "win32", reason="Windows folder flags")
def test_folder_flag_masks_and_native_conversion() -> None:
    flags = rdi.FolderFlag.FWF_AUTOARRANGE | rdi.FolderFlag.FWF_SNAPTOGRID
    assert isinstance(flags, rdi.FolderFlag)
    assert int(flags) == 5
    assert flags & rdi.FolderFlag.FWF_SNAPTOGRID
    assert int(~rdi.FolderFlag.FWF_AUTOARRANGE) == 0xFFFFFFFE
    assert flags.title is None and flags.description is None
    for mask in (flags, rdi.FolderFlag(0), rdi.FolderFlag(0x80000000), rdi.FolderFlag(0xFFFFFFFF)):
        assert FolderFlagOp.set(mask).flags == int(mask)
        assert FolderFlagOp.exactly(mask).flags == int(mask)
        AnimationOptions(before_flags=("set", mask), after_flags={"kind": "exactly", "flags": mask})


@pytest.mark.skipif(sys.platform == "win32", reason="Non-Windows export contract")
def test_folder_flag_not_exported_on_other_platforms() -> None:
    assert not hasattr(rdi, "FolderFlag")
    assert "FolderFlag" not in rdi.__all__


def test_folder_flag_op_forms():
    a = FolderFlagOp.set(0x4)
    b = FolderFlagOp.exactly(0x40)
    assert a.kind == "set" and a.flags == 0x4
    assert b.kind == "exactly" and b.flags == 0x40


def test_animation_options_dict_accepted_by_ctor():
    opts = AnimationOptions(
        tick_hz=60,
        position_tolerance_px=5,
        before_flags=("set", 0x4),
        after_flags={"kind": "exactly", "flags": 0x20},
    )
    assert opts.tick_hz == 60
    assert opts.position_tolerance_px == 5


# ---------------------------------------------------------------------------
# Logging bridge (loguru)
# ---------------------------------------------------------------------------

def test_init_logging_is_idempotent_and_returns_bool():
    # The extension installs the subscriber at import, so every call
    # from a test is a retune and must report False rather than raise.
    assert rdi.init_logging(level="warn") is False
    assert rdi.init_logging(level="debug", target="stderr") is False
    rdi.init_logging(level="warn")


def test_init_logging_accepts_python_alias_for_loguru():
    # "python" is a legacy target name; kept working on purpose.
    assert rdi.init_logging(level="warn", target="python") is False
    assert rdi.init_logging(level="warn", target="loguru") is False


def test_init_logging_rejects_stdout():
    # stdout carries the JSON-RPC stream for stdio MCP servers.
    with pytest.raises(ValueError, match="stdout"):
        rdi.init_logging(target="stdout")


def test_init_logging_rejects_unknown_target():
    with pytest.raises(ValueError, match="unknown log target"):
        rdi.init_logging(target="syslog")


def test_logger_name_is_exported():
    assert rdi.LOGGER_NAME == "rusty_desktop_icons"


def test_emit_routes_through_loguru_with_rust_source_info():
    """The emitter must attribute records to the Rust file, not to itself."""
    from loguru import logger

    from rusty_desktop_icons import _logging

    seen = []
    sink_id = logger.add(lambda m: seen.append(m.record), level="TRACE")
    try:
        _logging.emit(
            level="WARN",
            target="rdi_platform_windows::backend",
            message="probe",
            scope="animation",
            file="crates/rdi-platform-windows/src/backend.rs",
            line=1234,
            extras={"icons": "7"},
        )
    finally:
        logger.remove(sink_id)

    assert len(seen) == 1
    rec = seen[0]
    assert rec["message"] == "probe"
    assert rec["level"].name == "WARNING"
    # Source attribution comes from the Rust metadata, not the Python frame.
    assert rec["name"] == "rdi_platform_windows::backend"
    assert rec["function"] == "animation"
    assert rec["line"] == 1234
    assert rec["extra"]["icons"] == "7"
    assert rec["extra"]["rdi_target"] == "rdi_platform_windows::backend"


def test_emit_maps_trace_level_without_folding_onto_debug():
    """loguru has a real TRACE level; stdlib logging did not."""
    from loguru import logger

    from rusty_desktop_icons import _logging

    seen = []
    sink_id = logger.add(lambda m: seen.append(m.record), level="TRACE")
    try:
        _logging.emit(level="TRACE", target="rdi_core::engine", message="tick")
    finally:
        logger.remove(sink_id)

    assert seen[0]["level"].name == "TRACE"


def test_emit_never_raises_on_a_broken_sink():
    """A logging failure must not abort an in-flight animation."""
    from loguru import logger

    from rusty_desktop_icons import _logging

    def exploding_sink(_message):
        raise RuntimeError("sink is broken")

    sink_id = logger.add(exploding_sink, level="TRACE", catch=False)
    try:
        _logging.emit(level="ERROR", target="rdi_core::engine", message="boom")
    finally:
        logger.remove(sink_id)


# ---------------------------------------------------------------------------
# Error hierarchy
# ---------------------------------------------------------------------------

def test_error_hierarchy():
    for cls in (
        rdi.IconNotFound,
        rdi.InvalidCurve,
        rdi.InvalidDuration,
        rdi.UnsupportedPlatform,
        rdi.BackendUnavailable,
        rdi.AnimationBusy,
        rdi.WorkerCrashed,
        rdi.ComError,
    ):
        assert issubclass(cls, RustyDesktopError)
    assert issubclass(RustyDesktopError, Exception)


def test_invalid_curve_is_raised_and_catchable_as_base():
    with pytest.raises(RustyDesktopError):
        Curve.from_function(lambda t: 1e6, samples=4)  # exceeds VALUE_CLAMP


# ---------------------------------------------------------------------------
# StopMode enum
# ---------------------------------------------------------------------------

def test_stop_mode_enum_has_two_variants():
    assert StopMode.LeaveInPlace != StopMode.TeleportToTarget


# ---------------------------------------------------------------------------
# StubBackend behaviour (documented contract — only meaningful on non-Windows)
# ---------------------------------------------------------------------------

@pytest.mark.skipif(
    __import__("platform").system() == "Windows",
    reason="only the stub backend is expected to fail; Windows uses the real backend",
)
def test_desktop_controller_raises_unsupported_on_non_windows():
    with pytest.raises(UnsupportedPlatform):
        rdi.DesktopController().list_icons()


# ---------------------------------------------------------------------------
# MonitorInfo / Rect (Windows-only real desktop)
# ---------------------------------------------------------------------------

@pytest.mark.skipif(
    __import__("platform").system() != "Windows",
    reason="requires the Windows backend to enumerate real monitors",
)
def test_list_monitors_reports_at_least_primary():
    ctrl = rdi.DesktopController()
    monitors = ctrl.list_monitors()
    assert len(monitors) >= 1
    assert any(m.is_primary for m in monitors), "no primary monitor reported"
    for m in monitors:
        assert isinstance(m, MonitorInfo)
        assert isinstance(m.bounds, Rect)
        assert isinstance(m.work_area, Rect)
        assert m.bounds.width > 0
        assert m.bounds.height > 0
        assert m.scale_factor > 0.0


@pytest.mark.skipif(
    __import__("platform").system() != "Windows",
    reason="requires the Windows backend to enumerate real monitors",
)
def test_monitor_for_point_locates_origin():
    ctrl = rdi.DesktopController()
    # The primary monitor always contains (0, 0).
    m = ctrl.monitor_for_point((0, 0))
    assert m is not None
    assert m.is_primary is True


@pytest.mark.skipif(
    __import__("platform").system() != "Windows",
    reason="requires the Windows backend to enumerate real monitors",
)
def test_monitor_for_point_returns_none_far_away():
    ctrl = rdi.DesktopController()
    # A point 10 million pixels away is guaranteed to be off every
    # display on any reasonable configuration.
    assert ctrl.monitor_for_point((10_000_000, 10_000_000)) is None


def test_rect_contains_matches_half_open_bounds():
    # Rect has no public constructor; the only way to obtain one is
    # indirectly via a MonitorInfo, which needs a real Windows backend.
    import platform

    if platform.system() != "Windows":
        pytest.skip("no way to obtain a Rect off-Windows without a real backend")

    ctrl = rdi.DesktopController()
    for m in ctrl.list_monitors():
        b = m.bounds
        # Top-left is inside, bottom-right is outside (half-open).
        assert b.contains((b.left, b.top))
        assert not b.contains((b.right, b.bottom))
        # A point one column to the left of `right` is inside.
        assert b.contains((b.right - 1, b.top))
        # A point one row above `top` is outside.
        assert not b.contains((b.left, b.top - 1))
