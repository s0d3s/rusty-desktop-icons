"""Optimize only top-level PNG/APNG files; never modify originals or nested folders."""

from __future__ import annotations

import argparse
from contextlib import ExitStack
from fractions import Fraction
import hashlib
import io
import os
from pathlib import Path
import struct
import sys
import tempfile

from apng import APNG, PNG
from PIL import Image, ImageChops, PngImagePlugin

sys.path.insert(0, str(Path(__file__).resolve().parent))
import showcase_progress as progress


def digest(image: Image.Image) -> bytes:
    with image.convert("RGBA") as rgba:
        return hashlib.sha256(rgba.tobytes()).digest()


def frame_delays(path: Path) -> list[Fraction]:
    animation = APNG.open(path)
    return [Fraction(control.delay, control.delay_den or 100) if control else Fraction(0)
            for _, control in animation.frames]


def fits_delay(delay: Fraction) -> bool:
    return delay.numerator <= 65535 and delay.denominator <= 65535


def verify(path: Path, expected: list[tuple[bytes, Fraction]], size: tuple[int, int],
           animated: bool, loops: int) -> None:
    progress.report("Verifying frames", 0, len(expected))
    if frame_delays(path) != [delay for _, delay in expected]:
        raise ValueError("Encoded frame timing differs from input")
    with Image.open(path) as image:
        if image.size != size or image.n_frames != len(expected):
            raise ValueError("Encoded dimensions/frame count differ from input")
        if animated and image.info.get("loop") != loops:
            raise ValueError("Encoded loop count differs from input")
        for index, (expected_digest, _) in enumerate(expected):
            image.seek(index)
            if digest(image) != expected_digest:
                raise ValueError(f"Encoded pixels differ at frame {index}")
            progress.report("Verifying frames", index + 1, len(expected))


@progress.work_item("source")
def optimize(source: Path, output: Path, width: int | None = None,
             colors: int | None = None) -> dict[str, int]:
    if width is not None and (type(width) is not int or width <= 0):
        raise ValueError("width must be a positive integer")
    if colors is not None and (type(colors) is not int or not 2 <= colors <= 256):
        raise ValueError("colors must be an integer between 2 and 256")
    if output.exists() or source.resolve() == output.resolve():
        raise FileExistsError(output)
    with source.open("rb") as stream:
        header = stream.read(25)
    if header[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("Input must be PNG")
    if len(header) < 25 or header[24] == 16:
        raise ValueError("16-bit or truncated PNG inputs are not supported")

    progress.report("Reading frame metadata")
    delays = frame_delays(source)
    with ExitStack() as stack:
        image = stack.enter_context(Image.open(source))
        if image.info.get("default_image"):
            raise ValueError("APNG with a separate default/poster image is not supported")
        if image.n_frames > 10000 or image.width * image.height > 16777216:
            raise ValueError("Input exceeds 10000 frames or 16777216 pixels")
        animated = "loop" in image.info
        loops = int(image.info.get("loop", 0))
        count = image.n_frames
        if len(delays) != count:
            raise ValueError("PNG frame controls do not match decoded frames")
        target_width = min(width or image.width, image.width)
        size = (target_width, max(1, round(image.height * target_width / image.width)))
        metadata = PngImagePlugin.PngInfo()
        for key in ("gamma", "srgb", "chromaticity"):
            if key in image.info:
                if key == "gamma":
                    metadata.add(b"gAMA", struct.pack("!I", round(image.info[key] * 100000)))
                elif key == "srgb":
                    metadata.add(b"sRGB", bytes([image.info[key]]))
                else:
                    metadata.add(b"cHRM", struct.pack("!8I", *(round(value * 100000) for value in image.info[key])))
        profile = image.info.get("icc_profile")
        opaque = True
        sample_indices = {round(index * (count - 1) / 31) for index in range(32)} if colors else set()
        samples = Image.new("RGB", (128, 128 * max(1, len(sample_indices))))
        sample_index = 0
        progress.report("Scanning frames", 0, count)
        for index in range(count):
            image.seek(index)
            with image.convert("RGBA") as rgba:
                opaque = opaque and rgba.getchannel("A").getextrema() == (255, 255)
            if index in sample_indices:
                with image.convert("RGB") as rgb, rgb.resize((128, 128), Image.Resampling.BOX) as sample:
                    samples.paste(sample, (0, sample_index * 128))
                sample_index += 1
            progress.report("Scanning frames", index + 1, count)
        if colors and not opaque:
            samples.close()
            raise ValueError("Color reduction requires opaque composed images; alpha will not be discarded")
        palette = samples.quantize(colors=colors, method=Image.Quantize.MEDIANCUT) if colors else None
        samples.close()
        if palette is not None:
            entries = (palette.getpalette() or [])[:colors * 3]
            palette.putpalette(entries + entries[-3:] * (256 - len(entries) // 3))

        image.close()
        image = stack.enter_context(Image.open(source))
        animation = APNG(num_plays=loops)
        expected: list[tuple[bytes, Fraction]] = []
        previous: Image.Image | None = None
        progress.report("Optimizing frames", 0, count)
        for index, delay in enumerate(delays):
            image.seek(index)
            frame = image.convert("RGB" if opaque else "RGBA")
            if frame.size != size:
                resized = frame.resize(size, Image.Resampling.LANCZOS)
                frame.close()
                frame = resized
            if palette is not None:
                quantized = frame.quantize(palette=palette, dither=Image.Dither.NONE)
                frame.close()
                frame = quantized.convert("RGB")
                quantized.close()
            bounds = (0, 0, *size)
            if previous is not None:
                with ImageChops.difference(previous, frame) as difference:
                    bounds = difference.getbbox(alpha_only=False)
                combined = expected[-1][1] + delay
                if bounds is None and delay > 0 and expected[-1][1] > 0 and fits_delay(combined):
                    control = animation.frames[-1][1]
                    control.delay, control.delay_den = combined.numerator, combined.denominator
                    expected[-1] = (expected[-1][0], combined)
                    frame.close()
                    progress.report("Optimizing frames", index + 1, count)
                    continue
                previous.close()
            with frame.crop(bounds or (0, 0, 1, 1)) as crop, io.BytesIO() as encoded:
                with crop.quantize(palette=palette, dither=Image.Dither.NONE) if palette else crop.copy() as payload:
                    payload.save(encoded, format="PNG", optimize=True, compress_level=9,
                                 pnginfo=metadata, icc_profile=profile)
                animation.append(PNG.from_bytes(encoded.getvalue()),
                                 x_offset=bounds[0] if bounds else 0, y_offset=bounds[1] if bounds else 0,
                                 delay=delay.numerator, delay_den=delay.denominator, depose_op=0, blend_op=0)
            expected.append((digest(frame), delay))
            previous = frame
            progress.report("Optimizing frames", index + 1, count)
        if previous is not None:
            previous.close()
        if palette is not None:
            palette.close()

    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".optimize-", dir=output.parent) as directory:
        temporary = Path(directory) / "result.png"
        progress.report("Writing optimized PNG")
        if animated:
            animation.save(temporary)
        else:
            animation.frames[0][0].save(temporary)
        verify(temporary, expected, size, animated, loops)
        progress.report("Publishing output")
        os.link(temporary, output)
    return {"before_bytes": source.stat().st_size, "after_bytes": output.stat().st_size,
            "before_frames": count, "after_frames": len(expected), "width": size[0], "height": size[1]}


@progress.work_item("source")
def optimize_one(source: Path, output: Path, width: int | None, colors: int | None) -> None:
    destination = output / source.name
    if destination.exists():
        progress.skipped()
        progress.message(f"Skipped existing output: {destination}")
        return
    progress.message(f"Optimizing {source.name}...")
    result = optimize(source, destination, width, colors)
    reduction = 100 * (1 - result["after_bytes"] / result["before_bytes"])
    progress.message(f"{source.name}: {result['before_bytes'] / 1048576:.2f} -> "
                     f"{result['after_bytes'] / 1048576:.2f} MiB ({reduction:.1f}% smaller), "
                     f"{result['before_frames']} -> {result['after_frames']} frames, "
                     f"{result['width']}x{result['height']}; verified -> {destination}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, default=Path("target/showcases"))
    parser.add_argument("--output", type=Path, help="Default: sibling <input-name>-optimized folder")
    parser.add_argument("--width", type=int, help="Optional maximum width; preserve aspect ratio, never upscale")
    parser.add_argument("--colors", type=int, help="Optional shared palette of 2..256 colors (lossy, opaque only)")
    parser.add_argument("--no-progress", action="store_true", help="Disable live progress; keep summaries")
    arguments = parser.parse_args()
    root = arguments.input.resolve()
    output = (arguments.output or root.with_name(root.name + "-optimized")).resolve()
    if output == root or root in output.parents:
        parser.error("Output must be outside the input directory; nested folders must remain untouched")
    if not root.is_dir():
        parser.error(f"Not a directory: {root}")
    if arguments.width is not None and arguments.width <= 0:
        parser.error("--width must be positive")
    if arguments.colors is not None and not 2 <= arguments.colors <= 256:
        parser.error("--colors must be between 2 and 256")
    paths = sorted(path for path in root.glob("*.png") if path.is_file() and not path.is_symlink())
    if not paths:
        parser.error(f"No top-level PNG files in {root}")
    failed = False
    with progress.display("Optimizing showcases", len(paths), enabled=not arguments.no_progress):
        for source in paths:
            try:
                optimize_one(source, output, arguments.width, arguments.colors)
            except (OSError, ValueError):
                failed = True
    if failed:
        raise SystemExit(1)


if __name__ == "__main__":
    main()