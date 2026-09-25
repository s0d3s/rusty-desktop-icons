"""Discover trusted showcase definitions without a manual registry."""

from importlib import import_module, invalidate_caches
import inspect
import pkgutil
import re

from .base import CompressedOutputParams, Showcase


def discover_showcases() -> list[Showcase]:
    invalidate_caches()
    scenes: dict[str, Showcase] = {}
    for module_info in sorted(pkgutil.iter_modules(__path__), key=lambda item: item.name):
        if module_info.ispkg or module_info.name == "base" or module_info.name.startswith("_"):
            continue
        module = import_module(f"{__name__}.{module_info.name}")
        for _, candidate in inspect.getmembers(module, inspect.isclass):
            if (candidate.__module__ != module.__name__ or candidate is Showcase
                    or not issubclass(candidate, Showcase) or inspect.isabstract(candidate)):
                continue
            scene = candidate()
            if not re.fullmatch(r"[a-z0-9]+(?:[-_][a-z0-9]+)*", scene.name):
                raise ValueError(f"{module.__name__}.{candidate.__name__}: name must be a lowercase slug")
            if scene.name in scenes:
                raise ValueError(f"Duplicate showcase name: {scene.name}")
            scenes[scene.name] = scene
    if not scenes:
        raise ValueError("No showcase classes discovered")
    return [scenes[name] for name in sorted(scenes)]