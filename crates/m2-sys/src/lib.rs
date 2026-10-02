//! Raw FFI.
//!
//! Everything that calls into Win32, Direct3D 9 or the original executable
//! goes through this crate, so `unsafe` stays concentrated in one place.

#[cfg(windows)]
pub use windows;

/// Base address of the main executable in the current process.
#[cfg(windows)]
pub fn image_base() -> u32 {
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;

    // SAFETY: a null module name returns the handle of the calling process's
    // executable, which stays loaded for the life of the process.
    let module = unsafe { GetModuleHandleW(None) }.expect("main module handle");
    module.0 as usize as u32
}
