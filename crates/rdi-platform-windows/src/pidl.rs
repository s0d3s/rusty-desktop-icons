//! RAII wrappers for shell-allocated memory.
//!
//! Two resources need deterministic release:
//!
//! * `ITEMIDLIST*` (PIDL) — every PIDL returned by
//!   [`IEnumIDList::Next`] must be freed with `CoTaskMemFree`.
//! * `LPWSTR` returned by `StrRetToStrW` — likewise freed with
//!   `CoTaskMemFree`.
//!
//! `OwnedPidl` / `CoTaskMemWStr` hide the raw pointers behind `Drop`
//! implementations, so leaks are impossible even on error paths (`?`
//! bailouts, panics).
//!
//! [`IEnumIDList::Next`]: windows::Win32::UI::Shell::IEnumIDList

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::core::PWSTR;

/// Owning wrapper around a `*mut ITEMIDLIST`.
///
/// `Send` is asserted because ownership is transferred exclusively —
/// the engine's worker thread holds every PIDL and the raw pointer is
/// never shared.
pub(crate) struct OwnedPidl(*mut ITEMIDLIST);

// SAFETY: the pointer is uniquely owned; no other thread ever observes
// it. `Send` is required so the backend struct can be stored in a
// `HashMap` field that must itself be `Send` (the whole backend is
// `Send`-only per the trait).
unsafe impl Send for OwnedPidl {}

impl OwnedPidl {
    /// Wrap a freshly-allocated PIDL. Passing a null pointer is legal
    /// but discouraged — no work happens on drop.
    #[inline]
    pub(crate) fn from_raw(raw: *mut ITEMIDLIST) -> Self {
        Self(raw)
    }

    /// Borrow as a `*const ITEMIDLIST` for read-only shell APIs.
    #[inline]
    pub(crate) fn as_ptr(&self) -> *const ITEMIDLIST {
        self.0 as *const _
    }

    /// `true` if the wrapper holds a non-null pointer.
    #[inline]
    #[cfg(test)]
    pub(crate) fn is_null(&self) -> bool {
        self.0.is_null()
    }
}

impl Drop for OwnedPidl {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: `self.0` came from a shell allocation (either
            // `IEnumIDList::Next` or another `CoTaskMemAlloc`-returning
            // API) and is never freed elsewhere.
            unsafe { CoTaskMemFree(Some(self.0 as *const _)) };
        }
    }
}

/// Owning wrapper around an `LPWSTR` returned by `StrRetToStrW`.
///
/// Frees the memory with `CoTaskMemFree` on drop.
pub(crate) struct CoTaskMemWStr(PWSTR);

impl CoTaskMemWStr {
    #[inline]
    pub(crate) fn from_raw(ptr: PWSTR) -> Self {
        Self(ptr)
    }

    /// Copy the wide string into an owned `Vec<u16>` (**without** the
    /// trailing NUL). Returns an empty vec if the underlying pointer is
    /// null.
    pub(crate) fn to_vec(&self) -> Vec<u16> {
        if self.0.is_null() {
            return Vec::new();
        }
        let mut out = Vec::new();
        // SAFETY: `self.0` points at a NUL-terminated UTF-16 string
        // allocated by the shell — standard C-string walk, one `u16`
        // per step.
        unsafe {
            let mut ptr = self.0.0;
            while *ptr != 0 {
                out.push(*ptr);
                ptr = ptr.add(1);
            }
        }
        out
    }
}

impl Drop for CoTaskMemWStr {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer came from `StrRetToStrW` (or a similar
            // shell API) and is never freed elsewhere.
            unsafe { CoTaskMemFree(Some(self.0.0 as *const _)) };
        }
    }
}

/// Length of a wide buffer up to (but not including) the first NUL,
/// capped at `max` code units.
#[inline]
pub(crate) fn wcslen_bounded(buf: &[u16], max: usize) -> usize {
    buf.iter().take(max).position(|&c| c == 0).unwrap_or(max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wcslen_stops_at_first_nul() {
        let buf = [b'a' as u16, b'b' as u16, 0, b'c' as u16];
        assert_eq!(wcslen_bounded(&buf, 4), 2);
    }

    #[test]
    fn wcslen_capped_by_max() {
        let buf = [b'a' as u16; 10];
        assert_eq!(wcslen_bounded(&buf, 4), 4);
        assert_eq!(wcslen_bounded(&buf, 10), 10);
    }

    #[test]
    fn wcslen_zero_when_immediate_nul() {
        let buf = [0u16; 4];
        assert_eq!(wcslen_bounded(&buf, 4), 0);
    }

    #[test]
    fn owned_pidl_null_is_a_noop() {
        // Must not crash on drop when wrapping a null pointer.
        let p = OwnedPidl::from_raw(std::ptr::null_mut());
        assert!(p.is_null());
        drop(p);
    }
}
