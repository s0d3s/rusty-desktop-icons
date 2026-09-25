from dataclasses import dataclass
from pathlib import Path
from typing import Literal

import rusty_desktop_icons as rdi

from .base import Showcase


@dataclass(frozen=True)
class ParticleVortexShowcase(Showcase):
    name: str = "particle_vortex"
    shader: rdi.BuiltinShader | str | None = rdi.BuiltinShader.ParticleVortex
    duration: float = 4.0


@dataclass(frozen=True)
class SmallParticleVortexShowcase(ParticleVortexShowcase):
    name: str = "particle_vortex_small"
    output_grid: tuple[int, int] | None = (5, 3)
    max_icons: int | None = 6
    margin_dip: int = 12
    mockup_path: Path = Path(__file__).resolve().parents[1] / "assets/desktop-mockup-small.png"
    mockup_mode: Literal["auto", "fit", "crop", "native"] = "native"