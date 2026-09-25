# Inspect Your Desktop

After [installation](installation.md), run this in an interactive Windows
session. It only reads state; it does not change icon positions or flags.

```python
import rusty_desktop_icons as rdi

controller = rdi.DesktopController()
for icon in controller.list_icons():
    print(icon.id, icon.display_name, icon.position, icon.path)

desktop = controller.desktop_info()
print("Desktop:", desktop.bounds.as_tuple())
for monitor in desktop.monitors:
    print("Monitor:", monitor.name, monitor.resolution, monitor.scale_factor)
for grid in desktop.grids:
    print("Grid:", grid.monitor_id, grid.cell_size, grid.origin, grid.capacity)
print("Flags:", rdi.FolderFlag(controller.get_flags()))
```

See [desktop entities](../../concepts/desktop.md) for coordinate and ID semantics.
Next, try [operation recipes](../guides/basic-usage.md) or create an
[off-screen frame](../examples/showcases.md) without moving real icons.