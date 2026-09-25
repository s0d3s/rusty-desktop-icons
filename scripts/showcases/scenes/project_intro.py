"""Scattered icons gather into project lettering, then sway along its strokes."""

from __future__ import annotations

from dataclasses import dataclass
import math
import random

import rusty_desktop_icons as rdi

from .base import Point, Showcase
from ._lettering import lettering_centers


@dataclass(frozen=True)
class ProjectIntroShowcase(Showcase):
    name: str = "project_intro"
    shader: rdi.BuiltinShader | str | None = rdi.BuiltinShader.Glitch
    duration: float = 14.0
    gather_seconds: float = 4.0
    dance_seconds: float = 10.0
    dance_amplitude_dip: float = 6.0
    allow_overlap: bool = True
    max_icons: int | None = None

    def __post_init__(self) -> None:
        super().__post_init__()
        if (not math.isfinite(self.gather_seconds) or not math.isfinite(self.dance_seconds)
            or self.gather_seconds <= 0 or self.dance_seconds <= 0
                or not math.isclose(self.duration, self.gather_seconds + self.dance_seconds)
                or not math.isfinite(self.dance_amplitude_dip) or self.dance_amplitude_dip < 0):
            raise ValueError("project_intro requires positive phase durations summing to duration and nonnegative dance amplitude")

    def destinations(self, icons: list[rdi.IconSnapshot], positions: list[Point]) -> list[Point]:

        size = round(self.icon_size * self.dpi_scale)
        return [(round(horizontal - size / 2), round(vertical - size / 2))
                for horizontal, vertical in lettering_centers(self.width, self.height, size, len(icons))]

    def layout(self, icons: list[rdi.IconSnapshot]) -> list[Point]:
        targets = self.destinations(icons, [])
        generator = random.Random(self.seed)
        size = round(self.icon_size * self.dpi_scale)
        margin = size
        if self.width < 6 * size or self.height < 6 * size:
            raise ValueError("project_intro canvas must be at least six icon widths/heights")
        positions: list[Point] = []
        for target in targets:
            coordinates: list[int] = []
            for axis, extent in enumerate((self.width, self.height)):
                allowed = [value for value in range(margin, extent - 2 * size + 1) if abs(value - target[axis]) >= size]
                coordinates.append(generator.choice(allowed))
            positions.append((coordinates[0], coordinates[1]))
        return positions

    def dance_offset(self, target: Point, index: int, seconds: float) -> tuple[float, float]:
        if seconds <= 0 or seconds >= self.dance_seconds:
            return 0.0, 0.0
        generator = random.Random(self.seed + index)
        phase = generator.uniform(0, math.tau)
        frequency = generator.uniform(0.65, 0.95)
        fade = min(1.0, seconds, self.dance_seconds - seconds)
        fade = fade * fade * (3 - 2 * fade)
        amplitude = self.dance_amplitude_dip * self.dpi_scale * fade
        wave = math.tau * seconds / 2.8 - target[0] / (180 * self.dpi_scale)
        return (amplitude * (0.65 * math.sin(wave) + 0.35 * math.sin(seconds * frequency + phase)),
                amplitude * (0.65 * math.cos(wave) + 0.35 * math.cos(seconds * frequency * 1.3 + phase)))

    def motion_curve(self, origin: Point, target: Point, index: int, axis: int) -> rdi.Curve:
        delta = target[axis] - origin[axis]
        if delta == 0:
            raise ValueError("project_intro scatter must differ from lettering on both axes")
        def sample(progress: float) -> float:
            seconds = progress * self.duration
            if seconds <= self.gather_seconds:
                fraction = seconds / self.gather_seconds
                return fraction * fraction * (3 - 2 * fraction)
            return 1 + self.dance_offset(target, index, seconds - self.gather_seconds)[axis] / delta
        return rdi.Curve.from_function(sample, samples=math.ceil(self.duration * max(60, self.fps)) + 1)

    def glitch_envelope(self) -> rdi.Curve:
        boundary = self.gather_seconds / self.duration
        return rdi.Curve.keyframes([(0.0, 0.0), (boundary * 0.2, 1.0),
                                   (boundary * 0.7, 1.0), (boundary, 0.0), (1.0, 0.0)])

    def animations(self, icons: list[rdi.IconSnapshot], positions: list[Point], targets: list[Point]) -> list[rdi.IconAnimationSpec]:
        shader = rdi.Shader.compile(rdi.ShaderSource.builtin(self.shader))
        envelope = self.glitch_envelope()
        return [rdi.IconAnimationSpec(
            icon.id, target, rdi.Duration.fixed(self.duration),
            self.motion_curve(origin, target, index, 0), curve_y=self.motion_curve(origin, target, index, 1),
            effect=rdi.Effect(shader, seed=float(index), envelope=envelope, padding_px=24),
        ) for index, (icon, origin, target) in enumerate(zip(icons, positions, targets, strict=True))]

    def render_options(self) -> rdi.AnimationOptions:
        return rdi.AnimationOptions(draw_labels=False, draw_shortcut_overlay=False, draw_shield_overlay=False)
    