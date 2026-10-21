//! Reading the Windows registry (where installers record their folders).

/// The string `value` of the key `key` under `HKEY_LOCAL_MACHINE`
/// (`SOFTWARE\WOW6432Node\EA Games\...`), or `None` when the key, the
/// value or a string there is missing. Elsewhere than Windows, `None`.
#[cfg(windows)]
pub fn local_machine_string(key: &str, value: &str) -> Option<String> {
    use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
    use windows::core::PCWSTR;

    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (key, value) = (wide(key), wide(value));
    let mut buf = vec![0u16; 1024];
    let mut bytes = (buf.len() * 2) as u32;
    // SAFETY: both names are NUL-terminated UTF-16 that outlive the call;
    // `buf` holds `bytes` bytes, which the call writes at most and then
    // sets to the length written (the terminating NUL included).
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(key.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    if status.is_err() {
        return None;
    }
    let len = (bytes as usize / 2).min(buf.len());
    let s = String::from_utf16_lossy(&buf[..len]);
    let s = s.trim_end_matches('\0').to_owned();
    (!s.is_empty()).then_some(s)
}

#[cfg(not(windows))]
pub fn local_machine_string(_key: &str, _value: &str) -> Option<String> {
    None
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn reads_a_value_every_windows_has() {
        let root = local_machine_string(
            r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
            "SystemRoot",
        );
        assert!(root.is_some_and(|r| r.len() > 3), "no SystemRoot");
        assert_eq!(
            local_machine_string(r"SOFTWARE\No Such Key Here", "x"),
            None
        );
    }
}
