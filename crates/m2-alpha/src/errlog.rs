//! The log (standard error) written in batches when it goes to a file.
//!
//! Rust's standard error is unbuffered: each piece of an `eprintln!` is a
//! write of its own. To a file that is slow, and to one on a network
//! share very slow: measured on the dev machine, a line of the `anim:`
//! kind took 5.2 ms to a local file and 68 ms to the NAS, and an autoplay
//! run writes about 950 of them in a busy minute, inside the game's tick.
//!
//! So when standard error is not a console, [`start`] puts a pipe in its
//! place and a thread writes what comes through to the file, as much at
//! a time as has gathered. [`finish`] puts the file back and waits for
//! the rest to be written; a panic does the same before the process
//! ends (`keep_crash_reports`). A process killed from outside loses what
//! was still in the pipe (4 KB at most).
//!
//! Invented: nothing here is the original game's.

use std::io::{Read, Write};

/// Copies `from` to `to` until `from` ends, each read's worth in one
/// write.
fn pump(mut from: impl Read, mut to: impl Write) {
    let mut buffer = vec![0u8; 1 << 16];
    loop {
        match from.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                // A log that can't be written is lost; the game goes on.
                let _ = to.write_all(&buffer[..n]);
            }
        }
    }
    let _ = to.flush();
}

#[cfg(windows)]
mod piped {
    use std::io::IsTerminal;
    use std::os::windows::io::{AsHandle, OwnedHandle};
    use std::sync::Mutex;

    /// The pipe's writing end (the standard error while this stands), the
    /// file it stood in for, and the thread writing to that.
    struct Piped {
        writer: OwnedHandle,
        original: OwnedHandle,
        thread: std::thread::JoinHandle<()>,
    }

    static PIPED: Mutex<Option<Piped>> = Mutex::new(None);

    pub fn start() -> std::io::Result<()> {
        let stderr = std::io::stderr();
        if stderr.is_terminal() {
            return Ok(());
        }
        let mut piped = PIPED.lock().unwrap_or_else(|e| e.into_inner());
        if piped.is_some() {
            return Ok(());
        }
        let original = stderr.as_handle().try_clone_to_owned()?;
        let sink = std::fs::File::from(original.try_clone()?);
        let (reader, writer) = std::io::pipe()?;
        let writer = OwnedHandle::from(writer);
        let thread = std::thread::Builder::new()
            .name("log".into())
            .spawn(move || super::pump(reader, sink))?;
        m2_sys::stdio::set_stderr(&writer)?;
        *piped = Some(Piped {
            writer,
            original,
            thread,
        });
        Ok(())
    }

    pub fn finish() {
        let taken = PIPED.lock().unwrap_or_else(|e| e.into_inner()).take();
        let Some(p) = taken else {
            return;
        };
        if m2_sys::stdio::set_stderr(&p.original).is_ok() {
            // The standard error from here on: never closed.
            std::mem::forget(p.original);
        }
        // The pipe's only writing end: the thread reads to its end.
        drop(p.writer);
        let _ = p.thread.join();
    }
}

/// Starts batching the log, when standard error is a file or a pipe (not
/// a console, and not missing as in a windowed game started plainly).
/// Only on Windows; elsewhere, and when it can't be set up, the log is
/// written as before.
pub fn start() {
    #[cfg(windows)]
    let _ = piped::start();
}

/// Writes out what is left of the log and puts standard error back.
/// Safe to call twice, or without [`start`].
pub fn finish() {
    #[cfg(windows)]
    piped::finish();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reader that hands out its bytes a few at a time, as a pipe does.
    struct Trickle<'a>(&'a [u8]);

    impl Read for Trickle<'_> {
        fn read(&mut self, into: &mut [u8]) -> std::io::Result<usize> {
            let n = self.0.len().min(7).min(into.len());
            into[..n].copy_from_slice(&self.0[..n]);
            self.0 = &self.0[n..];
            Ok(n)
        }
    }

    #[test]
    fn pump_copies_everything_in_order() {
        let text: Vec<u8> = (0..1000u32)
            .flat_map(|i| format!("line {i}\n").into_bytes())
            .collect();
        let mut out = Vec::new();
        pump(Trickle(&text), &mut out);
        assert_eq!(out, text);
    }

    #[test]
    fn finish_without_start_does_nothing() {
        finish();
        finish();
    }
}
