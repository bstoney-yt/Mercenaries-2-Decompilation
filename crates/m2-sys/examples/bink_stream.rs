//! Reads a video through the `m2-bink` program, as a 64-bit program
//! would, and reports how it arrived.
//!
//! ```text
//! cargo run -p m2-sys --example bink_stream -- <m2-bink.exe> <game folder> <movie.bik> [--mute]
//! ```

use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use m2_sys::bink::BinkProcess;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [program, install, movie, rest @ ..] = &args[..] else {
        eprintln!("usage: bink_stream <m2-bink.exe> <game folder> <movie.bik> [--mute]");
        return ExitCode::FAILURE;
    };
    let mute = rest.iter().any(|a| a == "--mute");
    let mut video = match BinkProcess::spawn(
        Path::new(program),
        Path::new(install),
        Path::new(movie),
        mute,
    ) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let info = video.info;
    println!(
        "{}x{}, {} frames at {:.3} fps",
        info.width,
        info.height,
        info.frames,
        info.fps()
    );
    let start = Instant::now();
    let mut frame = Vec::new();
    let mut n = 0u32;
    let mut sum = 0u64;
    loop {
        match video.next_frame(&mut frame) {
            Ok(true) => {
                n += 1;
                sum += frame
                    .iter()
                    .step_by(4096)
                    .map(|&b| u64::from(b))
                    .sum::<u64>();
            }
            Ok(false) => break,
            Err(e) => {
                eprintln!("error after {n} frames: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    let secs = start.elapsed().as_secs_f64();
    println!(
        "{n} frames in {secs:.2} s ({:.2} s expected), pixel checksum {sum}",
        f64::from(info.frames) / info.fps()
    );
    ExitCode::SUCCESS
}
