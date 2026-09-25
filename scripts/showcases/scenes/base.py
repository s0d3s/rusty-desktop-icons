"""Shared scene declarations; hooks run before Rust rendering starts."""

from __future__ import annotations

from dataclasses import dataclass
import math
from pathlib import Path
import random
from typing import Literal

import rusty_desktop_icons as rdi

Point = tuple[int, int]


@dataclass(frozen=True)
class CompressedOutputParams:
    format: Literal["auto", "gif", "webp"] = "auto"
    size: tuple[int | None, int | None] = (960, 540)
    allow_upscale: bool = False
    fps: int = 15
    gif_colors: int = 128
    gif_dither: Literal["bayer", "sierra2_4a", "none"] = "bayer"
    webp_lossless: bool = False
    webp_quality: int = 80
    webp_method: int = 4

    def __post_init__(self) -> None:
        if self.format not in ("auto", "gif", "webp"):
            raise ValueError("Unknown compressed output format")
        if not isinstance(self.size, (tuple, list)) or len(self.size) != 2 or any(
            value is not None and (type(value) is not int or not 1 <= value <= 8192)
            for value in self.size
        ):
            raise ValueError("size must contain two dimensions in 1..8192 or None")
        object.__setattr__(self, "size", tuple(self.size))
        for name, lower, upper in (("fps", 1, 30), ("gif_colors", 4, 256),
                                   ("webp_quality", 0, 100), ("webp_method", 0, 6)):
            value = getattr(self, name)
            if type(value) is not int or not lower <= value <= upper:
                raise ValueError(f"{name} must be an integer in {lower}..{upper}")
        if self.gif_dither not in ("bayer", "sierra2_4a", "none"):
            raise ValueError("Unknown GIF dither")
        if type(self.allow_upscale) is not bool or type(self.webp_lossless) is not bool:
            raise ValueError("allow_upscale and webp_lossless must be booleans")


@dataclass(frozen=True)
class Showcase:
    name: str = ""
    shader: rdi.BuiltinShader | str | None = None
    simulation_resolution: tuple[int, int] | None = None
    width: int = 1920
    height: int = 1080
    dpi_scale: float = 1.0
    icon_size: int = 48
    duration: float = 2.0
    fps: int = 60
    seed: int = 0
    cell_width_dip: int = 76
    cell_height_dip: int = 98
    margin_dip: int = 48
    taskbar_height_dip: int = 64
    max_icons: int | None = 30
    allow_overlap: bool = False
    output_size: tuple[int, int] | None = None
    output_grid: tuple[int, int] | None = None
    mockup_background: bool = True
    keep_alpha: bool = False
    compressed_output_params: CompressedOutputParams = CompressedOutputParams()
    mockup_path: Path = Path(__file__).resolve().parents[1] / "assets/desktop-mockup.png"
    mockup_mode: Literal["auto", "fit", "crop", "native"] = "auto"

    def __post_init__(self) -> None:
        if self.simulation_resolution is not None:
            if (len(self.simulation_resolution) != 2 or any(
                type(value) is not int or not 8 <= value <= 1024
                for value in self.simulation_resolution
            )):
                raise ValueError("simulation_resolution must contain two integers in 8..1024")
            if self.shader is None:
                raise ValueError("simulation_resolution requires a shader")
        if not isinstance(self.compressed_output_params, CompressedOutputParams):
            raise ValueError("compressed_output_params must be CompressedOutputParams")
        if self.max_icons is not None and (type(self.max_icons) is not int or self.max_icons <= 0):
            raise ValueError("max_icons must be a positive integer or None")
        if self.output_size is not None and self.output_grid is not None:
            raise ValueError("Declare either output_size or output_grid, not both")
        for name, pair in (("desktop size", (self.width, self.height)),
                           ("output_size", self.output_size), ("output_grid", self.output_grid)):
            if pair is not None and (len(pair) != 2 or any(type(value) is not int or value <= 0 for value in pair)):
                raise ValueError(f"{name} must contain two positive integers")
        if not math.isfinite(self.dpi_scale) or self.dpi_scale <= 0:
            raise ValueError("dpi_scale must be positive and finite")
        if type(self.mockup_background) is not bool:
            raise ValueError("mockup_background must be a boolean")
        if type(self.keep_alpha) is not bool:
            raise ValueError("keep_alpha must be a boolean")
        if self.mockup_mode not in ("auto", "fit", "crop", "native"):
            raise ValueError("Unknown mockup_mode")

    def background_manifest(self) -> dict[str, str | None]:
        if not self.mockup_background:
            return {"mode": "none", "path": None}
        mode = self.mockup_mode
        if mode == "auto":
            mode = "crop" if self.output_size or self.output_grid else "fit"
        return {"mode": mode,
                "path": str(self.mockup_path.resolve())}

    def cell_metrics(self) -> tuple[int, int, int, int]:
        extra_size = max(0, self.icon_size - 48)
        cell_width = math.ceil((self.cell_width_dip + extra_size) * self.dpi_scale)
        cell_height = math.ceil((self.cell_height_dip + extra_size) * self.dpi_scale)
        margin = math.ceil(self.margin_dip * self.dpi_scale)
        if cell_width <= 0 or cell_height <= 0 or margin < 0 or self.taskbar_height_dip < 0:
            raise ValueError("Grid spacing must be positive; margins must be nonnegative")
        return cell_width, cell_height, margin, math.ceil(self.taskbar_height_dip * self.dpi_scale)

    def canvas_size(self) -> tuple[int, int]:
        size = self.output_size or (self.width, self.height)
        if self.output_grid is not None:
            cell_width, cell_height, margin, taskbar = self.cell_metrics()
            columns, rows = self.output_grid
            size = (columns * cell_width + 2 * margin, rows * cell_height + 2 * margin + taskbar)
        if size[0] > self.width or size[1] > self.height:
            raise ValueError("Output area must fit within the desktop size")
        if max(size) > 8192 or size[0] * size[1] > 16777216:
            raise ValueError("Output area exceeds Canvas limits")
        return size

    def grid_metrics(self) -> tuple[int, int, int, int, int]:
        cell_width, cell_height, margin, taskbar = self.cell_metrics()
        width, height = self.canvas_size()
        columns = (width - 2 * margin) // cell_width
        rows = (height - 2 * margin - taskbar) // cell_height
        return cell_width, cell_height, margin, columns, rows

    def layout(self, icons: list[rdi.IconSnapshot]) -> list[Point]:
        cell_width, cell_height, margin, columns, rows = self.grid_metrics()
        if columns <= 0 or rows <= 0 or len(icons) > columns * rows:
            raise ValueError(f"{len(icons)} icons do not fit {self.canvas_size()}; increase output area or reduce icon size/DPI")
        icon_width = math.ceil(self.icon_size * self.dpi_scale)
        return [(margin + index // rows * cell_width + (cell_width - icon_width) // 2,
                 margin + index % rows * cell_height) for index in range(len(icons))]

    def destinations(self, icons: list[rdi.IconSnapshot], positions: list[Point]) -> list[Point]:
        """Default grid shuffle; override alongside layout for non-grid scenes."""
        if not positions:
            return []
        cell_width, cell_height, margin, columns, rows = self.grid_metrics()
        if rows < 2:
            raise ValueError("Showcase shuffle needs at least two rows and two spare columns; increase canvas or reduce icon size/DPI")
        used_columns = (len(icons) + rows - 1) // rows
        if used_columns + 2 > columns:
            raise ValueError("Showcase shuffle needs at least two rows and two spare columns; increase canvas or reduce icon size/DPI")
        shuffled_rows = list(range(rows))
        generator = random.Random(self.seed)
        for index in range(rows - 1, 0, -1):
            other = generator.randrange(index)
            shuffled_rows[index], shuffled_rows[other] = shuffled_rows[other], shuffled_rows[index]
        vary_columns = used_columns + 3 <= columns
        return [(position[0] + (2 + (shuffled_rows[index % rows] % 2 if vary_columns else 0)) * cell_width,
                 margin + shuffled_rows[index % rows] * cell_height)
                for index, position in enumerate(positions)]

    def effect_envelope(self) -> rdi.Curve | None:
        return None

    def render_seconds(self, seconds: float) -> float:
        return seconds

    def animations(self, icons: list[rdi.IconSnapshot], positions: list[Point], targets: list[Point]) -> list[rdi.IconAnimationSpec]:
        source = rdi.ShaderSource.builtin(self.shader) if self.shader else None
        if source is not None and self.simulation_resolution is not None:
            source = source.with_simulation_resolution(*self.simulation_resolution)
        shader = rdi.Shader.compile(source) if source is not None else None
        envelope = self.effect_envelope() if shader else None
        return [rdi.IconAnimationSpec(
            id=icon.id, target=target,
            duration=rdi.Duration.fixed(seconds=self.duration), curve=rdi.Curve.ease_in_out(),
            effect=rdi.Effect(shader, envelope=envelope, seed=float(index), padding_px=24) if shader else None,
        ) for index, (icon, target) in enumerate(zip(icons, targets, strict=True))]

    def render_options(self) -> rdi.AnimationOptions:
        return rdi.AnimationOptions()