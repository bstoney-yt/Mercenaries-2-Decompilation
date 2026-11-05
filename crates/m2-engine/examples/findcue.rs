//! Finds where a cue's sound is heard in a recording of play: writes the
//! cue's sounds as WAV files into a folder of yours, then, given a mono
//! 16-bit WAV of the recording (at the sound's own rate), prints each
//! time the sound matches it (normalised cross-correlation over the
//! sound's length, peaks above a threshold, at least a second apart).
//!
//! ```text
//! cargo run --release -p m2-engine --example findcue -- "<install dir>" <out dir> <cue hex> [recording.wav] [threshold]
//! ```
//!
//! The sounds are of your own install's data, and the recording is
//! yours: keep both local. Prints times and scores only.

use std::io::{Read, Write};
use std::path::Path;

use m2_engine::Install;
use m2_engine::audio::{KeySounds, world_cue_tables};

fn wav(path: &Path, rate: u32, channels: u16, samples: &[i16]) -> std::io::Result<()> {
    let mut f = std::fs::File::create(path)?;
    let data = (samples.len() * 2) as u32;
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&channels.to_le_bytes())?;
    f.write_all(&rate.to_le_bytes())?;
    f.write_all(&(rate * u32::from(channels) * 2).to_le_bytes())?;
    f.write_all(&(channels * 2).to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data.to_le_bytes())?;
    for s in samples {
        f.write_all(&s.to_le_bytes())?;
    }
    Ok(())
}

/// A mono 16-bit WAV's samples (its `data` chunk), as floats.
fn read_wav(path: &Path) -> std::io::Result<Vec<f32>> {
    let mut b = Vec::new();
    std::fs::File::open(path)?.read_to_end(&mut b)?;
    let mut i = 12;
    while i + 8 <= b.len() {
        let len = u32::from_le_bytes([b[i + 4], b[i + 5], b[i + 6], b[i + 7]]) as usize;
        if &b[i..i + 4] == b"data" {
            let end = (i + 8 + len).min(b.len());
            return Ok(b[i + 8..end]
                .chunks_exact(2)
                .map(|c| f32::from(i16::from_le_bytes([c[0], c[1]])))
                .collect());
        }
        i += 8 + len + (len & 1);
    }
    Ok(Vec::new())
}

/// In-place radix-2 FFT of `re`/`im` (length a power of two); `inverse`
/// runs it backwards, unscaled.
fn fft(re: &mut [f64], im: &mut [f64], inverse: bool) {
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
        let ang = 2.0 * std::f64::consts::PI / len as f64 * if inverse { 1.0 } else { -1.0 };
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0, 0.0);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * cr - im[b] * ci;
                let ti = re[b] * ci + im[b] * cr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let ncr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = ncr;
            }
        }
        len <<= 1;
    }
}

/// Normalised cross-correlation of template `t` against `signal` at
/// every lag (overlap-save by FFT).
fn ncc(signal: &[f32], t: &[f32]) -> Vec<f64> {
    let m = t.len();
    if m == 0 || signal.len() < m {
        return Vec::new();
    }
    let mean = t.iter().map(|&v| f64::from(v)).sum::<f64>() / m as f64;
    let tz: Vec<f64> = t.iter().map(|&v| f64::from(v) - mean).collect();
    let tnorm = tz.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-9);
    let n = (4 * m).next_power_of_two().max(1 << 15);
    let step = n - m + 1;
    let (mut tr, mut ti) = (vec![0.0; n], vec![0.0; n]);
    tr[..m].copy_from_slice(&tz);
    fft(&mut tr, &mut ti, false);
    let mut csum = vec![0.0f64; signal.len() + 1];
    let mut csq = vec![0.0f64; signal.len() + 1];
    for (i, &v) in signal.iter().enumerate() {
        let v = f64::from(v);
        csum[i + 1] = csum[i] + v;
        csq[i + 1] = csq[i] + v * v;
    }
    let mut scores = vec![0.0f64; signal.len() - m + 1];
    let mut start = 0;
    while start < scores.len() {
        let (mut sr, mut si) = (vec![0.0; n], vec![0.0; n]);
        for (i, s) in sr.iter_mut().enumerate() {
            if let Some(&v) = signal.get(start + i) {
                *s = f64::from(v);
            }
        }
        fft(&mut sr, &mut si, false);
        for i in 0..n {
            let (a, b) = (sr[i], si[i]);
            let (c, d) = (tr[i], -ti[i]);
            sr[i] = a * c - b * d;
            si[i] = a * d + b * c;
        }
        fft(&mut sr, &mut si, true);
        for (lag, &value) in sr.iter().enumerate().take(step) {
            let at = start + lag;
            if at >= scores.len() {
                break;
            }
            let s = csum[at + m] - csum[at];
            let q = csq[at + m] - csq[at];
            let spread = (q - s * s / m as f64).max(0.0).sqrt();
            if spread > 1e-3 {
                scores[at] = value / n as f64 / (spread * tnorm);
            }
        }
        start += step;
    }
    scores
}

/// Hop and window of the band frames, samples.
const HOP: usize = 256;
const WINDOW: usize = 512;
/// Bands per frame, spaced evenly over the window's spectrum's bins 4..
const BANDS: usize = 24;

/// Each frame's log energy per band.
fn bands(x: &[f32]) -> Vec<[f32; BANDS]> {
    let mut out = Vec::new();
    let mut start = 0;
    let bins = WINDOW / 2;
    let per = (bins - 4) / BANDS;
    while start + WINDOW <= x.len() {
        let (mut re, mut im) = (vec![0.0f64; WINDOW], vec![0.0f64; WINDOW]);
        for (i, r) in re.iter_mut().enumerate() {
            let w = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / WINDOW as f64).cos();
            *r = f64::from(x[start + i]) * w;
        }
        fft(&mut re, &mut im, false);
        let mut f = [0.0f32; BANDS];
        for (b, e) in f.iter_mut().enumerate() {
            let s: f64 = (4 + b * per..4 + (b + 1) * per)
                .map(|i| re[i] * re[i] + im[i] * im[i])
                .sum();
            *e = (s + 1.0).ln() as f32;
        }
        out.push(f);
        start += HOP;
    }
    out
}

/// Pearson correlation of the template's band frames `t` with the
/// recording's `r` at each frame lag.
fn band_match(r: &[[f32; BANDS]], t: &[[f32; BANDS]]) -> Vec<f64> {
    let m = t.len();
    if m == 0 || r.len() < m {
        return Vec::new();
    }
    let flat_t: Vec<f64> = t.iter().flatten().map(|&v| f64::from(v)).collect();
    let mt = flat_t.iter().sum::<f64>() / flat_t.len() as f64;
    let tz: Vec<f64> = flat_t.iter().map(|v| v - mt).collect();
    let tn = tz.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-9);
    (0..=r.len() - m)
        .map(|at| {
            let w: Vec<f64> = r[at..at + m]
                .iter()
                .flatten()
                .map(|&v| f64::from(v))
                .collect();
            let mw = w.iter().sum::<f64>() / w.len() as f64;
            let (mut dot, mut wn) = (0.0, 0.0);
            for (a, b) in w.iter().zip(&tz) {
                let a = a - mw;
                dot += a * b;
                wn += a * a;
            }
            dot / (wn.sqrt().max(1e-9) * tn)
        })
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [root, out, cue, rest @ ..] = &args[..] else {
        return Err(
            "usage: findcue <install dir> <out dir> <cue hex> [recording.wav] [threshold]".into(),
        );
    };
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    // A cue's sounds, or with `bank:<name>` every sound of that bank.
    let sounds: Vec<m2_engine::SoundBuffer> = if let Some(bank) = cue.strip_prefix("bank:") {
        let library = m2_engine::SoundLibrary::load(&mut source, &[bank])?;
        library.bank(bank).to_vec()
    } else {
        let hash = u32::from_str_radix(cue.trim_start_matches("0x"), 16)?;
        let tables = world_cue_tables(&mut source);
        let found = KeySounds::resolve(&mut source, &tables, hash).ok_or("cue finds no sound")?;
        found.sounds.iter().map(|(s, _)| s.clone()).collect()
    };
    let cue = cue.replace(':', "_");
    // Sounds taken every `decimate`th sample (averaged), for a recording
    // read at that fraction of their rate.
    let decimate: usize = std::env::var("FINDCUE_DECIMATE")
        .ok()
        .and_then(|d| d.parse().ok())
        .unwrap_or(1)
        .max(1);
    let mut templates = Vec::new();
    for (k, s) in sounds.iter().enumerate() {
        if s.frames() == 0 {
            continue;
        }
        let path = Path::new(out).join(format!("cue_{cue}_{k}_{:08x}.wav", s.hash));
        wav(&path, s.rate, s.channels, &s.samples)?;
        println!(
            "sound {k}: {:08x}, {} Hz, {} ch, {:.2} s -> {}",
            s.hash,
            s.rate,
            s.channels,
            s.seconds(),
            path.display()
        );
        let ch = usize::from(s.channels.max(1));
        let mono: Vec<f32> = s
            .samples
            .chunks(ch * decimate)
            .map(|c| c.iter().map(|&v| f32::from(v)).sum::<f32>() / c.len() as f32)
            .collect();
        templates.push((k, s.rate / decimate as u32, mono));
    }
    let Some(rec) = rest.first() else {
        return Ok(());
    };
    let threshold: f64 = rest.get(1).and_then(|t| t.parse().ok()).unwrap_or(0.5);
    let signal = read_wav(Path::new(rec))?;
    println!("recording: {} samples", signal.len());
    // `FINDCUE_SPECTRAL` matches log band energies instead of the wave:
    // steadier where the sound is heard under others.
    let spectral = std::env::var_os("FINDCUE_SPECTRAL").is_some();
    let rec_bands = spectral.then(|| bands(&signal));
    for (k, rate, t) in templates {
        let (scores, per_second) = match &rec_bands {
            Some(r) => (
                band_match(r, &bands(&t)),
                f64::from(rate) / HOP as f64,
            ),
            None => (ncc(&signal, &t), f64::from(rate)),
        };
        let rate = per_second;
        let gap = rate as usize;
        let (mut i, mut hits) = (0, 0);
        while i < scores.len() {
            if scores[i] >= threshold {
                let end = (i + gap).min(scores.len());
                let (best, score) = scores[i..end]
                    .iter()
                    .copied()
                    .enumerate()
                    .max_by(|a, b| a.1.total_cmp(&b.1))
                    .unwrap_or((0, 0.0));
                let at = (i + best) as f64 / rate;
                println!(
                    "sound {k}: at {:02}:{:05.2} score {score:.2}",
                    (at / 60.0) as u32,
                    at % 60.0
                );
                hits += 1;
                i += best + gap;
            } else {
                i += 1;
            }
        }
        println!("sound {k}: {hits} matches at {threshold}");
    }
    Ok(())
}
