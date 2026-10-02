//! The DLL that gets loaded into the original game process.
//!
//! `DllMain` runs under the loader lock, so it only records that we are
//! attached; real initialization (reading the symbol tables, installing
//! hooks) happens later from a safe point in the game's startup.

#[cfg(windows)]
mod dll {
    use windows::Win32::Foundation::{HINSTANCE, TRUE};
    use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
    use windows::core::BOOL;

    #[unsafe(no_mangle)]
    extern "system" fn DllMain(
        _module: HINSTANCE,
        reason: u32,
        _reserved: *mut core::ffi::c_void,
    ) -> BOOL {
        if reason == DLL_PROCESS_ATTACH {
            // Intentionally empty: see the crate docs.
        }
        TRUE
    }
}
