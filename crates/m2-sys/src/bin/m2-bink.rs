//! Plays one of the game's Bink videos with the install's own
//! `binkw32.dll` (a 32-bit library, so this program is built for
//! `i686-pc-windows-msvc`).
//!
//! ```text
//! m2-bink <game folder> <movie.bik> --stream [--mute]
//! m2-bink <game folder> <movie.bik> [--mute] [--ppm <frame> <out.ppm>]
//! ```
//!
//! `--stream` writes the video to stdout in the format `m2_sys::bink`
//! describes, for a 64-bit program to show; the sound plays from here.
//! Without it the video plays (sound only) and prints its size, frame
//! count and rate, how many frames decoded and how long it took, and
//! `--ppm` saves one frame (counted from 1) as an image for checking. The
//! image holds the game's own video: keep it out of the repository.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(all(windows, target_arch = "x86")))]
fn run(_: &[String]) -> Result<(), String> {
    Err("build for i686-pc-windows-msvc: binkw32.dll is a 32-bit library".into())
}

#[cfg(all(windows, target_arch = "x86"))]
fn run(args: &[String]) -> Result<(), String> {
    use std::io::Write;
    use std::path::Path;

    use m2_sys::bink::Bink;

    let [install, movie, rest @ ..] = args else {
        return Err("usage: m2-bink <game folder> <movie.bik> [--stream] [--mute] [--ppm <frame> <out.ppm>]".into());
    };
    let (mut stream, mut mute, mut ppm) = (false, false, None);
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--stream" => stream = true,
            "--mute" => mute = true,
            "--ppm" => {
                let n: u32 = it
                    .next()
                    .ok_or("--ppm <frame> <out>")?
                    .parse()
                    .map_err(|_| "--ppm <frame> <out>")?;
                ppm = Some((n, it.next().ok_or("--ppm <frame> <out>")?.clone()));
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    let mut bink = Bink::open(Path::new(install), Path::new(movie), !mute)?;
    let info = bink.info;
    let mut frame = Vec::new();
    if stream {
        let out = std::io::stdout();
        let mut out = out.lock();
        out.write_all(&info.header()).map_err(|e| e.to_string())?;
        while bink.next_frame(&mut frame) {
            // The reader went away: stop playing.
            if out.write_all(&frame).and_then(|()| out.flush()).is_err() {
                break;
            }
        }
        return Ok(());
    }
    println!(
        "{}x{}, {} frames at {:.3} fps ({}/{})",
        info.width,
        info.height,
        info.frames,
        info.fps(),
        info.rate,
        info.rate_div
    );
    let start = std::time::Instant::now();
    let mut n = 0;
    while bink.next_frame(&mut frame) {
        n += 1;
        if let Some((want, path)) = &ppm
            && *want == n
        {
            let mut img = format!("P6\n{} {}\n255\n", info.width, info.height).into_bytes();
            for px in frame.chunks_exact(4) {
                img.extend_from_slice(&px[..3]);
            }
            std::fs::write(path, img).map_err(|e| format!("{path}: {e}"))?;
            println!("wrote frame {n} to {path}");
        }
    }
    println!(
        "decoded {n} frames in {:.2} s",
        start.elapsed().as_secs_f64()
    );
    Ok(())
}
