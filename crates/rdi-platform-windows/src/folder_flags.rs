//! Windows desktop folder flags, including unsupported and deprecated SDK values.
//!
//! Reference: [Microsoft FOLDERFLAGS documentation](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/ne-shobjidl_core-folderflags).

macro_rules! folder_flags {
    ($($variant:ident, $sdk:ident, $bits:literal, $title:literal, $description:literal;)+) => {
        /// A Windows folder-view flag or an arbitrary raw mask (R-FLAG-2).
        ///
        /// Pass [`Self::bits`] to integer-based controller operations.
        /// See [Microsoft FOLDERFLAGS](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/ne-shobjidl_core-folderflags)
        /// for platform support; legacy UI descriptions do not guarantee support.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum FolderFlag {
            $(#[doc = $description] $variant,)+
            /// An arbitrary unsigned mask, including combinations of flags.
            Raw(u32),
        }

        impl FolderFlag {
            /// Every documented flag, including zero, in Microsoft SDK order.
            pub const ALL: &'static [Self] = &[$(Self::$variant,)+];

            /// Raw unsigned Windows mask.
            pub const fn bits(self) -> u32 {
                match self {
                    $(Self::$variant => $bits,)+
                    Self::Raw(bits) => bits,
                }
            }

            /// Windows SDK name; absent for raw masks.
            pub const fn name(self) -> Option<&'static str> {
                match self {
                    $(Self::$variant => Some(stringify!($sdk)),)+
                    Self::Raw(_) => None,
                }
            }

            /// Display title; absent for raw masks.
            pub const fn title(self) -> Option<&'static str> {
                match self {
                    $(Self::$variant => Some($title),)+
                    Self::Raw(_) => None,
                }
            }

            /// Display description; absent for raw masks.
            pub const fn description(self) -> Option<&'static str> {
                match self {
                    $(Self::$variant => Some($description),)+
                    Self::Raw(_) => None,
                }
            }
        }

        #[cfg(test)]
        #[test]
        fn folder_flag_catalog_matches_windows_sdk() {
            $(assert_eq!(FolderFlag::$variant.bits(), windows::Win32::UI::Shell::$sdk.0 as u32);)+
        }
    };
}

folder_flags! {
    None, FWF_NONE, 0x00000000, "None", "No special view options";
    AutoArrange, FWF_AUTOARRANGE, 0x00000001, "Auto Arrange", "Automatically arrange the icons";
    AbbreviatedNames, FWF_ABBREVIATEDNAMES, 0x00000002, "Abbreviated Names", "Not supported";
    SnapToGrid, FWF_SNAPTOGRID, 0x00000004, "Snap to grid", "Snap icon positions to grid";
    OwnerData, FWF_OWNERDATA, 0x00000008, "Owner Data", "Not supported";
    BestFitWindow, FWF_BESTFITWINDOW, 0x00000010, "Best Fit Window", "Not supported";
    Desktop, FWF_DESKTOP, 0x00000020, "Desktop", "Use desktop behavior; implies no client edge and no scroll bars";
    SingleSelect, FWF_SINGLESEL, 0x00000040, "Single Select", "Prevents selection of multiple icons";
    NoSubfolders, FWF_NOSUBFOLDERS, 0x00000080, "Hide Subfolders", "Don't show subfolders";
    Transparent, FWF_TRANSPARENT, 0x00000100, "Transparent", "Draw transparently on the desktop";
    NoClientEdge, FWF_NOCLIENTEDGE, 0x00000200, "No Client Edge", "Not supported";
    NoScroll, FWF_NOSCROLL, 0x00000400, "No Scroll Bars", "Don't add scroll bars to the desktop view";
    AlignLeft, FWF_ALIGNLEFT, 0x00000800, "Align Left", "Align the view to the left";
    NoIcons, FWF_NOICONS, 0x00001000, "Hide Icons", "Don't show icons";
    ShowSelectionAlways, FWF_SHOWSELALWAYS, 0x00002000, "Always Show Selection", "Deprecated since Windows XP; has no effect";
    NoVisible, FWF_NOVISIBLE, 0x00004000, "Not Visible", "Not supported";
    SingleClickActivate, FWF_SINGLECLICKACTIVATE, 0x00008000, "One Click Activate", "Icons open with one click";
    NoWebView, FWF_NOWEBVIEW, 0x00010000, "No Web View", "Don't display the folder as a web view";
    HideFileNames, FWF_HIDEFILENAMES, 0x00020000, "Hide Filenames", "Don't show filenames";
    CheckSelect, FWF_CHECKSELECT, 0x00040000, "Checkbox Select #0", "Rudiment. Check for fun";
    NoEnumRefresh, FWF_NOENUMREFRESH, 0x00080000, "No Enumeration Refresh", "Keep existing contents without re-enumerating on refresh";
    NoGrouping, FWF_NOGROUPING, 0x00100000, "No Grouping", "Don't allow grouping in the view";
    FullRowSelect, FWF_FULLROWSELECT, 0x00200000, "Full Row Select", "Highlight the item and all its sub-items when selected";
    NoFilters, FWF_NOFILTERS, 0x00400000, "No Filters", "Don't display filters in the view";
    NoColumnHeader, FWF_NOCOLUMNHEADER, 0x00800000, "No Column Header", "Don't display a column header in any view mode";
    NoHeaderInAllViews, FWF_NOHEADERINALLVIEWS, 0x01000000, "Header in Details Only", "Show the column header only in details view";
    ExtendedTiles, FWF_EXTENDEDTILES, 0x02000000, "Extended Tiles", "Extend each tile to the width of the view";
    TriCheckSelect, FWF_TRICHECKSELECT, 0x04000000, "Checkbox Select #1", "Rudiment. Check for fun";
    AutoCheckSelect, FWF_AUTOCHECKSELECT, 0x08000000, "Checkbox Select #2", "Rudiment. Check for fun";
    NoBrowserViewState, FWF_NOBROWSERVIEWSTATE, 0x10000000, "Don't Save View State", "Don't save view state in the browser";
    SubsetGroups, FWF_SUBSETGROUPS, 0x20000000, "Subset Groups", "Show the displayed item count in each group";
    UseSearchFolder, FWF_USESEARCHFOLDER, 0x40000000, "Use Search Folder", "Use the search folder for stacking and searching";
    AllowRtlReading, FWF_ALLOWRTLREADING, 0x80000000, "Allow Right-to-Left Reading", "Use right-to-left reading layout on right-to-left systems";
}

impl From<FolderFlag> for u32 {
    fn from(flag: FolderFlag) -> Self {
        flag.bits()
    }
}

impl std::ops::BitOr for FolderFlag {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self::Raw(self.bits() | other.bits())
    }
}

#[cfg(test)]
mod tests {
    use super::FolderFlag;

    #[test]
    fn folder_flag_catalog_is_complete_and_unique() {
        let expected = std::iter::once(0).chain((0..32).map(|shift| 1u32 << shift));
        assert_eq!(FolderFlag::ALL.len(), 33);
        let mut names = std::collections::HashSet::new();
        for (flag, bits) in FolderFlag::ALL.iter().zip(expected) {
            assert_eq!(flag.bits(), bits);
            assert!(names.insert(flag.name().unwrap()));
            assert!(flag.name().unwrap().starts_with("FWF_"));
            assert!(!flag.title().unwrap().is_empty());
            assert!(!flag.description().unwrap().is_empty());
        }
    }

    #[test]
    fn folder_flag_raw_and_combined_masks_are_lossless() {
        let combined =
            FolderFlag::AutoArrange | FolderFlag::SnapToGrid | FolderFlag::Raw(0x80000000);
        assert_eq!(u32::from(combined), 0x80000005);
        assert_eq!(FolderFlag::Raw(u32::MAX).bits(), u32::MAX);
        assert_eq!(FolderFlag::Raw(0).bits(), 0);
        assert_eq!(combined.name(), None);
        assert_eq!(combined.title(), None);
        assert_eq!(combined.description(), None);
    }
}
