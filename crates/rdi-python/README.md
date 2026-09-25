# rusty-desktop-icons (Python)

Rust-powered controller and animator for Windows desktop icons. Built with
[PyO3](https://pyo3.rs) 0.29 and [maturin](https://maturin.rs) 1.14+.

## Install (from PyPI)

```powershell
pip install rusty-desktop-icons
```

## Install (from source)

```powershell
python -m venv .venv
.venv\Scripts\Activate.ps1
pip install maturin
maturin develop --release -m crates/rdi-python/Cargo.toml
```

Or run the one-shot helper from the repo root:

```powershell
.\scripts\build-wheel.ps1
```

## Quick example

```python
import rusty_desktop_icons as rdi
from rusty_desktop_icons import Curve, Duration

ctrl = rdi.DesktopController()
icons = ctrl.list_icons()
specs = [
    {
        "id": icon.id,
        "target": (100, 100 + i * 120),
        "duration": Duration.fixed(seconds=0.6),
        "curve": Curve.ease_in_out(),
    }
    for i, icon in enumerate(icons[:5])
]
handle = ctrl.animate(specs)
handle.wait()
```

## Examples in this crate

- [`examples/smoke.py`](examples/smoke.py) — end-to-end sanity check.
- [`examples/curve_from_function.py`](examples/curve_from_function.py) —
  demonstrates `Curve.from_function` with a damped-cosine curve.
- [`examples/curve_from_motion.py`](examples/curve_from_motion.py) —
  demonstrates `Curve.from_motion_function` with a parametric gravity
  curve.

## Tests

```powershell
.venv\Scripts\python.exe -m pytest crates\rdi-python\tests -v
```

## License

Licensed under the [Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0).
The license text is included in the distribution.
