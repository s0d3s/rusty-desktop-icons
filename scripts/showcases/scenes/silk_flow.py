from dataclasses import dataclass
import math
from pathlib import Path
from typing import Literal

import rusty_desktop_icons as rdi

from .base import CompressedOutputParams, Point, Showcase


@dataclass(frozen=True)
class SilkFlowSoloShowcase(Showcase):
    name: str = "silk_flow_solo"
    shader: rdi.BuiltinShader | str | None = rdi.BuiltinShader.SilkFlow
    output_size: tuple[int, int] | None = (960, 140)
    icon_size: int = 64
    max_icons: int | None = 1
    duration: float = 5.0
    mockup_background: bool = False
    keep_alpha: bool = True
    taskbar_height_dip: int = 0
    compressed_output_params: CompressedOutputParams = CompressedOutputParams(
        format="webp", size=(output_size[0] // 2, output_size[1] // 2), fps=20, webp_quality=95,
    )

    def layout(self, icons: list[rdi.IconSnapshot]) -> list[Point]:
        if len(icons) > 1:
            raise ValueError("Silk Flow showcase displays one icon")
        _, height = self.canvas_size()
        icon_width = math.ceil(self.icon_size * self.dpi_scale)
        margin = math.ceil(8 * self.dpi_scale)
        return [(margin, (height - icon_width) // 2)] if icons else []

    def destinations(self, icons: list[rdi.IconSnapshot], positions: list[Point]) -> list[Point]:
        width, _ = self.canvas_size()
        icon_width = math.ceil(self.icon_size * self.dpi_scale)
        return [(width - left - icon_width, top) for left, top in positions]

    def effect_envelope(self) -> rdi.Curve:
        return rdi.Curve.keyframes([(0.0, 1.0), (1.0, 1.0)], interp=rdi.KeyframeInterp.Linear)

    def render_seconds(self, seconds: float) -> float:
        progress = max(0.0, min(1.0, (seconds / self.duration - 0.10) / 0.78))
        return progress * self.duration


@dataclass(frozen=True)
class SilkFlowShowcase(Showcase):
    name: str = "silk_flow"
    shader: rdi.BuiltinShader | str | None = rdi.BuiltinShader.SilkFlow
    duration: float = 5.0

    def effect_envelope(self) -> rdi.Curve:
        return rdi.Curve.keyframes([(0.0, 1.0), (1.0, 1.0)], interp=rdi.KeyframeInterp.Linear)


@dataclass(frozen=True)
class SmallSilkFlowShowcase(SilkFlowShowcase):
    name: str = "silk_flow_small"
    output_grid: tuple[int, int] | None = (5, 3)
    max_icons: int | None = 6
    margin_dip: int = 12
    mockup_path: Path = Path(__file__).resolve().parents[1] / "assets/desktop-mockup-small.png"
    mockup_mode: Literal["auto", "fit", "crop", "native"] = "native"
