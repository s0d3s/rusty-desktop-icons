"""Verify that a wheel ships the checked Python interface and native extension."""

from __future__ import annotations

import argparse
from pathlib import Path, PurePosixPath
import zipfile


def check_wheel(wheel_dir: Path, package: Path) -> None:
    wheels = list(wheel_dir.glob("*.whl"))
    if len(wheels) != 1:
        raise SystemExit(f"Expected exactly one wheel in {wheel_dir}, found {len(wheels)}")
    expected = {path.name for pattern in ("*.py", "*.pyi", "py.typed")
                for path in package.glob(pattern)}
    with zipfile.ZipFile(wheels[0]) as wheel:
        names = wheel.namelist()
        if len(names) != len(set(names)):
            raise SystemExit("Wheel contains duplicate entries")
        extensions: list[str] = []
        for name in names:
            path = PurePosixPath(name)
            if path.is_absolute() or ".." in path.parts or "\\" in name:
                raise SystemExit(f"Wheel contains unsafe path: {name}")
            if path.suffix.lower() in {".pdb", ".ilk", ".exp", ".lib"}:
                raise SystemExit(f"Wheel contains debug/build artifact: {name}")
            if name.endswith("/"):
                continue
            if len(path.parts) >= 2 and path.parts[0].startswith("rusty_desktop_icons-") and path.parts[0].endswith(".dist-info"):
                continue
            if len(path.parts) != 2 or path.parts[0] != "rusty_desktop_icons":
                raise SystemExit(f"Wheel contains unexpected file: {name}")
            if path.name.startswith("_rusty_desktop_icons.") and path.suffix in {".pyd", ".so"}:
                extensions.append(name)
            elif path.name not in expected:
                raise SystemExit(f"Wheel contains unexpected package file: {name}")
        for name in sorted(expected):
            member = f"rusty_desktop_icons/{name}"
            if member not in names:
                raise SystemExit(f"Wheel missing {member}")
            if wheel.read(member) != (package / name).read_bytes():
                raise SystemExit(f"Wheel contains stale or incomplete {member}")
        if len(extensions) != 1:
            raise SystemExit(f"Expected one native extension, found {len(extensions)}")
    print(f"Verified Python sources, generated stubs, py.typed and native extension in {wheels[0].name}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("wheel_dir", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    check_wheel(args.wheel_dir, root / "crates/rdi-python/python/rusty_desktop_icons")


if __name__ == "__main__":
    main()