//! Reading the Windows clipboard's text (where the join page leaves the
//! co-op invite).

/// The clipboard's text, or `None` when it holds none or is busy.
/// Elsewhere than Windows, `None`.
#[cfg(windows)]
pub fn text() -> Option<String> {
    use windows::Win32::Foundation::{HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
    use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};

    /// `CF_UNICODETEXT`: UTF-16, NUL-terminated.
    const UNICODE_TEXT: u32 = 13;
    // SAFETY: opening with no owner window is allowed; it is closed below
    // on every path.
    unsafe { OpenClipboard(None) }.ok()?;
    // SAFETY: the clipboard is open. The handle stays the clipboard's (it
    // is not freed here); while it is locked, its memory is `GlobalSize`
    // bytes of UTF-16 that nothing else writes.
    let text = unsafe {
        GetClipboardData(UNICODE_TEXT).ok().and_then(|h: HANDLE| {
            let global = HGLOBAL(h.0);
            let ptr = GlobalLock(global) as *const u16;
            if ptr.is_null() {
                return None;
            }
            let units = GlobalSize(global) / 2;
            let wide = std::slice::from_raw_parts(ptr, units);
            let end = wide.iter().position(|&c| c == 0).unwrap_or(units);
            let s = String::from_utf16_lossy(&wide[..end]);
            let _ = GlobalUnlock(global);
            Some(s)
        })
    };
    // SAFETY: opened above.
    let _ = unsafe { CloseClipboard() };
    text
}

#[cfg(not(windows))]
pub fn text() -> Option<String> {
    None
}

/// Puts `s` on the clipboard as text. `false` when it is busy (or not
/// on Windows).
#[cfg(windows)]
pub fn set_text(s: &str) -> bool {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};

    const UNICODE_TEXT: u32 = 13;
    let wide: Vec<u16> = s.encode_utf16().chain(Some(0)).collect();
    // SAFETY: as in `text`; closed below on every path.
    if unsafe { OpenClipboard(None) }.is_err() {
        return false;
    }
    // SAFETY: the clipboard is open and ours to empty. The block is
    // allocated movable as the clipboard requires, `wide.len()` units
    // long, and written only while locked; once `SetClipboardData` takes
    // it the clipboard owns it (on failure it is left to the process,
    // a few bytes).
    let done = unsafe {
        EmptyClipboard().is_ok()
            && GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2).is_ok_and(|global| {
                let ptr = GlobalLock(global) as *mut u16;
                if ptr.is_null() {
                    return false;
                }
                std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
                let _ = GlobalUnlock(global);
                SetClipboardData(UNICODE_TEXT, Some(HANDLE(global.0))).is_ok()
            })
    };
    // SAFETY: opened above.
    let _ = unsafe { CloseClipboard() };
    done
}

#[cfg(not(windows))]
pub fn set_text(_s: &str) -> bool {
    false
}
