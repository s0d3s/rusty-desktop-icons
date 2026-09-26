# Contributing Documentation and Types

Application users should begin with [Python installation](python/getting-started/installation.md)
or [Rust installation](rust/getting-started/installation.md). This page is the
checkout-based workflow for contributors, not a prerequisite for using either
package.

## Choose the Source

| Change | Edit |
| --- | --- |
| Shared entity behavior/limits | `docs/concepts/`, with links to both language recipes |
| Application workflow | `docs/python/` or `docs/rust/` guides/examples, linking back to concepts |
| Python signature or API description | Rust/PyO3 binding metadata, then regenerate stubs |
| Rust API description | Rust doc comments, verified by native Windows rustdoc |

Keep package installation registry-first. Examples should be complete or clearly
define reusable helpers, identify live desktop writes, and report completion
instead of treating startup success as proof. Validate Rust snippets by compiling
them and Python recipes against actual constructors; do not run a live desktop
example merely to check its syntax. Off-screen examples can be checked with
pixel assertions and before/after desktop snapshots.

The Rust/PyO3 bindings own Python signatures and API docstrings. Edit the Rust
implementation and its `#[pyo3(signature = ...)]` type metadata, not generated
`.pyi` files. Guides use MyST Markdown; API docstrings use reStructuredText.
The independent MCP application is outside this documentation pipeline.

## Root uv Tooling

The root `pyproject.toml` is a non-packaged tooling project for Python 3.12+.
It does not change the library's supported Python versions or Maturin backend.
The committed `uv.lock` resolves the pinned dependencies. The old requirements
files contain installation pointers only; `pip install -r` on them installs
nothing. Edit dependencies in the root manifest and update the lock with `uv lock`.

Use a separate environment so an exact sync cannot remove a native wheel from
the existing development or wheel-test environments. Set the environment
variable in each new PowerShell session before running root uv commands:

```powershell
$env:UV_PROJECT_ENVIRONMENT = Join-Path (Get-Location) '.tools-venv'
uv sync --locked --python 3.12
uv run --locked python -m sphinx -E -W -n -b dirhtml docs docs/_build/html
uv run --locked python scripts/showcases/assemble-showcase.py --help
```

Run these commands from the repository root. By default, uv installs both
`docs` and `scripts`. The `docs` group includes shared `dev` tools (Maturin,
mypy and pytest); `scripts` contains the showcase image/video dependencies.
For a smaller environment, use `uv sync --locked --only-group docs` or
`uv sync --locked --only-group scripts`, and pass the same `--only-group`
selection to `uv run` (otherwise its default groups will be installed again).

Static documentation and image processing do not need the native library.
`render-showcases.py` does: sync tooling first, then explicitly install the
freshly built compatible wheel using `uv pip install --python
.tools-venv/Scripts/python.exe <wheel-path>`. Use `uv run --no-sync` afterwards
to retain that installation; a subsequent exact `uv sync` removes the undeclared
wheel. Native Rust changes still require rebuilding the wheel. MCP and legacy
packages are not members of this tooling project.

## Generate Python Types

Use Python 3.12 or newer. For the locked environment, follow the root uv setup
above and activate `.tools-venv` before using the plain `python` commands below.
Alternatively, with uv installed, install the declared docs group into the
Python environment already on PATH (this resolves directly, without the lock):

```console
uv pip install --python python --group docs
python scripts/generate-stubs.py
python scripts/generate-stubs.py --check
```

Commit all generated files under
`crates/rdi-python/python/rusty_desktop_icons/`. Maturin 1.15.0 reads PyO3
0.29.2's experimental inspection metadata. Generation compiles the native
extension for the current host; it does not use a previously installed wheel.
Linux uses the stub backend. CI regenerates on both Linux and Windows.
Windows-only folder flags are declared once by `folder_flags!` in
`crates/rdi-platform-windows/src/folder_flags.rs`. Windows generation builds
`_flags.pyi` from that native catalog and the Python metadata property signatures.
Linux retains the checked-in Windows-only declarations; only a Windows build
can validate or regenerate them. The public `FolderFlag` export is conditional.

String-backed enums live in `_enums.py`; the generator derives `_enums.pyi`
from those declarations. Windows generation also produces `_builtin_shaders.py`
and its stub from `BuiltinShader::ALL`; non-Windows builds retain that checked-in
catalog. Do not maintain a second Python list of built-in shaders. Enum-valued
native getters supply PyO3 `IntoPyObject::OUTPUT_TYPE` metadata, while input
signatures explicitly accept enum members or legacy strings.

PyO3 currently omits `create_exception!` classes. The generator completes only
exception declarations from the freshly built extension's names, bases and
docstrings, and derives wrapper re-exports from its Python AST. It rejects any
other missing public export instead of inferring callable signatures. This
native import happens during generation only, never during Sphinx builds.

Release wheels consume the checked-in generated files. Do not pass raw
`--generate-stubs` to a wheel build: that bypasses exception completion.

```powershell
$env:MYPYPATH = 'crates/rdi-python/python'
python -m mypy --strict crates/rdi-python/tests/typing/api.py
python -m mypy --strict scripts/generate-stubs.py scripts/assemble-docs.py scripts/check-wheel.py
python scripts/generate-stubs.py --check
python -m maturin build --release -m crates/rdi-python/Cargo.toml --out target/docs-wheels -i python
python scripts/check-wheel.py target/docs-wheels
python -m unittest discover -s scripts -p test_publication.py -v
```

Use a fresh wheel output directory, or one containing only the wheel being
validated. The validator compares Python sources and generated typing against
the checkout and rejects extra package files and native debug/build artifacts.
Debug symbols are excluded from both wheels and source distributions; Git ignore
rules alone do not control archive contents.
Install that wheel and run `python -m pytest crates/rdi-python/tests -v`
to check the packaged public exports, methods, properties and annotations.

## Documentation Background

Furo loads `docs/_static/particles.css` and `docs/_static/particles.js` through
`docs/conf.py`. With a docked right-hand contents panel, the canvas spans from
the article's left edge to the main area's right edge. Squares fade smoothly
to half opacity across the first 48px beyond the article. The docked contents
panel is transparent, with its text and controls above the animation. The
left navigation, footer and separate rustdoc reference remain unchanged.
On narrow screens the canvas stays inside the article and the contents drawer
remains opaque. Its viewport-sized buffer follows scrolling and stops at the
article's bottom edge without allocating an image as tall as the document.
Only expanded layouts isolate the main stacking context; article-only layouts
isolate the article so mobile drawers stay above their click-to-close overlays.

The script controls square tile density, size, drift, spin, cursor radius and
impulse strength. CSS controls opacity; the neutral gray fill follows Furo's
light/dark selection. A Motion checkbox remembers the reader's preference.
Reduced-motion preferences hide the effect, and hidden/offscreen pages stop
simulating. Touch scrolling does not trigger cursor repulsion.

Squares are 4-9 CSS pixels with 1px rounded corners. Density is one square per
7,800 square CSS pixels, clamped to 8-125 in the expanded wide-screen field
and 8-95 otherwise: 125 in a 1197x900 expanded viewport, 85 in a 736x900
article-only viewport, and 37 in a 343x844 mobile viewport. Formations use
the full available field, with no collision boundary at the article edge.

After each random 15-25 second drift interval, the simulation selects a desktop
grid, geometric shape, local clusters or traveling wave. Geometric shapes
randomly select a ring, square outline or static wave. Local clusters recruit
one third of the squares into two small grids; other patterns select enough
members to preserve target spacing. Remaining squares continue drifting.
Velocity steering gathers members without teleporting or disabling collisions.
Gathering time scales with travel distance, followed by a random 2-3 second
hold and 2.5 second release. The traveling wave advances during its hold.
Cursor proximity releases affected members for the rest of that cycle.
Formation timers advance only with simulation steps, so pause, reduced motion
and hidden/offscreen suspension do not skip ahead. Resizing starts a fresh
drift interval and rebuilds targets for the available space.

Matter.js 0.20.0 is vendored unmodified under `docs/_static/vendor/`, with its
MIT license. The adapter lowers its internal resting-contact threshold so
slow tiles bounce instead of settling; verify that behavior on upgrades.
Run the deterministic physics checks with Node.js 22 or newer:

```console
node --test scripts/test-docs-particles.cjs
```

After a strict Sphinx build, verify desktop/mobile scrolling, light/dark,
cursor avoidance, the Motion checkbox and reduced-motion settings in a browser.
Verify each formation gathers, holds and releases without overriding cursor
avoidance. Check the half-opacity right-side region and mobile contents drawer
hit targets after resizing across the docked-panel breakpoint. The animation
must remain behind selectable text and outside the footer.

## Publish a Release

Publishing a GitHub Release triggers `.github/workflows/release.yml`. A tag push
or draft release alone does not publish packages. Use a tag such as `v0.2.22`
whose version exactly matches the Cargo workspace and native Python project.
Commit the workflows before creating the release tag.

Before each release, update `workspace.package.version`, the three local
`workspace.dependencies` version requirements in `Cargo.toml`, and
`project.version` in `crates/rdi-python/pyproject.toml`. Refresh `Cargo.lock`
and keep the root tooling version/lock metadata aligned. Commit these changes,
tag that commit as `vX.Y.Z`, and publish its GitHub Release. Version mismatches
stop the workflow before any upload. Prerelease flags do not select TestPyPI:
every published release that passes validation targets the production registries.


## Build the Site

Sphinx parses only `.pyi` files through AutoAPI, without importing the package.
An AST-normalized documentation copy removes a positional-only separator when
only `self` or `cls` precedes it, working around AutoAPI 3.8.1's annotation shift.
It also exposes Windows-conditional declarations to AutoAPI so the full Windows
API is documented on every documentation build host.
Private string-enum bases are shown as `str` and `enum.Enum` in documentation
copies, and private enum type references resolve to their public exports.
The packaged stubs and public positional-only parameters are unchanged. Assembly
checks representative rendered controller, curve and timeline signatures.
It can run on Linux without Rust or a compiled extension once types are generated:

```console
python -m sphinx -E -W -n -b dirhtml docs docs/_build/html
```

Build the native Rust reference on Windows, where Windows-only APIs are visible:

```powershell
$env:RUSTDOCFLAGS = '-D warnings'
cargo doc --no-deps -p rdi-core -p rdi-platform-windows --target-dir target/docs-rust
python scripts/assemble-docs.py
python -m http.server 8000 --bind 127.0.0.1 --directory site
```

Open `http://127.0.0.1:8000/`. Assembly requires a fresh output directory; pass
`--out another-directory` for subsequent builds. `--check-only` validates an
existing output. The assembler copies all rustdoc assets beneath `rust/`, adds
`.nojekyll`, and checks files, anchors and asset links as served beneath
`/rusty-desktop-icons/`. It understands rustdoc's encoded IDs and source-line
ranges. A narrowly scoped repair redirects the broken relative dispatcher link
in inherited `tracing` trait documentation to its upstream reference.
Rustdoc 1.96 can omit the implementor script for `SceneRenderer`, whose only
production implementation is crate-private. Assembly generates the empty
registration asset only when the rendered table is empty and no public
implementation is documented. Existing assets are never overwritten; other
missing assets still fail validation. `--check-only` never performs repairs.

Sphinx warnings and unresolved references fail the build. External link checking
is a separate scheduled job so network failures do not block PR validation:

```console
python -m sphinx -W -n -b linkcheck docs docs/_build/linkcheck
```
