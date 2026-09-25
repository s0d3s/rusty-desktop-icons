# Inspect Your Desktop

After [adding the crates](installation.md), put this program in your application's
`src/main.rs` and run `cargo run`. It does not move icons or change flags.

```rust
use rdi_core::DesktopController;
use rdi_platform_windows::{FolderFlag, WindowsBackend};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let controller = DesktopController::new(WindowsBackend::new())?;
    for icon in controller.list_icons()? {
        println!("{}: {:?}, {:?}, {}", icon.display_name, icon.position, icon.path, icon.id);
    }
    let desktop = controller.desktop_info()?;
    println!("Desktop: {desktop:#?}");
    let flags = controller.get_flags()?;
    println!("Snap enabled: {}", flags & FolderFlag::SnapToGrid.bits() != 0);
    Ok(())
}
```

The [desktop concept page](../../concepts/desktop.md) explains the returned
objects and coordinates. Continue with [guides](../guides/index.md) for concrete
recipes or [examples](../examples/index.md) for complete programs.