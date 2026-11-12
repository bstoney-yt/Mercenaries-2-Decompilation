//! The process's standard handles.

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
