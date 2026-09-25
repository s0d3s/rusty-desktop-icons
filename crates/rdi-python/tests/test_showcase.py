"""Image pipeline tests that do not change the real desktop."""

from __future__ import annotations

import importlib.util
import json
import os
import sys
from collections.abc import Iterator
from pathlib import Path
from types import ModuleType
from unittest.mock import MagicMock, Mock

import pytest
Image = pytest.importorskip("PIL.Image")
pytest.importorskip("apng")


@pytest.fixture
def images() -> ModuleType:
    path = Path(__file__).resolve().parents[3] / "scripts/showcases/assemble-showcase.py"
    spec = importlib.util.spec_from_file_location("assemble_showcase", path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@pytest.mark.parametrize("keep_alpha", [False, True, None])
@pytest.mark.parametrize("mockup", [False, True])
def test_streamed_apng_alpha_policy(images: ModuleType, tmp_path: Path, keep_alpha: bool | None, mockup: bool) -> None:
    source = tmp_path / "sequence"
    source.mkdir()
    colors = [(255, 0, 0, 128), (0, 255, 0, 255), (0, 0, 0, 0), (255, 0, 0, 128)]
    for index, color in enumerate(colors):
        Image.new("RGBA", (4, 4), color).save(source / f"{index}.png")
    background = tmp_path / "background.png"
    Image.new("RGBA", (4, 4), (20, 40, 60, 128)).save(background)
    manifest = {"version": 2, "width": 4, "height": 4,
                "background": {"mode": "native", "path": str(background)} if mockup else {"mode": "none"},
                "frames": [{"file": f"{index}.png", "duration_ms": delay}
                           for index, delay in enumerate([17, 16, 17, 65535])]}
    if keep_alpha is not None:
        manifest["keep_alpha"] = keep_alpha
    (source / "manifest.json").write_text(json.dumps(manifest))
    outputs = [tmp_path / "serial.png", tmp_path / "parallel.png"]
    for encoders, output in zip((1, 4), outputs):
        images.assemble(source, output, encoders=encoders)
        with Image.open(output) as actual, images.background_image(manifest) as backdrop:
            assert actual.mode == ("RGB" if keep_alpha is False else "RGBA")
            assert actual.n_frames == 4 and actual.info["loop"] == 0
            for index, color in enumerate(colors):
                actual.seek(index)
                with Image.new("RGBA", (4, 4), color) as overlay:
                    with Image.alpha_composite(backdrop, overlay) as composed:
                        expected = composed.convert("RGB").convert("RGBA") if keep_alpha is False else composed
                        assert actual.convert("RGBA").tobytes() == expected.tobytes()
                assert actual.info["duration"] == manifest["frames"][index]["duration_ms"]
    assert outputs[0].read_bytes() == outputs[1].read_bytes()


def test_transparent_passthrough_and_normalization(images: ModuleType, tmp_path: Path,
                                                  monkeypatch: pytest.MonkeyPatch) -> None:
    path = tmp_path / "rgba.png"
    Image.new("RGBA", (4, 4), (80, 40, 20, 128)).save(path, compress_level=1)
    raw = path.read_bytes()
    with monkeypatch.context() as patch:
        patch.setattr(Image.Image, "save", Mock(side_effect=AssertionError("re-encoded PNG")))
        assert images.encode_frame(path, (4, 4), None, True).to_bytes() == raw
    from PIL.PngImagePlugin import PngInfo
    metadata = PngInfo()
    metadata.add_text("note", "normalize ancillary chunks")
    Image.new("RGBA", (4, 4), (80, 40, 20, 128)).save(path, pnginfo=metadata)
    assert all(chunk.type in {"IHDR", "IDAT", "IEND"}
               for chunk in images.encode_frame(path, (4, 4), None, True).chunks)
    Image.new("P", (4, 4), 0).save(path, transparency=0)
    normalized = images.encode_frame(path, (4, 4), None, True)
    assert normalized.hdr[16:21] == bytes([8, 6, 0, 0, 0])
    import io
    with Image.open(io.BytesIO(normalized.to_bytes())) as decoded:
        assert decoded.getchannel("A").getextrema() == (0, 0)


@pytest.mark.parametrize("failure", ["missing", "size", "crc", "animated"])
def test_stream_failure_cleans_only_owned_partial(images: ModuleType, tmp_path: Path, failure: str) -> None:
    source = tmp_path / "sequence"
    source.mkdir()
    Image.new("RGBA", (4, 4), "red").save(source / "good.png")
    broken = source / "bad.png"
    if failure == "size":
        Image.new("RGBA", (8, 4), "red").save(broken)
    elif failure == "crc":
        from apng import PNG
        png = PNG.open(source / "good.png")
        damaged = bytearray(png.to_bytes())
        offset = damaged.index(b"IDAT") + 4
        damaged[offset] ^= 1
        broken.write_bytes(damaged)
    elif failure == "animated":
        Image.new("RGBA", (4, 4), "red").save(broken, save_all=True,
            append_images=[Image.new("RGBA", (4, 4), "blue")], duration=20)
    manifest = {"version": 2, "width": 4, "height": 4, "keep_alpha": True,
                "background": {"mode": "none"},
                "frames": [{"file": name, "duration_ms": 20} for name in ("good.png", "bad.png")]}
    (source / "manifest.json").write_text(json.dumps(manifest))
    output = tmp_path / "animation.png"
    with pytest.raises((OSError, ValueError, SyntaxError)):
        images.assemble(source, output, encoders=2)
    assert not output.exists() and not output.with_suffix(".png.partial").exists()
    partial = output.with_suffix(".png.partial")
    partial.write_bytes(b"another run")
    images.assemble(source, output)
    images.assemble(source, output, overwrite=True)
    assert partial.read_bytes() == b"another run"


def test_stream_writer_is_incremental(images: ModuleType, monkeypatch: pytest.MonkeyPatch) -> None:
    import io
    from apng import APNG, PNG

    monkeypatch.setattr(APNG, "save", Mock(side_effect=AssertionError("buffered save")))
    monkeypatch.setattr(APNG, "to_bytes", Mock(side_effect=AssertionError("buffered animation")))
    with io.BytesIO() as encoded:
        Image.new("RGBA", (4, 4), "red").save(encoded, format="PNG")
        png = PNG.from_bytes(encoded.getvalue())
    destination = io.BytesIO()

    def frames() -> Iterator[object]:
        previous = 0
        for _ in range(3):
            yield png
            assert destination.tell() > previous
            previous = destination.tell()

    images.write_apng(destination, frames(), [17, 16, 17])
    destination.seek(0)
    with Image.open(destination) as decoded:
        assert decoded.n_frames == 3


@pytest.mark.parametrize("size,capacity", [((4, 4), 8), ((1920, 1080), 4), ((8192, 2048), 1)])
def test_assembly_queue_bounds(images: ModuleType, monkeypatch: pytest.MonkeyPatch,
                               size: tuple[int, int], capacity: int) -> None:
    pending: list[Mock] = []
    peak = 0
    pool = MagicMock()
    pool.__enter__.return_value = pool

    def submit(function: object, path: Path, *arguments: object) -> Mock:
        nonlocal peak
        future = Mock()
        def result() -> Path:
            pending.remove(future)
            return path
        future.result.side_effect = result
        pending.append(future)
        peak = max(peak, len(pending))
        return future

    pool.submit.side_effect = submit
    monkeypatch.setattr(images, "ThreadPoolExecutor", Mock(return_value=pool))
    paths = [Path(str(index)) for index in range(20)]
    assert list(images.encoded_frames(paths, size, None, True, 4)) == paths
    assert peak == capacity and not pending


def test_parallel_assembly_cli(images: ModuleType, tmp_path: Path) -> None:
    import subprocess

    for name in ("first", "second"):
        sequence = tmp_path / name
        sequence.mkdir()
        Image.new("RGBA", (4, 4), (255, 0, 0, 128)).save(sequence / "frame.png")
        manifest = {"version": 2, "width": 4, "height": 4, "keep_alpha": name == "first",
                    "background": {"mode": "none"}, "frames": [{"file": "frame.png", "duration_ms": 17}]}
        (sequence / "manifest.json").write_text(json.dumps(manifest))
    command = [sys.executable, str(images.__file__), "--input", str(tmp_path), "--jobs", "2", "--encoders", "2"]
    subprocess.run(command, check=True, capture_output=True, text=True)
    for name, mode in (("first", "RGBA"), ("second", "RGB")):
        with Image.open(tmp_path / f"{name}.png") as decoded:
            assert decoded.mode == mode and decoded.info["duration"] == 17
    original = (tmp_path / "first.png").read_bytes()
    skipped = subprocess.run(command, check=True, capture_output=True, text=True)
    assert skipped.stdout.count("Exists:") == 2
    assert (tmp_path / "first.png").read_bytes() == original
    Image.new("RGBA", (4, 4), "blue").save(tmp_path / "first/frame.png")
    subprocess.run([*command, "--overwrite"], check=True, capture_output=True, text=True)
    assert (tmp_path / "first.png").read_bytes() != original


def test_showcase_alpha_defaults(showcases: ModuleType) -> None:
    assert not showcases.Showcase().keep_alpha
    for scene in showcases.discover_showcases():
        assert scene.keep_alpha == (scene.name in ("effects_comparison", "silk_flow_solo"))
    with pytest.raises(ValueError, match="keep_alpha"):
        showcases.Showcase(keep_alpha=1)


def test_stream_write_failure_cleanup(images: ModuleType, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    Image.new("RGBA", (4, 4), "red").save(tmp_path / "frame.png")
    manifest = {"version": 2, "width": 4, "height": 4, "background": {"mode": "none"},
                "frames": [{"file": "frame.png", "duration_ms": 17}] * 8}
    (tmp_path / "manifest.json").write_text(json.dumps(manifest))

    def fail(destination: object, frames: Iterator[object], durations: list[int]) -> None:
        next(frames)
        raise OSError("disk full probe")

    monkeypatch.setattr(images, "write_apng", fail)
    with pytest.raises(OSError, match="disk full probe"):
        images.assemble(tmp_path, tmp_path / "output.png", encoders=4)
    assert not (tmp_path / "output.png").exists()
    assert not (tmp_path / "output.png.partial").exists()


def test_alpha_and_schedule(images: ModuleType) -> None:
    image = images.rgba_image(2, 1, bytes([0, 0, 128, 128, 0, 0, 0, 0]))
    assert image.getpixel((0, 0)) == (255, 0, 0, 128)
    assert image.getpixel((1, 0)) == (0, 0, 0, 0)
    schedule = images.frame_schedule(2.0, 60)
    assert len(schedule) == 240
    assert schedule[0][0] == 0.0
    assert schedule[120][0] == 2.0
    assert schedule[121][0] == pytest.approx(119 / 60)
    assert sum(delay for _, delay in schedule) == 4000
    for duration, fps in [(0, 60), (float("nan"), 60), (0.1234, 60), (2, 0)]:
        with pytest.raises(ValueError):
            images.frame_schedule(duration, fps)


@pytest.fixture
def optimizer() -> ModuleType:
    path = Path(__file__).resolve().parents[3] / "scripts/showcases/optimize-showcase.py"
    spec = importlib.util.spec_from_file_location("optimize_showcase", path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@pytest.fixture
def gif_converter() -> ModuleType:
    pytest.importorskip("imageio_ffmpeg")
    path = Path(__file__).resolve().parents[3] / "scripts/showcases/convert-showcase-gif.py"
    spec = importlib.util.spec_from_file_location("convert_showcase_gif", path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@pytest.mark.parametrize("fps,plays", [(15, 0), (20, 1), (30, 3)])
def test_gif_conversion_timing_motion_and_loops(gif_converter: ModuleType, tmp_path: Path, fps: int, plays: int) -> None:
    import io
    from apng import APNG, PNG

    source = tmp_path / "animation.png"
    animation = APNG(num_plays=plays)
    for index in range(60):
        frame = Image.new("RGB", (160, 90), "navy")
        frame.paste("red", (index * 2, 20, index * 2 + 12, 32))
        with io.BytesIO() as encoded:
            frame.save(encoded, format="PNG")
            animation.append(PNG.from_bytes(encoded.getvalue()), delay=1, delay_den=60, depose_op=0, blend_op=0)
    animation.save(source)
    original = source.read_bytes()
    output = tmp_path / "animation.gif"
    result = gif_converter.convert(source, output, fps)
    assert source.read_bytes() == original
    assert result["frames"] == fps
    assert abs(result["duration_ms"] - 1000) <= 10
    with Image.open(output) as decoded:
        assert decoded.info.get("loop", -1) == 0
        assert decoded.size == (160, 90)
        first = decoded.convert("RGB").tobytes()
        decoded.seek(decoded.n_frames - 1)
        assert decoded.convert("RGB").tobytes() != first
    original_output = output.read_bytes()
    assert gif_converter.convert(source, output) is None
    assert output.read_bytes() == original_output
    replacement = gif_converter.convert(source, output, fps=10, overwrite=True)
    assert replacement["frames"] == 10


def test_gif_hd_nonrecursive_and_safety(gif_converter: ModuleType, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    root = tmp_path / "showcases"
    nested = root / "frames"
    nested.mkdir(parents=True)
    (nested / "invalid.png").write_bytes(b"do not read")
    Image.new("RGB", (1920, 1080), "red").save(root / "still.png")
    monkeypatch.setattr(sys, "argv", ["convert-showcase-gif.py", "--input", str(root)])
    gif_converter.main()
    output = tmp_path / "showcases-compressed/still.gif"
    with Image.open(output) as decoded:
        assert decoded.size == (960, 540)
        assert decoded.n_frames == 1
    assert gif_converter.output_size((2400, 3840)) == (960, 540)
    assert gif_converter.output_size((640, 480)) == (640, 480)
    original = output.read_bytes()
    gif_converter.main()
    assert output.read_bytes() == original
    assert list(nested.iterdir()) == [nested / "invalid.png"]
    for fps in (0, 31):
        with pytest.raises(ValueError, match="fps"):
            gif_converter.convert(root / "still.png", tmp_path / "invalid.gif", fps)
    Image.new("RGBA", (20, 20), (255, 0, 0, 128)).save(root / "alpha.png")
    with pytest.raises(ValueError, match="opaque"):
        gif_converter.convert(root / "alpha.png", tmp_path / "alpha.gif")


def test_gif_encoder_failure_cleans_up(gif_converter: ModuleType, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    source = tmp_path / "input.png"
    frames = [Image.new("RGB", (32, 24), color) for color in ("red", "blue")]
    frames[0].save(source, save_all=True, append_images=frames[1:], duration=[100, 200], loop=0)
    output = tmp_path / "gifs/output.gif"

    def fail(arguments: list[str]) -> None:
        raise ValueError("injected encoder failure")

    monkeypatch.setattr(gif_converter, "run_encoder", fail)
    with pytest.raises(ValueError, match="injected"):
        gif_converter.convert(source, output)
    assert not output.exists()
    assert not list(output.parent.iterdir())
    monkeypatch.setattr(sys, "argv", ["converter", "--input", str(tmp_path), "--output", str(tmp_path / "nested")])
    with pytest.raises(SystemExit) as error:
        gif_converter.main()
    assert error.value.code == 2


@pytest.mark.parametrize("dimensions,expected", [((None, None), (1920, 1080)),
                                                ((960, None), (960, 1080)),
                                                ((None, 540), (1920, 540))])
def test_compressed_native_axes(gif_converter: ModuleType, dimensions: tuple[int | None, int | None],
                                expected: tuple[int, int]) -> None:
    params = gif_converter.CompressedOutputParams(size=dimensions)
    assert gif_converter.output_size((1920, 1080), params) == expected
    assert gif_converter.output_size((404, 382)) == (404, 382)
    assert gif_converter.output_size((404, 382), gif_converter.CompressedOutputParams(allow_upscale=True)) == (960, 540)


@pytest.mark.parametrize("settings", [{"format": "avif"}, {"size": (0, None)}, {"size": (True, 20)},
                                     {"size": (20,)}, {"fps": True}, {"gif_colors": 257},
                                     {"gif_dither": "invalid"}, {"webp_quality": 101},
                                     {"webp_method": 7}, {"webp_lossless": 1}, {"allow_upscale": 1}])
def test_compressed_invalid_params(gif_converter: ModuleType, settings: dict[str, object]) -> None:
    with pytest.raises(ValueError):
        gif_converter.CompressedOutputParams(**settings)


@pytest.mark.parametrize("lossless", [False, True])
def test_compressed_webp_alpha(gif_converter: ModuleType, tmp_path: Path, lossless: bool) -> None:
    source = tmp_path / "alpha.png"
    frames = []
    for index in range(15):
        frame = Image.new("RGBA", (64, 40), (0, 0, 0, 0))
        frame.paste((200, 60, 20, 128), (index * 2, 8, index * 2 + 12, 20))
        frames.append(frame)
    frames[0].save(source, save_all=True, append_images=frames[1:], duration=100, loop=0,
                   disposal=0, blend=0)
    original = source.read_bytes()
    output = tmp_path / "alpha.webp"
    params = gif_converter.CompressedOutputParams(format="webp", size=(None, None), fps=10,
                                                 webp_lossless=lossless)
    result = gif_converter.convert(source, output, params=params)
    assert source.read_bytes() == original
    assert result["frames"] == 15
    assert abs(result["duration_ms"] - 1500) <= 10
    with Image.open(output) as decoded:
        assert decoded.size == (64, 40)
        assert decoded.info["loop"] == 0
        for index, frame in enumerate(frames):
            decoded.seek(index)
            actual = decoded.convert("RGBA")
            assert actual.getchannel("A").tobytes() == frame.getchannel("A").tobytes()
            if lossless:
                assert actual.tobytes() == frame.tobytes()
    original_output = output.read_bytes()
    assert gif_converter.convert(source, output, params=params) is None
    assert output.read_bytes() == original_output
    resized = gif_converter.CompressedOutputParams(format="webp", size=(32, 20), fps=10,
                                                   webp_lossless=lossless)
    gif_converter.convert(source, output, params=resized, overwrite=True)
    with Image.open(output) as decoded:
        assert decoded.size == (32, 20)


@pytest.mark.parametrize("lossless", [False, True])
def test_compressed_constant_webp(gif_converter: ModuleType, tmp_path: Path, lossless: bool) -> None:
    import io
    from apng import APNG, PNG

    source = tmp_path / "constant.png"
    frame = Image.new("RGBA", (64, 40), (120, 60, 20, 128))
    with io.BytesIO() as buffer:
        frame.save(buffer, format="PNG")
        animation = APNG()
        for index in range(2):
            animation.append(PNG.from_bytes(buffer.getvalue()), delay=500)
        animation.save(source)
    output = tmp_path / "constant.webp"
    result = gif_converter.convert(source, output, params=gif_converter.CompressedOutputParams(webp_lossless=lossless))
    assert result["duration_ms"] == 1000
    with Image.open(output) as image:
        image.load()
        assert image.info["loop"] == 0
        assert image.info["duration"] == 1000
        assert image.convert("RGBA").getchannel("A").getextrema() == (128, 128)
        if lossless:
            assert image.convert("RGBA").tobytes() == frame.tobytes()


def test_compressed_metadata_without_decode(gif_converter: ModuleType, tmp_path: Path,
                                             monkeypatch: pytest.MonkeyPatch) -> None:
    from PIL import PngImagePlugin

    source = tmp_path / "rgb.png"
    frames = [Image.new("RGB", (32, 24), color) for color in ("red", "blue")]
    frames[0].save(source, save_all=True, append_images=frames[1:], duration=[100, 200], loop=0)
    def forbidden(*args: object, **kwargs: object) -> None:
        raise AssertionError("Unexpected source pixel decode")
    monkeypatch.setattr(PngImagePlugin.PngImageFile, "load", forbidden)
    assert gif_converter.inspect_png(source) == ((32, 24), True, 300, False)
    result = gif_converter.convert(source, tmp_path / "rgb.gif", fps=10)
    assert result["duration_ms"] == 300


@pytest.mark.parametrize("damage", ["truncate", "crc", "sequence", "delay", "count"])
def test_compressed_rejects_bad_metadata(gif_converter: ModuleType, tmp_path: Path, damage: str) -> None:
    from apng import make_chunk, parse_chunks

    source = tmp_path / "broken.png"
    frames = [Image.new("RGB", (32, 24), color) for color in ("red", "blue")]
    frames[0].save(source, save_all=True, append_images=frames[1:], duration=[100, 200], loop=0)
    original = source.read_bytes()
    if damage == "truncate":
        source.write_bytes(original[:-7])
    elif damage == "crc":
        changed = bytearray(original)
        changed[original.index(b"IDAT") + 5] ^= 1
        source.write_bytes(changed)
    else:
        chunks = []
        for kind, chunk in parse_chunks(original):
            payload = bytearray(chunk[8:-4])
            if damage == "sequence" and kind == "fcTL":
                payload[3] = 99
            elif damage == "delay" and kind == "fcTL":
                payload[20:22] = b"\0\0"
            elif damage == "count" and kind == "acTL":
                payload[:4] = (3).to_bytes(4, "big")
            chunks.append(make_chunk(kind, bytes(payload)))
        source.write_bytes(original[:8] + b"".join(chunks))
    with pytest.raises((OSError, ValueError)):
        gif_converter.convert(source, tmp_path / "bad.gif")
    assert not (tmp_path / "bad.gif").exists()
    assert not list(tmp_path.glob(".compressed-*"))


def test_compressed_manifest_and_parallel_cli(gif_converter: ModuleType, tmp_path: Path) -> None:
    import subprocess
    from dataclasses import asdict

    root = tmp_path / "inputs"
    root.mkdir()
    for name, mode in (("opaque", "RGB"), ("transparent", "RGBA")):
        source = root / (name + ".png")
        frames = [Image.new(mode, (64, 40), color) for color in
                  ((200, 20, 40, 128), (20, 40, 200, 200))]
        frames[0].save(source, save_all=True, append_images=frames[1:], duration=200, loop=0)
        sequence = root / name
        sequence.mkdir()
        params = gif_converter.CompressedOutputParams(size=(32, None), fps=10, webp_lossless=True)
        (sequence / "manifest.json").write_text(json.dumps({"compressed_output_params": asdict(params),
                                                            "keep_alpha": mode == "RGBA"}))
        assert gif_converter.conversion_params(source) == (params, mode == "RGBA")
    command = [sys.executable, gif_converter.__file__, "--input", str(root), "--jobs", "2"]
    subprocess.run(command, check=True, capture_output=True, text=True)
    outputs = tmp_path / "inputs-compressed"
    for name in ("opaque.gif", "transparent.webp"):
        with Image.open(outputs / name) as image:
            assert image.size == (32, 40)
            assert image.info["loop"] == 0
    before = {path.name: path.read_bytes() for path in outputs.iterdir()}
    skipped = subprocess.run(command, check=True, capture_output=True, text=True)
    assert skipped.stdout.count("Exists:") == 2
    assert {path.name: path.read_bytes() for path in outputs.iterdir()} == before
    for manifest_path in root.glob("*/manifest.json"):
        manifest = json.loads(manifest_path.read_text())
        manifest["compressed_output_params"]["size"] = [16, 20]
        manifest_path.write_text(json.dumps(manifest))
    subprocess.run([*command, "--overwrite"], check=True, capture_output=True, text=True)
    for output_path in outputs.iterdir():
        with Image.open(output_path) as image:
            assert image.size == (16, 20)
    (root / "invalid.png").write_bytes(b"invalid")
    failed = subprocess.run(command, capture_output=True, text=True)
    assert failed.returncode == 1
    assert "Failed invalid.png" in failed.stdout
    assert not list(outputs.glob(".compressed-*"))


def test_optimizer_lossless_regions_merging_and_timing(optimizer: ModuleType, tmp_path: Path) -> None:
    import io
    from apng import APNG, PNG

    frames = [Image.new("RGBA", (80, 60), "navy") for _ in range(4)]
    for frame in frames[2:]:
        frame.paste("red", (31, 21, 35, 25))
    frames[3].paste("lime", (31, 21, 35, 25))
    source = tmp_path / "source.png"
    animation = APNG(num_plays=3)
    for frame in frames:
        with io.BytesIO() as encoded:
            frame.save(encoded, format="PNG")
            animation.append(PNG.from_bytes(encoded.getvalue()), delay=1, delay_den=60,
                             depose_op=0, blend_op=0)
    animation.save(source)
    original = source.read_bytes()
    output = tmp_path / "optimized.png"
    result = optimizer.optimize(source, output)
    assert source.read_bytes() == original
    assert result["after_frames"] == 3
    encoded = APNG.open(output)
    assert [(control.width, control.height) for _, control in encoded.frames] == [(80, 60), (4, 4), (4, 4)]
    assert [(control.delay, control.delay_den) for _, control in encoded.frames] == [(1, 30), (1, 60), (1, 60)]
    with Image.open(output) as decoded:
        assert decoded.mode == "RGB"
        assert decoded.info["loop"] == 3
        for index, expected in enumerate((frames[0], frames[2], frames[3])):
            decoded.seek(index)
            assert decoded.convert("RGBA").tobytes() == expected.tobytes()
    with pytest.raises(FileExistsError):
        optimizer.optimize(source, output)


def test_optimizer_static_resize_colors_and_nonrecursive_batch(
    optimizer: ModuleType, tmp_path: Path, monkeypatch: pytest.MonkeyPatch,
) -> None:
    root = tmp_path / "showcases"
    nested = root / "frames"
    nested.mkdir(parents=True)
    source = root / "still.png"
    Image.linear_gradient("L").resize((120, 80)).convert("RGB").save(source)
    (nested / "invalid.png").write_bytes(b"must not be read")
    monkeypatch.setattr(sys, "argv", ["optimize-showcase.py", "--input", str(root), "--width", "60", "--colors", "8"])
    optimizer.main()
    output = tmp_path / "showcases-optimized/still.png"
    with Image.open(output) as image:
        assert image.size == (60, 40)
        assert len(image.getcolors(256)) <= 8
        assert image.n_frames == 1
    assert list(nested.iterdir()) == [nested / "invalid.png"]
    before = output.read_bytes()
    optimizer.main()
    assert output.read_bytes() == before


@pytest.mark.parametrize("colors", [None, 8])
def test_optimizer_reprocesses_cropped_animation(optimizer: ModuleType, tmp_path: Path, colors: int | None) -> None:
    import io
    from apng import APNG, PNG

    source = tmp_path / "source.png"
    animation = APNG(num_plays=2)
    for index in range(3):
        frame = Image.new("RGB", (64, 48), "navy")
        frame.paste("red", (index * 10, 10, index * 10 + 4, 14))
        with io.BytesIO() as encoded:
            frame.save(encoded, format="PNG")
            animation.append(PNG.from_bytes(encoded.getvalue()), delay=17, delay_den=1000, depose_op=0, blend_op=0)
    animation.save(source)
    first = tmp_path / "first.png"
    second = tmp_path / "second.png"
    optimizer.optimize(source, first, width=32, colors=colors)
    optimizer.optimize(first, second)
    with Image.open(first) as original, Image.open(second) as decoded:
        for index in range(original.n_frames):
            original.seek(index)
            decoded.seek(index)
            assert decoded.convert("RGBA").tobytes() == original.convert("RGBA").tobytes()
        if colors:
            assert original.mode == "P"
            assert len(original.convert("RGB").getcolors(256)) <= colors


def test_optimizer_preserves_alpha_and_long_delays(optimizer: ModuleType, tmp_path: Path) -> None:
    import io
    from apng import APNG, PNG

    source = tmp_path / "alpha.png"
    animation = APNG()
    frames = []
    for color in ((255, 0, 0, 128), (255, 0, 0, 128), (0, 255, 0, 128), (0, 0, 0, 0)):
        frame = Image.new("RGBA", (16, 16), color)
        frames.append(frame)
        with io.BytesIO() as encoded:
            frame.save(encoded, format="PNG")
            animation.append(PNG.from_bytes(encoded.getvalue()), delay=40000, delay_den=1, depose_op=0, blend_op=0)
    animation.save(source)
    output = tmp_path / "output.png"
    result = optimizer.optimize(source, output)
    assert result["after_frames"] == 4
    with Image.open(output) as decoded:
        assert decoded.mode == "RGBA"
        for index, frame in enumerate(frames):
            decoded.seek(index)
            assert decoded.convert("RGBA").tobytes() == frame.tobytes()
    with pytest.raises(ValueError, match="requires opaque"):
        optimizer.optimize(source, tmp_path / "palette.png", colors=8)
    assert not (tmp_path / "palette.png").exists()


def test_optimizer_verification_failure_and_invalid_options(
    optimizer: ModuleType, tmp_path: Path, monkeypatch: pytest.MonkeyPatch,
) -> None:
    source = tmp_path / "source.png"
    output = tmp_path / "output/result.png"
    Image.new("RGB", (8, 8), "red").save(source)
    for settings in ({"width": 0}, {"colors": 1}, {"colors": 257}):
        with pytest.raises(ValueError):
            optimizer.optimize(source, output, **settings)
    monkeypatch.setattr(optimizer, "verify", Mock(side_effect=ValueError("verification probe")))
    with pytest.raises(ValueError, match="verification probe"):
        optimizer.optimize(source, output)
    assert not output.exists()
    assert list(output.parent.iterdir()) == []
    monkeypatch.setattr(sys, "argv", ["optimize-showcase.py", "--input", str(tmp_path),
                                    "--output", str(tmp_path / "nested")])
    with pytest.raises(SystemExit):
        optimizer.main()


def test_composed_apng_pixels_timing_and_size(images: ModuleType, tmp_path: Path) -> None:
    mockup = tmp_path / "desktop.png"
    Image.new("RGB", (8, 8), (0, 0, 255)).save(mockup)
    records = []
    for index, color in enumerate([(255, 0, 0, 128), (0, 255, 0, 128), (0, 0, 0, 0)]):
        filename = f"{index:06d}.png"
        Image.new("RGBA", (4, 4), color).save(tmp_path / filename)
        records.append({"file": filename, "duration_ms": 20 + index})
    manifest = {"version": 2, "width": 4, "height": 4, "frames": records,
                "background": {"mode": "fit", "path": str(mockup)}}
    (tmp_path / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
    output = tmp_path / "result.png"
    images.assemble(tmp_path, output)
    with Image.open(output) as animation:
        assert animation.n_frames == 3
        assert animation.info["loop"] == 0
        assert animation.convert("RGBA").getpixel((0, 0)) == (128, 0, 127, 255)
        for index in range(3):
            animation.seek(index)
            assert animation.info["duration"] == 20 + index
        assert animation.convert("RGBA").getpixel((0, 0)) == (0, 0, 255, 255)
    with pytest.raises(ValueError, match="upscaling"):
        images.fit_mockup(mockup, (16, 4))
    records[0]["file"] = "../outside.png"
    (tmp_path / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
    with pytest.raises(ValueError, match="basenames"):
        images.assemble(tmp_path, tmp_path / "bad.png")


def test_canvas_api_validation() -> None:
    import rusty_desktop_icons as rdi

    canvas = rdi.Canvas(640, 480, icon_size=64)
    assert (canvas.width, canvas.height, canvas.icon_size, canvas.dpi_scale) == (640, 480, 64, 1.0)
    for width, height, scale in [(0, 480, 1), (8192, 8192, 1), (640, 480, float("nan"))]:
        with pytest.raises(rdi.InvalidEffect):
            rdi.Canvas(width, height, dpi_scale=scale)
    assert callable(rdi.TimelineSession.capture)
    assert callable(rdi.TimelineSession.seek_and_capture)


@pytest.fixture
def showcases(monkeypatch: pytest.MonkeyPatch) -> ModuleType:
    path = Path(__file__).resolve().parents[3] / "scripts/showcases/render-showcases.py"
    monkeypatch.syspath_prepend(str(path.parent))
    spec = importlib.util.spec_from_file_location("render_showcases", path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    monkeypatch.setitem(sys.modules, spec.name, module)
    spec.loader.exec_module(module)
    return module


@pytest.mark.parametrize("encoders,compression", [(1, 6), (2, 1), (4, 0)])
def test_parallel_export_pixels_order_and_reverse(
    showcases: ModuleType, images: ModuleType, tmp_path: Path, encoders: int, compression: int,
) -> None:
    scene = showcases.Showcase("parallel", width=640, height=480, duration=1, fps=4, mockup_background=False)
    session = MagicMock()
    session.__enter__.return_value = session
    session.duration = scene.duration
    schedule = images.frame_schedule(scene.duration, scene.fps)
    pixels = [bytes([0, 0, 16 * index, 128]) * (640 * 480) for index in range(len(schedule))]
    session.render_at.side_effect = [Mock(width=640, height=480, pixels=data) for data in pixels]
    controller = Mock()
    controller.prepare_scene.return_value = session
    showcases.export(controller, [Mock(id="icon")], scene, tmp_path,
                     encoders=encoders, png_compression=compression)
    assert [call.args[0] for call in session.render_at.call_args_list] == [seconds for seconds, _ in schedule]
    manifest = json.loads((tmp_path / "parallel/manifest.json").read_text())
    assert manifest["keep_alpha"] is False
    assert manifest["compressed_output_params"] == {
        "format": "auto", "size": [960, 540], "allow_upscale": False, "fps": 15,
        "gif_colors": 128, "gif_dither": "bayer", "webp_lossless": False,
        "webp_quality": 80, "webp_method": 4,
    }
    assert manifest["frames"] == [{"file": f"{index:06d}.png", "seconds": seconds, "duration_ms": delay}
                                   for index, (seconds, delay) in enumerate(schedule)]
    for index, entry in enumerate(manifest["frames"]):
        with Image.open(tmp_path / "parallel" / entry["file"]) as actual:
            with images.rgba_image(640, 480, pixels[index]) as expected:
                assert actual.tobytes() == expected.tobytes()
    session.__exit__.assert_called_once()


def test_showcase_existing_outputs_report_exists_without_work(
    showcases: ModuleType, images: ModuleType, gif_converter: ModuleType,
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch,
) -> None:
    from queue import Queue

    events = Queue()
    monkeypatch.setattr(showcases.progress, "_events", events)
    controller_factory = Mock(side_effect=AssertionError("desktop accessed during skip"))
    monkeypatch.setattr(showcases.rdi, "DesktopController", controller_factory)
    scene = showcases.Showcase("existing")
    output = tmp_path / scene.name
    output.mkdir()
    final_file = tmp_path / "existing.png"
    final_file.write_bytes(b"keep existing")
    controller = Mock()
    showcases.export(controller, [], scene, tmp_path)
    showcases.export_showcase(scene.name, tmp_path, 1, 1)
    images.assemble(tmp_path / "missing", final_file)
    assert gif_converter.convert(tmp_path / "missing.png", final_file) is None
    controller.prepare_scene.assert_not_called()
    controller_factory.assert_not_called()
    assert final_file.read_bytes() == b"keep existing"
    recorded = []
    while not events.empty():
        recorded.append(events.get_nowait())
    assert [event.stage for event in recorded if event.kind == "end"] == ["Exists"] * 4
    assert not any(event.kind == "update" for event in recorded)


def test_export_overwrite_replaces_sequence_only(showcases: ModuleType, tmp_path: Path) -> None:
    scene = showcases.Showcase("replace", width=640, height=480, duration=1, fps=1,
                               mockup_background=False)
    output = tmp_path / scene.name
    output.mkdir()
    stale = output / "999999.png"
    stale.write_bytes(b"stale frame")
    sibling = tmp_path / "unrelated.txt"
    sibling.write_bytes(b"keep sibling")
    session = MagicMock()
    session.__enter__.return_value = session
    session.duration = 1
    session.render_at.return_value = Mock(width=640, height=480, pixels=bytes(640 * 480 * 4))
    controller = Mock()
    controller.prepare_scene.return_value = session
    showcases.export(controller, [Mock(id="icon")], scene, tmp_path)
    assert stale.read_bytes() == b"stale frame"
    controller.prepare_scene.assert_not_called()
    showcases.export(controller, [Mock(id="icon")], scene, tmp_path, encoders=1, overwrite=True)
    assert not stale.exists()
    assert sorted(path.name for path in output.iterdir()) == ["000000.png", "000001.png", "manifest.json"]
    assert sibling.read_bytes() == b"keep sibling"


@pytest.mark.parametrize("jobs", [1, 2])
def test_render_overwrite_worker_propagation(showcases: ModuleType, tmp_path: Path,
                                           monkeypatch: pytest.MonkeyPatch, jobs: int) -> None:
    worker = Mock()
    monkeypatch.setattr(showcases, "export_showcase", worker)
    pool = MagicMock()
    pool.__enter__.return_value = pool
    monkeypatch.setattr(showcases, "ProcessPoolExecutor", Mock(return_value=pool))
    monkeypatch.setattr(showcases, "as_completed", lambda futures: futures)
    showcases.render_showcases(["first", "second"], tmp_path, jobs=jobs, encoders=1,
                               png_compression=1, overwrite=True)
    calls = worker.call_args_list if jobs == 1 else pool.submit.call_args_list
    assert len(calls) == 2
    assert all(call.kwargs == {"overwrite": True} for call in calls)


def test_render_cli_existing_outputs_skip_in_workers(showcases: ModuleType, tmp_path: Path) -> None:
    import subprocess

    scenes = showcases.discover_showcases()
    for scene in scenes:
        (tmp_path / scene.name).mkdir()
    result = subprocess.run([sys.executable, showcases.__file__, "--output", str(tmp_path),
                             "--jobs", "2", "--no-progress"],
                            check=True, capture_output=True, text=True)
    assert result.stdout.count("Exists:") == len(scenes)
    assert all(not list((tmp_path / scene.name).iterdir()) for scene in scenes)


@pytest.mark.parametrize("format_name", ["gif", "webp"])
def test_compressed_overwrite_failure_preserves_output(
    gif_converter: ModuleType, tmp_path: Path, monkeypatch: pytest.MonkeyPatch, format_name: str,
) -> None:
    source = tmp_path / "source.png"
    Image.new("RGB", (4, 4), "red").save(source)
    original_source = source.read_bytes()
    output = tmp_path / f"output.{format_name}"
    output.write_bytes(b"previous output")
    monkeypatch.setattr(gif_converter, "run_encoder", Mock(side_effect=ValueError("encoder failed")))
    with pytest.raises(ValueError, match="encoder failed"):
        gif_converter.convert(source, output, overwrite=True)
    assert output.read_bytes() == b"previous output"
    assert not list(tmp_path.glob(".compressed-*"))
    with pytest.raises(FileExistsError):
        gif_converter.convert(source, source, overwrite=True)
    assert source.read_bytes() == original_source


def test_assembly_overwrite_failure_preserves_output(images: ModuleType, tmp_path: Path) -> None:
    manifest = {"version": 2, "width": 4, "height": 4, "background": {"mode": "none"},
                "frames": [{"file": "missing.png", "duration_ms": 20}]}
    (tmp_path / "manifest.json").write_text(json.dumps(manifest))
    output = tmp_path / "result.png"
    output.write_bytes(b"previous output")
    with pytest.raises(FileNotFoundError):
        images.assemble(tmp_path, output, overwrite=True)
    assert output.read_bytes() == b"previous output"
    assert not output.with_suffix(".png.partial").exists()


def test_parallel_export_failure_has_no_manifest(
    showcases: ModuleType, tmp_path: Path, monkeypatch: pytest.MonkeyPatch,
) -> None:
    scene = showcases.Showcase("failure", width=640, height=480, duration=1, fps=4, mockup_background=False)
    session = MagicMock()
    session.__enter__.return_value = session
    session.duration = 1
    session.render_at.return_value = Mock(width=640, height=480, pixels=bytes(640 * 480 * 4))
    controller = Mock()
    controller.prepare_scene.return_value = session
    monkeypatch.setattr(showcases, "save_frame", Mock(side_effect=OSError("encoder failure")))
    with pytest.raises(OSError, match="encoder failure"):
        showcases.export(controller, [Mock(id="icon")], scene, tmp_path, encoders=2)
    assert not (tmp_path / "failure/manifest.json").exists()
    assert session.render_at.call_count <= 4
    session.__exit__.assert_called_once()


@pytest.mark.parametrize("size,capacity", [((640, 480), 4), ((8192, 2048), 2)])
def test_parallel_export_bounds_outstanding_bytes(
    showcases: ModuleType, tmp_path: Path, monkeypatch: pytest.MonkeyPatch,
    size: tuple[int, int], capacity: int,
) -> None:
    scene = showcases.Showcase("bounded", width=size[0], height=size[1], duration=1, fps=4,
                               mockup_background=False)
    pending: list[Mock] = []
    peak = 0
    pool = MagicMock()
    pool.__enter__.return_value = pool

    def submit(*arguments: object) -> Mock:
        nonlocal peak
        future = Mock()
        future.result.side_effect = lambda: pending.remove(future)
        pending.append(future)
        peak = max(peak, len(pending))
        return future

    def render(seconds: float) -> Mock:
        assert len(pending) < capacity
        return Mock(width=1, height=1, pixels=bytes(4))

    pool.submit.side_effect = submit
    monkeypatch.setattr(showcases, "ThreadPoolExecutor", Mock(return_value=pool))
    session = MagicMock()
    session.__enter__.return_value = session
    session.duration = 1
    session.render_at.side_effect = render
    controller = Mock()
    controller.prepare_scene.return_value = session
    showcases.export(controller, [Mock(id="icon")], scene, tmp_path, encoders=2)
    assert peak == capacity
    assert pending == []
    assert session.render_at.call_count == pool.submit.call_count == 8


def test_png_workers_overlap_without_moving_render_calls(
    showcases: ModuleType, tmp_path: Path, monkeypatch: pytest.MonkeyPatch,
) -> None:
    from threading import Barrier, get_ident

    owner = get_ident()
    barrier = Barrier(2)
    writer = showcases.save_frame

    def save(*arguments: object) -> None:
        assert get_ident() != owner
        barrier.wait(timeout=5)
        writer(*arguments)

    def render(seconds: float) -> Mock:
        assert get_ident() == owner
        return Mock(width=1, height=1, pixels=bytes([0, 0, 128, 128]))

    monkeypatch.setattr(showcases, "save_frame", save)
    session = MagicMock()
    session.__enter__.return_value = session
    session.duration = 1
    session.render_at.side_effect = render
    controller = Mock()
    controller.prepare_scene.return_value = session
    showcases.export(controller, [Mock(id="icon")],
                     showcases.Showcase("overlap", duration=1, fps=2, mockup_background=False),
                     tmp_path, encoders=2)
    assert session.render_at.call_count == 4


@pytest.mark.parametrize("arguments", [
    ["--jobs", "0"], ["--jobs", "-1"], ["--encoders", "0"],
    ["--png-compression", "-1"], ["--png-compression", "10"],
])
def test_render_cli_rejects_invalid_workers(
    showcases: ModuleType, monkeypatch: pytest.MonkeyPatch, arguments: list[str],
) -> None:
    runner = Mock()
    monkeypatch.setattr(showcases, "render_showcases", runner)
    monkeypatch.setattr(sys, "argv", ["render-showcases.py", *arguments])
    with pytest.raises(SystemExit) as error:
        showcases.main()
    assert error.value.code == 2
    runner.assert_not_called()


@pytest.mark.parametrize("cpus,jobs,encoders", [(None, 1, 1), (2, 2, 1), (16, 4, 4)])
def test_render_cli_defaults_and_overrides(
    showcases: ModuleType, tmp_path: Path, monkeypatch: pytest.MonkeyPatch,
    cpus: int | None, jobs: int, encoders: int,
) -> None:
    runner = Mock()
    monkeypatch.setattr(showcases, "render_showcases", runner)
    monkeypatch.setattr(showcases.os, "cpu_count", lambda: cpus)
    monkeypatch.setattr(sys, "argv", ["render-showcases.py", "--output", str(tmp_path)])
    showcases.main()
    runner.assert_called_once_with([scene.name for scene in showcases.discover_showcases()], tmp_path,
                                   jobs=jobs, encoders=encoders, png_compression=1, overwrite=False)
    runner.reset_mock()
    monkeypatch.setattr(sys, "argv", ["render-showcases.py", "--output", str(tmp_path),
                                      "--showcase", "movement", "--jobs", "3", "--encoders", "2",
                                      "--png-compression", "6", "--overwrite"])
    showcases.main()
    runner.assert_called_once_with(["movement"], tmp_path, jobs=1, encoders=2,
                                   png_compression=6, overwrite=True)


def test_scene_process_dispatch_and_failure(
    showcases: ModuleType, tmp_path: Path, monkeypatch: pytest.MonkeyPatch,
) -> None:
    from concurrent.futures import Future

    runner = Mock()
    monkeypatch.setattr(showcases, "export_showcase", runner)
    executor = Mock()
    monkeypatch.setattr(showcases, "ProcessPoolExecutor", executor)
    showcases.render_showcases(["movement"], tmp_path, jobs=4, encoders=2, png_compression=1)
    runner.assert_called_once_with("movement", tmp_path, 2, 1, overwrite=False)
    executor.assert_not_called()
    pool = MagicMock()
    pool.__enter__.return_value = pool
    executor.return_value = pool
    failed: Future[None] = Future()
    failed.set_exception(RuntimeError("child failure"))
    queued: Future[None] = Future()
    pool.submit.side_effect = [failed, queued]
    with pytest.raises(RuntimeError, match="child failure"):
        showcases.render_showcases(["movement", "glitch"], tmp_path, jobs=4, encoders=2, png_compression=1)
    assert executor.call_args.kwargs["max_workers"] == 2
    assert executor.call_args.kwargs["mp_context"].get_start_method() == "spawn"
    assert [call.args for call in pool.submit.call_args_list] == [
        (runner, name, tmp_path, 2, 1) for name in ("movement", "glitch")]
    assert all(call.kwargs == {"overwrite": False} for call in pool.submit.call_args_list)
    assert queued.cancelled()
    pool.__exit__.assert_called_once()


def test_layout_windows_spacing_and_column_order(showcases: ModuleType) -> None:
    import rusty_desktop_icons as rdi

    icons: list[rdi.IconSnapshot] = []
    assert showcases.Showcase("test", None).layout(icons) == []
    icons = [Mock(spec=rdi.IconSnapshot) for _ in range(10)]
    positions = showcases.Showcase("test", None).layout(icons)
    assert positions[0] == (62, 48)
    assert positions[1] == (62, 146)
    assert positions[8] == (62, 832)
    assert positions[9] == (138, 48)
    scaled = showcases.Showcase("test", None, width=3840, height=2160, dpi_scale=2).layout(icons)
    assert scaled == [(horizontal * 2, vertical * 2) for horizontal, vertical in positions]
    with pytest.raises(ValueError, match="do not fit"):
        showcases.Showcase("test", None, width=100, height=100).layout(icons)


@pytest.mark.parametrize("scale,icon_size", [(1.0, 48), (1.5, 48), (2.0, 64)])
def test_showcase_destinations_are_unique_shuffled_grid_cells(showcases: ModuleType, scale: float, icon_size: int) -> None:
    import rusty_desktop_icons as rdi

    scene = showcases.Showcase("test", None, dpi_scale=scale, icon_size=icon_size)
    cell_width, cell_height, margin, columns, rows = scene.grid_metrics()
    capacity = (columns - 2) * rows
    icons = [Mock(spec=rdi.IconSnapshot) for _ in range(capacity + 1)]
    for count in range(capacity + 1):
        selected = icons[:count]
        positions = scene.layout(selected)
        targets = scene.destinations(selected, positions)
        assert targets == scene.destinations(selected, positions)
        assert len(set(targets)) == count
        for position, target in zip(positions, targets):
            assert target[0] - position[0] in (2 * cell_width, 3 * cell_width)
            assert target[1] != position[1]
            assert (target[1] - margin) % cell_height == 0
            assert margin <= target[1] < margin + rows * cell_height
            assert margin <= target[0] < margin + columns * cell_width
        if count == capacity:
            assert all(target[0] - position[0] == 2 * cell_width for position, target in zip(positions, targets))
        if count == rows:
            assert {target[0] - position[0] for position, target in zip(positions, targets)} == {2 * cell_width, 3 * cell_width}
            assert {target[1] for target in targets} == {position[1] for position in positions}
    with pytest.raises(ValueError, match="two spare columns"):
        scene.destinations(icons, scene.layout(icons))
    with pytest.raises(ValueError, match="at least two rows"):
        short = showcases.Showcase("short", None, height=260)
        short.destinations(icons[:1], short.layout(icons[:1]))


@pytest.mark.parametrize("scale,icon_size", [(1.0, 48), (1.25, 48), (2.0, 64)])
def test_small_output_grid_preserves_desktop_metrics(showcases: ModuleType, scale: float, icon_size: int) -> None:
    scene = showcases.Showcase(width=3840, height=2400, dpi_scale=scale, icon_size=icon_size,
                               output_grid=(6, 5), max_icons=15)
    cell_width, cell_height, margin, columns, rows = scene.grid_metrics()
    assert (columns, rows) == (6, 5)
    assert scene.canvas_size() == (6 * cell_width + 2 * margin,
                                   5 * cell_height + 2 * margin + scene.cell_metrics()[3])
    icons = [Mock() for _ in range(15)]
    positions = scene.layout(icons)
    targets = scene.destinations(icons, positions)
    assert len(set(targets)) == 15
    for origin, target in zip(positions, targets):
        assert target[0] - origin[0] in (2 * cell_width, 3 * cell_width)
        assert margin <= target[0] < margin + 6 * cell_width
        assert margin <= target[1] < margin + 5 * cell_height
    explicit = showcases.Showcase(width=3840, height=2400, dpi_scale=scale, icon_size=icon_size,
                                  output_size=scene.canvas_size())
    assert explicit.grid_metrics() == scene.grid_metrics()
    assert explicit.layout(icons) == positions
    assert (scene.width, scene.height, scene.icon_size, scene.dpi_scale) == (3840, 2400, icon_size, scale)


@pytest.mark.parametrize("settings", [
    {"output_grid": (0, 5)}, {"output_size": (True, 600)}, {"output_size": (600,)},
    {"output_size": (600, 600), "output_grid": (6, 5)}, {"output_size": (2000, 600)},
    {"output_grid": (100, 100)}, {"dpi_scale": float("nan")},
])
def test_invalid_output_geometry(showcases: ModuleType, settings: dict[str, object]) -> None:
    with pytest.raises(ValueError):
        showcases.Showcase(**settings).canvas_size()


@pytest.mark.parametrize("background", [True, False])
def test_small_export_and_assembly(showcases: ModuleType, images: ModuleType, tmp_path: Path,
                                  background: bool, monkeypatch: pytest.MonkeyPatch) -> None:
    import rusty_desktop_icons as rdi

    mockup = tmp_path / "custom-mockup.png"
    if background:
        Image.new("RGB", (1920, 1080), (21, 43, 65)).save(mockup)
    scene = showcases.Showcase("small", output_grid=(6, 5), max_icons=15, duration=1, fps=1,
                               mockup_background=background, mockup_path=mockup, keep_alpha=True)
    controller = Mock(spec=rdi.DesktopController)
    session = MagicMock()
    session.__enter__.return_value = session
    session.duration = 1.0
    width, height = scene.canvas_size()
    pixels = bytes([0, 0, 128, 128]) + bytes(width * height * 4 - 4)
    session.render_at.return_value = Mock(width=width, height=height, pixels=pixels)
    controller.prepare_scene.return_value = session
    icons = [Mock(spec=rdi.IconSnapshot, id=f"icon-{index}") for index in range(230)]
    if not background:
        monkeypatch.setattr(Image, "open", Mock(side_effect=AssertionError("loaded mockup")))
    showcases.export(controller, icons, scene, tmp_path)
    monkeypatch.undo()
    canvas, animations, origins = controller.prepare_scene.call_args.args
    assert (canvas.width, canvas.height, canvas.icon_size, canvas.dpi_scale) == (552, 650, 48, 1.0)
    assert len(animations) == len(origins) == 15
    manifest = json.loads((tmp_path / "small/manifest.json").read_text())
    assert manifest["keep_alpha"] is True
    assert manifest["desktop_size"] == [1920, 1080]
    assert manifest["version"] == 2
    assert manifest["background"] == {"mode": "crop" if background else "none",
                                       "path": str(mockup.resolve()) if background else None}
    output = tmp_path / "small.png"
    monkeypatch.chdir(tmp_path.parent)
    images.assemble(tmp_path / "small", output)
    with Image.open(output) as decoded:
        assert decoded.size == (552, 650)
        assert decoded.n_frames == 2
        assert decoded.info["duration"] == 1000
        assert decoded.convert("RGBA").getpixel((1, 0))[3] == (255 if background else 0)
        if background:
            assert decoded.convert("RGBA").getpixel((1, 0)) == (21, 43, 65, 255)
        if not background:
            assert decoded.convert("RGBA").getpixel((0, 0)) == (255, 0, 0, 128)


def test_cropped_mockup_native_pixels_and_left_start(images: ModuleType, tmp_path: Path) -> None:
    source = tmp_path / "desktop.png"
    Image.linear_gradient("L").resize((800, 800)).convert("RGB").save(source)
    manifest = {"width": 552, "height": 650, "desktop_size": [800, 800],
                "background": {"mode": "crop", "path": str(source)}, "dpi_scale": 1, "taskbar_height": 64}
    with images.background_image(manifest) as cropped, Image.open(source) as original:
        assert cropped.crop((0, 0, 552, 586)).tobytes() == original.convert("RGBA").crop((0, 0, 552, 586)).tobytes()
        assert cropped.getpixel((20, 607)) == (38, 121, 151, 255)
        assert cropped.getpixel((276, 607)) == (227, 239, 237, 255)
    with images.background_image({**manifest, "background": {"mode": "none", "path": None}}) as transparent:
        assert transparent.getbbox() is None
    with pytest.raises(ValueError, match="upscaling"):
        images.background_image({**manifest, "desktop_size": [1920, 1080]})


def test_background_manifest_validation_and_legacy_default(images: ModuleType, monkeypatch: pytest.MonkeyPatch) -> None:
    loader = Mock(return_value=Image.new("RGBA", (4, 4)))
    monkeypatch.setattr(images, "fit_mockup", loader)
    with images.background_image({"version": 1, "width": 4, "height": 4}):
        loader.assert_called_once_with(images.DEFAULT_MOCKUP, (4, 4))
    loader.reset_mock()
    for settings in ({}, {"background": "none"}, {"background": {"mode": "unknown"}},
                     {"background": {"mode": "fit", "path": None}}):
        with pytest.raises(ValueError):
            images.background_image({"version": 2, "width": 4, "height": 4, **settings})
    loader.assert_not_called()


def test_small_variants_inherit_scene_behavior(showcases: ModuleType, images: ModuleType) -> None:
    scenes = {scene.name: scene for scene in showcases.discover_showcases()}
    assert set(name for name in scenes if name.endswith("_small")) == {
        "movement_small", "glitch_small", "dust_transfer_small", "particle_vortex_small", "silk_flow_small"}
    for name in ("movement", "glitch", "dust_transfer", "particle_vortex", "silk_flow"):
        original, small = scenes[name], scenes[name + "_small"]
        assert isinstance(small, type(original))
        if name == "silk_flow":
            assert original.canvas_size() == (1920, 1080)
            assert original.max_icons == 30
            for scene in (original, small):
                assert not scene.keep_alpha
                assert all(scene.effect_envelope().eval(progress) == 1.0 for progress in (0, .5, 1))
        assert (small.shader, small.duration, small.fps, small.dpi_scale, small.icon_size) == (
            original.shader, original.duration, original.fps, original.dpi_scale, original.icon_size)
        assert small.grid_metrics()[3:] == (5, 3)
        assert small.canvas_size() == (404, 382)
        assert small.margin_dip == 12
        assert original.margin_dip == 48
        assert small.max_icons == 6
        icons = [Mock() for _ in range(small.max_icons)]
        positions = small.layout(icons)
        assert positions == [(26, 12), (26, 110), (26, 208),
                     (102, 12), (102, 110), (102, 208)]
        targets = small.destinations(icons, positions)
        assert len(set(targets)) == 6
        for horizontal, vertical in targets:
            assert horizontal in (178, 254, 330)
            assert vertical in (12, 110, 208)
        assert small.mockup_mode == "native"
        assert small.mockup_path.name == "desktop-mockup-small.png"
        manifest = {"version": 2, "width": 404, "height": 382, "background": small.background_manifest()}
        with images.background_image(manifest) as background, Image.open(small.mockup_path) as source:
            assert source.size == (404, 382)
            assert background.tobytes() == source.convert("RGBA").tobytes()
            assert len(background.crop((0, 318, 404, 382)).getcolors(404 * 64)) > 20
            assert background.crop((150, 322, 404, 382)).getcolors() == [(254 * 60, (237, 242, 245, 255))]
        with pytest.raises(ValueError, match="must match output"):
            images.background_image({**manifest, "width": 553})


@pytest.mark.parametrize("name", ["dust_transfer", "dust_transfer_small", "glitch", "particle_vortex", "movement"])
def test_dust_showcase_envelope_override(showcases: ModuleType, monkeypatch: pytest.MonkeyPatch, name: str) -> None:
    import rusty_desktop_icons as rdi

    scene = next(scene for scene in showcases.discover_showcases() if scene.name == name)
    effect = Mock(wraps=rdi.Effect)
    monkeypatch.setattr(rdi, "Effect", effect)
    icons = [Mock(spec=rdi.IconSnapshot, id="icon")]
    positions = scene.layout(icons)
    scene.animations(icons, positions, scene.destinations(icons, positions))
    if name == "movement":
        effect.assert_not_called()
    else:
        effect.assert_called_once()
        envelope = effect.call_args.kwargs["envelope"]
        assert effect.call_args.kwargs["padding_px"] == 24
        assert effect.call_args.kwargs["seed"] == 0.0
        if name.startswith("dust_transfer"):
            assert all(envelope.eval(index / 100) == 1.0 for index in range(101))
        else:
            assert envelope is None


def test_showcase_export_uses_shuffled_targets(showcases: ModuleType, tmp_path: Path) -> None:
    import rusty_desktop_icons as rdi

    icons = [Mock(spec=rdi.IconSnapshot, id=f"icon-{index}") for index in range(10)]
    controller = Mock(spec=rdi.DesktopController)
    controller.prepare_scene.side_effect = RuntimeError("preparation probe")
    scene = showcases.Showcase("test", None)
    with pytest.raises(RuntimeError, match="preparation probe"):
        showcases.export(controller, icons, scene, tmp_path)
    _, specs, positions = controller.prepare_scene.call_args.args
    assert [spec.target for spec in specs] == scene.destinations(icons, scene.layout(icons))
    assert positions == [(icon.id, point) for icon, point in zip(icons, scene.layout(icons))]


@pytest.mark.parametrize("limit,expected", [(None, 3), (1, 1), (3, 3), (5, 3)])
def test_showcase_icon_limit_applies_before_layout_and_to_manifest(
    showcases: ModuleType, tmp_path: Path, monkeypatch: pytest.MonkeyPatch, limit: int | None, expected: int,
) -> None:
    import rusty_desktop_icons as rdi

    icons = [Mock(spec=rdi.IconSnapshot, id=f"icon-{index}") for index in range(3)]
    original_icons = icons.copy()
    scene = showcases.Showcase("limited", width=640, height=480, duration=1.0, fps=1, max_icons=limit)
    original_layout = showcases.Showcase.layout

    def layout(self: object, selected: list[rdi.IconSnapshot]) -> list[tuple[int, int]]:
        assert selected == original_icons[:expected]
        assert selected is not icons
        return original_layout(self, selected)

    monkeypatch.setattr(showcases.Showcase, "layout", layout)
    controller = Mock(spec=rdi.DesktopController)
    session = MagicMock()
    session.__enter__.return_value = session
    session.duration = 1.0
    session.render_at.return_value = Mock(width=640, height=480, pixels=bytes(640 * 480 * 4))
    controller.prepare_scene.return_value = session
    showcases.export(controller, icons, scene, tmp_path)
    _, animations, positions = controller.prepare_scene.call_args.args
    assert [animation.id for animation in animations] == [icon.id for icon in original_icons[:expected]]
    assert [icon_id for icon_id, _ in positions] == [icon.id for icon in original_icons[:expected]]
    manifest = json.loads((tmp_path / "limited/manifest.json").read_text(encoding="utf-8"))
    assert manifest["icons"] == expected
    assert len(manifest["frames"]) == 2
    assert icons == original_icons


@pytest.mark.parametrize("limit", [0, -1, True, False, 1.5, "2"])
def test_showcase_rejects_invalid_icon_limits(showcases: ModuleType, limit: object) -> None:
    with pytest.raises(ValueError, match="max_icons must be a positive integer or None"):
        showcases.Showcase("invalid", max_icons=limit)


@pytest.mark.parametrize("name", ["dust_transfer", "glitch", "movement", "particle_vortex", "project_intro",
                                  "dust_transfer_small", "glitch_small", "movement_small", "particle_vortex_small"])
def test_crowded_desktop_export_respects_scene_limits(
    showcases: ModuleType, tmp_path: Path, monkeypatch: pytest.MonkeyPatch, name: str,
) -> None:
    import rusty_desktop_icons as rdi

    scene = next(scene for scene in showcases.discover_showcases() if scene.name == name)
    expected = 230 if name == "project_intro" else (6 if name.endswith("_small") else 30)
    if name == "project_intro":
        if sys.platform != "win32":
            pytest.skip("project intro uses Windows Segoe UI")
        for dependency in ("numpy", "scipy", "skimage"):
            pytest.importorskip(dependency)
        monkeypatch.setattr(showcases.Showcase, "grid_metrics", Mock(side_effect=AssertionError("intro used grid")))
    assert scene.max_icons == (None if name == "project_intro" else expected)
    icons = [Mock(spec=rdi.IconSnapshot, id=f"icon-{index}") for index in range(230)]
    original_icons = icons.copy()
    controller = Mock(spec=rdi.DesktopController)
    controller.prepare_scene.side_effect = RuntimeError("crowded desktop preparation probe")
    with pytest.raises(RuntimeError, match="crowded desktop preparation probe"):
        showcases.export(controller, icons, scene, tmp_path)
    _, animations, positions = controller.prepare_scene.call_args.args
    assert [animation.id for animation in animations] == [icon.id for icon in icons[:expected]]
    assert [icon_id for icon_id, _ in positions] == [icon.id for icon in icons[:expected]]
    assert icons == original_icons


def test_showcase_discovery_preserves_presets(showcases: ModuleType) -> None:
    expected = [("dust_transfer", "dust-transfer", 4.0), ("dust_transfer_small", "dust-transfer", 4.0),
                ("effects_comparison", None, 4.0),
                ("glitch", "glitch", 2.0), ("glitch_small", "glitch", 2.0),
                ("movement", None, 2.0), ("movement_small", None, 2.0),
                ("particle_vortex", "particle-vortex", 4.0), ("particle_vortex_small", "particle-vortex", 4.0),
                ("project_intro", "glitch", 14.0), ("silk_flow", "silk-flow", 5.0),
                ("silk_flow_small", "silk-flow", 5.0), ("silk_flow_solo", "silk-flow", 5.0)]
    for _ in range(2):
        scenes = showcases.discover_showcases()
        assert [(scene.name, scene.shader, scene.duration) for scene in scenes] == expected
        assert len({type(scene) for scene in scenes}) == 13


def test_effects_comparison_timing(showcases: ModuleType) -> None:
    from dataclasses import replace

    scenes = showcases.discover_showcases()
    comparison = next(scene for scene in scenes if scene.name == "effects_comparison")
    for duration in (1.0, 4.0):
        scene = replace(comparison, duration=duration)
        assert all(scene.render_seconds(duration * progress) == 0 for progress in (0, .075, .15))
        assert all(scene.render_seconds(duration * progress) == duration for progress in (.8, .9, 1))
        samples = [scene.render_seconds(duration * index / 1000) for index in range(1001)]
        assert samples == sorted(samples)
        assert scene.render_seconds(duration * .475) == pytest.approx(duration / 2)
        slow = scene.render_seconds(duration * .2) - scene.render_seconds(duration * .15)
        fast = scene.render_seconds(duration * .5) - scene.render_seconds(duration * .45)
        assert fast > 5 * slow
        assert fast / (duration * .05) > 2
    for scene in scenes:
        if scene.name not in (comparison.name, "silk_flow_solo"):
            assert scene.render_seconds(scene.duration * .3) == scene.duration * .3


def test_silk_flow_showcase_layout_and_timing(showcases: ModuleType) -> None:
    scene = next(scene for scene in showcases.discover_showcases() if scene.name == "silk_flow_solo")
    icons = [Mock(id="silk")]
    assert scene.canvas_size() == (960, 140)
    assert scene.max_icons == 1 and scene.keep_alpha
    assert scene.background_manifest() == {"mode": "none", "path": None}
    assert scene.layout(icons) == [(8, 38)]
    assert scene.destinations(icons, scene.layout(icons)) == [(888, 38)]
    resized = type(scene)(output_size=(800, 160), icon_size=48, dpi_scale=1.5)
    assert resized.layout(icons) == [(12, 44)]
    assert resized.destinations(icons, resized.layout(icons)) == [(716, 44)]
    assert scene.layout([]) == scene.destinations([], []) == []
    with pytest.raises(ValueError, match="one icon"):
        scene.layout(icons * 2)
    assert scene.render_seconds(0.5) == 0
    assert scene.render_seconds(4.5) == scene.duration
    assert scene.render_seconds(2.45) == pytest.approx(scene.duration / 2)
    samples = [scene.render_seconds(index / 100) for index in range(501)]
    assert samples == sorted(samples)
    assert all(scene.effect_envelope().eval(progress) == 1 for progress in (0, 0.1, 0.5, 0.9, 1))
    assert scene.animations(icons, scene.layout(icons), [(768, 112)])[0].target == (768, 112)


@pytest.mark.skipif(os.environ.get("RDI_SCENE_TESTS") != "1", reason="opt-in real artwork GPU rendering")
def test_silk_flow_showcase_real_artwork(showcases: ModuleType, images: ModuleType, tmp_path: Path) -> None:
    import rusty_desktop_icons as rdi

    controller = rdi.DesktopController()
    icons = controller.list_icons()
    if not icons:
        pytest.skip("No desktop artwork")
    positions_before = {icon.id: icon.position for icon in icons}
    flags_before = controller.get_flags()
    preferred = ("firefox", "google chrome", "chrome", "brave", "vivaldi")
    icons.sort(key=lambda icon: (next((index for index, name in enumerate(preferred)
                                    if name in icon.display_name.casefold()), len(preferred)), icon.display_name))
    scene = next(scene for scene in showcases.discover_showcases() if scene.name == "silk_flow_solo")
    showcases.export(controller, icons, scene, tmp_path)
    root = tmp_path / scene.name
    manifest = json.loads((root / "manifest.json").read_text())
    assert manifest["icons"] == 1 and manifest["keep_alpha"]
    assert len(manifest["frames"]) == 600
    for forward in (30, 90, 150, 210, 270):
        with Image.open(root / f"{forward:06d}.png") as first, Image.open(root / f"{600 - forward:06d}.png") as second:
            assert first.tobytes() == second.tobytes()
    for background, name in (((8, 12, 19, 255), "dark"), ((235, 237, 240, 255), "light")):
        sheet = Image.new("RGBA", (960, 140 * 6), background)
        for row, index in enumerate((0, 70, 130, 190, 245, 300)):
            with Image.open(root / f"{index:06d}.png") as frame:
                assert frame.size == (960, 140) and frame.getchannel("A").getbbox() is not None
                sheet.alpha_composite(frame.convert("RGBA"), (0, row * 140))
        sheet.convert("RGB").save(tmp_path / f"silk-flow-phases-{name}.png")
    images.assemble(root, tmp_path / "silk_flow.png")
    with Image.open(tmp_path / "silk_flow.png") as animation:
        assert animation.n_frames == 600
        total = 0.0
        for index, record in enumerate(manifest["frames"]):
            animation.seek(index)
            total += animation.info["duration"]
            with Image.open(root / record["file"]) as original:
                assert animation.convert("RGBA").tobytes() == original.tobytes()
        assert total == pytest.approx(10000)
    assert {icon.id: icon.position for icon in controller.list_icons()} == positions_before
    assert controller.get_flags() == flags_before


def test_effects_comparison_layout_specs_and_transparency(
    showcases: ModuleType, images: ModuleType, tmp_path: Path, monkeypatch: pytest.MonkeyPatch,
) -> None:
    import rusty_desktop_icons as rdi
    from dataclasses import replace

    scene = next(scene for scene in showcases.discover_showcases() if scene.name == "effects_comparison")
    icons = [Mock(spec=rdi.IconSnapshot, id=f"icon-{index}") for index in range(4)]
    assert scene.canvas_size() == (616, 106)
    assert scene.grid_metrics()[3:] == (8, 1)
    assert scene.max_icons == 4
    assert scene.taskbar_height_dip == 0
    assert scene.background_manifest() == {"mode": "none", "path": None}
    positions = scene.layout(icons)
    assert positions == [(18, 4), (170, 4), (322, 4), (474, 4)]
    targets = scene.destinations(icons, positions)
    assert targets == [(94, 4), (246, 4), (398, 4), (550, 4)]
    effect = Mock(wraps=rdi.Effect)
    spec = Mock(wraps=rdi.IconAnimationSpec)
    shader = Mock(wraps=rdi.Shader)
    monkeypatch.setattr(rdi, "Effect", effect)
    monkeypatch.setattr(rdi, "IconAnimationSpec", spec)
    monkeypatch.setattr(rdi, "Shader", shader)
    animations = scene.animations(icons, positions, targets)
    assert [animation.target for animation in animations] == targets
    assert shader.compile.call_count == 3
    assert spec.call_args_list[0].kwargs["effect"] is None
    for call, name in zip(shader.compile.call_args_list,
                          (rdi.BuiltinShader.Glitch, rdi.BuiltinShader.DustTransfer, rdi.BuiltinShader.ParticleVortex)):
        assert call.args[0].vertex == rdi.ShaderSource.builtin(name).vertex
        assert call.args[0].pixel == rdi.ShaderSource.builtin(name).pixel
    assert effect.call_args_list[0].kwargs["envelope"] is None
    assert effect.call_args_list[2].kwargs["envelope"] is None
    envelope = effect.call_args_list[1].kwargs["envelope"]
    assert all(envelope.eval(progress) == 1 for progress in (0, .075, .5, .925, 1))
    for count in (0, 1, 3, 5):
        with pytest.raises(ValueError, match="four icons"):
            scene.layout([Mock() for _ in range(count)])
    controller = Mock()
    session = MagicMock()
    session.__enter__.return_value = session
    session.duration = 1
    session.render_at.return_value = Mock(width=616, height=106, pixels=bytes([0, 0, 128, 128]) + bytes(616 * 106 * 4 - 4))
    controller.prepare_scene.return_value = session
    exported = replace(scene, duration=1, fps=10)
    showcases.export(controller, icons + [Mock(id="excess")], exported, tmp_path)
    manifest = json.loads((tmp_path / scene.name / "manifest.json").read_text())
    assert manifest["icons"] == 4
    assert manifest["background"] == {"mode": "none", "path": None}
    assert [call.args[0] for call in session.render_at.call_args_list] == [
        exported.render_seconds(frame["seconds"]) for frame in manifest["frames"]]
    assert all(frame.get("render_seconds", frame["seconds"]) == exported.render_seconds(frame["seconds"])
               for frame in manifest["frames"])
    images.assemble(tmp_path / scene.name, tmp_path / "comparison.png")
    with Image.open(tmp_path / "comparison.png") as decoded:
        assert decoded.size == (616, 106)
        assert decoded.n_frames == 20
        assert decoded.convert("RGBA").getpixel((0, 0)) == (255, 0, 0, 128)
        assert decoded.convert("RGBA").getpixel((615, 105)) == (0, 0, 0, 0)


@pytest.mark.skipif(sys.platform != "win32", reason="project intro uses Windows Segoe UI")
def test_project_intro_lettering_and_dance(showcases: ModuleType) -> None:
    import rusty_desktop_icons as rdi

    for dependency in ("numpy", "scipy", "skimage"):
        pytest.importorskip(dependency)
    scene = next(scene for scene in showcases.discover_showcases() if scene.name == "project_intro")
    icons = [Mock(spec=rdi.IconSnapshot, id=f"icon-{index}") for index in range(230)]
    positions = scene.layout(icons)
    targets = scene.destinations(icons, positions)
    assert scene.allow_overlap
    assert scene.duration - scene.gather_seconds == scene.dance_seconds == 10
    assert scene.layout(icons) == positions
    assert scene.destinations(icons, positions) == targets
    assert len(positions) == len(targets) == len(icons)
    assert len({horizontal % 48 for horizontal, _ in targets}) > 20
    assert len({vertical % 48 for _, vertical in targets}) > 20
    assert all(0 <= horizontal < scene.width - 48 and 0 <= vertical < scene.height - 48
               for horizontal, vertical in targets + positions)
    assert any(abs(first[0] - second[0]) < 48 and abs(first[1] - second[1]) < 48
               for index, first in enumerate(targets) for second in targets[index + 1:])
    rows = sorted({vertical for _, vertical in targets})
    gaps = [index for index in range(1, len(rows)) if rows[index] - rows[index - 1] > 48]
    assert len(gaps) == 2
    edges = [rows[0]] + [rows[index] for index in gaps] + [rows[-1] + 1]
    for lower, upper in zip(edges, edges[1:]):
        horizontal = [point[0] + 24 for point in targets if lower <= point[1] < upper]
        assert abs((min(horizontal) + max(horizontal)) / 2 - scene.width / 2) < 20
    assert abs((min(point[1] for point in targets) + max(point[1] for point in targets)) / 2 + 24 - scene.height / 2) < 20
    animations = scene.animations(icons, positions, targets)
    assert len(animations) == len(icons)
    for index in (0, 75, 150, 229):
        origin, target = positions[index], targets[index]
        for axis in (0, 1):
            curve = scene.motion_curve(origin, target, index, axis)
            assert curve.eval(0) == pytest.approx(0)
            assert curve.eval(4 / 14) == pytest.approx(1)
            assert curve.eval(1) == pytest.approx(1)
            offsets = [(curve.eval(seconds / 14) - 1) * (target[axis] - origin[axis])
                       for seconds in (4, 5, 6.5, 9, 12, 13, 14)]
            assert max(abs(offset) for offset in offsets) <= scene.dance_amplitude_dip + 0.001
            assert any(abs(offset) > 0.5 for offset in offsets)
        envelope = scene.glitch_envelope()
        assert envelope.eval(2 / 14) > 0.9
        assert all(envelope.eval(seconds / 14) == 0 for seconds in (0, 4, 6, 9, 14))
    options = scene.render_options()
    assert not options.draw_labels and not options.draw_shortcut_overlay and not options.draw_shield_overlay
    with pytest.raises(ValueError, match="at least 96"):
        scene.layout(icons[:10])
    for settings in ({"duration": 10.0}, {"gather_seconds": float("inf")}, {"dance_amplitude_dip": -1}):
        with pytest.raises(ValueError, match="positive phase durations"):
            type(scene)(**settings)


def test_showcase_can_explicitly_allow_duplicate_targets(showcases: ModuleType, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    import rusty_desktop_icons as rdi

    icons = [Mock(spec=rdi.IconSnapshot, id=f"icon-{index}") for index in range(2)]
    monkeypatch.setattr(showcases.Showcase, "destinations", lambda self, icons, positions: [(100, 100)] * len(icons))
    controller = Mock(spec=rdi.DesktopController)
    controller.prepare_scene.side_effect = RuntimeError("overlap probe")
    with pytest.raises(ValueError, match="unique target"):
        showcases.export(controller, icons, showcases.Showcase("strict"), tmp_path)
    controller.prepare_scene.assert_not_called()
    with pytest.raises(RuntimeError, match="overlap probe"):
        showcases.export(controller, icons, showcases.Showcase("overlap", allow_overlap=True), tmp_path)
    assert [spec.target for spec in controller.prepare_scene.call_args.args[1]] == [(100, 100)] * 2


@pytest.fixture
def discovery_folder(showcases: ModuleType, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Iterator[Path]:
    package = sys.modules[showcases.discover_showcases.__module__]
    monkeypatch.setattr(package, "__path__", [str(tmp_path)])
    loaded = set(sys.modules)
    try:
        yield tmp_path
    finally:
        for name in set(sys.modules) - loaded:
            if name.startswith(f"{package.__name__}."):
                sys.modules.pop(name, None)


def test_added_showcase_is_discovered_and_controls_export(showcases: ModuleType, discovery_folder: Path) -> None:
    import rusty_desktop_icons as rdi

    source = '''from dataclasses import dataclass
import rusty_desktop_icons as rdi
from .base import Showcase, Point

@dataclass(frozen=True)
class CustomShowcase(Showcase):
    name: str = "custom"
    width: int = 640
    height: int = 480
    fps: int = 12
    max_icons: int | None = 2

    def layout(self, icons: list[rdi.IconSnapshot]) -> list[Point]:
        return [(40 + index * 80, 50) for index in range(len(icons))]

    def destinations(self, icons: list[rdi.IconSnapshot], positions: list[Point]) -> list[Point]:
        return [(horizontal, vertical + 200) for horizontal, vertical in reversed(positions)]

    def animations(self, icons: list[rdi.IconSnapshot], positions: list[Point], targets: list[Point]) -> list[rdi.IconAnimationSpec]:
        return [rdi.IconAnimationSpec(icon.id, target, rdi.Duration.fixed(self.duration), rdi.Curve.linear())
                for icon, target in zip(icons, targets, strict=True)]

    def render_options(self) -> rdi.AnimationOptions:
        return rdi.AnimationOptions(draw_labels=False)
'''
    (discovery_folder / "custom.py").write_text(source, encoding="utf-8")
    (discovery_folder / "_helper.py").write_text("raise RuntimeError('private helper imported')", encoding="utf-8")
    (discovery_folder / "imported.py").write_text("from .custom import CustomShowcase", encoding="utf-8")
    (discovery_folder / "abstract.py").write_text(
        "from abc import ABC, abstractmethod\nfrom .base import Showcase\n"
        "class AbstractScene(Showcase, ABC):\n    @abstractmethod\n    def layout(self, icons): ...\n", encoding="utf-8")
    scenes = showcases.discover_showcases()
    assert [scene.name for scene in scenes] == ["custom"]
    scene = scenes[0]
    icons = [Mock(spec=rdi.IconSnapshot, id=f"icon-{index}") for index in range(3)]
    controller = Mock(spec=rdi.DesktopController)
    controller.prepare_scene.side_effect = RuntimeError("custom preparation probe")
    with pytest.raises(RuntimeError, match="custom preparation probe"):
        showcases.export(controller, icons, scene, discovery_folder / "output")
    canvas, animations, positions = controller.prepare_scene.call_args.args
    assert (canvas.width, canvas.height) == (640, 480)
    assert positions == [("icon-0", (40, 50)), ("icon-1", (120, 50))]
    assert [animation.target for animation in animations] == [(120, 250), (40, 250)]
    assert controller.prepare_scene.call_args.kwargs["options"].draw_labels is False


@pytest.mark.parametrize("name,error", [("same", "Duplicate showcase"), ("../escape", "lowercase slug")])
def test_showcase_discovery_rejects_bad_names(showcases: ModuleType, discovery_folder: Path, name: str, error: str) -> None:
    for module in ("first", "second"):
        source = ("from dataclasses import dataclass\nfrom .base import Showcase\n"
                  f"@dataclass(frozen=True)\nclass Scene(Showcase):\n    name: str = {name!r}\n")
        (discovery_folder / f"{module}.py").write_text(source, encoding="utf-8")
    with pytest.raises(ValueError, match=error):
        showcases.discover_showcases()


def test_showcase_discovery_reports_empty_and_broken_modules(showcases: ModuleType, discovery_folder: Path) -> None:
    with pytest.raises(ValueError, match="No showcase"):
        showcases.discover_showcases()
    (discovery_folder / "broken.py").write_text("raise RuntimeError('broken scene')", encoding="utf-8")
    with pytest.raises(RuntimeError, match="broken scene"):
        showcases.discover_showcases()


@pytest.mark.parametrize("hook,error", [("layout", "one position"), ("destinations", "one unique target"),
                                       ("animations", "exactly one spec")])
def test_showcase_export_rejects_incomplete_hooks(showcases: ModuleType, tmp_path: Path, monkeypatch: pytest.MonkeyPatch,
                                                hook: str, error: str) -> None:
    import rusty_desktop_icons as rdi

    scene = showcases.Showcase("incomplete")
    icons = [Mock(spec=rdi.IconSnapshot, id="icon")]
    monkeypatch.setattr(showcases.Showcase, hook, lambda *args: [])
    controller = Mock(spec=rdi.DesktopController)
    with pytest.raises(ValueError, match=error):
        showcases.export(controller, icons, scene, tmp_path)
    controller.prepare_scene.assert_not_called()
    assert not (tmp_path / scene.name).exists()


def test_assembly_discovers_root_and_nested_manifests(images: ModuleType, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    root = tmp_path / "target/showcases"
    mockup = tmp_path / "mockup.png"
    Image.new("RGB", (8, 8), "blue").save(mockup)
    directories = [root, root / "movement", root / "nested/dust-transfer"]
    for directory in directories:
        directory.mkdir(parents=True, exist_ok=True)
        Image.new("RGBA", (4, 4), (255, 0, 0, 128)).save(directory / "000000.png")
        Image.new("RGBA", (4, 4), (0, 255, 0, 128)).save(directory / "000001.png")
        manifest = {"version": 2, "width": 4, "height": 4,
                "background": {"mode": "fit", "path": str(mockup)},
                    "frames": [{"file": f"{index:06d}.png", "duration_ms": 20} for index in range(2)]}
        (directory / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr(sys, "argv", ["assemble-showcase.py", "--jobs", "1"])
    images.main()
    for directory in directories:
        with Image.open(directory.parent / f"{directory.name}.png") as animation:
            assert animation.n_frames == 2
            assert animation.size == (4, 4)
    outputs = images.assemble_directory(root)
    original = {output: output.read_bytes() for output in outputs}
    for directory in directories:
        Image.new("RGBA", (4, 4), "yellow").save(directory / "000000.png")
    assert images.assemble_directory(root) == outputs
    assert all(output.read_bytes() == original[output] for output in outputs)
    assert images.assemble_directory(root, overwrite=True) == outputs
    assert all(output.read_bytes() != original[output] for output in outputs)
    with pytest.raises(NotADirectoryError):
        images.assemble_directory(tmp_path / "missing")
    empty = tmp_path / "empty"
    empty.mkdir()
    with pytest.raises(FileNotFoundError, match="No manifest"):
        images.assemble_directory(empty)
    monkeypatch.setattr(sys, "argv", ["assemble-showcase.py", "--input", str(empty)])
    with pytest.raises(FileNotFoundError, match="No manifest"):
        images.main()


@pytest.mark.skipif(os.environ.get("RDI_SCENE_TESTS") != "1", reason="opt-in real desktop artwork extraction")
def test_effects_comparison_rendered_phases(showcases: ModuleType) -> None:
    import rusty_desktop_icons as rdi

    controller = rdi.DesktopController()
    all_icons = controller.list_icons()
    if len(all_icons) < 4:
        pytest.skip("comparison requires four desktop icons")
    before = {icon.id: icon.position for icon in all_icons}
    flags = controller.get_flags()
    scene = next(scene for scene in showcases.discover_showcases() if scene.name == "effects_comparison")
    icons = all_icons[:4]
    positions = scene.layout(icons)
    targets = scene.destinations(icons, positions)
    origins = [(icon.id, point) for icon, point in zip(icons, positions, strict=True)]
    canvas = rdi.Canvas(*scene.canvas_size(), dpi_scale=scene.dpi_scale, icon_size=scene.icon_size)
    with controller.prepare_scene(canvas, scene.animations(icons, positions, targets), origins) as renderer:
        first = renderer.render_at(scene.render_seconds(0)).pixels
        last = renderer.render_at(scene.render_seconds(4)).pixels
        for seconds in (.15, .3, .59, .6):
            assert renderer.render_at(scene.render_seconds(seconds)).pixels == first
        for seconds in (3.2, 3.4, 3.8, 4):
            assert renderer.render_at(scene.render_seconds(seconds)).pixels == last
        middle = renderer.render_at(scene.render_seconds(1.9)).pixels
        later = renderer.render_at(scene.render_seconds(2.3)).pixels
        assert renderer.render_at(scene.render_seconds(1.9)).pixels == middle
        assert renderer.render_at(scene.render_seconds(0)).pixels == first
        with Image.frombytes("RGBA", (616, 106), first) as start_image, \
                Image.frombytes("RGBA", (616, 106), middle) as middle_image, \
                Image.frombytes("RGBA", (616, 106), later) as later_image, \
                Image.frombytes("RGBA", (616, 106), last) as end_image:
            for index in range(4):
                bounds = (index * 152, 0, (index + 1) * 152, 106)
                frames = [image.crop(bounds).tobytes() for image in
                          (start_image, middle_image, later_image, end_image)]
                assert len(set(frames)) == 4
    clean = [rdi.IconAnimationSpec(icon.id, target, rdi.Duration.fixed(4), rdi.Curve.linear())
             for icon, target in zip(icons, targets, strict=True)]
    with controller.prepare_scene(canvas, clean, origins) as renderer:
        assert renderer.render_at(0).pixels == first
        assert renderer.render_at(4).pixels == last
    assert {icon.id: icon.position for icon in controller.list_icons()} == before
    assert controller.get_flags() == flags


@pytest.mark.skipif(os.environ.get("RDI_SCENE_TESTS") != "1", reason="opt-in real desktop artwork extraction")
@pytest.mark.parametrize("name", ["dust_transfer", "dust_transfer_small"])
def test_dust_showcase_has_no_envelope_movement_leak(showcases: ModuleType, name: str) -> None:
    import rusty_desktop_icons as rdi

    controller = rdi.DesktopController()
    all_icons = controller.list_icons()
    if not all_icons:
        pytest.skip("desktop has no icons")
    before = {icon.id: icon.position for icon in all_icons}
    flags = controller.get_flags()
    scene = next(scene for scene in showcases.discover_showcases() if scene.name == name)
    icons = all_icons[:1]
    positions = scene.layout(icons)
    targets = scene.destinations(icons, positions)
    origins = [(icons[0].id, positions[0])]
    canvas = rdi.Canvas(*scene.canvas_size(), dpi_scale=scene.dpi_scale, icon_size=scene.icon_size)
    samples = (0.0, 0.01, 0.075, 0.1, 0.5, 0.9, 0.925, 0.99, 1.0)
    with controller.prepare_scene(canvas, scene.animations(icons, positions, targets), origins) as renderer:
        actual = [renderer.render_at(progress * scene.duration).pixels for progress in samples]
        assert renderer.render_at(0.075 * scene.duration).pixels == actual[2]
        assert all(any(pixels[3::4]) for pixels in actual)
        assert actual[0] != actual[4] != actual[-1]
    shader = rdi.Shader.compile(rdi.ShaderSource.builtin(rdi.BuiltinShader.DustTransfer))
    flat = rdi.Curve.keyframes([(0.0, 1.0), (1.0, 1.0)], interp=rdi.KeyframeInterp.Linear)
    reference = rdi.IconAnimationSpec(
        icons[0].id, targets[0], rdi.Duration.fixed(scene.duration), rdi.Curve.linear(),
        effect=rdi.Effect(shader, envelope=flat, padding_px=24, seed=0.0),
    )
    with controller.prepare_scene(canvas, [reference], origins) as renderer:
        for progress, expected in zip(samples, actual):
            assert renderer.render_at(progress * scene.duration).pixels == expected
    clean = rdi.IconAnimationSpec(icons[0].id, targets[0], rdi.Duration.fixed(scene.duration), rdi.Curve.linear())
    with controller.prepare_scene(canvas, [clean], origins) as renderer:
        assert renderer.render_at(0).pixels == actual[0]
        assert renderer.render_at(scene.duration).pixels == actual[-1]
    assert {icon.id: icon.position for icon in controller.list_icons()} == before
    assert controller.get_flags() == flags


@pytest.mark.skipif(os.environ.get("RDI_SCENE_TESTS") != "1", reason="opt-in real desktop artwork extraction")
@pytest.mark.parametrize("name", ["movement_small", "glitch_small", "dust_transfer_small", "particle_vortex_small"])
def test_small_scene_matches_full_canvas_crop(showcases: ModuleType, name: str) -> None:
    import rusty_desktop_icons as rdi
    from PIL import ImageChops

    controller = rdi.DesktopController()
    all_icons = controller.list_icons()
    if not all_icons:
        pytest.skip("desktop has no icons")
    before = {icon.id: icon.position for icon in all_icons}
    flags = controller.get_flags()
    scene = next(scene for scene in showcases.discover_showcases() if scene.name == name)
    icons = all_icons[:scene.max_icons]
    positions = scene.layout(icons)
    targets = scene.destinations(icons, positions)
    animations = scene.animations(icons, positions, targets)
    origins = [(icon.id, point) for icon, point in zip(icons, positions)]
    size = scene.canvas_size()
    samples = [0.0, scene.duration / 2, scene.duration]
    with controller.prepare_scene(rdi.Canvas(*size, dpi_scale=scene.dpi_scale, icon_size=scene.icon_size),
                                  animations, origins, options=scene.render_options()) as renderer:
        small = [renderer.render_at(seconds).pixels for seconds in samples]
        assert renderer.render_at(samples[1]).pixels == small[1]
        assert all(any(pixels[3::4]) for pixels in small)
        assert len(set(small)) == 3
    with controller.prepare_scene(rdi.Canvas(scene.width, scene.height, dpi_scale=scene.dpi_scale,
                                             icon_size=scene.icon_size),
                                  animations, origins, options=scene.render_options()) as renderer:
        for seconds, expected in zip(samples, small):
            frame = renderer.render_at(seconds)
            with Image.frombytes("RGBA", (frame.width, frame.height), frame.pixels) as full:
                with Image.frombytes("RGBA", size, expected) as cropped:
                    difference = ImageChops.difference(full.crop((0, 0, *size)), cropped)
                    if seconds in (0, scene.duration) or name in ("movement_small", "glitch_small"):
                        assert all(maximum == 0 for _, maximum in difference.getextrema())
                    else:
                        assert max(maximum for _, maximum in difference.getextrema()) <= 8
                        histogram = difference.histogram()
                        assert sum((index % 256) * count for index, count in enumerate(histogram)) / len(expected) < 0.01
    assert {icon.id: icon.position for icon in controller.list_icons()} == before
    assert controller.get_flags() == flags


@pytest.mark.skipif(os.environ.get("RDI_SCENE_TESTS") != "1", reason="opt-in real desktop artwork extraction")
def test_scene_desktop_pixels_and_state() -> None:
    import rusty_desktop_icons as rdi

    controller = rdi.DesktopController()
    icons = controller.list_icons()
    if not icons:
        pytest.skip("desktop has no icons")
    before = {icon.id: icon.position for icon in icons}
    flags = controller.get_flags()
    shader = rdi.Shader.compile(rdi.ShaderSource.builtin("dust-transfer"))
    positions = [(icon.id, (32 + index % 8 * 160, 32 + index // 8 * 160)) for index, icon in enumerate(icons)]
    specs = [rdi.IconAnimationSpec(icon.id, (point[0] + 48, point[1] + 24), rdi.Duration.fixed(2.0),
                                  rdi.Curve.linear(), effect=rdi.Effect(shader, seed=float(index)))
             for index, (icon, (_, point)) in enumerate(zip(icons, positions))]
    with controller.prepare_scene(rdi.Canvas(1600, 1200), specs, positions) as renderer:
        assert renderer.duration == 2.0
        first = renderer.render_at(0.5)
        assert first.pixel_format == "BGRA8_PREMULTIPLIED"
        assert first.stride == 6400
        assert len(first.pixels) == 1600 * 1200 * 4
        assert any(first.pixels)
        renderer.render_at(2.0)
        assert renderer.render_at(0.5).pixels == first.pixels
        with pytest.raises(rdi.InvalidDuration):
            renderer.render_at(3.0)
        with pytest.raises(rdi.AnimationBusy):
            controller.set_positions([])
    assert {icon.id: icon.position for icon in controller.list_icons()} == before
    assert controller.get_flags() == flags
    with pytest.raises(rdi.BackendUnavailable):
        renderer.render_at(0.0)