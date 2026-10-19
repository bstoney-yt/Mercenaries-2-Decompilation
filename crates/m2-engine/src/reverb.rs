//! Environmental reverb: the preset the sound bootstrap defines, run over
//! the sounds placed in the world.
//!
//! `MrxSoundBootstrap.Init` defines one preset and turns it on for the
//! whole game, and no other script touches reverb:
//!
//! ```text
//! Sound.DefineReverbPreset(1, "CITY_KG_LIGHT_REFLECTIONS",
//!     -1000, -1000, 0, 0.09, 0.23, -602, 0.02, -698, 0.03, 100, 100, 5000,
//!     0, 5000, -5000)
//! Sound.SetReverbPreset("CITY_KG_LIGHT_REFLECTIONS")   -- library 10 and up
//! Sound.SetReverb(1)
//! ```
//!
//! The first twelve numbers are read as an I3DL2 reverb in the standard's
//! order ([`ReverbPreset`]): every value sits in its I3DL2 range, and
//! diffusion 100 %, density 100 % and HF reference 5000 Hz are the
//! standard's defaults. The last three are kept but not applied.
//!
//! The model is ordinary I3DL2 practice: the send is filtered by `RoomHF`
//! at the HF reference; early reflections are four taps from
//! `ReflectionsDelay` on; the late reverb is a four-line feedback delay
//! network from `ReflectionsDelay + ReverbDelay` on, whose lines lose
//! 60 dB over `DecayTime` (and over `DecayTime × DecayHFRatio` at the HF
//! reference). Each part's energy is set to its level (`Room +
//! Reflections`, `Room + Reverb`, in millibels) relative to the sound
//! sent.
//!
//! # Provenance
//!
//! - The preset's numbers and that it is on for the whole game: from the
//!   scripts (`mrxsoundbootstrap`, `vz.wad`).
//! - Reading them as I3DL2, in that order: inferred from their ranges and
//!   defaults; not checked against the Xbox 360 code. The meaning of the
//!   last three (0, 5000, -5000) is not known.
//! - Which sounds are sent (those placed in the world, not music, the
//!   interface or flat dialogue), the tap pattern and the delay network:
//!   invented.

/// A reverb preset as `Sound.DefineReverbPreset` takes it (I3DL2 order).
#[derive(Clone, Debug, PartialEq)]
pub struct ReverbPreset {
    pub name: String,
    /// Level of the whole effect, millibels.
    pub room: f32,
    /// Its level at the HF reference, relative to `room`, millibels.
    pub room_hf: f32,
    pub room_rolloff: f32,
    /// Seconds for the late reverb to fall 60 dB at low frequencies.
    pub decay_time: f32,
    /// High-frequency decay time over `decay_time`.
    pub decay_hf_ratio: f32,
    /// Early reflections' level relative to `room`, millibels.
    pub reflections: f32,
    /// Seconds from the sound to the first reflection.
    pub reflections_delay: f32,
    /// Late reverb's level relative to `room`, millibels.
    pub reverb: f32,
    /// Seconds from the first reflection to the late reverb.
    pub reverb_delay: f32,
    /// Percent.
    pub diffusion: f32,
    /// Percent.
    pub density: f32,
    /// Hz.
    pub hf_reference: f32,
    /// The three numbers after the I3DL2 ones (0, 5000, -5000 in the
    /// bootstrap's preset); not known, not applied.
    pub unk_extra: Vec<f32>,
}

impl ReverbPreset {
    /// A preset from `DefineReverbPreset`'s arguments after the index:
    /// the name, then at least twelve numbers.
    pub fn from_args(name: &str, numbers: &[f32]) -> Option<Self> {
        let n: [f32; 12] = numbers.get(..12)?.try_into().ok()?;
        Some(Self {
            name: name.to_owned(),
            room: n[0],
            room_hf: n[1],
            room_rolloff: n[2],
            decay_time: n[3],
            decay_hf_ratio: n[4],
            reflections: n[5],
            reflections_delay: n[6],
            reverb: n[7],
            reverb_delay: n[8],
            diffusion: n[9],
            density: n[10],
            hf_reference: n[11],
            unk_extra: numbers[12..].to_vec(),
        })
    }

    /// The bootstrap's preset, `CITY_KG_LIGHT_REFLECTIONS`.
    pub fn city_light_reflections() -> Self {
        Self::from_args(
            "CITY_KG_LIGHT_REFLECTIONS",
            &[
                -1000.0, -1000.0, 0.0, 0.09, 0.23, -602.0, 0.02, -698.0, 0.03, 100.0, 100.0,
                5000.0, 0.0, 5000.0, -5000.0,
            ],
        )
        .expect("twelve numbers")
    }
}

/// Millibels to an amplitude gain.
pub fn millibels(mb: f32) -> f32 {
    10f32.powf(mb / 2000.0)
}

/// The coefficient `b` of a one-pole low-pass `y = (1-b)x + b·y'` whose
/// gain is `gain` (at most 1) at `freq` Hz, unity at 0 Hz.
fn lowpass_coefficient(gain: f32, freq: f32, rate: f32) -> f32 {
    let g2 = (gain * gain).clamp(0.0, 1.0);
    if g2 >= 0.9999 {
        return 0.0;
    }
    let c = (std::f32::consts::TAU * freq / rate)
        .min(std::f32::consts::PI)
        .cos();
    let a = 1.0 - g2;
    let b = 1.0 - g2 * c;
    ((b - (b * b - a * a).max(0.0).sqrt()) / a).clamp(0.0, 0.999)
}

#[derive(Clone, Debug, Default)]
struct OnePole {
    b: f32,
    y: f32,
}

impl OnePole {
    fn run(&mut self, x: f32) -> f32 {
        self.y = (1.0 - self.b) * x + self.b * self.y;
        self.y
    }
}

/// A delay line of fixed length.
#[derive(Clone, Debug)]
struct Line {
    buf: Vec<f32>,
    at: usize,
}

impl Line {
    fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0; len.max(1)],
            at: 0,
        }
    }

    /// The sample written `back` samples ago (1 = the last one).
    fn tap(&self, back: usize) -> f32 {
        let n = self.buf.len();
        self.buf[(self.at + n - back.min(n)) % n]
    }

    fn push(&mut self, x: f32) {
        self.buf[self.at] = x;
        self.at = (self.at + 1) % self.buf.len();
    }
}

/// Early-reflection taps after the first, seconds (invented: spread over
/// 12 ms, primes of a millisecond apart so they don't comb).
const TAPS: [f32; 4] = [0.0, 0.0031, 0.0071, 0.0119];
/// The late network's line lengths, seconds (invented, mutually prime in
/// samples at the usual rates).
const LINES: [f32; 4] = [0.0113, 0.0137, 0.0161, 0.0191];

/// The reverb running at one sample rate.
#[derive(Clone, Debug)]
pub struct Reverb {
    pub preset: ReverbPreset,
    hf: OnePole,
    /// The send, delayed: early taps and the late input read from it.
    input: Line,
    taps: [usize; 4],
    late_at: usize,
    early_gain: f32,
    lines: [Line; 4],
    damp: [OnePole; 4],
    feedback: [f32; 4],
    late_gain: f32,
}

impl Reverb {
    pub fn new(preset: ReverbPreset, rate: u32) -> Self {
        let rate = rate.max(1) as f32;
        let samples = |s: f32| (s.max(0.0) * rate).round() as usize;
        let first = samples(preset.reflections_delay);
        let taps = TAPS.map(|t| first + samples(t));
        let late_at = first + samples(preset.reverb_delay);
        let room = millibels(preset.room);
        // Four taps sharing the reflections' energy.
        let early_gain = room * millibels(preset.reflections) / 2.0;
        let decay = preset.decay_time.max(0.01);
        let decay_hf = decay * preset.decay_hf_ratio.clamp(0.05, 2.0);
        let lines = LINES.map(|s| Line::new(samples(s)));
        let mut feedback = [0.0; 4];
        let mut damp: [OnePole; 4] = Default::default();
        for i in 0..4 {
            let len = lines[i].buf.len() as f32;
            let lf = 10f32.powf(-3.0 * len / (decay * rate));
            let hf = 10f32.powf(-3.0 * len / (decay_hf * rate));
            feedback[i] = lf;
            damp[i].b = lowpass_coefficient((hf / lf).min(1.0), preset.hf_reference, rate);
        }
        let hf = OnePole {
            b: lowpass_coefficient(millibels(preset.room_hf), preset.hf_reference, rate),
            y: 0.0,
        };
        let input = Line::new(late_at.max(taps[3]) + 2);
        let mut reverb = Self {
            preset,
            hf,
            input,
            taps,
            late_at,
            early_gain,
            lines,
            damp,
            feedback,
            late_gain: 1.0,
        };
        // Scale the network so an impulse's tail carries the late level's
        // energy.
        let energy = reverb.clone().late_energy(rate);
        reverb.late_gain = if energy > 0.0 {
            room * millibels(reverb.preset.reverb) / energy.sqrt()
        } else {
            0.0
        };
        reverb
    }

    /// The energy of the unscaled network's answer to a unit impulse.
    fn late_energy(&mut self, rate: f32) -> f32 {
        self.late_gain = 1.0;
        let n = (rate * (self.preset.decay_time.max(0.01) * 4.0 + 0.1)) as usize;
        (0..n)
            .map(|i| {
                let (l, r) = self.late(if i == 0 { 1.0 } else { 0.0 });
                (l * l + r * r) * 0.5
            })
            .sum()
    }

    /// One step of the late network fed `x`: its stereo output.
    fn late(&mut self, x: f32) -> (f32, f32) {
        let out = [0, 1, 2, 3].map(|i| self.lines[i].tap(self.lines[i].buf.len()));
        // Householder mixing: each line takes the others' back, energy kept.
        let sum = out.iter().sum::<f32>() * 0.5;
        for (((line, damp), feedback), o) in self
            .lines
            .iter_mut()
            .zip(&mut self.damp)
            .zip(self.feedback)
            .zip(out)
        {
            let back = damp.run((o - sum) * feedback);
            line.push(back + x * 0.5);
        }
        (
            (out[0] - out[1] + out[2] - out[3]) * 0.5 * self.late_gain,
            (out[0] + out[1] - out[2] - out[3]) * 0.5 * self.late_gain,
        )
    }

    /// Runs one sample of the send: the stereo sound to add.
    pub fn run(&mut self, x: f32) -> (f32, f32) {
        let x = self.hf.run(x);
        self.input.push(x);
        let t = self.taps.map(|d| self.input.tap(d + 1));
        let early = (
            (t[0] + t[2]) * self.early_gain * std::f32::consts::SQRT_2,
            (t[1] + t[3]) * self.early_gain * std::f32::consts::SQRT_2,
        );
        let late = self.late(self.input.tap(self.late_at + 1));
        (early.0 + late.0, early.1 + late.1)
    }

    /// Runs `send` (mono) and adds the result into `out` (interleaved
    /// stereo, as long again).
    pub fn process(&mut self, send: &[f32], out: &mut [f32]) {
        for (x, o) in send.iter().zip(out.chunks_exact_mut(2)) {
            let (l, r) = self.run(*x);
            o[0] += l;
            o[1] += r;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn impulse(preset: ReverbPreset, n: usize) -> Vec<(f32, f32)> {
        let mut r = Reverb::new(preset, RATE);
        (0..n)
            .map(|i| r.run(if i == 0 { 1.0 } else { 0.0 }))
            .collect()
    }

    fn energy(s: &[(f32, f32)]) -> f32 {
        s.iter().map(|(l, r)| (l * l + r * r) * 0.5).sum()
    }

    fn flat_hf() -> ReverbPreset {
        ReverbPreset {
            room_hf: 0.0,
            ..ReverbPreset::city_light_reflections()
        }
    }

    #[test]
    fn the_bootstrap_preset_reads_as_i3dl2() {
        let p = ReverbPreset::city_light_reflections();
        assert_eq!(p.room, -1000.0);
        assert_eq!(p.decay_time, 0.09);
        assert_eq!(p.reflections_delay, 0.02);
        assert_eq!(p.reverb_delay, 0.03);
        assert_eq!(p.hf_reference, 5000.0);
        assert_eq!(p.unk_extra, [0.0, 5000.0, -5000.0]);
        assert!(ReverbPreset::from_args("short", &[0.0; 11]).is_none());
    }

    #[test]
    fn reflections_and_late_reverb_start_at_their_delays() {
        let ir = impulse(flat_hf(), RATE as usize / 2);
        let first = ir.iter().position(|s| s.0 != 0.0 || s.1 != 0.0).unwrap();
        assert_eq!(first, 960, "first reflection 20 ms on");
        // The late part's first sound: 50 ms plus its shortest line.
        let late_start = 2400 + (LINES[0] * RATE as f32).round() as usize;
        assert!(
            ir[2400..late_start]
                .iter()
                .all(|s| s.0 == 0.0 && s.1 == 0.0)
        );
        assert!(ir[late_start].0 != 0.0 || ir[late_start].1 != 0.0);
    }

    #[test]
    fn each_part_carries_its_level() {
        let ir = impulse(flat_hf(), RATE as usize / 2);
        // Reflections: the 12 ms of taps; late: from 50 ms on.
        let early = energy(&ir[..2400]);
        let late = energy(&ir[2400..]);
        let want_early = 10f32.powf((-1000.0 - 602.0) / 1000.0);
        let want_late = 10f32.powf((-1000.0 - 698.0) / 1000.0);
        assert!(
            (early / want_early - 1.0).abs() < 0.01,
            "{early} vs {want_early}"
        );
        assert!(
            (late / want_late - 1.0).abs() < 0.05,
            "{late} vs {want_late}"
        );
    }

    #[test]
    fn the_tail_falls_60_db_over_the_decay_time() {
        let ir = impulse(
            ReverbPreset {
                decay_hf_ratio: 1.0,
                ..flat_hf()
            },
            RATE as usize,
        );
        let tail: Vec<f32> = ir[2400..]
            .iter()
            .map(|(l, r)| (l * l + r * r) * 0.5)
            .collect();
        // Schroeder: energy still to come, in dB; T60 from -5 to -25 dB.
        let mut left: Vec<f32> = tail.clone();
        for i in (0..left.len() - 1).rev() {
            left[i] += left[i + 1];
        }
        let total = left[0];
        let db = |i: usize| 10.0 * (left[i] / total).log10();
        let at = |d: f32| (0..left.len()).find(|&i| db(i) <= d).unwrap();
        let t60 = (at(-25.0) - at(-5.0)) as f32 / RATE as f32 * 3.0;
        assert!((t60 / 0.09 - 1.0).abs() < 0.15, "T60 {t60}");
    }

    #[test]
    fn room_hf_cuts_the_send_at_the_hf_reference() {
        let rate = RATE as f32;
        for (freq, want) in [(5000.0, millibels(-1000.0)), (50.0, 1.0)] {
            let mut f = OnePole {
                b: lowpass_coefficient(millibels(-1000.0), 5000.0, rate),
                y: 0.0,
            };
            let peak = (0..RATE as usize)
                .map(|i| f.run((std::f32::consts::TAU * freq * i as f32 / rate).sin()))
                .skip(RATE as usize / 2)
                .fold(0f32, |m, y| m.max(y.abs()));
            assert!(
                (peak / want - 1.0).abs() < 0.03,
                "{freq} Hz: {peak} vs {want}"
            );
        }
    }
}
