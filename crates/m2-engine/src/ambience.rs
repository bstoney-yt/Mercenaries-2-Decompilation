//! Zone ambience: what an ambience cue plays, from the game's `ambience`
//! cue table, and when its one-shots come.
//!
//! The table (`ambience`, in `sound_resident`) keys a cue per region kind
//! (`amb_jungle`, `amb_citymed`, `amb_ocean`, ... 17 of them) as a looped
//! mixed cue: one track whose record plays a stream of `ambience.pws` (its
//! variant names the table itself, and the stream's index) from 0 s, and
//! up to six more whose records each pick one of a bank's short sounds by
//! weight (`amb_jungle`'s records name `amb_jungle`'s sounds 4 to 15, and
//! one names `amb_shared`'s). One more region cue (`5eb93649`, name not
//! known) is a plain looped cue playing a stream alone.
//!
//! [`AmbiencePlan::of`] reads a cue into its bed and layers;
//! [`Layers`] decides when each layer fires.
//!
//! # Provenance
//!
//! - The cues, their tracks, records, volumes (0.30 on every record),
//!   distances and the variants' weights: from the data (`vz.wad`, the
//!   survey example's `ambience` section).
//! - That the stream track is the bed and loops: from the data (it alone
//!   names the stream table; every region cue is a loop, -1 s).
//! - What a one-shot track's start and weight mean in a looped mix is not
//!   known (on a gun's shot the start is a delay within the shot). They
//!   are taken as the mean seconds between its sounds and the chance each
//!   comes (inferred, invented in detail: each wait is drawn from half to
//!   one and a half times the start). Not checked against the game's code.
//! - Where the one-shots sound from (10 to 40 m around the listener) is
//!   invented, as before.

use m2_assets::CueTable;

/// A bed stream: which stream of `ambience.pws`, and how loud.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bed {
    pub stream: usize,
    pub volume: f32,
}

/// A layer of one-shots: a record's sounds by weight, every so often.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    /// Mean seconds between its sounds (the track's start).
    pub every: f32,
    /// The chance each comes (the track's weight).
    pub chance: f32,
    pub volume: f32,
    pub min_distance: f32,
    pub max_distance: f32,
    /// (bank hash, sound index, weight).
    pub sounds: Vec<(u32, u32, f32)>,
}

impl Layer {
    /// The sound `pick` (0 to 1) lands on by weight.
    pub fn pick(&self, pick: f32) -> Option<(u32, u32)> {
        let total: f32 = self.sounds.iter().map(|s| s.2.max(0.0)).sum();
        if self.sounds.is_empty() {
            return None;
        }
        if total <= 0.0 {
            let i = ((pick * self.sounds.len() as f32) as usize).min(self.sounds.len() - 1);
            return Some((self.sounds[i].0, self.sounds[i].1));
        }
        let mut left = pick.clamp(0.0, 1.0) * total;
        for s in &self.sounds {
            left -= s.2.max(0.0);
            if left <= 0.0 {
                return Some((s.0, s.1));
            }
        }
        self.sounds.last().map(|s| (s.0, s.1))
    }
}

/// Shortest mean wait a layer is given, seconds (a track starting at 0
/// that is not the bed would otherwise fire every frame; invented).
const SHORTEST: f32 = 1.0;

/// What an ambience cue plays.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AmbiencePlan {
    pub cue: u32,
    pub bed: Option<Bed>,
    pub layers: Vec<Layer>,
}

impl AmbiencePlan {
    /// Cue `cue` of `table` (the `ambience` table), if the table has it.
    pub fn of(table: &CueTable, cue: u32) -> Option<Self> {
        let bed_of = |record: &m2_assets::voice::Record| {
            record
                .variants
                .iter()
                .find(|v| v.0 == table.hash)
                .map(|v| Bed {
                    stream: v.1 as usize,
                    volume: record.volume,
                })
        };
        if let Some(m) = table.mix(cue) {
            let mut plan = Self {
                cue,
                ..Self::default()
            };
            for t in m.tracks.iter().filter(|t| t.table == table.hash) {
                let Some(r) = table.records.get(t.record as usize) else {
                    continue;
                };
                match bed_of(r) {
                    Some(bed) if plan.bed.is_none() => plan.bed = Some(bed),
                    Some(_) => {}
                    None => plan.layers.push(Layer {
                        every: t.start.max(SHORTEST),
                        chance: t.weight.clamp(0.0, 1.0),
                        volume: r.volume * t.level,
                        min_distance: r.min_distance,
                        max_distance: r.max_distance,
                        sounds: r.variants.clone(),
                    }),
                }
            }
            return Some(plan);
        }
        let r = table.cue_record(cue)?;
        Some(Self {
            cue,
            bed: bed_of(r),
            layers: Vec::new(),
        })
    }

    /// The banks its layers play from.
    pub fn banks(&self) -> Vec<u32> {
        let mut b: Vec<u32> = self
            .layers
            .iter()
            .flat_map(|l| l.sounds.iter().map(|s| s.0))
            .collect();
        b.sort_unstable();
        b.dedup();
        b
    }
}

/// When each of a plan's layers fires.
#[derive(Clone, Debug, Default)]
pub struct Layers {
    /// Seconds until each layer's next try.
    wait: Vec<f32>,
}

impl Layers {
    /// Starts `plan`'s layers, each a draw of its wait from now.
    pub fn new(plan: &AmbiencePlan, mut random: impl FnMut() -> f32) -> Self {
        Self {
            wait: plan
                .layers
                .iter()
                .map(|l| l.every * (0.5 + random()))
                .collect(),
        }
    }

    /// Moves on `dt` seconds: the layers whose sound comes now.
    pub fn step(
        &mut self,
        plan: &AmbiencePlan,
        dt: f32,
        mut random: impl FnMut() -> f32,
    ) -> Vec<usize> {
        let mut out = Vec::new();
        for (i, (w, l)) in self.wait.iter_mut().zip(&plan.layers).enumerate() {
            *w -= dt;
            if *w <= 0.0 {
                *w += l.every * (0.5 + random());
                if random() < l.chance {
                    out.push(i);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_assets::voice::{MixedCue, PlainCue, Record, Track};

    const TABLE: u32 = 0x7871_f925;
    const JUNGLE: u32 = 0xcab2_a2ad;

    fn record(variants: Vec<(u32, u32, f32)>) -> Record {
        Record {
            cue: 0,
            volume: 0.3,
            min_distance: 10.0,
            max_distance: 1000.0,
            variants,
        }
    }

    fn track(record: u32, start: f32, weight: f32) -> Track {
        Track {
            table: TABLE,
            record,
            start,
            weight,
            level: 1.0,
        }
    }

    /// `amb_jungle` as the install's table has it, and the plain cue.
    fn table() -> CueTable {
        CueTable {
            hash: TABLE,
            lines: Vec::new(),
            groups: Vec::new(),
            records: vec![
                record(vec![(TABLE, 12, 1.0)]),
                record((7..12).map(|i| (JUNGLE, i, 0.2)).collect()),
                record(vec![(0x7250_8dc1, 2, 0.5), (0x7250_8dc1, 3, 0.5)]),
                record(vec![(TABLE, 9, 1.0)]),
            ],
            cues: vec![PlainCue {
                cue: 0x5eb9_3649,
                record: 3,
                a: 0.32,
                seconds: -1.0,
            }],
            mixes: vec![MixedCue {
                cue: JUNGLE,
                seconds: -1.0,
                tracks: vec![
                    track(0, 0.0, 1.0),
                    track(1, 5.11, 1.0),
                    track(2, 26.97, 0.25),
                ],
            }],
        }
    }

    #[test]
    fn a_region_cue_is_a_stream_bed_and_its_layers() {
        let t = table();
        let p = AmbiencePlan::of(&t, JUNGLE).unwrap();
        assert_eq!(
            p.bed,
            Some(Bed {
                stream: 12,
                volume: 0.3
            })
        );
        assert_eq!(p.layers.len(), 2);
        assert_eq!((p.layers[0].every, p.layers[0].chance), (5.11, 1.0));
        assert_eq!((p.layers[1].every, p.layers[1].chance), (26.97, 0.25));
        assert_eq!(p.banks(), [0x7250_8dc1, JUNGLE]);
        let plain = AmbiencePlan::of(&t, 0x5eb9_3649).unwrap();
        assert_eq!(plain.bed.map(|b| b.stream), Some(9));
        assert!(plain.layers.is_empty());
        assert!(AmbiencePlan::of(&t, 1).is_none());
        // Picks by weight.
        assert_eq!(p.layers[0].pick(0.0), Some((JUNGLE, 7)));
        assert_eq!(p.layers[0].pick(0.5), Some((JUNGLE, 9)));
        assert_eq!(p.layers[0].pick(1.0), Some((JUNGLE, 11)));
    }

    #[test]
    fn layers_come_about_as_often_as_their_start_and_weight_say() {
        let p = AmbiencePlan::of(&table(), JUNGLE).unwrap();
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut random = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 40) as f32 / (1u64 << 24) as f32
        };
        let mut layers = Layers::new(&p, &mut random);
        let mut counts = [0usize; 2];
        let (dt, seconds) = (1.0 / 60.0, 100_000.0);
        for _ in 0..(seconds / dt) as usize {
            for i in layers.step(&p, dt, &mut random) {
                counts[i] += 1;
            }
        }
        // 100,000 s: 19,569 of the first (every 5.11 s), 927 of the second
        // (a quarter of every 26.97 s).
        let want = [seconds / 5.11, seconds / 26.97 * 0.25];
        for (n, w) in counts.iter().zip(want) {
            assert!((*n as f32 / w - 1.0).abs() < 0.08, "{counts:?} vs {want:?}");
        }
    }
}
