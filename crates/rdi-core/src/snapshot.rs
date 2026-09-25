//! Immutable snapshot of a single desktop icon's observable state.

use std::path::PathBuf;

use crate::{IconId, Point};

/// Everything the enumeration layer knows about one desktop icon at the
/// moment the snapshot was taken.
///
/// `path` is `None` for virtual items (e.g. *This PC*, *Recycle Bin*).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconSnapshot {
    pub id: IconId,
    pub display_name: String,
    pub path: Option<PathBuf>,
    pub is_virtual: bool,
    pub position: Point,
}

impl IconSnapshot {
    #[inline]
    pub fn new(
        id: IconId,
        display_name: impl Into<String>,
        path: Option<PathBuf>,
        is_virtual: bool,
        position: Point,
    ) -> Self {
        Self {
            id,
            display_name: display_name.into(),
            path,
            is_virtual,
            position,
        }
    }
}
