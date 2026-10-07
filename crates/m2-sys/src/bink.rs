//! Playing the game's Bink videos (`data\Movies\*.bik`) with the Bink
//! library the player's own install ships (`binkw32.dll`).
//!
//! The library is 32-bit, so it can only be loaded into a 32-bit process.
//! [`Bink`] does that (i686 Windows builds only); the `m2-bink` program
//! built from this crate wraps it and streams the frames over a pipe, so a
//! 64-bit program plays videos through [`BinkProcess`] instead.
//!
//! Only the library's exported functions are used, through their public
//! names and the calling conventions of the published Bink SDK headers;
//! of the `BINK` structure only the leading fields every Bink 1 version
//! shares (size, frame count, current frame, frame rate) are read.
//!
//! # Stream format (`m2-bink` to [`BinkProcess`])
//!
//! ```text
//! "M2BK"  u32 width, height, frames, rate, rate divisor  (little endian)
//! then per frame: width * height * 4 bytes of RGBA8, top row first
//! ```

use std::io::{self, Read};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};

/// What a video is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    /// Frames per second is `rate / rate_div`.
    pub rate: u32,
    pub rate_div: u32,
}

impl VideoInfo {
    pub fn fps(&self) -> f64 {
        f64::from(self.rate) / f64::from(self.rate_div.max(1))
    }

    pub fn frame_len(&self) -> usize {
        self.width as usize * self.height as usize * 4
    }

    /// The stream header.
    pub fn header(&self) -> [u8; 24] {
        let mut h = [0; 24];
        h[..4].copy_from_slice(b"M2BK");
        for (i, v) in [
            self.width,
            self.height,
            self.frames,
            self.rate,
            self.rate_div,
        ]
        .into_iter()
        .enumerate()
        {
            h[4 + i * 4..8 + i * 4].copy_from_slice(&v.to_le_bytes());
        }
        h
    }

    /// Reads a stream header.
    pub fn read_header(r: &mut impl Read) -> io::Result<Self> {
        let mut h = [0; 24];
        r.read_exact(&mut h)?;
        if &h[..4] != b"M2BK" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not an m2-bink stream",
            ));
        }
        let word = |i: usize| u32::from_le_bytes(h[4 + i * 4..8 + i * 4].try_into().unwrap());
        let info = Self {
            width: word(0),
            height: word(1),
            frames: word(2),
            rate: word(3),
            rate_div: word(4),
        };
        if info.width == 0 || info.height == 0 || info.width > 8192 || info.height > 8192 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "bad video size"));
        }
        Ok(info)
    }
}

/// A video played by the `m2-bink` program, frames read as they come.
/// The program plays the sound itself and keeps to the video's frame rate,
/// so reading a frame blocks until it is due.
#[derive(Debug)]
pub struct BinkProcess {
    child: Child,
    out: ChildStdout,
    pub info: VideoInfo,
    read: u32,
}

impl BinkProcess {
    /// Starts `program` (the `m2-bink` executable) on `movie`, a `.bik`
    /// file, with `binkw32.dll` from `install` (the game's folder).
    pub fn spawn(program: &Path, install: &Path, movie: &Path, mute: bool) -> io::Result<Self> {
        let mut cmd = Command::new(program);
        cmd.arg(install).arg(movie).arg("--stream");
        if mute {
            cmd.arg("--mute");
        }
        let mut child = cmd.stdout(Stdio::piped()).stdin(Stdio::null()).spawn()?;
        let mut out = child.stdout.take().expect("piped");
        let info = match VideoInfo::read_header(&mut out) {
            Ok(i) => i,
            Err(e) => {
                let _ = child.kill();
                return Err(e);
            }
        };
        Ok(Self {
            child,
            out,
            info,
            read: 0,
        })
    }

    /// Reads the next frame into `rgba` (resized to fit). `Ok(false)` at
    /// the end of the video.
    pub fn next_frame(&mut self, rgba: &mut Vec<u8>) -> io::Result<bool> {
        if self.read >= self.info.frames {
            return Ok(false);
        }
        rgba.resize(self.info.frame_len(), 0);
        match self.out.read_exact(rgba) {
            Ok(()) => {
                self.read += 1;
                Ok(true)
            }
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Stops the video (and its sound).
    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for BinkProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(all(windows, target_arch = "x86"))]
pub use native::Bink;

#[cfg(all(windows, target_arch = "x86"))]
mod native {
    use std::ffi::{CStr, CString, c_char, c_void};
    use std::path::Path;

    use windows::Win32::Foundation::{FreeLibrary, HMODULE};
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows::core::{HSTRING, PCSTR};

    use super::VideoInfo;

    /// The leading fields of the library's `BINK` structure.
    #[repr(C)]
    struct RawBink {
        width: u32,
        height: u32,
        frames: u32,
        frame_num: u32,
        last_frame_num: u32,
        frame_rate: u32,
        frame_rate_div: u32,
    }
    m2_layout::assert_layout!(RawBink, size = 0x1c, {
        width: 0x00,
        height: 0x04,
        frames: 0x08,
        frame_num: 0x0c,
        last_frame_num: 0x10,
        frame_rate: 0x14,
        frame_rate_div: 0x18,
    });

    type HBink = *mut RawBink;
    type SoundOpen = unsafe extern "stdcall" fn(param: u32) -> *mut c_void;

    /// `BinkCopyToBuffer` surface: 32-bit, bytes B, G, R, x.
    const SURFACE_32: u32 = 3;
    /// Copy the whole frame, not only what changed since the last one.
    const COPY_ALL: u32 = 0x8000_0000;

    struct Api {
        open: unsafe extern "stdcall" fn(name: *const c_char, flags: u32) -> HBink,
        close: unsafe extern "stdcall" fn(HBink),
        do_frame: unsafe extern "stdcall" fn(HBink) -> i32,
        next_frame: unsafe extern "stdcall" fn(HBink),
        wait: unsafe extern "stdcall" fn(HBink) -> i32,
        copy_to_buffer:
            unsafe extern "stdcall" fn(HBink, *mut c_void, i32, u32, u32, u32, u32) -> i32,
        get_error: unsafe extern "stdcall" fn() -> *const c_char,
        set_sound_system: unsafe extern "stdcall" fn(SoundOpen, u32) -> i32,
        open_direct_sound: SoundOpen,
    }

    /// An open video.
    pub struct Bink {
        module: HMODULE,
        api: Api,
        handle: HBink,
        pub info: VideoInfo,
        /// The last frame has been shown.
        done: bool,
    }

    impl std::fmt::Debug for Bink {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Bink").field("info", &self.info).finish()
        }
    }

    impl Bink {
        /// Loads `binkw32.dll` from `install` (the game's folder) and opens
        /// `movie`. With `sound`, the library plays the video's sound
        /// through DirectSound as frames are shown.
        pub fn open(install: &Path, movie: &Path, sound: bool) -> Result<Self, String> {
            let dll = install.join("binkw32.dll");
            // SAFETY: loading the library runs its DllMain; it is the Bink
            // library the player installed with the game.
            let module = unsafe { LoadLibraryW(&HSTRING::from(dll.as_os_str())) }
                .map_err(|e| format!("{}: {e}", dll.display()))?;
            let api = match Api::load(module) {
                Ok(a) => a,
                Err(e) => {
                    // SAFETY: the module was loaded above and nothing from
                    // it is in use.
                    unsafe {
                        let _ = FreeLibrary(module);
                    }
                    return Err(e);
                }
            };
            let path = CString::new(movie.to_string_lossy().as_bytes())
                .map_err(|_| "movie path has a NUL".to_owned())?;
            // SAFETY: the signatures are the SDK's; the path outlives the
            // call, and the sound system is set before opening as the SDK
            // requires.
            let handle = unsafe {
                if sound {
                    (api.set_sound_system)(api.open_direct_sound, 0);
                }
                (api.open)(path.as_ptr(), 0)
            };
            if handle.is_null() {
                // SAFETY: BinkGetError returns a static NUL-terminated
                // string (or null).
                let why = unsafe {
                    let p = (api.get_error)();
                    if p.is_null() {
                        "unknown error".to_owned()
                    } else {
                        CStr::from_ptr(p).to_string_lossy().into_owned()
                    }
                };
                // SAFETY: as above.
                unsafe {
                    let _ = FreeLibrary(module);
                }
                return Err(format!("{}: {why}", movie.display()));
            }
            // SAFETY: a non-null handle points at the library's BINK
            // structure, valid until BinkClose.
            let raw = unsafe { &*handle };
            let info = VideoInfo {
                width: raw.width,
                height: raw.height,
                frames: raw.frames,
                rate: raw.frame_rate,
                rate_div: raw.frame_rate_div,
            };
            Ok(Self {
                module,
                api,
                handle,
                info,
                done: false,
            })
        }

        /// Waits until the next frame is due (the library keeps time with
        /// the sound), decodes it into `rgba` (RGBA8, top row first) and
        /// moves on. `false` once every frame has been shown.
        pub fn next_frame(&mut self, rgba: &mut Vec<u8>) -> bool {
            // SAFETY: the handle is open until Drop.
            let current = unsafe { (*self.handle).frame_num };
            if self.done || current == 0 {
                return false;
            }
            // SAFETY: as above; the buffer holds width * height 32-bit
            // pixels at a pitch of width * 4.
            unsafe {
                while (self.api.wait)(self.handle) != 0 {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                (self.api.do_frame)(self.handle);
                rgba.resize(self.info.frame_len(), 0);
                (self.api.copy_to_buffer)(
                    self.handle,
                    rgba.as_mut_ptr().cast(),
                    (self.info.width * 4) as i32,
                    self.info.height,
                    0,
                    0,
                    SURFACE_32 | COPY_ALL,
                );
            }
            for px in rgba.chunks_exact_mut(4) {
                px.swap(0, 2);
                px[3] = 255;
            }
            if current < self.info.frames {
                // SAFETY: as above.
                unsafe { (self.api.next_frame)(self.handle) };
            } else {
                self.done = true;
            }
            true
        }
    }

    impl Drop for Bink {
        fn drop(&mut self) {
            // SAFETY: the handle and module are open, and nothing uses them
            // after this.
            unsafe {
                (self.api.close)(self.handle);
                let _ = FreeLibrary(self.module);
            }
        }
    }

    impl Api {
        fn load(module: HMODULE) -> Result<Self, String> {
            /// Looks an export up by its decorated stdcall name (`@n` is
            /// the bytes of arguments) as a function of type `F`.
            ///
            /// # Safety
            ///
            /// `F` must be the export's signature.
            unsafe fn get<F: Copy>(module: HMODULE, name: &CStr) -> Result<F, String> {
                const { assert!(size_of::<F>() == size_of::<*const c_void>()) };
                // SAFETY: the module is loaded and the name NUL-terminated.
                let f = unsafe { GetProcAddress(module, PCSTR(name.as_ptr().cast())) }
                    .ok_or_else(|| format!("binkw32.dll has no {}", name.to_string_lossy()))?;
                let p = f as *const c_void;
                // SAFETY: the caller names the export's signature; a
                // function pointer is pointer-sized (checked above).
                Ok(unsafe { std::mem::transmute_copy::<*const c_void, F>(&p) })
            }
            // SAFETY: each type is the signature the SDK headers give the
            // export, and the decorated names confirm the argument sizes.
            unsafe {
                Ok(Self {
                    open: get(module, c"_BinkOpen@8")?,
                    close: get(module, c"_BinkClose@4")?,
                    do_frame: get(module, c"_BinkDoFrame@4")?,
                    next_frame: get(module, c"_BinkNextFrame@4")?,
                    wait: get(module, c"_BinkWait@4")?,
                    copy_to_buffer: get(module, c"_BinkCopyToBuffer@28")?,
                    get_error: get(module, c"_BinkGetError@0")?,
                    set_sound_system: get(module, c"_BinkSetSoundSystem@8")?,
                    open_direct_sound: get(module, c"_BinkOpenDirectSound@4")?,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_round_trips() {
        let info = VideoInfo {
            width: 1280,
            height: 720,
            frames: 150,
            rate: 30000,
            rate_div: 1001,
        };
        let h = info.header();
        assert_eq!(VideoInfo::read_header(&mut &h[..]).unwrap(), info);
        assert!((info.fps() - 29.97).abs() < 0.01);
        let mut bad = h;
        bad[0] = b'X';
        assert!(VideoInfo::read_header(&mut &bad[..]).is_err());
    }
}
