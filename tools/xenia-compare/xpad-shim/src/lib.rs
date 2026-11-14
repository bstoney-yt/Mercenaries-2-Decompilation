//! `xinput1_4.dll` stand-in for driving an emulator's pad from a script.
//!
//! Placed beside an emulator executable (Windows finds a DLL in the
//! application folder before the system one, and `xinput1_4.dll` is not a
//! KnownDLL), it answers the XInput calls for pad 0 from a 16-byte shared
//! memory block named `Local\xpad-shim` that a driver script writes
//! (`pad.py`). Pads 1 to 3, and pad 0 while no script has the block open,
//! report "not connected". No input device, driver or desktop session is
//! needed, so it also works from a disconnected remote session.
//!
//! Block layout (little endian): `u32 seq` (bumped on every write, used as
//! the packet number), `u16 buttons` (XINPUT_GAMEPAD bits), `u8 lt`,
//! `u8 rt`, `i16 lx, ly, rx, ry`.

#![allow(non_snake_case, clippy::missing_safety_doc)]

use core::ffi::c_void;

const ERROR_SUCCESS: u32 = 0;
const ERROR_DEVICE_NOT_CONNECTED: u32 = 1167;
const ERROR_EMPTY: u32 = 4306;
const FILE_MAP_READ: u32 = 4;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn OpenFileMappingW(access: u32, inherit: i32, name: *const u16) -> *mut c_void;
    fn MapViewOfFile(h: *mut c_void, access: u32, hi: u32, lo: u32, n: usize) -> *mut c_void;
    fn UnmapViewOfFile(p: *const c_void) -> i32;
    fn CloseHandle(h: *mut c_void) -> i32;
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Gamepad {
    buttons: u16,
    lt: u8,
    rt: u8,
    lx: i16,
    ly: i16,
    rx: i16,
    ry: i16,
}

#[repr(C)]
pub struct State {
    packet: u32,
    pad: Gamepad,
}

#[repr(C)]
pub struct Capabilities {
    kind: u8,
    sub_type: u8,
    flags: u16,
    pad: Gamepad,
    vibration: [u16; 2],
}

/// Reads the block, mapping it afresh each call so a script started after
/// the emulator (or restarted) is picked up; `None` when no script has it.
fn read() -> Option<(u32, Gamepad)> {
    let name: Vec<u16> = "Local\\xpad-shim\0".encode_utf16().collect();
    // SAFETY: `name` is a NUL-terminated UTF-16 string that outlives the call.
    let h = unsafe { OpenFileMappingW(FILE_MAP_READ, 0, name.as_ptr()) };
    if h.is_null() {
        return None;
    }
    // SAFETY: `h` is a valid mapping handle; 16 bytes are within the block.
    let p = unsafe { MapViewOfFile(h, FILE_MAP_READ, 0, 0, 16) } as *const u8;
    let out = if p.is_null() {
        None
    } else {
        // SAFETY: `p` points at a readable 16-byte view; unaligned reads.
        let v = unsafe {
            let seq = (p as *const u32).read_unaligned();
            let g = (p.add(4) as *const Gamepad).read_unaligned();
            UnmapViewOfFile(p as *const c_void);
            (seq, g)
        };
        Some(v)
    };
    // SAFETY: `h` came from OpenFileMappingW above and is closed once.
    unsafe { CloseHandle(h) };
    out
}

/// Calls by export and pad index, logged every 2 s to `XPAD_SHIM_LOG` when
/// that variable names a file (for finding out what an emulator asks).
fn trace(what: usize, user: u32) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTS: [AtomicU64; 32] = [const { AtomicU64::new(0) }; 32];
    static LAST: AtomicU64 = AtomicU64::new(0);
    let Some(path) = std::env::var_os("XPAD_SHIM_LOG") else {
        return;
    };
    COUNTS[what * 8 + (user as usize).min(7)].fetch_add(1, Ordering::Relaxed);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    if LAST.swap(now, Ordering::Relaxed) / 2 != now / 2 {
        let c: Vec<u64> = COUNTS.iter().map(|c| c.load(Ordering::Relaxed)).collect();
        let pad = read().map(|(s, g)| (s, g.buttons));
        let line = format!(
            "{now} state{:?} caps{:?} key{:?} set{:?} pad {pad:?}\n",
            &c[0..8],
            &c[8..16],
            &c[16..24],
            &c[24..32]
        );
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn XInputGetState(user: u32, state: *mut State) -> u32 {
    trace(0, user);
    if user != 0 || state.is_null() {
        return ERROR_DEVICE_NOT_CONNECTED;
    }
    match read() {
        Some((seq, pad)) => {
            // SAFETY: the caller passes a writable XINPUT_STATE.
            unsafe { state.write(State { packet: seq, pad }) };
            ERROR_SUCCESS
        }
        None => ERROR_DEVICE_NOT_CONNECTED,
    }
}

/// Ordinal 100 (`XInputGetStateEx`, with the guide button).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn XInputGetStateEx(user: u32, state: *mut State) -> u32 {
    // SAFETY: same contract as XInputGetState.
    unsafe { XInputGetState(user, state) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn XInputSetState(user: u32, _vibration: *mut c_void) -> u32 {
    trace(3, user);
    if user == 0 && read().is_some() {
        ERROR_SUCCESS
    } else {
        ERROR_DEVICE_NOT_CONNECTED
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn XInputGetCapabilities(
    user: u32,
    _flags: u32,
    caps: *mut Capabilities,
) -> u32 {
    trace(1, user);
    if user != 0 || caps.is_null() || read().is_none() {
        return ERROR_DEVICE_NOT_CONNECTED;
    }
    let full = Gamepad {
        buttons: 0xF3FF,
        lt: 255,
        rt: 255,
        lx: -64,
        ly: -64,
        rx: -64,
        ry: -64,
    };
    // SAFETY: the caller passes a writable XINPUT_CAPABILITIES.
    unsafe {
        caps.write(Capabilities {
            kind: 1,     // XINPUT_DEVTYPE_GAMEPAD
            sub_type: 1, // XINPUT_DEVSUBTYPE_GAMEPAD
            flags: 0,
            pad: full,
            vibration: [0xFFFF, 0xFFFF],
        })
    };
    ERROR_SUCCESS
}

#[repr(C)]
pub struct Keystroke {
    virtual_key: u16,
    unicode: u16,
    flags: u16,
    user: u8,
    hid_code: u8,
}

const KEYSTROKE_KEYDOWN: u16 = 1;
const KEYSTROKE_KEYUP: u16 = 2;

/// Button bits and their `VK_PAD_*` codes, for keystrokes.
const PAD_KEYS: [(u16, u16); 14] = [
    (0x1000, 0x5800), // A
    (0x2000, 0x5801), // B
    (0x4000, 0x5802), // X
    (0x8000, 0x5803), // Y
    (0x0200, 0x5804), // right shoulder
    (0x0100, 0x5805), // left shoulder
    (0x0001, 0x5810), // d-pad up
    (0x0002, 0x5811), // d-pad down
    (0x0004, 0x5812), // d-pad left
    (0x0008, 0x5813), // d-pad right
    (0x0010, 0x5814), // start
    (0x0020, 0x5815), // back
    (0x0040, 0x5816), // left thumb press
    (0x0080, 0x5817), // right thumb press
];

/// Pseudo-buttons past the 16 real ones: triggers and stick directions,
/// so they make keystrokes too.
fn virtual_buttons(g: &Gamepad) -> u32 {
    const T: u8 = 30;
    const S: i16 = 20000;
    let mut b = u32::from(g.buttons);
    let mut set = |bit: u32, on: bool| {
        if on {
            b |= 1 << bit;
        }
    };
    set(16, g.lt > T);
    set(17, g.rt > T);
    set(18, g.ly > S);
    set(19, g.ly < -S);
    set(20, g.lx > S);
    set(21, g.lx < -S);
    set(22, g.ry > S);
    set(23, g.ry < -S);
    set(24, g.rx > S);
    set(25, g.rx < -S);
    b
}

fn key_for(bit: u32) -> u16 {
    match bit {
        16 => 0x5806, // left trigger
        17 => 0x5807, // right trigger
        18 => 0x5820, // left thumb up
        19 => 0x5821, // down
        20 => 0x5822, // right
        21 => 0x5823, // left
        22 => 0x5830, // right thumb up
        23 => 0x5831,
        24 => 0x5832,
        25 => 0x5833,
        _ => PAD_KEYS
            .iter()
            .find(|(m, _)| u32::from(*m) == 1 << bit)
            .map_or(0, |k| k.1),
    }
}

/// What the keystroke queue last saw, and the keystrokes not yet taken.
static KEYS: std::sync::Mutex<(u32, Vec<(u16, u16)>)> = std::sync::Mutex::new((0, Vec::new()));

/// Keystrokes are made from the changes in the shared state between calls,
/// as XInput does for a real pad (no auto-repeat).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn XInputGetKeystroke(user: u32, _r: u32, k: *mut Keystroke) -> u32 {
    trace(2, user);
    if !(user == 0 || user == 0xFF) || k.is_null() {
        return ERROR_DEVICE_NOT_CONNECTED;
    }
    let Some((_, pad)) = read() else {
        return ERROR_DEVICE_NOT_CONNECTED;
    };
    let mut keys = KEYS.lock().unwrap_or_else(|e| e.into_inner());
    let now = virtual_buttons(&pad);
    let changed = now ^ keys.0;
    for bit in (0..26).filter(|b| changed & (1 << b) != 0) {
        let flag = if now & (1 << bit) != 0 {
            KEYSTROKE_KEYDOWN
        } else {
            KEYSTROKE_KEYUP
        };
        let vk = key_for(bit);
        if vk != 0 {
            keys.1.push((vk, flag));
        }
    }
    keys.0 = now;
    if keys.1.is_empty() {
        return ERROR_EMPTY;
    }
    let (virtual_key, flags) = keys.1.remove(0);
    // SAFETY: the caller passes a writable XINPUT_KEYSTROKE.
    unsafe {
        k.write(Keystroke {
            virtual_key,
            unicode: 0,
            flags,
            user: 0,
            hid_code: 0,
        })
    };
    ERROR_SUCCESS
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn XInputGetBatteryInformation(
    _user: u32,
    _kind: u8,
    info: *mut [u8; 2],
) -> u32 {
    if !info.is_null() {
        // SAFETY: the caller passes a writable 2-byte XINPUT_BATTERY_INFORMATION.
        unsafe { info.write([1, 3]) }; // wired, full
    }
    ERROR_SUCCESS
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn XInputEnable(_enable: i32) {}
