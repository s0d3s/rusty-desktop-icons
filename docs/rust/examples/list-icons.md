# Compare Layout Snapshots

This standalone, read-only program compares two snapshots without treating
display names as IDs. It is a useful starting point for a layout history tool.
Install the [two crates](../getting-started/installation.md), then run it with
`cargo run`. See [desktop management](../../concepts/desktop.md) for ID lifetime
and coordinate semantics.

```rust
use std::collections::HashMap;
use std::io;
use rdi_core::DesktopController;
use rdi_platform_windows::WindowsBackend;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let controller = DesktopController::new(WindowsBackend::new())?;
    let before: HashMap<_, _> = controller.list_icons()?.into_iter()
        .map(|icon| (icon.id, icon.position)).collect();
    println!("Move an icon manually, then press Enter to compare.");
    io::stdin().read_line(&mut String::new())?;
    let after = controller.list_icons()?;
    for icon in &after {
        match before.get(&icon.id) {
            Some(origin) if *origin != icon.position =>
                println!("Moved {}: {origin:?} -> {:?}", icon.display_name, icon.position),
            None => println!("Added: {}", icon.display_name),
            _ => {}
        }
    }
    let missing = before.keys().filter(|identity|
        !after.iter().any(|icon| &icon.id == *identity)).count();
    println!("No longer present: {missing}");
    Ok(())
}
```

To build restoration, retain the `(IconId, Point)` pairs, review them against
the current monitors, then pass selected pairs to `set_positions`. That step is
a real Shell write and can be affected by Auto arrange. For animated restoration,
construct specs from the saved targets and use the
[lifecycle recipe](../guides/animation-lifecycle.md).