//! Icon identifier encoding.
//!
//! Icons on the desktop are identified by a **hex-encoded UTF-16 string**
//! representing either the desktop-relative path (e.g. `notes.txt`) or —
//! for virtual items such as *This PC* — the display name. Each `u16`
//! becomes four lowercase hex characters (`'A'` → `"0041"`).
//!
//! The encoding matches
//! the legacy library's `escape_wchars` function
//! exactly, so IDs stored by older versions of the Python extension keep
//! working.
//!
//! # Why hex-encoded UTF-16?
//!
//! * It round-trips **any** shell path or display name (including
//!   non-BMP characters, control codes, whitespace, or filesystem-invalid
//!   sequences) into an opaque ASCII token — safe for JSON, filenames,
//!   command-line arguments, MCP tool arguments, etc.
//! * The encoded string sorts stably and can be used as a dict/HashMap
//!   key without normalisation surprises.
//! * Decoding is trivial and lossless.

use rdi_core::IconId;

/// Maximum number of UTF-16 code units scanned for the path prefix.
/// Matches `MAX_PATH` on Windows (260). Kept as an untyped constant
/// here so the module compiles on non-Windows hosts too (unit tests).
const MAX_PATH_WCHARS: usize = 260;

/// Lowercase hex table used by [`escape_wchars`].
const CHARSET: &[u8; 16] = b"0123456789abcdef";

/// Encode a NUL-terminated UTF-16 buffer as lowercase hex.
///
/// Encoding stops at the first `0` code unit if present; otherwise the
/// whole slice is consumed. Each `u16` becomes exactly four hex
/// characters, so the output length is always `4 * (code units consumed)`.
pub fn escape_wchars(source: &[u16]) -> String {
    let mut out = Vec::with_capacity(source.len() * 4);
    for &wc in source {
        if wc == 0 {
            break;
        }
        out.push(CHARSET[((wc >> 12) & 0xF) as usize]);
        out.push(CHARSET[((wc >> 8) & 0xF) as usize]);
        out.push(CHARSET[((wc >> 4) & 0xF) as usize]);
        out.push(CHARSET[(wc & 0xF) as usize]);
    }
    // The output is guaranteed to be ASCII hex, so UTF-8 conversion is safe.
    String::from_utf8(out).expect("hex table contains only ASCII")
}

/// Skip `C:\\Users\\User\\Desktop\\` — return the index just past the
/// **fourth** backslash within the first `MAX_PATH` code units, or
/// `None` if fewer than four backslashes are found. Matches the C++
/// `findDesktopPathPrefixLength` helper (returns `-1` there).
pub fn find_desktop_prefix_len(path: &[u16]) -> Option<usize> {
    let mut backslashes = 0;
    for (i, &wc) in path.iter().take(MAX_PATH_WCHARS).enumerate() {
        if wc == b'\\' as u16 {
            backslashes += 1;
            if backslashes == 4 {
                return Some(i + 1);
            }
        }
    }
    None
}

/// Build an [`IconId`] the same way the legacy extension did.
///
/// * If `path` is empty (virtual icon), the display name is hex-encoded.
/// * Otherwise the part *after* the desktop prefix is encoded (falling
///   back to the whole path if the prefix cannot be located).
///
/// The caller is responsible for supplying the shell display name and the
/// filesystem path (both as UTF-16 code units, **without** a trailing
/// NUL). The internal `com` module provides the shell-side helpers that produce them.
pub fn build_icon_id(path: &[u16], display_name: &[u16]) -> IconId {
    let encoded = if path.is_empty() {
        escape_wchars(display_name)
    } else {
        let start = find_desktop_prefix_len(path).unwrap_or(0);
        escape_wchars(&path[start..])
    };
    IconId::from(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn escape_ascii_letter() {
        // 'A' == U+0041 → "0041"
        assert_eq!(escape_wchars(&utf16("A")), "0041");
    }

    #[test]
    fn escape_stops_at_nul() {
        let mut buf = utf16("ab");
        buf.push(0);
        buf.push(b'c' as u16);
        // Only "ab" should be encoded — 'c' is past the NUL.
        assert_eq!(escape_wchars(&buf), "00610062");
    }

    #[test]
    fn escape_full_word() {
        // "Desktop" = D(0044) e(0065) s(0073) k(006b) t(0074) o(006f) p(0070)
        let hex = escape_wchars(&utf16("Desktop"));
        assert_eq!(hex, "004400650073006b0074006f0070");
    }

    #[test]
    fn escape_bmp_and_supplementary() {
        // U+1F600 is a surrogate pair U+D83D U+DE00 in UTF-16 → 8 hex chars.
        let hex = escape_wchars(&utf16("\u{1F600}"));
        assert_eq!(hex, "d83dde00");
    }

    #[test]
    fn escape_empty() {
        assert_eq!(escape_wchars(&[]), "");
    }

    #[test]
    fn prefix_finds_fourth_backslash() {
        // C:\Users\U\Desktop\notes.txt → index just past 4th backslash.
        let path = utf16(r"C:\Users\U\Desktop\notes.txt");
        let idx = find_desktop_prefix_len(&path).expect("should find prefix");
        // The suffix at that index should be "notes.txt".
        let suffix: String = String::from_utf16_lossy(&path[idx..]);
        assert_eq!(suffix, "notes.txt");
    }

    #[test]
    fn prefix_returns_none_when_short() {
        let path = utf16(r"C:\Users\Desktop");
        // Only 3 backslashes — never reaches four.
        assert!(find_desktop_prefix_len(&path).is_none());
    }

    #[test]
    fn prefix_stops_at_max_path() {
        // A long path with 4 backslashes past MAX_PATH_WCHARS is not found.
        let mut path = vec![b'a' as u16; MAX_PATH_WCHARS + 10];
        // Place 4 backslashes right at the end (past the scan window).
        for i in 0..4 {
            path[MAX_PATH_WCHARS + i] = b'\\' as u16;
        }
        assert!(find_desktop_prefix_len(&path).is_none());
    }

    #[test]
    fn icon_id_uses_path_suffix_when_available() {
        let path = utf16(r"C:\Users\U\Desktop\notes.txt");
        let display = utf16("notes.txt");
        let id = build_icon_id(&path, &display);
        // "notes.txt" = U+006E U+006F U+0074 U+0065 U+0073 U+002E
        //               U+0074 U+0078 U+0074 → 9 code units × 4 hex chars.
        assert_eq!(
            id.as_str(),
            "006e006f007400650073002e007400780074"
        );
    }

    #[test]
    fn icon_id_uses_display_name_for_virtual_icons() {
        let display = utf16("This PC");
        let id = build_icon_id(&[], &display);
        // "This PC" is 7 code units → 28 hex chars.
        assert_eq!(id.as_str().len(), 28);
        assert!(id.as_str().starts_with("00540068"));
    }

    #[test]
    fn icon_id_falls_back_to_full_path_when_prefix_missing() {
        // Path without four backslashes → whole path is encoded.
        let path = utf16(r"D:\loose\file.txt");
        let display = utf16("file.txt");
        let id = build_icon_id(&path, &display);
        // Expect the encoding to be that of the WHOLE path (17 chars).
        assert_eq!(id.as_str().len(), path.len() * 4);
    }
}
