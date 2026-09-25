"""Declare and render desktop-icon showcases without changing the desktop."""

from __future__ import annotations

import argparse
from collections import deque
from dataclasses import asdict
from concurrent.futures import Future, ProcessPoolExecutor, ThreadPoolExecutor, as_completed
import importlib.util
import json
import multiprocessing
import os
from pathlib import Path
import shutil
import sys
from time import perf_counter
from types import ModuleType

import rusty_desktop_icons as rdi

sys.path.insert(0, str(Path(__file__).resolve().parent))
import showcase_progress as progress
from scenes import Showcase, discover_showcases


def save_frame(images: ModuleType, width: int, height: int, pixels: bytes,
               output: Path, compression: int) -> None:
    with images.rgba_image(width, height, pixels) as image:
        image.save(output, compress_level=compression)


@progress.work_item("scene")
def export(controller: rdi.DesktopController, icons: list[rdi.IconSnapshot], scene: Showcase, root: Path,
           *, encoders: int = 4, png_compression: int = 1, overwrite: bool = False) -> None:
    if type(encoders) is not int or encoders < 1:
        raise ValueError("encoders must be a positive integer")
    if type(png_compression) is not int or not 0 <= png_compression <= 9:
        raise ValueError("png_compression must be an integer in 0..9")
    output = root / scene.name
    if output.exists() and not overwrite:
        progress.skipped("Exists")
        progress.message(f"Exists: {output}")
        return
    started = perf_counter()
    progress.report("Preparing scene")
    icons = icons[:scene.max_icons]
    spec = importlib.util.spec_from_file_location("assemble_showcase", Path(__file__).with_name("assemble-showcase.py"))
    assert spec is not None and spec.loader is not None
    images = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(images)
    width, height = scene.canvas_size()
    manifest = {"version": 2, "width": width, "height": height, "fps": scene.fps,
                "playback": "forward_reverse", "icons": len(icons),
                "desktop_size": [scene.width, scene.height], "dpi_scale": scene.dpi_scale,
                "icon_size": scene.icon_size, "taskbar_height": scene.cell_metrics()[3],
                "background": scene.background_manifest(), "keep_alpha": scene.keep_alpha,
                "compressed_output_params": asdict(scene.compressed_output_params)}
    images.background_image(manifest).close()
    schedule = images.frame_schedule(scene.duration, scene.fps)
    canvas = rdi.Canvas(width, height, dpi_scale=scene.dpi_scale, icon_size=scene.icon_size)
    positions = scene.layout(icons)
    if len(positions) != len(icons):
        raise ValueError(f"{scene.name}: layout must return one position per icon")
    targets = scene.destinations(icons, positions)
    if len(targets) != len(icons) or (not scene.allow_overlap and len(set(targets)) != len(targets)):
        raise ValueError(f"{scene.name}: destinations must return one unique target per icon")
    animations = scene.animations(icons, positions, targets)
    if len(animations) != len(icons) or {animation.id for animation in animations} != {icon.id for icon in icons}:
        raise ValueError(f"{scene.name}: animations must contain exactly one spec per icon")
    options = scene.render_options()
    if overwrite and output.exists():
        shutil.rmtree(output)
    output.mkdir(parents=True, exist_ok=False)
    records: list[dict[str, object]] = []
    pending: deque[Future[None]] = deque()
    completed = 0
    capacity = max(1, min(2 * encoders, (128 * 1024 * 1024) // (width * height * 4)))
    with controller.prepare_scene(
        canvas, animations, [(icon.id, position) for icon, position in zip(icons, positions, strict=True)],
        options=options,
    ) as renderer, ThreadPoolExecutor(max_workers=encoders, thread_name_prefix="showcase-png") as pool:
        try:
            progress.report("Rendering / PNG", 0, len(schedule))
            for index, (seconds, delay) in enumerate(schedule):
                if len(pending) >= capacity:
                    pending.popleft().result()
                    completed += 1
                    progress.report("Rendering / PNG", completed, len(schedule))
                render_seconds = min(scene.render_seconds(seconds), renderer.duration)
                frame = renderer.render_at(render_seconds)
                filename = f"{index:06d}.png"
                if encoders == 1:
                    save_frame(images, frame.width, frame.height, frame.pixels, output / filename, png_compression)
                    completed += 1
                    progress.report("Rendering / PNG", completed, len(schedule))
                else:
                    pending.append(pool.submit(save_frame, images, frame.width, frame.height,
                                               frame.pixels, output / filename, png_compression))
                records.append({"file": filename, "seconds": seconds, "duration_ms": delay})
                if render_seconds != seconds:
                    records[-1]["render_seconds"] = render_seconds
            while pending:
                pending.popleft().result()
                completed += 1
                progress.report("Rendering / PNG", completed, len(schedule))
        finally:
            for future in pending:
                future.cancel()
    progress.report("Publishing manifest")
    manifest["frames"] = records
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    elapsed = perf_counter() - started
    progress.message(f"{scene.name}: {len(records)} frames in {output} "
                     f"({elapsed:.2f}s, {len(records) / elapsed:.1f} frames/s)")


@progress.work_item("name")
def export_showcase(name: str, root: Path, encoders: int, png_compression: int,
                    overwrite: bool = False) -> None:
    output = root / name
    if output.exists() and not overwrite:
        progress.skipped("Exists")
        progress.message(f"Exists: {output}")
        return
    progress.report("Reading desktop icons")
    scene = next(scene for scene in discover_showcases() if scene.name == name)
    controller = rdi.DesktopController()
    icons = sorted(controller.list_icons(), key=lambda icon: (icon.display_name.casefold(), icon.id))
    if not icons:
        raise ValueError("No desktop icons available")
    export(controller, icons, scene, root, encoders=encoders, png_compression=png_compression,
           overwrite=overwrite)


def render_showcases(names: list[str], root: Path, *, jobs: int, encoders: int, png_compression: int,
                     overwrite: bool = False) -> None:
    if jobs == 1 or len(names) <= 1:
        for name in names:
            export_showcase(name, root, encoders, png_compression, overwrite=overwrite)
        return
    with ProcessPoolExecutor(max_workers=min(jobs, len(names)),
                             mp_context=multiprocessing.get_context("spawn"),
                             initializer=progress.connect, initargs=(progress.connection(),)) as pool:
        futures = [pool.submit(export_showcase, name, root, encoders, png_compression, overwrite=overwrite)
               for name in names]
        try:
            for future in as_completed(futures):
                future.result()
        finally:
            for future in futures:
                future.cancel()


def main() -> None:
    scenes = discover_showcases()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path("target/showcases"))
    parser.add_argument("--showcase", choices=[scene.name for scene in scenes])
    parser.add_argument("--jobs", type=int, default=min(4, os.cpu_count() or 1),
                        help="Concurrent showcase processes (default: up to 4)")
    parser.add_argument("--encoders", type=int,
                        help="PNG workers per showcase (default: up to 4, divided across jobs; 1 is synchronous)")
    parser.add_argument("--png-compression", type=int, choices=range(10), default=1,
                        help="Lossless intermediate PNG compression, 0 fastest / 9 smallest (default: 1)")
    parser.add_argument("--no-progress", action="store_true", help="Disable live progress; keep summaries")
    parser.add_argument("--overwrite", action="store_true", help="Replace existing output sequences (default: skip)")
    arguments = parser.parse_args()
    if arguments.jobs < 1 or (arguments.encoders is not None and arguments.encoders < 1):
        parser.error("--jobs and --encoders must be positive")
    names = [scene.name for scene in scenes if arguments.showcase is None or scene.name == arguments.showcase]
    jobs = min(arguments.jobs, len(names))
    encoders = arguments.encoders or max(1, min(4, (os.cpu_count() or 1) // max(1, jobs)))
    print(f"Rendering with {jobs} showcase process(es), {encoders} PNG worker(s) each, "
          f"compression {arguments.png_compression}", flush=True)
    with progress.display("Rendering showcases", len(names), enabled=not arguments.no_progress, processes=jobs > 1):
        render_showcases(names, arguments.output, jobs=jobs, encoders=encoders,
                         png_compression=arguments.png_compression, overwrite=arguments.overwrite)


if __name__ == "__main__":
    main()