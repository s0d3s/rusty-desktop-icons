"""Regression checks for release artifact and documentation tooling."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import tempfile
from types import ModuleType
import unittest
import zipfile


def load_script(name: str) -> ModuleType:
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + ".py"))
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class WheelTests(unittest.TestCase):
    def test_wheel_contract(self) -> None:
        validator = load_script("check-wheel")
        package = Path(__file__).resolve().parents[1] / "crates/rdi-python/python/rusty_desktop_icons"
        members = {f"rusty_desktop_icons/{path.name}": path.read_bytes()
                   for pattern in ("*.py", "*.pyi", "py.typed") for path in package.glob(pattern)}
        members["rusty_desktop_icons/_rusty_desktop_icons.cp312-win_amd64.pyd"] = b"native fixture"
        members["rusty_desktop_icons-0.0.0.dist-info/METADATA"] = b"Name: rusty-desktop-icons\n"
        cases = {
            "valid": (None, None),
            "stale": ("rusty_desktop_icons/__init__.pyi", b"stale"),
            "missing": ("rusty_desktop_icons/py.typed", None),
            "native": ("rusty_desktop_icons/_rusty_desktop_icons.cp312-win_amd64.pyd", None),
            "symbols": ("rusty_desktop_icons/native.PDB", b"debug fixture"),
            "unexpected": ("rusty_desktop_icons/scratch.txt", b"scratch fixture"),
            "traversal": ("../outside.txt", b"unsafe fixture"),
            "extra native": ("rusty_desktop_icons/_rusty_desktop_icons.other.pyd", b"native fixture"),
        }
        for case, (member, content) in cases.items():
            with self.subTest(case=case), tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                entries = members.copy()
                if member is not None:
                    if content is None:
                        entries.pop(member)
                    else:
                        entries[member] = content
                with zipfile.ZipFile(directory / "fixture.whl", "w") as wheel:
                    for name, data in entries.items():
                        wheel.writestr(name, data)
                if case == "valid":
                    validator.check_wheel(directory, package)
                else:
                    with self.assertRaises(SystemExit):
                        validator.check_wheel(directory, package)

    def test_requires_one_wheel(self) -> None:
        validator = load_script("check-wheel")
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            with self.assertRaisesRegex(SystemExit, "found 0"):
                validator.check_wheel(directory, directory)
            (directory / "first.whl").touch()
            (directory / "second.whl").touch()
            with self.assertRaisesRegex(SystemExit, "found 2"):
                validator.check_wheel(directory, directory)


class RustdocTests(unittest.TestCase):
    def test_empty_trait_asset(self) -> None:
        assembler = load_script("assemble-docs")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            page = root / "rdi_core/scene/trait.SceneRenderer.html"
            page.parent.mkdir(parents=True)
            page.write_text(
                '<div id="implementors-list"></div>'
                '<script src="../../trait.impl/rdi_core/scene/trait.SceneRenderer.js" async></script>',
                encoding="utf-8",
            )
            assembler.complete_rustdoc_assets(root)
            asset = root / "trait.impl/rdi_core/scene/trait.SceneRenderer.js"
            source = asset.read_text(encoding="utf-8")
            self.assertIn('"rdi_core": []', source)
            self.assertIn("window.register_implementors(implementors)", source)
            self.assertIn("window.pending_implementors = implementors", source)
            asset.write_text("existing implementors", encoding="utf-8")
            assembler.complete_rustdoc_assets(root)
            self.assertEqual(asset.read_text(encoding="utf-8"), "existing implementors")

    def test_missing_asset_with_implementors_is_not_repaired(self) -> None:
        assembler = load_script("assemble-docs")
        for content in ('<div id="implementors-list"><section>impl</section></div>',
                        '<div id="implementors-list">impl</div>', '<div></div>',
                        '<div id="implementors-list"></div><section id="impl-SceneRenderer-for-PublicRenderer"></section>'):
            with self.subTest(content=content), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                page = root / "rdi_core/scene/trait.SceneRenderer.html"
                page.parent.mkdir(parents=True)
                page.write_text(content + '<script src="../../trait.impl/rdi_core/scene/trait.SceneRenderer.js"></script>',
                                encoding="utf-8")
                with self.assertRaises(SystemExit):
                    assembler.complete_rustdoc_assets(root)
                self.assertFalse((root / "trait.impl").exists())

    def test_unrelated_missing_assets_still_fail(self) -> None:
        assembler = load_script("assemble-docs")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "index.html").write_text('<script src="missing.js"></script>', encoding="utf-8")
            assembler.complete_rustdoc_assets(root)
            with self.assertRaisesRegex(SystemExit, "missing file: missing.js"):
                assembler.check_site(root, "/rusty-desktop-icons/")
            self.assertFalse((root / "missing.js").exists())


if __name__ == "__main__":
    unittest.main()