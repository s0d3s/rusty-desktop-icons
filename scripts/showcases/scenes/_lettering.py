"""Sample smooth centerlines from real font glyphs, without a placement grid."""

from __future__ import annotations

import os
from pathlib import Path

import numpy as np
from numpy.typing import NDArray
from PIL import Image, ImageDraw, ImageFont
from scipy.ndimage import gaussian_filter1d
from skimage.morphology import skeletonize


def stroke_paths(mask: Image.Image) -> list[NDArray[np.float64]]:
    reduced = mask.resize((mask.width // 2, mask.height // 2), Image.Resampling.LANCZOS)
    skeleton = skeletonize(np.asarray(reduced) >= 128)
    pixels = {(int(column), int(row)) for row, column in np.argwhere(skeleton)}
    neighbors: dict[tuple[int, int], list[tuple[int, int]]] = {}
    for column, row in sorted(pixels):
        adjacent: list[tuple[int, int]] = []
        for delta_column in (-1, 0, 1):
            for delta_row in (-1, 0, 1):
                candidate = (column + delta_column, row + delta_row)
                if candidate == (column, row) or candidate not in pixels:
                    continue
                if delta_column and delta_row and (
                    (column + delta_column, row) in pixels or (column, row + delta_row) in pixels
                ):
                    continue
                adjacent.append(candidate)
        neighbors[column, row] = adjacent
    visited: set[frozenset[tuple[int, int]]] = set()
    paths: list[NDArray[np.float64]] = []
    for start in sorted(pixels, key=lambda point: (len(neighbors[point]) == 2, point)):
        for following in neighbors[start]:
            edge = frozenset((start, following))
            if edge in visited:
                continue
            visited.add(edge)
            chain = [start, following]
            previous, current = start, following
            while len(neighbors[current]) == 2 and current != start:
                following = next(point for point in neighbors[current] if point != previous)
                edge = frozenset((current, following))
                if edge in visited:
                    break
                visited.add(edge)
                chain.append(following)
                previous, current = current, following
            if len(chain) < 3:
                continue
            coordinates = np.asarray(chain, dtype=float) * 2 + 0.5
            closed = chain[0] == chain[-1]
            if closed:
                coordinates = coordinates[:-1]
            smoothed = gaussian_filter1d(coordinates, sigma=2, axis=0, mode="wrap" if closed else "nearest")
            if closed:
                smoothed = np.vstack((smoothed, smoothed[0]))
            else:
                smoothed[0], smoothed[-1] = coordinates[0], coordinates[-1]
            if np.linalg.norm(np.diff(smoothed, axis=0), axis=1).sum() >= 4:
                paths.append(smoothed)
    return paths


def lettering_centers(width: int, height: int, icon_size: int, count: int) -> list[tuple[float, float]]:
    if count < 96:
        raise ValueError("project_intro needs at least 96 icons for readable three-line lettering")
    font_path = Path(os.environ.get("WINDIR", "C:/Windows")) / "Fonts/segoeui.ttf"
    words = ("Rusty", "Desktop", "Icons")
    font_size = round(height * 0.28)
    while font_size >= 16:
        font = ImageFont.truetype(str(font_path), font_size)
        boxes = [font.getbbox(word) for word in words]
        gap = round(font_size * 0.22)
        text_height = sum(box[3] - box[1] for box in boxes) + 2 * gap
        if max(box[2] - box[0] for box in boxes) <= width * 0.8 and text_height <= height * 0.78:
            break
        font_size -= 2
    else:
        raise ValueError("project_intro canvas is too small for lettering")
    mask = Image.new("L", (width, height))
    drawing = ImageDraw.Draw(mask)
    top = (height - text_height) / 2
    for word, box in zip(words, boxes, strict=True):
        drawing.text(((width - box[2] + box[0]) / 2 - box[0], top - box[1]), word, font=font, fill=255)
        top += box[3] - box[1] + gap
    paths = stroke_paths(mask)
    if not paths or count < len(paths):
        raise ValueError("Not enough icons to cover every lettering stroke")
    lengths = [np.r_[0.0, np.cumsum(np.linalg.norm(np.diff(path, axis=0), axis=1))] for path in paths]
    allocations = [1] * len(paths)
    for _ in range(count - len(paths)):
        selected = max(range(len(paths)), key=lambda index: lengths[index][-1] / allocations[index])
        allocations[selected] += 1
    coverage_scale = min(1.0, count * icon_size * 0.70 / sum(length[-1] for length in lengths))
    centers: list[tuple[float, float]] = []
    for path, length, amount in zip(paths, lengths, allocations, strict=True):
        samples = np.linspace(0, length[-1], amount) if amount > 1 else [length[-1] / 2]
        for distance in samples:
            horizontal = float(np.interp(distance, length, path[:, 0]))
            vertical = float(np.interp(distance, length, path[:, 1]))
            centers.append((width / 2 + (horizontal - width / 2) * coverage_scale,
                            height / 2 + (vertical - height / 2) * coverage_scale))
    return centers