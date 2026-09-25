"""Rebuild the original desktop mockup shipped with the showcase tools."""

import argparse
import math
from pathlib import Path
import sys

from PIL import Image, ImageDraw, ImageOps

sys.path.insert(0, str(Path(__file__).resolve().parent))
import showcase_progress as progress


def small_mockup() -> Image.Image:
    scale = 3
    size = (404, 382)
    ramp = Image.linear_gradient("L").resize((size[0] * scale, size[1] * scale))
    image = ImageOps.colorize(ramp, "#183c53", "#348b9b").convert("RGB")
    artwork = ImageDraw.Draw(image)

    def line(points: list[tuple[float, float]], fill: str, width: int = 1) -> None:
        artwork.line([(round(horizontal * scale), round(vertical * scale)) for horizontal, vertical in points],
                     fill=fill, width=width * scale, joint="curve")

    def rectangle(box: tuple[int, int, int, int], fill: str, radius: int = 0) -> None:
        artwork.rounded_rectangle(tuple(value * scale for value in box), radius=radius * scale, fill=fill)

    def ellipse(box: tuple[int, int, int, int], outline: str, width: int = 2) -> None:
        artwork.ellipse(tuple(value * scale for value in box), outline=outline, width=width * scale)

    for offset, color in ((0, "#27788b"), (38, "#43a4ad"), (70, "#77c8ca"),
                          (84, "#a7ded8"), (103, "#4793aa"), (161, "#24556f")):
        edge = [(horizontal, 160 + offset * 0.55 + 60 * math.sin(horizontal / 150 + 0.4))
            for horizontal in range(-20, 426, 3)]
        polygon = edge + [(426, 318), (-20, 318)]
        artwork.polygon([(round(horizontal * scale), round(vertical * scale)) for horizontal, vertical in polygon], fill=color)
        line(edge, color)

    rectangle((0, 318, 404, 382), "#edf2f5")
    line([(0, 318), (404, 318)], "#c9d8df")
    for horizontal in (20, 33):
        for vertical in (339, 352):
            rectangle((horizontal, vertical, horizontal + 10, vertical + 10), "#087fc3")
    ellipse((70, 339, 86, 355), "#344b5d")
    line([(84, 353), (91, 360)], "#344b5d", 2)
    rectangle((113, 343, 138, 361), "#e5af42", 2)
    rectangle((113, 339, 125, 346), "#f1c666", 2)
    rectangle((113, 348, 138, 351), "#ffd67d")
    return image.resize(size, Image.Resampling.LANCZOS)


@progress.work_item()
def generate_mockup(small: bool) -> None:
    progress.report("Drawing mockup", 0, 2)
    if small:
        destination = Path(__file__).parent / "assets/desktop-mockup-small.png"
        destination.parent.mkdir(parents=True, exist_ok=True)
        with small_mockup() as image:
            progress.report("Saving PNG", 1, 2)
            image.save(destination, optimize=True)
            progress.report("Saving PNG", 2, 2)
            progress.message(f"Generated {destination}: {image.width}x{image.height}")
        return
    size = (3840, 2400)
    ramp = Image.linear_gradient("L").resize(size)
    image = ImageOps.colorize(ramp, "#164f63", "#40aaa0").convert("RGBA")
    artwork = ImageDraw.Draw(image)
    artwork.polygon([(1450, 0), (2450, 0), (3840, 1740), (3840, 2400)], fill="#70c6bd")
    artwork.polygon([(2050, 0), (2490, 0), (3840, 1700), (3840, 2200)], fill="#c5e3df")
    artwork.polygon([(2450, 0), (2870, 0), (3840, 1140), (3840, 1710)], fill="#e2a98e")
    artwork.polygon([(2870, 0), (3250, 0), (3840, 660), (3840, 1130)], fill="#e7d9c9")
    artwork.rectangle((0, 2304, 3840, 2400), fill="#e3efed")
    artwork.line((0, 2304, 3840, 2304), fill="#b5ceca", width=2)
    for horizontal in (1748, 1776):
        for vertical in (2330, 2358):
            artwork.rectangle((horizontal, vertical, horizontal + 22, vertical + 22), fill="#267997")
    artwork.ellipse((1840, 2330, 1874, 2364), outline="#3c565b", width=5)
    artwork.line((1868, 2360, 1886, 2380), fill="#3c565b", width=5)
    artwork.rounded_rectangle((1920, 2336, 1976, 2376), radius=5, fill="#d6a44a")
    artwork.rectangle((1920, 2328, 1944, 2342), fill="#e8c572")
    artwork.ellipse((2020, 2328, 2072, 2380), fill="#3283a5")
    artwork.ellipse((2034, 2342, 2058, 2366), fill="#e3efed")
    artwork.rounded_rectangle((3598, 2334, 3638, 2370), radius=4, outline="#3c565b", width=3)
    artwork.line((3660, 2342, 3720, 2342), fill="#3c565b", width=4)
    artwork.line((3660, 2360, 3700, 2360), fill="#637f83", width=4)
    destination = Path(__file__).parent / "assets/desktop-mockup.png"
    destination.parent.mkdir(parents=True, exist_ok=True)
    progress.report("Saving PNG", 1, 2)
    image.convert("RGB").save(destination, optimize=True)
    progress.report("Saving PNG", 2, 2)
    progress.message(f"Generated {destination}: {size[0]}x{size[1]}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--small", action="store_true", help="Generate the native 404x382 small showcase mockup")
    parser.add_argument("--no-progress", action="store_true", help="Disable live progress; keep summaries")
    arguments = parser.parse_args()
    with progress.display("Generating mockup", 1, enabled=not arguments.no_progress):
        generate_mockup(arguments.small)


if __name__ == "__main__":
    main()