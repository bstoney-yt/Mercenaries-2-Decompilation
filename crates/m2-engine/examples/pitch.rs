//! Measures sound the way the alpha picks engine loops, to compare an
//! engine heard in a playthrough video with the game's banks: for each
//! second of a 16-bit PCM WAV file (a clip of the video's audio, cut with
//! ffmpeg), its level and the strongest repeating rate between 20 and
//! 400 Hz (an engine's firing rate, [`SoundBuffer::fundamental`]); then
//! the same for every sound of the banks named, marking the loops the
//! alpha plays.
//!
//! ```text
//! ffmpeg -i clip.webm -ss 930 -t 60 -ac 1 -ar 22050 -c:a pcm_s16le tank.wav
//! cargo run --release -p m2-engine --example pitch -- tank.wav ["<install dir>" veh_tankmed ...]
//! ```
//!
//! Prints numbers and hashes only.

use std::path::Path;

use m2_engine::Install;
use m2_engine::audio::{EngineSounds, SoundBuffer, SoundLibrary};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(wav) = args.first() else {
        eprintln!("usage: pitch <clip.wav> [<install dir> <bank>...]");
        std::process::exit(2);
    };
    let clip = read_wav(Path::new(wav))?;
    println!(
        "{wav}: {} channel(s), {} Hz, {:.1} s",
        clip.channels,
        clip.rate,
        clip.seconds()
    );
    println!("second  level dB  pitch Hz  crossings/s");
    let ch = usize::from(clip.channels.max(1));
    let second = clip.rate as usize * ch;
    for (k, window) in clip.samples.chunks(second).enumerate() {
        if window.len() < second / 2 {
            break;
        }
        let part = SoundBuffer {
            hash: 0,
            channels: clip.channels,
            rate: clip.rate,
            samples: window.into(),
        };
        println!(
            "{k:>6}  {:>8.1}  {:>8}  {:>11.0}",
            db(part.loudness()),
            part.fundamental()
                .map_or("-".to_owned(), |f| format!("{f:.0}")),
            part.crossing_rate()
        );
    }
    if let Some(root) = args.get(1) {
        let install = Install::find(Path::new(root))?;
        let mut source = install.open_world()?;
        let names: Vec<&str> = args[2..].iter().map(String::as_str).collect();
        let library = SoundLibrary::load(&mut source, &names)?;
        for name in names {
            let sounds = library.bank(name);
            let picked = EngineSounds::pick(name, sounds);
            println!(
                "{name}: {} sounds; the alpha loops {} ({})",
                sounds.len(),
                picked
                    .loops
                    .iter()
                    .map(|(s, _)| format!("{:08x}", s.hash))
                    .collect::<Vec<_>>()
                    .join(", "),
                if picked.named { "by name" } else { "measured" }
            );
            println!("  sound     seconds  level dB  pitch Hz  crossings/s  steady");
            for s in sounds {
                println!(
                    "  {:08x}  {:>7.2}  {:>8.1}  {:>8}  {:>11.0}  {}",
                    s.hash,
                    s.seconds(),
                    db(s.loudness()),
                    s.fundamental()
                        .map_or("-".to_owned(), |f| format!("{f:.0}")),
                    s.crossing_rate(),
                    s.is_steady()
                );
            }
        }
    }
    Ok(())
}

/// A level (root-mean-square, 0 to 1) in dB below full scale.
fn db(level: f32) -> f32 {
    if level > 0.0 {
        20.0 * level.log10()
    } else {
        -100.0
    }
}

/// Reads a 16-bit PCM WAV file.
fn read_wav(path: &Path) -> Result<SoundBuffer, Box<dyn std::error::Error>> {
    let data = std::fs::read(path)?;
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(format!("{}: not a WAV file", path.display()).into());
    }
    let (mut channels, mut rate, mut bits) = (0u16, 0u32, 0u16);
    let mut at = 12;
    while at + 8 <= data.len() {
        let id = &data[at..at + 4];
        let len =
            u32::from_le_bytes([data[at + 4], data[at + 5], data[at + 6], data[at + 7]]) as usize;
        let body = &data[at + 8..(at + 8 + len).min(data.len())];
        match id {
            b"fmt " if body.len() >= 16 => {
                let format = u16::from_le_bytes([body[0], body[1]]);
                if format != 1 {
                    return Err(format!("{}: not PCM (format {format})", path.display()).into());
                }
                channels = u16::from_le_bytes([body[2], body[3]]);
                rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
                bits = u16::from_le_bytes([body[14], body[15]]);
            }
            b"data" => {
                if bits != 16 || channels == 0 {
                    return Err(format!(
                        "{}: {bits}-bit, {channels} channel(s); expected 16-bit PCM",
                        path.display()
                    )
                    .into());
                }
                return Ok(SoundBuffer {
                    hash: 0,
                    channels,
                    rate,
                    samples: body
                        .chunks_exact(2)
                        .map(|b| i16::from_le_bytes([b[0], b[1]]))
                        .collect(),
                });
            }
            _ => {}
        }
        at += 8 + len + (len & 1);
    }
    Err(format!("{}: no data", path.display()).into())
}
