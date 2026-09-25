"""Assemble Sphinx and complete rustdoc outputs and check repository-subpath links."""

from __future__ import annotations

import argparse
from html.parser import HTMLParser
from pathlib import Path
import shutil
from urllib.parse import unquote, urljoin, urlsplit


class Page(HTMLParser):
    def __init__(self, source: str) -> None:
        super().__init__(convert_charrefs=True)
        self.links: list[str] = []
        self.anchors: set[str] = set()
        self.repairs: list[tuple[str, str]] = []
        self.signatures: dict[str, str] = {}
        self.empty_implementors = False
        self._implementors = False
        self._signature: str | None = None
        self.feed(source)

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        values = dict(attrs)
        if self._implementors:
            self.empty_implementors = False
        if tag == "div" and values.get("id") == "implementors-list":
            self._implementors = True
            self.empty_implementors = True
        if tag == "dt" and (values.get("id") or "").startswith("rusty_desktop_icons."):
            self._signature = values["id"]
            assert self._signature is not None
            self.signatures[self._signature] = ""
        inherited_link = "dispatcher#setting-the-default-subscriber"
        if tag == "a" and values.get("href") == inherited_link:
            original = self.get_starttag_text()
            assert original is not None
            self.repairs.append((original, original.replace(
                inherited_link,
                "https://docs.rs/tracing/latest/tracing/dispatcher/index.html#setting-the-default-subscriber",
            )))
        for attribute in ("href", "src"):
            if values.get(attribute):
                self.links.append(values[attribute] or "")
        for attribute in ("id", "name"):
            if values.get(attribute):
                self.anchors.add(unquote(values[attribute] or ""))

    def handle_endtag(self, tag: str) -> None:
        if tag == "div":
            self._implementors = False
        if tag == "dt":
            self._signature = None

    def handle_data(self, data: str) -> None:
        if self._implementors and data.strip():
            self.empty_implementors = False
        if self._signature is not None:
            self.signatures[self._signature] += data


def complete_rustdoc_assets(rustdoc: Path) -> None:
    trait_page = Path("rdi_core/scene/trait.SceneRenderer.html")
    asset = rustdoc / "trait.impl" / trait_page.with_suffix(".js")
    page_path = rustdoc / trait_page
    if asset.exists() or not page_path.is_file():
        return
    page = Page(page_path.read_text(encoding="utf-8"))
    if "../../trait.impl/rdi_core/scene/trait.SceneRenderer.js" not in page.links:
        return
    if not page.empty_implementors:
        raise SystemExit("Missing SceneRenderer asset with a nonempty or unknown implementor table")
    for path in rustdoc.rglob("*.html"):
        anchors = Page(path.read_text(encoding="utf-8")).anchors
        if any(anchor.startswith(("impl-SceneRenderer-for-", "impl-rdi_core::SceneRenderer-for-"))
               for anchor in anchors):
            raise SystemExit("Missing SceneRenderer asset with documented implementations")
    asset.parent.mkdir(parents=True, exist_ok=True)
    asset.write_text(
        "(function() {\n"
        '    const implementors = {"rdi_core": []};\n'
        "    if (window.register_implementors) {\n"
        "        window.register_implementors(implementors);\n"
        "    } else {\n"
        "        window.pending_implementors = implementors;\n"
        "    }\n"
        "})();\n",
        encoding="utf-8", newline="\n",
    )


def check_site(site: Path, base_path: str) -> None:
    base = "https://pages.invalid/" + base_path.strip("/") + "/"
    base_prefix = urlsplit(base).path
    pages = {path.relative_to(site).as_posix(): Page(path.read_text(encoding="utf-8"))
             for path in site.rglob("*.html")}
    failures: list[str] = []
    count = 0
    for relative, page in pages.items():
        page_url = urljoin(base, relative.removesuffix("index.html"))
        for link in page.links:
            resolved = urlsplit(urljoin(page_url, link))
            if resolved.scheme not in {"http", "https"} or resolved.netloc != "pages.invalid":
                continue
            count += 1
            path = unquote(resolved.path)
            if not path.startswith(base_prefix):
                failures.append(f"{relative}: escapes repository subpath: {link}")
                continue
            target_name = path[len(base_prefix):]
            target = site / target_name
            if target.is_dir():
                target_name = target_name.rstrip("/") + "/index.html" if target_name else "index.html"
                target = site / target_name
            if not target.is_file():
                failures.append(f"{relative}: missing file: {link}")
            elif resolved.fragment and target_name in pages:
                fragment = unquote(resolved.fragment)
                anchors = pages[target_name].anchors
                endpoints = fragment.split("-")
                source_range = target_name.startswith("rust/src/") and len(endpoints) == 2 and all(
                    part.isdecimal() and part in anchors for part in endpoints
                )
                if fragment not in anchors and not source_range:
                    failures.append(f"{relative}: missing anchor: {link}")
    for required in (
        "index.html", "python/api/rusty_desktop_icons/DesktopController/index.html",
        "python/api/rusty_desktop_icons/Curve/index.html",
        "python/api/rusty_desktop_icons/TimelineSession/index.html",
        "rust/rdi_core/index.html", "rust/rdi_platform_windows/struct.WindowsBackend.html",
        "rust/rdi_platform_windows/shader/index.html",
    ):
        if required not in pages:
            failures.append(f"Required reference page missing: {required}")
    signature_checks = {
        "DesktopController.animate": (
            "specs:Iterable[IconAnimationSpec|dict[str,object]]",
            "options:AnimationOptions|dict[str,object]|None=None", "->AnimationHandle",
        ),
        "Curve.from_motion_function": (
            "f:Callable[[MotionContext,float],float]", "origin:Sequence[int]",
            "target:Sequence[int]", "duration_seconds:float", "params:dict[str,float]|None=None",
            "samples:int=64", "interp:KeyframeInterp|str=linear", "->Curve",
        ),
        "TimelineSession.play_to": ("position:float", "speed:float=1.0", "->PlaybackHandle"),
    }
    for member, expected in signature_checks.items():
        class_name = member.split(".")[0]
        page_name = f"python/api/rusty_desktop_icons/{class_name}/index.html"
        if page_name not in pages:
            continue
        signature = pages[page_name].signatures.get(f"rusty_desktop_icons.{member}", "")
        compact = "".join(signature.split()).replace("\u2192", "->").replace("'", "").replace('"', "")
        if any(fragment not in compact for fragment in expected) or "(/," in compact:
            failures.append(f"Incorrect rendered signature for {member}: {signature}")
    if failures:
        raise SystemExit("\n".join(sorted(set(failures))))
    print(f"Checked {len(pages)} pages and {count} local links under {base_prefix}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sphinx", type=Path, default=Path("docs/_build/html"))
    parser.add_argument("--rustdoc", type=Path, default=Path("target/docs-rust/doc"))
    parser.add_argument("--out", type=Path, default=Path("site"))
    parser.add_argument("--base-path", default="/rusty-desktop-icons/")
    parser.add_argument("--check-only", action="store_true")
    args = parser.parse_args()
    if not args.check_only:
        if args.out.exists():
            raise SystemExit(f"Output already exists: {args.out}; use a fresh directory or --check-only")
        shutil.copytree(args.sphinx, args.out)
        shutil.copytree(args.rustdoc, args.out / "rust", dirs_exist_ok=True)
        complete_rustdoc_assets(args.out / "rust")
        for path in (args.out / "rust").rglob("*.html"):
            source = path.read_text(encoding="utf-8")
            page = Page(source)
            if page.repairs:
                for original, replacement in page.repairs:
                    source = source.replace(original, replacement)
                path.write_text(source, encoding="utf-8", newline="\n")
        (args.out / ".nojekyll").touch()
    check_site(args.out, args.base_path)


if __name__ == "__main__":
    main()