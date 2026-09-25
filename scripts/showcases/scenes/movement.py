from dataclasses import dataclass
from pathlib import Path
from typing import Literal

from .base import Showcase


@dataclass(frozen=True)
class MovementShowcase(Showcase):
    name: str = "movement"


@dataclass(frozen=True)
class SmallMovementShowcase(MovementShowcase):
    name: str = "movement_small"
    output_grid: tuple[int, int] | None = (5, 3)
    max_icons: int | None = 6
    margin_dip: int = 12
    mockup_path: Path = Path(__file__).resolve().parents[1] / "assets/desktop-mockup-small.png"
    mockup_mode: Literal["auto", "fit", "crop", "native"] = "native"