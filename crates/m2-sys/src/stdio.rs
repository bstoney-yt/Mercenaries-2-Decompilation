//! The process's standard handles, and the one message box a windowed
//! game without a console shows when it can't start.

/// Makes `to` the process's standard error: what is written to it from
/// then on (Rust's `eprintln!` asks Windows for the handle at each write)
/// goes to `to`. The caller keeps `to` open for as long as it is the
/// standard error; writes to it once closed fail.
#[cfg(windows)]
pub fn set_stderr(to: &std::os::windows::io::OwnedHandle) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Console::{STD_ERROR_HANDLE, SetStdHandle};

    // SAFETY: `to` is an open handle (it is owned), and the call only
    // records its value in the process's parameters; nothing is read
    // through it here.
    unsafe { SetStdHandle(STD_ERROR_HANDLE, HANDLE(to.as_raw_handle())) }
        .map_err(std::io::Error::other)
}

/// Makes `to` the process's standard output, as [`set_stderr`] does the
/// standard error.
#[cfg(windows)]
pub fn set_stdout(to: &std::os::windows::io::OwnedHandle) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Console::{STD_OUTPUT_HANDLE, SetStdHandle};

    // SAFETY: as in `set_stderr`: the handle is open and only recorded.
    unsafe { SetStdHandle(STD_OUTPUT_HANDLE, HANDLE(to.as_raw_handle())) }
        .map_err(std::io::Error::other)
}

/// Whether the process has no standard handle of this kind at all (a
/// windowed program started from Explorer or a shortcut), as opposed to
/// one going to a console, a file or a pipe.
#[cfg(windows)]
pub fn is_missing(handle: std::os::windows::io::BorrowedHandle<'_>) -> bool {
    use std::os::windows::io::AsRawHandle;
    let raw = handle.as_raw_handle();
    // INVALID_HANDLE_VALUE is -1.
    raw.is_null() || raw as isize == -1
}

/// Shows `text` in a message box with an error icon and waits for it to
/// be closed.
#[cfg(windows)]
pub fn error_box(title: &str, text: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    use windows::core::PCWSTR;

    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (title, text) = (wide(title), wide(text));
    // SAFETY: both strings are NUL-terminated and outlive the call; with no
    // owner window the box is the process's own.
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}
