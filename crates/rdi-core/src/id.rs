//! Opaque, backend-defined identifiers for desktop icons.

use core::fmt;

/// Stable, opaque identifier for a single desktop icon.
///
/// On the Windows backend this is the hex-encoded UTF-16 representation of
/// the desktop-relative path (or display name for virtual items — *This PC*,
/// *Recycle Bin*, etc.), byte-identical to the legacy `positioner_c_part`
/// implementation.
///
/// The identifier is designed to survive a JSON round-trip, so it is safe to
/// return from a Python binding, hand off to an MCP client, and pass back on
/// a subsequent call.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IconId(String);

impl IconId {
    /// Wrap a raw identifier string.
    ///
    /// This does **not** validate the format — validation is the backend's
    /// responsibility.
    #[inline]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[inline]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for IconId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for IconId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<String> for IconId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for IconId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let id = IconId::new("0041 0042 0043");
        assert_eq!(id.as_str(), "0041 0042 0043");
        assert_eq!(id.clone().into_string(), "0041 0042 0043");
        assert_eq!(format!("{id}"), "0041 0042 0043");
    }

    #[test]
    fn hash_eq() {
        use std::collections::HashSet;
        let mut s = HashSet::new();
        s.insert(IconId::from("x"));
        assert!(s.contains(&IconId::from("x")));
        assert!(!s.contains(&IconId::from("y")));
    }
}
