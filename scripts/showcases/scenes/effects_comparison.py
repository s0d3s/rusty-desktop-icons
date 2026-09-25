"""Four effects move alternating icons into their neighboring empty cells."""

from dataclasses import dataclass

import rusty_desktop_icons as rdi

from .base import Point, Showcase
from .dust_transfer import DustTransferShowcase
from .glitch import GlitchShowcase
from .movement import MovementShowcase
from .particle_vortex import ParticleVortexShowcase


@dataclass(frozen=True)
class EffectsComparisonShowcase(Showcase):
    name: str = "effects_comparison"
    duration: float = 4.0
    output_grid: tuple[int, int] | None = (8, 1)
    max_icons: int | None = 4
    margin_dip: int = 4
    taskbar_height_dip: int = 0
    mockup_background: bool = False
    keep_alpha: bool = True

    def render_seconds(self, seconds: float) -> float:
        travel = min(1.0, max(0.0, (seconds / self.duration - 0.15) / 0.65))
        return self.duration * travel * travel * (3.0 - 2.0 * travel)

    def layout(self, icons: list[rdi.IconSnapshot]) -> list[Point]:
        cell_width, _, _, columns, rows = self.grid_metrics()
        if len(icons) != 4 or (columns, rows) != (8, 1):
            raise ValueError("effects_comparison requires four icons and an eight-column, one-row grid")
        positions = super().layout(icons)
        return [(horizontal + index * cell_width, vertical)
                for index, (horizontal, vertical) in enumerate(positions)]

    def destinations(self, icons: list[rdi.IconSnapshot], positions: list[Point]) -> list[Point]:
        cell_width = self.grid_metrics()[0]
        return [(horizontal + cell_width, vertical) for horizontal, vertical in positions]

    def animations(self, icons: list[rdi.IconSnapshot], positions: list[Point], targets: list[Point]) -> list[rdi.IconAnimationSpec]:
        presets = (MovementShowcase, GlitchShowcase, DustTransferShowcase, ParticleVortexShowcase)
        return [preset(duration=self.duration).animations([icon], [position], [target])[0]
                for preset, icon, position, target in zip(presets, icons, positions, targets, strict=True)]