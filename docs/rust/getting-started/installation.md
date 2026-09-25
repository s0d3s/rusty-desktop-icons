# Install for Rust

Use **Rust 1.88+**, Windows 10/11 x64, Visual Studio C++ Build Tools and the
Windows SDK. Desktop operations need an interactive session with Explorer.
No Python installation or wheel is needed.

Create an application and add the library crates from crates.io:

```powershell
cargo new desktop-layout
cd desktop-layout
cargo add rdi-core
cargo add rdi-platform-windows
```

[`rdi-core`](https://crates.io/crates/rdi-core) contains the controller and
platform-independent animation types.
[`rdi-platform-windows`](https://crates.io/crates/rdi-platform-windows) provides
the Windows backend and HLSL compiler. Use compatible versions of the two crates
and commit your application's Cargo lockfile. This proof of concept does not
promise a stable API between releases.

For a cross-platform application, make the Windows dependency target-specific
and gate its use with `cfg(windows)`. The core can be used independently for
curve evaluation/testing; a non-Windows stub is not a working desktop backend.

## Install the Interactive Demo

The `rusty-desktop-icons` package contains demo binaries, not a facade library:

```powershell
cargo install rusty-desktop-icons --bin animate_timeline
animate_timeline --dry-run
```

The dry run only reports a plan. Running without it opens a live desktop
animation; review [Demo App: Timeline Editor](../../demo-app.md) first.

Continue with the [read-only quickstart](quickstart.md). Library development and
documentation builds are covered in [contributing](../../contributing.md).