"""Public library documentation, built without importing the extension."""

import ast
from datetime import datetime, timezone
from pathlib import Path
import tomllib
from typing import TYPE_CHECKING

if TYPE_CHECKING:
	from docutils.nodes import Element
	from sphinx.application import Sphinx
	from sphinx.environment import BuildEnvironment

ROOT = Path(__file__).resolve().parents[1]
GITHUB_ASSETS_URL = "https://raw.githubusercontent.com/s0d3s/rusty-desktop-icons/assets_storage/assets_storage"
project = "Rusty Desktop Icons"
author = "s0d3s"
copyright = f"{datetime.now(timezone.utc).year}, {author}"
release = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]
extensions = ["myst_parser", "autoapi.extension", "sphinx.ext.intersphinx"]
intersphinx_mapping = {"python": ("https://docs.python.org/3", None)}
html_theme = "furo"
html_theme_options = {
	"footer_icons": [{
		"name": "GitHub repository",
		"url": "https://github.com/s0d3s/rusty-desktop-icons",
		"html": "GitHub",
		"class": "",
	}],
}
html_title = f"{project} {release}"
html_favicon = f"{GITHUB_ASSETS_URL}/icon/rusty-desktop-icons.svg"
myst_enable_extensions = ["substitution"]
myst_substitutions = {
	"github_assets_url": GITHUB_ASSETS_URL,
	"timeline_main": f"![animate_timeline Main tab with playback and duration controls]({GITHUB_ASSETS_URL}/animate_timeline_screenshots/animate_timeline_0_main.png)",
	"timeline_curves": f"![animate_timeline Curves tab with keyframe graph and JSON editor]({GITHUB_ASSETS_URL}/animate_timeline_screenshots/animate_timeline_1_curves.png)",
	"timeline_shader_editor": f"![animate_timeline Shader Editor tab with HLSL source]({GITHUB_ASSETS_URL}/animate_timeline_screenshots/animate_timeline_2_shader_editor.png)",
}
source_suffix = {".md": "markdown", ".rst": "restructuredtext"}
exclude_patterns = ["_build", "_generated", "[A-Z]*.md"]
STUBS = ROOT / "docs/_generated/api_stubs/rusty_desktop_icons"
STUBS.mkdir(parents=True, exist_ok=True)
for stub in (ROOT / "crates/rdi-python/python/rusty_desktop_icons").glob("*.pyi"):
	tree = ast.parse(stub.read_text(encoding="utf-8"))
	tree.body = [
		member
		for node in tree.body
		for member in (
			node.body if isinstance(node, ast.If) and ast.unparse(node.test) == "sys.platform == 'win32'"
			else [node]
		)
	]
	for node in ast.walk(tree):
		if isinstance(node, ast.ClassDef) and any(
				isinstance(base, ast.Name) and base.id == "_StringEnum" for base in node.bases):
			node.bases = [ast.Name(id="str"), ast.Attribute(value=ast.Name(id="enum"), attr="Enum")]
		if (isinstance(node, ast.FunctionDef) and len(node.args.posonlyargs) == 1
				and node.args.posonlyargs[0].arg in {"self", "cls"}):
			node.args.args = node.args.posonlyargs + node.args.args
			node.args.posonlyargs = []
	(STUBS / stub.name).write_text("import enum\n" + ast.unparse(tree) + "\n", encoding="utf-8")
autoapi_dirs = [str(STUBS)]
autoapi_file_patterns = ["*.pyi"]
autoapi_root = "python/api"
autoapi_add_toctree_entry = False
autoapi_options = ["members", "undoc-members", "show-inheritance", "imported-members"]
autoapi_member_order = "groupwise"
autoapi_python_class_content = "both"
autoapi_own_page_level = "class"
nitpicky = True
html_copy_source = False
html_show_sourcelink = False
linkcheck_anchors = False
linkcheck_retries = 2
linkcheck_timeout = 20


def resolve_public_type(app: "Sphinx", env: "BuildEnvironment", node: "Element", content: "Element") -> "Element | None":
	target = node.get("reftarget", "")
	if target.startswith("rusty_desktop_icons._enums."):
		target = target.rsplit(".", 1)[-1]
	domain = env.domains["py"]
	qualified = f"rusty_desktop_icons.{target}"
	if node.get("refdomain") == "py" and qualified in domain.objects:
		return domain.resolve_xref(env, node["refdoc"], app.builder, node["reftype"], qualified, node, content)
	return None


def setup(app: "Sphinx") -> None:
	app.connect("missing-reference", resolve_public_type)