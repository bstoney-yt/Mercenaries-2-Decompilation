//! Measures sound the way the alpha picks engine loops, to compare an
//! engine heard in a playthrough video with the game's banks: for each
//! second of a 16-bit PCM WAV file (a clip of the video's audio, cut with
//! ffmpeg), its level and the strongest repeating rate between 20 and
//! 400 Hz (an engine's firing rate, [`SoundBuffer::fundamental`]); then
//! the same for every sound of the banks named, marking the loops the
//! alpha plays.
//!
//! Then which of the banks' sounds the clip holds, and how far it is
//! pitched: every sound's spectrum (on a log-frequency scale, 24 steps an
//! octave) is slid up and down an octave against the clip's, and the
//! best fit is kept. Two scores, as correlations (1 is a perfect fit):
//! `peaks` compares the spectra's peaks against their surroundings (an
//! engine's harmonics, the clip's music adds its own), `shape` the
//! spectra as they are (a gunshot's colour). For the whole clip, its
//! loudest second, and each second.
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
        let candidates: Vec<(&str, Spectrum)> = names
            .iter()
            .flat_map(|&name| library.bank(name).iter().map(move |s| (name, s)))
            .filter_map(|(name, s)| Some((name, Spectrum::of(s, 0, s.frames())?)))
            .collect();
        let whole = Spectrum::of(&clip, 0, clip.frames());
        if let Some(whole) = &whole {
            println!("the whole clip against the banks' sounds (best 12):");
            print_matches(&whole.best(&candidates), 12);
        }
        let per = clip.rate as usize;
        let seconds = clip.frames() / per;
        let loudest = (0..seconds).max_by(|&a, &b| {
            let level = |k: usize| {
                SoundBuffer {
                    hash: 0,
                    channels: clip.channels,
                    rate: clip.rate,
                    samples: clip.samples[k * second..(k + 1) * second].into(),
                }
                .loudness()
            };
            level(a).total_cmp(&level(b))
        });
        if let Some(k) = loudest
            && let Some(sp) = Spectrum::of(&clip, k * per, per)
        {
            println!("its loudest second ({k}) against the banks' sounds (best 8):");
            print_matches(&sp.best(&candidates), 8);
        }
        println!("each second's best two: second  bank sound ratio peaks shape");
        for k in 0..seconds {
            let Some(sp) = Spectrum::of(&clip, k * per, per) else {
                continue;
            };
            let best = sp.best(&candidates);
            let line: Vec<String> = best
                .iter()
                .take(2)
                .map(|m| {
                    format!(
                        "{} {:08x} x{:.2} {:.2} {:.2}",
                        m.bank, m.sound, m.ratio, m.peaks, m.shape
                    )
                })
                .collect();
            println!("{k:>6}  {}", line.join("  |  "));
        }
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

/// The spectrum's lowest frequency, Hz, steps an octave, and octaves.
const LOW: f32 = 50.0;
const STEPS: usize = 24;
const OCTAVES: usize = 7;
const BINS: usize = STEPS * OCTAVES;
/// How far a sound is slid against the clip, steps (an octave).
const SLIDE: i32 = STEPS as i32;
/// Samples a transform.
const FFT: usize = 8192;

/// A sound's power spectrum on a log-frequency scale, dB: as it is, and
/// its peaks (each step against the octave around it).
struct Spectrum {
    hash: u32,
    shape: Vec<f32>,
    peaks: Vec<f32>,
}

/// How well a bank's sound fits a clip, slid by `ratio`.
struct Match<'a> {
    bank: &'a str,
    sound: u32,
    ratio: f32,
    peaks: f32,
    shape: f32,
}

impl Spectrum {
    /// The spectrum of `frames` frames of `buffer` from frame `from`
    /// (channels mixed), the mean of half-overlapping Hann-windowed
    /// transforms; `None` if silent.
    fn of(buffer: &SoundBuffer, from: usize, frames: usize) -> Option<Self> {
        let ch = usize::from(buffer.channels.max(1));
        let end = (from + frames).min(buffer.frames());
        let mono: Vec<f64> = (from..end)
            .map(|i| {
                let f = &buffer.samples[i * ch..(i + 1) * ch];
                f.iter().map(|&v| f64::from(v)).sum::<f64>() / ch as f64
            })
            .collect();
        if mono.len() < 64 {
            return None;
        }
        let mut power = vec![0.0f64; FFT / 2];
        let mut count = 0;
        let mut at = 0;
        loop {
            let take = (mono.len() - at).min(FFT);
            let (mut re, mut im) = (vec![0.0f64; FFT], vec![0.0f64; FFT]);
            for (k, v) in mono[at..at + take].iter().enumerate() {
                let w = 0.5 - 0.5 * (std::f64::consts::TAU * k as f64 / take as f64).cos();
                re[k] = v * w;
            }
            fft(&mut re, &mut im);
            for (p, (r, i)) in power.iter_mut().zip(re.iter().zip(&im)) {
                *p += r * r + i * i;
            }
            count += 1;
            if at + FFT >= mono.len() {
                break;
            }
            at += FFT / 2;
        }
        if power.iter().all(|&p| p <= 0.0) {
            return None;
        }
        let hz = f64::from(buffer.rate) / FFT as f64;
        let edge = |b: f64| f64::from(LOW) * 2f64.powf(b / STEPS as f64);
        let shape: Vec<f32> = (0..BINS)
            .map(|b| {
                let (lo, hi) = (edge(b as f64 - 0.5), edge(b as f64 + 0.5));
                let (i0, i1) = ((lo / hz).ceil() as usize, (hi / hz).floor() as usize);
                let p = if i1 >= i0 && i1 < power.len() {
                    power[i0..=i1].iter().sum::<f64>() / (i1 - i0 + 1) as f64
                } else {
                    // Narrower than the transform's steps: between the two
                    // nearest.
                    let x = edge(b as f64) / hz;
                    let k = (x.floor() as usize).min(power.len() - 2);
                    let t = x - k as f64;
                    power[k] * (1.0 - t) + power[k + 1] * t
                };
                (10.0 * (p / count as f64 + 1e-9).log10()) as f32
            })
            .collect();
        let half = STEPS / 2;
        let peaks = (0..BINS)
            .map(|b| {
                let around = &shape[b.saturating_sub(half)..(b + half + 1).min(BINS)];
                shape[b] - around.iter().sum::<f32>() / around.len() as f32
            })
            .collect();
        Some(Self {
            hash: buffer.hash,
            shape,
            peaks,
        })
    }

    /// Every candidate's best fit to this spectrum, best first.
    fn best<'a>(&self, candidates: &'a [(&'a str, Spectrum)]) -> Vec<Match<'a>> {
        let mut out: Vec<Match<'a>> = candidates
            .iter()
            .map(|(bank, c)| {
                let (k, peaks) = (-SLIDE..=SLIDE)
                    .map(|k| (k, correlate(&self.peaks, &c.peaks, k)))
                    .max_by(|a, b| a.1.total_cmp(&b.1))
                    .expect("slides");
                Match {
                    bank,
                    sound: c.hash,
                    ratio: 2f32.powf(k as f32 / STEPS as f32),
                    peaks,
                    shape: correlate(&self.shape, &c.shape, k),
                }
            })
            .collect();
        out.sort_by(|a, b| (b.peaks + b.shape).total_cmp(&(a.peaks + a.shape)));
        out
    }
}

/// The correlation of `clip` with `sound` slid up `k` steps (sound step
/// `i` against clip step `i + k`), over the steps both have.
fn correlate(clip: &[f32], sound: &[f32], k: i32) -> f32 {
    let pairs: Vec<(f32, f32)> = (0..sound.len() as i32)
        .filter_map(|i| {
            let j = usize::try_from(i + k).ok()?;
            Some((*clip.get(j)?, sound[i as usize]))
        })
        .collect();
    let n = pairs.len() as f32;
    if n < 2.0 {
        return 0.0;
    }
    let (ma, mb) = pairs
        .iter()
        .fold((0.0, 0.0), |(a, b), p| (a + p.0 / n, b + p.1 / n));
    let (mut ab, mut aa, mut bb) = (0.0, 0.0, 0.0);
    for (a, b) in pairs {
        ab += (a - ma) * (b - mb);
        aa += (a - ma) * (a - ma);
        bb += (b - mb) * (b - mb);
    }
    if aa <= 0.0 || bb <= 0.0 {
        0.0
    } else {
        ab / (aa * bb).sqrt()
    }
}

/// An in-place radix-2 Fourier transform (`re.len()` a power of two).
fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let step = -std::f64::consts::TAU / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (wr, wi) = ((step * k as f64).cos(), (step * k as f64).sin());
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * wr - im[b] * wi, re[b] * wi + im[b] * wr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

fn print_matches(matches: &[Match<'_>], n: usize) {
    println!("  bank                  sound     ratio  peaks  shape");
    for m in matches.iter().take(n) {
        println!(
            "  {:<20}  {:08x}  x{:.2}  {:>5.2}  {:>5.2}",
            m.bank, m.sound, m.ratio, m.peaks, m.shape
        );
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// An engine-like note: a 40-harmonic buzz at `hz`, with harmonic
    /// strengths set by `colour`.
    fn buzz(
        hash: u32,
        hz: f32,
        rate: u32,
        seconds: f32,
        colour: impl Fn(usize) -> f32,
    ) -> SoundBuffer {
        let n = (rate as f32 * seconds) as usize;
        let samples = (0..n)
            .map(|i| {
                let t = i as f32 / rate as f32;
                let v: f32 = (1..=40)
                    .map(|h| colour(h) * (std::f32::consts::TAU * hz * h as f32 * t).sin())
                    .sum();
                (v * 600.0).clamp(-32_000.0, 32_000.0) as i16
            })
            .collect();
        SoundBuffer {
            hash,
            channels: 1,
            rate,
            samples,
        }
    }

    #[test]
    fn a_sound_is_found_at_its_pitch() {
        let a = buzz(1, 45.0, 22_050, 2.0, |h| 1.0 / h as f32);
        let b = buzz(2, 45.0, 22_050, 2.0, |h| if h % 3 == 0 { 1.0 } else { 0.1 });
        // The clip: sound 1 played 1.3 times as fast, at another rate.
        let clip = buzz(0, 45.0 * 1.3, 32_000, 3.0, |h| 1.0 / h as f32);
        let candidates = vec![
            ("bank", Spectrum::of(&a, 0, a.frames()).unwrap()),
            ("bank", Spectrum::of(&b, 0, b.frames()).unwrap()),
        ];
        let best = Spectrum::of(&clip, 0, clip.frames())
            .unwrap()
            .best(&candidates);
        assert_eq!(best[0].sound, 1);
        assert!(
            (best[0].ratio - 1.3).abs() < 0.04,
            "ratio {}",
            best[0].ratio
        );
        assert!(best[0].peaks > best[1].peaks);
    }

    #[test]
    fn the_transform_finds_a_tone() {
        let mut re: Vec<f64> = (0..64)
            .map(|i| (std::f64::consts::TAU * 5.0 * f64::from(i) / 64.0).cos())
            .collect();
        let mut im = vec![0.0; 64];
        fft(&mut re, &mut im);
        let mag: Vec<f64> = re.iter().zip(&im).map(|(r, i)| r.hypot(*i)).collect();
        let peak = (0..32).max_by(|&a, &b| mag[a].total_cmp(&mag[b])).unwrap();
        assert_eq!(peak, 5);
        assert!((mag[5] - 32.0).abs() < 1e-6);
    }
}
