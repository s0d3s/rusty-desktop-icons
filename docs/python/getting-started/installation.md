# Install for Python

Use **CPython 3.9+ on Windows 10/11 x64**, with Explorer running in an
interactive user session. Install from [PyPI](https://pypi.org/project/rusty-desktop-icons/):

```powershell
python -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install --upgrade pip
python -m pip install rusty-desktop-icons
python -c "import rusty_desktop_icons as rdi; print(rdi.__version__)"
```

Prebuilt wheels do not require Rust or Visual Studio. The package includes type
stubs for editors/type checkers and installs loguru for native diagnostics.
Installing/importing it does not move icons or launch a separate application.

Use `python -m pip install --upgrade rusty-desktop-icons` to upgrade. For an
application release, pin the version you tested in your dependency lockfile.

## When Installation Fails

- Check that Python is 64-bit CPython and that pip belongs to the intended venv.
- A missing compatible wheel may trigger a source build. That requires Rust
	1.88+, Visual Studio C++ Build Tools and the Windows SDK.
- macOS/Linux builds use a stub backend; successful installation does not provide
	desktop control there. Windows services/noninteractive sessions are not the
	target environment.

For work on the library itself, see [contributor setup](../../contributing.md).
For application usage, continue to the [read-only quickstart](quickstart.md).