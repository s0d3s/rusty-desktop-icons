"""Windows folder flags.

https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/ne-shobjidl_core-folderflags
"""

from __future__ import annotations

from enum import IntFlag

from ._rusty_desktop_icons import _folder_flag_catalog

_catalog = {
    name: (bits, title, description)
    for bits, name, title, description in _folder_flag_catalog()
}
_metadata = {bits: (title, description) for bits, title, description in _catalog.values()}


class _FolderFlag(IntFlag):
    """Combinable Windows flags accepted by all integer flag APIs.

    Construct ``FolderFlag(raw_mask)`` to retain unnamed bits. Titles and
    descriptions preserve the legacy UI where applicable, not support claims.
    Includes unsupported and deprecated flags; see Microsoft's reference:
    https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/ne-shobjidl_core-folderflags
    """

    @property
    def title(self) -> str | None:
        """Display title, or None for a combined or unnamed mask."""
        return _metadata.get(int(self), (None, None))[0]

    @property
    def description(self) -> str | None:
        """Display description, or None for a combined or unnamed mask."""
        return _metadata.get(int(self), (None, None))[1]


FolderFlag = _FolderFlag(
    "FolderFlag",
    {name: bits for name, (bits, _, _) in _catalog.items()},
    module=__package__,
)
FolderFlag.__doc__ = _FolderFlag.__doc__