"""Convert top-level showcase PNG/APNGs to scene-configured GIF or WebP outputs."""

from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import replace
import json
import math
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import zlib

import imageio_ffmpeg
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parent))
import showcase_progress as progress
from scenes import CompressedOutputParams


def output_size(size: tuple[int, int], params: CompressedOutputParams | None = None) -> tuple[int, int]:
    params = params or CompressedOutputParams()
    width, height = (native if requested is None else requested if params.allow_upscale else min(native, requested)
                     for native, requested in zip(size, params.size, strict=True))
    if width * height > 16777216:
        raise ValueError("Output exceeds 16777216 pixels")
    return width, height


def inspect_png(source: Path) -> tuple[tuple[int, int], bool, float, bool]:
    source_bytes = source.stat().st_size
    progress.report("Checking PNG bytes", 0, source_bytes)
    with Image.open(source) as image:
        if image.format != "PNG" or image.info.get("default_image"):
            raise ValueError("Expected PNG/APNG without a separate poster image")
        size = image.size
        expected_frames = image.n_frames
        animated = "loop" in image.info
        possible_alpha = "A" in image.getbands() or "transparency" in image.info
        if expected_frames > 10000 or max(size) > 8192 or size[0] * size[1] > 16777216:
            raise ValueError("Input exceeds frame or pixel limits")
    duration_ms = 0.0
    frames = 0
    sequence = 0
    has_data = False
    with source.open("rb") as stream:
        if stream.read(8) != b"\x89PNG\r\n\x1a\n":
            raise ValueError("Invalid PNG signature")
        while True:
            header = stream.read(8)
            if len(header) != 8:
                raise ValueError("Truncated PNG chunk")
            length, kind = struct.unpack(">I4s", header)
            checksum = zlib.crc32(kind)
            prefix = b""
            remaining = length
            while remaining:
                block = stream.read(min(remaining, 65536))
                if not block:
                    raise ValueError("Truncated PNG data")
                prefix += block[:max(0, 26 - len(prefix))]
                checksum = zlib.crc32(block, checksum)
                remaining -= len(block)
                progress.report("Checking PNG bytes", stream.tell(), source_bytes)
            recorded = stream.read(4)
            if len(recorded) != 4 or struct.unpack(">I", recorded)[0] != checksum:
                raise ValueError("PNG CRC mismatch")
            if kind == b"fcTL":
                if length != 26 or not animated or (frames and not has_data):
                    raise ValueError("Invalid APNG frame control")
                number, width, height, left, top, numerator, denominator, disposal, blend = struct.unpack(">IIIIIHHBB", prefix)
                if number != sequence or not width or not height or left + width > size[0] or top + height > size[1] or disposal > 2 or blend > 1:
                    raise ValueError("Invalid APNG frame geometry or sequence")
                sequence += 1
                delay = 1000 * numerator / (denominator or 100)
                if delay <= 0:
                    raise ValueError("Animated frames require positive finite durations")
                duration_ms += delay
                frames += 1
                has_data = False
            elif kind == b"fdAT":
                if length < 4 or frames < 2 or struct.unpack(">I", prefix[:4])[0] != sequence:
                    raise ValueError("Invalid APNG data sequence")
                sequence += 1
                has_data = True
            elif kind == b"IDAT":
                has_data = True
            elif kind == b"tRNS":
                possible_alpha = True
            elif kind == b"IEND":
                if length or not has_data or stream.read(1):
                    raise ValueError("Invalid PNG end")
                break
    if animated and frames != expected_frames:
        raise ValueError("APNG frame count mismatch")
    progress.report("Checking PNG bytes", source_bytes, source_bytes)
    return size, animated, duration_ms, possible_alpha


def run_encoder(arguments: list[str]) -> None:
    with tempfile.TemporaryFile(mode="w+", encoding="utf-8") as errors:
        with subprocess.Popen([imageio_ffmpeg.get_ffmpeg_exe(), "-hide_banner", "-loglevel", "error",
                               "-nostdin", "-n", "-xerror", "-filter_threads", "1",
                               "-filter_complex_threads", "1", "-nostats", "-stats_period", "0.2",
                               "-progress", "pipe:1", *arguments], stdout=subprocess.PIPE,
                              stderr=errors, text=True) as process:
            try:
                assert process.stdout is not None
                for line in process.stdout:
                    key, _, value = line.strip().partition("=")
                    if key == "out_time_us":
                        try:
                            milliseconds = max(0, int(value) / 1000)
                        except ValueError:
                            continue
                        progress.advance_to(milliseconds)
                returncode = process.wait()
            except BaseException:
                process.kill()
                process.wait()
                raise
        if returncode:
            errors.seek(0)
            raise ValueError(f"FFmpeg failed: {errors.read().strip()}")


def conversion_params(source: Path) -> tuple[CompressedOutputParams, bool | None]:
    manifest = source.with_suffix("") / "manifest.json"
    if not manifest.is_file():
        return CompressedOutputParams(), None
    data = json.loads(manifest.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise ValueError("Manifest must be an object")
    settings = data.get("compressed_output_params", {})
    if not isinstance(settings, dict):
        raise ValueError("compressed_output_params must be an object")
    try:
        params = CompressedOutputParams(**settings)
    except TypeError as error:
        raise ValueError(f"Invalid compressed_output_params: {error}") from error
    alpha = data.get("keep_alpha")
    if alpha is not None and type(alpha) is not bool:
        raise ValueError("keep_alpha must be a boolean")
    return params, alpha


def retain_webp_duration(path: Path, duration_ms: float) -> None:
    with Image.open(path) as image:
        if image.is_animated:
            return
        width, height = image.size
        alpha = "A" in image.getbands()
    encoded = path.read_bytes()
    payload = bytearray()
    offset = 12
    while offset < len(encoded):
        length = struct.unpack_from("<I", encoded, offset + 4)[0]
        end = offset + 8 + length + (length & 1)
        if encoded[offset:offset + 4] in (b"VP8 ", b"VP8L", b"ALPH"):
            payload.extend(encoded[offset:end])
        offset = end
    dimensions = (width - 1).to_bytes(3, "little") + (height - 1).to_bytes(3, "little")
    def chunk(kind: bytes, data: bytes) -> bytes:
        return kind + struct.pack("<I", len(data)) + data + b"\0" * (len(data) & 1)
    with path.open("wb") as stream:
        stream.write(b"RIFF\0\0\0\0WEBP")
        stream.write(chunk(b"VP8X", bytes([2 | (16 if alpha else 0), 0, 0, 0]) + dimensions))
        stream.write(chunk(b"ANIM", b"\0" * 6))
        remaining = max(1, round(duration_ms))
        while remaining:
            delay = min(remaining, 0xFFFFFF)
            stream.write(chunk(b"ANMF", b"\0" * 6 + dimensions + delay.to_bytes(3, "little") + b"\x02" + payload))
            remaining -= delay
        length = stream.tell() - 8
        stream.seek(4)
        stream.write(struct.pack("<I", length))


@progress.work_item("source")
def convert(source: Path, output: Path, fps: int | None = None,
            *, params: CompressedOutputParams | None = None, threads: int | None = None,
            overwrite: bool = False) -> dict[str, int | float] | None:
    if source.resolve() == output.resolve():
        raise FileExistsError(output)
    if output.exists() and not overwrite:
        progress.skipped("Exists")
        progress.message(f"Exists: {output}")
        return None
    params = params or conversion_params(source)[0]
    if fps is not None:
        params = replace(params, fps=fps)
    fps = params.fps
    threads = threads if threads is not None else min(8, os.cpu_count() or 1)
    if type(threads) is not int or threads < 1:
        raise ValueError("threads must be a positive integer")
    format_name = output.suffix.lower().lstrip(".")
    if format_name not in ("gif", "webp") or params.format not in ("auto", format_name):
        raise ValueError("Output extension must match GIF/WebP configuration")
    source_size, animated, duration_ms, possible_alpha = inspect_png(source)
    size = output_size(source_size, params)
    if format_name == "gif" and possible_alpha:
        with Image.open(source) as image:
            progress.report("Checking opacity", 0, image.n_frames)
            for index in range(image.n_frames):
                image.seek(index)
                with image.convert("RGBA") as rgba, rgba.getchannel("A") as alpha:
                    if alpha.getextrema() != (255, 255):
                        raise ValueError("GIF conversion requires opaque assembled images; use WebP")
                progress.report("Checking opacity", index + 1, image.n_frames)
    if animated and duration_ms < 1000 / fps:
        raise ValueError("Animation is shorter than one output frame")

    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".compressed-", dir=output.parent) as directory:
        temporary = Path(directory) / ("result." + format_name)
        palette = Path(directory) / "palette.png"
        sampling = (f"fps={fps}," if animated else "") + f"scale={size[0]}:{size[1]}:flags=lanczos"
        input_arguments = ["-threads", str(threads)] + (["-ignore_loop", "1"] if animated else []) + ["-i", str(source.resolve())]
        if format_name == "gif":
            progress.report("GIF palette (ms)", 0, duration_ms if animated else None)
            run_encoder([*input_arguments, "-vf", sampling + f",palettegen=max_colors={params.gif_colors}:stats_mode=diff",
                         "-frames:v", "1", "-update", "1", str(palette)])
            progress.report("Encoding GIF (ms)", 0, duration_ms if animated else None)
            run_encoder([*input_arguments, "-i", str(palette), "-filter_complex",
                         f"[0:v]{sampling}[frames];[frames][1:v]paletteuse=dither={params.gif_dither}:bayer_scale=5:diff_mode=rectangle",
                         "-fps_mode", "passthrough", "-loop", "0", str(temporary)])
        else:
            progress.report("Encoding WebP (ms)", 0, duration_ms if animated else None)
            run_encoder([*input_arguments, "-vf", sampling, "-c:v", "libwebp_anim",
                         "-lossless", str(int(params.webp_lossless)), "-quality", str(params.webp_quality),
                         "-compression_level", str(params.webp_method), "-fps_mode", "passthrough",
                         "-loop", "0", str(temporary)])
            if animated:
                retain_webp_duration(temporary, duration_ms)

        with Image.open(temporary) as result:
            if result.format != format_name.upper() or result.size != size:
                raise ValueError("Output dimensions or format failed verification")
            if (format_name == "gif" or result.is_animated) and result.info.get("loop", -1) != 0:
                raise ValueError("Output loop count failed verification")
            output_frames = result.n_frames
            actual_ms = 0
            progress.report("Verifying frames", 0, output_frames)
            for index in range(output_frames):
                result.seek(index)
                result.load()
                delay = int(result.info.get("duration", 0))
                interval = math.floor(100 / fps) * 10 if format_name == "gif" else math.floor(1000 / fps)
                if animated and delay < interval:
                    raise ValueError("Output frame interval exceeds requested FPS")
                actual_ms += delay
                progress.report("Verifying frames", index + 1, output_frames)
            if animated:
                if abs(actual_ms - duration_ms) > 1000 / fps + 10:
                    raise ValueError(f"Output duration mismatch: {actual_ms}ms versus {duration_ms}ms")
                if output_frames > math.ceil(duration_ms * fps / 1000):
                    raise ValueError("Output has too many frames")
        progress.report("Publishing output")
        if overwrite:
            os.replace(temporary, output)
        else:
            try:
                os.link(temporary, output)
            except FileExistsError:
                progress.skipped("Exists")
                progress.message(f"Exists: {output}")
                return None
    return {"before_bytes": source.stat().st_size, "after_bytes": output.stat().st_size,
            "width": size[0], "height": size[1], "frames": output_frames, "duration_ms": actual_ms}


@progress.work_item("source")
def convert_one(source: Path, output: Path, fps: int | None = None, threads: int | None = None,
                *, overwrite: bool = False) -> None:
    params, keep_alpha = conversion_params(source)
    format_name = params.format
    if format_name == "auto":
        with Image.open(source) as image:
            alpha = "A" in image.getbands() or "transparency" in image.info
        format_name = "webp" if keep_alpha is True or alpha else "gif"
    destination = output / (source.stem + "." + format_name)
    if destination.exists() and not overwrite:
        progress.skipped("Exists")
        progress.message(f"Exists: {destination}")
        return
    progress.message(f"Converting {source.name}...")
    result = convert(source, destination, fps, params=params, threads=threads, overwrite=overwrite)
    if result is None:
        return
    progress.message(f"{source.name}: {result['width']}x{result['height']}, {result['frames']} frames, "
                     f"{result['duration_ms'] / 1000:.2f}s; {result['before_bytes'] / 1048576:.2f} -> "
                     f"{result['after_bytes'] / 1048576:.2f} MiB -> {destination}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, default=Path("target/showcases"))
    parser.add_argument("--output", type=Path, help="Default: sibling <input-name>-compressed folder")
    parser.add_argument("--fps", type=int, help="Override scene sampling rate, 1..30")
    parser.add_argument("--jobs", type=int, default=min(4, os.cpu_count() or 1))
    parser.add_argument("--no-progress", action="store_true", help="Disable live progress; keep summaries")
    parser.add_argument("--overwrite", action="store_true", help="Replace existing outputs (default: skip)")
    arguments = parser.parse_args()
    root = arguments.input.resolve()
    output = (arguments.output or root.with_name(root.name + "-compressed")).resolve()
    if output == root or root in output.parents:
        parser.error("Output must be outside the input directory")
    if not root.is_dir():
        parser.error(f"Not a directory: {root}")
    if arguments.jobs < 1 or (arguments.fps is not None and not 1 <= arguments.fps <= 30):
        parser.error("--jobs must be positive; --fps must be between 1 and 30")
    paths = sorted(path for path in root.glob("*.png") if path.is_file() and not path.is_symlink())
    if not paths:
        parser.error(f"No top-level PNG files in {root}")
    failed = False
    jobs = min(arguments.jobs, len(paths))
    threads = max(1, min(8, (os.cpu_count() or 1) // jobs))
    with progress.display("Converting showcases", len(paths), enabled=not arguments.no_progress), ThreadPoolExecutor(max_workers=jobs) as pool:
        futures = {pool.submit(convert_one, source, output, arguments.fps, threads,
                       overwrite=arguments.overwrite): source
                   for source in sorted(paths, key=lambda path: path.stat().st_size, reverse=True)}
        for future in as_completed(futures):
            try:
                future.result()
            except (OSError, ValueError, RuntimeError):
                failed = True
    if failed:
        raise SystemExit(1)


if __name__ == "__main__":
    main()