from dataclasses import dataclass
from pathlib import Path
from typing import Literal

import rusty_desktop_icons as rdi

from .base import Showcase


@dataclass(frozen=True)
class DustTransferShowcase(Showcase):
    name: str = "dust_transfer"
    shader: rdi.BuiltinShader | str | None = rdi.BuiltinShader.DustTransfer
    duration: float = 4.0

    def effect_envelope(self) -> rdi.Curve:
        return rdi.Curve.keyframes([(0.0, 1.0), (1.0, 1.0)], interp=rdi.KeyframeInterp.Linear)


@dataclass(frozen=True)
class SmallDustTransferShowcase(DustTransferShowcase):
    name: str = "dust_transfer_small"
    output_grid: tuple[int, int] | None = (5, 3)
    max_icons: int | None = 6
    margin_dip: int = 12
    mockup_path: Path = Path(__file__).resolve().parents[1] / "assets/desktop-mockup-small.png"
    mockup_mode: Literal["auto", "fit", "crop", "native"] = "native"