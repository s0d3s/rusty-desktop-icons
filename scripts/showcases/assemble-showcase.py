"""Compose a transparent PNG sequence onto a desktop mockup and write APNG."""

from __future__ import annotations

import argparse
from collections import deque
from concurrent.futures import Future, ProcessPoolExecutor, ThreadPoolExecutor, as_completed
import io
import json
import math
import multiprocessing
import os
from pathlib import Path
import struct
import sys
from time import perf_counter
from typing import BinaryIO, Iterable, Iterator

from PIL import Image, ImageDraw, ImageOps
from apng import FrameControl, PNG, PNG_SIGN, make_chunk

sys.path.insert(0, str(Path(__file__).resolve().parent))
import showcase_progress as progress

DEFAULT_MOCKUP = Path(__file__).parent / "assets/desktop-mockup.png"


def rgba_image(width: int, height: int, pixels: bytes) -> Image.Image:
    if width <= 0 or height <= 0 or len(pixels) != width * height * 4:
        raise ValueError("Invalid packed BGRA frame")
    return Image.frombytes("RGBa", (width, height), pixels, "raw", "BGRa").convert("RGBA")


def frame_schedule(duration: float, fps: int) -> list[tuple[float, int]]:
    if not math.isfinite(duration) or duration <= 0 or not 1 <= fps <= 120:
        raise ValueError("Positive finite duration and FPS in 1..120 required")
    steps = round(duration * fps)
    if steps < 1 or not math.isclose(duration * fps, steps, abs_tol=1e-7, rel_tol=0):
        raise ValueError("Pass duration must contain a whole number of frames")
    if steps * 2 > 10000:
        raise ValueError("Showcase exceeds 10000 frames")
    return [(min(index, 2 * steps - index) / fps,
             round((index + 1) * 1000 / fps) - round(index * 1000 / fps))
            for index in range(2 * steps)]


def fit_mockup(path: Path, size: tuple[int, int]) -> Image.Image:
    with Image.open(path) as source:
        if source.width < size[0] or source.height < size[1]:
            raise ValueError(f"Mockup {source.size} cannot cover {size} without upscaling")
        return ImageOps.fit(source.convert("RGBA"), size, Image.Resampling.LANCZOS,
                            centering=(0.5, 1.0))


def background_image(manifest: dict) -> Image.Image:
    size = (int(manifest["width"]), int(manifest["height"]))
    settings = manifest.get("background")
    if settings is None and manifest.get("version") == 1:
        settings = {"mode": "fit", "path": str(DEFAULT_MOCKUP)}
    if not isinstance(settings, dict):
        raise ValueError("Manifest must declare background settings")
    mode = settings.get("mode")
    if mode == "none":
        return Image.new("RGBA", size)
    if mode not in ("fit", "crop", "native"):
        raise ValueError(f"Unknown background mode: {mode}")
    path = settings.get("path")
    if not isinstance(path, str) or not path:
        raise ValueError("Manifest background must declare a mockup path")
    mockup = Path(path)
    if mode == "native":
        with Image.open(mockup) as source:
            if source.size != size:
                raise ValueError(f"Native mockup {source.size} must match output {size}")
            return source.convert("RGBA")
    if mode == "fit":
        return fit_mockup(mockup, size)
    desktop_size = tuple(manifest["desktop_size"])
    if (len(desktop_size) != 2 or any(type(value) is not int or value <= 0 for value in desktop_size)
            or size[0] > desktop_size[0] or size[1] > desktop_size[1]):
        raise ValueError("Output area must fit within the desktop size")
    scale = float(manifest["dpi_scale"])
    taskbar = manifest["taskbar_height"]
    if (not math.isfinite(scale) or scale <= 0 or type(taskbar) is not int
            or not 0 <= taskbar <= size[1]):
        raise ValueError("Invalid mockup DPI or taskbar height")
    with fit_mockup(mockup, desktop_size) as desktop:
        background = desktop.crop((0, 0, *size))
    if taskbar:
        artwork = ImageDraw.Draw(background)
        top = size[1] - taskbar
        artwork.rectangle((0, top, size[0], size[1]), fill="#e3efed")
        artwork.line((0, top, size[0], top), fill="#b5ceca", width=max(1, round(scale)))
        pane = max(1, round(10 * scale))
        gap = max(1, round(3 * scale))
        extent = 2 * pane + gap
        if extent <= taskbar and round(20 * scale) + extent <= size[0]:
            left = round(20 * scale)
            vertical = top + (taskbar - extent) // 2
            for offset_x in (0, pane + gap):
                for offset_y in (0, pane + gap):
                    artwork.rectangle((left + offset_x, vertical + offset_y,
                                       left + offset_x + pane - 1, vertical + offset_y + pane - 1),
                                      fill="#267997")
    return background


def encode_frame(path: Path, size: tuple[int, int], background: Image.Image | None,
                 keep_alpha: bool) -> PNG:
    with path.open("rb") as source:
        with Image.open(source) as probe:
            if probe.format != "PNG" or probe.size != size or getattr(probe, "n_frames", 1) != 1:
                raise ValueError(f"Frame size mismatch or unsupported PNG: {path}")
            probe.verify()
        source.seek(0)
        if background is None and keep_alpha:
            png = PNG.from_bytes(source.read())
            if (png.hdr[16:21] == bytes([8, 6, 0, 0, 0])
                    and all(chunk.type in {"IHDR", "IDAT", "IEND"} for chunk in png.chunks)):
                return png
            source.seek(0)
        with Image.open(source) as overlay, overlay.convert("RGBA") as rgba:
            image = Image.alpha_composite(background, rgba) if background is not None else rgba.copy()
    with image:
        if keep_alpha:
            with io.BytesIO() as encoded:
                image.save(encoded, format="PNG", compress_level=6)
                return PNG.from_bytes(encoded.getvalue())
        with image.convert("RGB") as rgb, io.BytesIO() as encoded:
            rgb.save(encoded, format="PNG", compress_level=6)
            return PNG.from_bytes(encoded.getvalue())


def encoded_frames(paths: list[Path], size: tuple[int, int], background: Image.Image | None,
                   keep_alpha: bool, encoders: int) -> Iterator[PNG]:
    if encoders == 1:
        for path in paths:
            yield encode_frame(path, size, background, keep_alpha)
        return
    capacity = max(1, min(2 * encoders, (128 * 1024 * 1024) // (size[0] * size[1] * 16)))
    pending: deque[Future[PNG]] = deque()
    remaining = iter(paths)
    with ThreadPoolExecutor(max_workers=min(encoders, capacity), thread_name_prefix="apng-frame") as pool:
        try:
            for path in remaining:
                pending.append(pool.submit(encode_frame, path, size, background, keep_alpha))
                if len(pending) == capacity:
                    break
            while pending:
                yield pending.popleft().result()
                path = next(remaining, None)
                if path is not None:
                    pending.append(pool.submit(encode_frame, path, size, background, keep_alpha))
        finally:
            for future in pending:
                future.cancel()


def write_apng(destination: BinaryIO, frames: Iterable[PNG], durations: list[int]) -> None:
    sequence_number = 0
    header: bytes | None = None
    progress.report("Assembling frames", 0, len(durations))
    for index, (png, delay) in enumerate(zip(frames, durations, strict=True)):
        if index == 0:
            header = png.hdr
            destination.write(PNG_SIGN)
            destination.write(header)
            destination.write(make_chunk("acTL", struct.pack("!II", len(durations), 0)))
        elif png.hdr != header:
            raise ValueError("Incompatible PNG frame headers")
        control = FrameControl(width=png.width, height=png.height, delay=delay,
                               delay_den=1000, depose_op=0, blend_op=0)
        destination.write(make_chunk("fcTL", struct.pack("!I", sequence_number) + control.to_bytes()))
        sequence_number += 1
        for chunk in png.chunks:
            if chunk.type == "IDAT":
                if index == 0:
                    destination.write(chunk.data)
                else:
                    destination.write(make_chunk("fdAT", struct.pack("!I", sequence_number) + chunk.data[8:-4]))
                    sequence_number += 1
                progress.report("Assembling frames", index + 1, len(durations))
    if header is None:
        raise ValueError("No APNG frames")
    destination.write(make_chunk("IEND", b""))


@progress.work_item("sequence")
def assemble(sequence: Path, output: Path, *, encoders: int = 4, overwrite: bool = False) -> None:
    if type(encoders) is not int or encoders < 1:
        raise ValueError("encoders must be a positive integer")
    temporary = output.with_suffix(output.suffix + ".partial")
    existing = output if output.exists() and not overwrite else temporary if temporary.exists() else None
    if existing is not None:
        progress.skipped("Exists")
        progress.message(f"Exists: {existing}")
        return
    started = perf_counter()
    progress.report("Validating manifest")
    manifest = json.loads((sequence / "manifest.json").read_text(encoding="utf-8"))
    if manifest["version"] not in (1, 2):
        raise ValueError("Unsupported sequence manifest")
    size = (int(manifest["width"]), int(manifest["height"]))
    if min(size) <= 0 or max(size) > 8192 or size[0] * size[1] > 16777216:
        raise ValueError("Invalid canvas size")
    entries = manifest["frames"]
    if not entries or len(entries) > 10000:
        raise ValueError("Invalid frame count")
    keep_alpha = manifest.get("keep_alpha", True)
    if type(keep_alpha) is not bool:
        raise ValueError("keep_alpha must be a boolean")
    durations: list[int] = []
    paths: list[Path] = []
    for entry in entries:
        filename = entry["file"]
        if not isinstance(filename, str) or Path(filename).name != filename or not filename.endswith(".png"):
            raise ValueError("Frame paths must be PNG basenames")
        delay = entry["duration_ms"]
        if type(delay) is not int or not 1 <= delay <= 65535:
            raise ValueError("Invalid frame duration")
        durations.append(delay)
        paths.append(sequence / filename)

    output.parent.mkdir(parents=True, exist_ok=True)
    progress.report("Preparing background")
    background = background_image(manifest)
    owns_temporary = False
    try:
        no_background = manifest.get("background", {}).get("mode") == "none"
        frames = encoded_frames(paths, size, None if no_background else background, keep_alpha, encoders)
        with temporary.open("xb") as destination:
            owns_temporary = True
            try:
                write_apng(destination, frames, durations)
            finally:
                frames.close()
        progress.report("Publishing APNG")
        if overwrite:
            temporary.replace(output)
        else:
            try:
                os.link(temporary, output)
            except FileExistsError:
                progress.skipped("Exists")
                progress.message(f"Exists: {output}")
                return
    finally:
        if owns_temporary:
            temporary.unlink(missing_ok=True)
        background.close()
    progress.message(f"Created {output} ({len(entries)} frames, {perf_counter() - started:.2f}s)")


def assemble_directory(root: Path, *, jobs: int = 1, encoders: int = 4,
                       overwrite: bool = False) -> list[Path]:
    if type(jobs) is not int or jobs < 1 or type(encoders) is not int or encoders < 1:
        raise ValueError("jobs and encoders must be positive integers")
    root = root.resolve()
    if not root.is_dir():
        raise NotADirectoryError(root)
    manifests = sorted(root.rglob("manifest.json"))
    if not manifests:
        raise FileNotFoundError(f"No manifest.json found under {root}")
    sequences = [manifest.parent for manifest in manifests]
    outputs = [sequence.parent / f"{sequence.name}.png" for sequence in sequences]
    if jobs == 1 or len(sequences) == 1:
        for sequence, output in zip(sequences, outputs, strict=True):
            assemble(sequence, output, encoders=encoders, overwrite=overwrite)
    else:
        with ProcessPoolExecutor(max_workers=min(jobs, len(sequences)),
                                 mp_context=multiprocessing.get_context("spawn"),
                                 initializer=progress.connect, initargs=(progress.connection(),)) as pool:
            futures = [pool.submit(assemble, sequence, output, encoders=encoders, overwrite=overwrite)
                       for sequence, output in zip(sequences, outputs, strict=True)]
            try:
                for future in as_completed(futures):
                    future.result()
            finally:
                for future in futures:
                    future.cancel()
    return outputs


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, default=Path("target/showcases"),
                        help="Recursively search this directory for manifest.json (default: target/showcases)")
    parser.add_argument("--jobs", type=int, default=min(4, os.cpu_count() or 1),
                        help="Concurrent showcase processes (default: up to 4)")
    parser.add_argument("--encoders", type=int,
                        help="Frame workers per showcase (default: up to 4, CPU-adjusted; 1 is serial)")
    parser.add_argument("--no-progress", action="store_true", help="Disable live progress; keep summaries")
    parser.add_argument("--overwrite", action="store_true", help="Replace existing outputs (default: skip)")
    arguments = parser.parse_args()
    if arguments.jobs < 1 or (arguments.encoders is not None and arguments.encoders < 1):
        parser.error("--jobs and --encoders must be positive")
    encoders = arguments.encoders or max(1, min(4, (os.cpu_count() or 1) // arguments.jobs))
    with progress.display("Assembling showcases", len(list(arguments.input.rglob("manifest.json"))),
                          enabled=not arguments.no_progress, processes=arguments.jobs > 1):
        assemble_directory(arguments.input, jobs=arguments.jobs, encoders=encoders, overwrite=arguments.overwrite)


if __name__ == "__main__":
    main()