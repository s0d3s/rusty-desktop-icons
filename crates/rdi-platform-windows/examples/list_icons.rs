//! Print every icon on the current user's desktop.
//!
//! Run with:
//!
//! ```powershell
//! cargo run --example list_icons -p rdi-platform-windows
//! ```

use rdi_core::DesktopController;
use rdi_platform_windows::WindowsBackend;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ctrl = DesktopController::new(WindowsBackend::new())?;
    let icons = ctrl.list_icons()?;
    let flags = ctrl.get_flags()?;

    println!("desktop folder flags = 0x{flags:08x}");
    println!("{} icons found:", icons.len());
    println!(
        "{:>4}  {:<40}  {:>7}  {:>7}  {}",
        "#", "display name", "x", "y", "id"
    );
    for (i, icon) in icons.iter().enumerate() {
        let name = if icon.display_name.chars().count() > 38 {
            let mut trimmed: String = icon.display_name.chars().take(37).collect();
            trimmed.push('…');
            trimmed
        } else {
            icon.display_name.clone()
        };
        println!(
            "{:>4}  {:<40}  {:>7}  {:>7}  {}",
            i,
            name,
            icon.position.x,
            icon.position.y,
            icon.id.as_str()
        );
    }
    Ok(())
}
