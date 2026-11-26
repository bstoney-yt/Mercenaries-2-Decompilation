//! The Gallery page's music: every piece of the game's score, read from
//! the player's own install when the page is opened, and played through
//! the launcher's own sound output without ffmpeg (the native Xbox ADPCM
//! decoder in `m2-assets`). Nothing is extracted, cached as a file or
//! committed.
//!
//! - **The list** (from samples): the `music` cue table in `vz.wad` names
//!   which stream of `data\Audios\music.pws` each record plays. Each
//!   stream is one row, keyed by the cue that plays it: a plain cue
//!   pointing at the record, else the record's own cue, else a group
//!   naming the stream.
//! - **Names** (from samples): the tables hold only name hashes. The cue
//!   names (`mu_fac_pmc_explore_01`) are the string constants of the
//!   install's scripts starting with `mu_`, matched by their hash
//!   ([`names_from_strings`]).
//! - **Readable labels** (**invented**): `mu_fac_pmc_explore_01` reads
//!   "PMC: explore 1" by our own rule ([`readable`]); a known name that
//!   doesn't fit the rule shows as itself; a stream with no known name
//!   shows as "Track <n>".
//! - **Locks**: a save holds no record of which music was heard, so every
//!   piece is shown unlocked, and the page says so (decision of
//!   2026-11-26, `docs/gallery.md`).

use std::collections::HashMap;

use m2_assets::{CueTable, StreamIndex, name_hash};

/// One piece of music on offer.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    /// The stream's index in `music.pws`.
    pub stream: usize,
    /// The cue that plays it (its name hash), 0 if none does.
    pub cue: u32,
    /// The cue's name from the install's scripts, if found.
    pub name: Option<String>,
    /// What the page shows.
    pub label: String,
    /// The group heading it is listed under.
    pub group: String,
    pub seconds: f32,
}

/// The cue names among `strings` (the scripts' string constants), by
/// their name hash.
pub fn names_from_strings<'a>(strings: impl IntoIterator<Item = &'a str>) -> HashMap<u32, String> {
    strings
        .into_iter()
        .filter(|s| s.len() > 3 && s.len() < 64 && s.to_ascii_lowercase().starts_with("mu_"))
        .filter(|s| s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
        .map(|s| (name_hash(s), s.to_ascii_lowercase()))
        .collect()
}

/// Our own name for a faction's or area's short form (invented).
fn side(short: &str) -> String {
    match short {
        "pmc" => "PMC".into(),
        "ch" | "chi" => "China".into(),
        "al" | "an" | "all" => "Allied Nations".into(),
        "pr" | "pir" => "Pirates".into(),
        "up" | "oil" => "Universal Petroleum".into(),
        "plav" | "gur" => "PLAV".into(),
        "vz" | "vza" => "Venezuela".into(),
        "civ" => "Civilians".into(),
        other => {
            let mut c = other.chars();
            c.next()
                .map(|f| f.to_ascii_uppercase().to_string() + c.as_str())
                .unwrap_or_default()
        }
    }
}

/// "explore_01" as "explore 1".
fn words(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|p| match p.parse::<u32>() {
            Ok(n) => n.to_string(),
            Err(_) => (*p).to_owned(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A cue name's readable label and the group it goes under (invented
/// rule): `mu_fac_<side>_<what>` "<Side>: <what>", `mu_src_<side>_...`
/// "<Side> radio: ...", `mu_freeplay_<area>_...` "Free play: <area>, ...",
/// `mu_shell_<n>` "Menus <n>"; anything else is its own name, under
/// "Other".
pub fn readable(name: &str) -> (String, String) {
    let parts: Vec<&str> = name
        .trim_start_matches("mu_")
        .split('_')
        .filter(|p| !p.is_empty())
        .collect();
    match parts.as_slice() {
        ["fac", s, rest @ ..] if !rest.is_empty() => {
            let s = side(s);
            (format!("{s}: {}", words(rest)), s)
        }
        ["src", s, rest @ ..] => {
            let s = side(s);
            let what = if rest.is_empty() {
                "radio".to_owned()
            } else {
                format!("radio, {}", words(rest))
            };
            (format!("{s}: {what}"), format!("{s} radio"))
        }
        ["freeplay", area, rest @ ..] => {
            let what = if rest.is_empty() {
                String::new()
            } else {
                format!(", {}", words(rest))
            };
            (format!("Free play: {area}{what}"), "Free play".into())
        }
        ["shell", rest @ ..] => (
            format!("Menus {}", words(rest)).trim_end().to_owned(),
            "Menus".into(),
        ),
        _ => (name.to_owned(), "Other".into()),
    }
}

/// Every stream `table` plays from `index` (its `music.pws`), one row
/// each, named from `names`; sorted by group, then label.
pub fn tracks(table: &CueTable, index: &StreamIndex, names: &HashMap<u32, String>) -> Vec<Track> {
    // Stream -> the cue that plays it, best first.
    let mut cue_of: HashMap<usize, u32> = HashMap::new();
    let mut offer = |stream: usize, cue: u32, strong: bool| {
        if cue == 0 {
            cue_of.entry(stream).or_insert(0);
            return;
        }
        let e = cue_of.entry(stream).or_insert(cue);
        if strong || *e == 0 {
            *e = cue;
        }
    };
    for (i, r) in table.records.iter().enumerate() {
        for &(bank, stream, _) in &r.variants {
            if bank != table.hash {
                continue;
            }
            let plain = table
                .cues
                .iter()
                .find(|c| c.record as usize == i)
                .map(|c| c.cue);
            match plain {
                Some(c) => offer(stream as usize, c, true),
                None => offer(stream as usize, r.cue, false),
            }
        }
    }
    for g in &table.groups {
        for &(stream, _) in &g.streams {
            offer(stream as usize, g.cue, false);
        }
    }
    let mut out: Vec<Track> = cue_of
        .into_iter()
        .filter_map(|(stream, cue)| {
            let s = index.streams.get(stream)?;
            let name = names.get(&cue).cloned();
            let (label, group) = match &name {
                Some(n) => readable(n),
                None => (format!("Track {stream}"), "Unnamed".to_owned()),
            };
            Some(Track {
                stream,
                cue,
                name,
                label,
                group,
                seconds: s.seconds(),
            })
        })
        .collect();
    out.sort_by(|a, b| {
        (a.group == "Unnamed")
            .cmp(&(b.group == "Unnamed"))
            .then(a.group.cmp(&b.group))
            .then(a.label.cmp(&b.label))
            .then(a.stream.cmp(&b.stream))
    });
    out
}

/// `seconds` as `m:ss`.
pub fn length(seconds: f32) -> String {
    let s = seconds.max(0.0).round() as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

/// What the page needs from the install, read on a thread.
pub struct Music {
    pub tracks: Vec<Track>,
    pub streams: m2_engine::audio::Streams,
}

impl std::fmt::Debug for Music {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Music")
            .field("tracks", &self.tracks.len())
            .finish_non_exhaustive()
    }
}

/// Reads the music table, the stream index and the scripts' cue names
/// from the install.
pub fn load(install: &m2_engine::Install) -> Result<Music, String> {
    use m2_assets::BlockContents;
    use m2_assets::lua::{Chunk, SCRIPT_TYPE};
    let mut source = install.open_world().map_err(|e| e.to_string())?;
    let streams =
        m2_engine::audio::Streams::load(install, &mut source).map_err(|e| e.to_string())?;
    let table = source
        .asset(name_hash("music"), m2_assets::voice::CUE_TABLE_TYPE)
        .map_err(|e| e.to_string())
        .and_then(|d| CueTable::parse(&d).map_err(|e| e.to_string()))?;
    // The scripts live in the `scripts` and `resident` blocks.
    let blocks: Vec<usize> = {
        let index = source.archive().index();
        (0..index.blocks().len())
            .filter(|&b| {
                let p = index.blocks()[b].path.to_ascii_lowercase();
                p.contains("scripts") || p.contains("resident")
            })
            .collect()
    };
    let mut names = HashMap::new();
    for b in blocks {
        let Ok(decoded) = source.block(b) else {
            continue;
        };
        let Ok(contents) = BlockContents::parse(&decoded) else {
            continue;
        };
        for (e, data) in contents.iter() {
            if e.type_hash != SCRIPT_TYPE {
                continue;
            }
            if let Ok(chunk) = Chunk::from_asset(data) {
                names.extend(names_from_strings(chunk.main.strings()));
            }
        }
    }
    let tracks = tracks(&table, &streams.music, &names);
    Ok(Music { tracks, streams })
}

/// The launcher's sound output while a piece plays: dropped, it stops.
pub struct Player {
    #[cfg(windows)]
    _stream: cpal::Stream,
    pub stream: usize,
    started: std::time::Instant,
    pub seconds: f32,
}

impl std::fmt::Debug for Player {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Player")
            .field("stream", &self.stream)
            .finish_non_exhaustive()
    }
}

/// `samples` (interleaved, `from_channels` at `from_rate`) as `to_channels`
/// at `to_rate`, by linear interpolation: the first two channels kept,
/// mono copied to both, others silent.
pub fn convert(
    samples: &[i16],
    from_channels: u16,
    from_rate: u32,
    to_channels: u16,
    to_rate: u32,
) -> Vec<f32> {
    let fc = usize::from(from_channels.max(1));
    let tc = usize::from(to_channels.max(1));
    let frames = samples.len() / fc;
    if frames == 0 || from_rate == 0 || to_rate == 0 {
        return Vec::new();
    }
    let out_frames = (frames as u64 * u64::from(to_rate) / u64::from(from_rate)) as usize;
    let step = f64::from(from_rate) / f64::from(to_rate);
    let at =
        |f: usize, c: usize| f32::from(samples[f.min(frames - 1) * fc + c.min(fc - 1)]) / 32768.0;
    let mut out = Vec::with_capacity(out_frames * tc);
    for o in 0..out_frames {
        let pos = o as f64 * step;
        let i = pos as usize;
        let t = (pos - i as f64) as f32;
        for c in 0..tc {
            let v = if c < 2 {
                at(i, c) * (1.0 - t) + at(i + 1, c) * t
            } else {
                0.0
            };
            out.push(v);
        }
    }
    out
}

impl Player {
    /// Decodes stream `stream` and starts it on the default output.
    #[cfg(windows)]
    pub fn start(music: &Music, stream: usize) -> Result<Self, String> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let buffer = music
            .streams
            .decode(&music.streams.music, stream)
            .map_err(|e| e.to_string())?;
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("no sound output device")?;
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let pcm = convert(
            &buffer.samples,
            buffer.channels,
            buffer.rate,
            config.channels,
            config.sample_rate,
        );
        let seconds = buffer.frames() as f32 / buffer.rate.max(1) as f32;
        let pcm = std::sync::Arc::new(pcm);
        let mut at = 0usize;
        let error = |e| eprintln!("gallery: sound: {e}");
        let s = match format {
            cpal::SampleFormat::F32 => {
                let pcm = pcm.clone();
                device.build_output_stream::<f32, _, _>(
                    config,
                    move |data, _| {
                        for d in data.iter_mut() {
                            *d = pcm.get(at).copied().unwrap_or(0.0);
                            at += 1;
                        }
                    },
                    error,
                    None,
                )
            }
            cpal::SampleFormat::I16 => {
                let pcm = pcm.clone();
                device.build_output_stream::<i16, _, _>(
                    config,
                    move |data, _| {
                        for d in data.iter_mut() {
                            *d = (pcm.get(at).copied().unwrap_or(0.0) * 32767.0) as i16;
                            at += 1;
                        }
                    },
                    error,
                    None,
                )
            }
            other => return Err(format!("unsupported sample format {other:?}")),
        }
        .map_err(|e| e.to_string())?;
        s.play().map_err(|e| e.to_string())?;
        Ok(Self {
            _stream: s,
            stream,
            started: std::time::Instant::now(),
            seconds,
        })
    }

    #[cfg(not(windows))]
    pub fn start(_music: &Music, _stream: usize) -> Result<Self, String> {
        Err("music plays on Windows only for now".into())
    }

    pub fn elapsed(&self) -> f32 {
        self.started.elapsed().as_secs_f32()
    }

    pub fn done(&self) -> bool {
        self.elapsed() > self.seconds + 0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_assets::sound::Stream;
    use m2_assets::voice::{Group, PlainCue, Record, StartDraw};

    fn table() -> CueTable {
        let hash = name_hash("music");
        let record = |cue: &str, stream: u32| Record {
            cue: if cue.is_empty() { 0 } else { name_hash(cue) },
            volume: 1.0,
            min_distance: 10.0,
            max_distance: 1000.0,
            start: StartDraw::FLAT,
            positional: false,
            exponent: 1.0,
            variants: vec![(hash, stream, 1.0)],
        };
        let mut other = record("not_music", 3);
        other.variants = vec![(name_hash("ui_hud"), 3, 1.0)];
        CueTable {
            hash,
            lines: Vec::new(),
            groups: vec![Group {
                cue: name_hash("mu_shell_01"),
                category: 0,
                volume: 1.0,
                min_distance: 10.0,
                max_distance: 1000.0,
                streams: vec![(2, 1.0)],
            }],
            records: vec![record("", 0), record("whatever", 1), record("", 2), other],
            cues: vec![PlainCue {
                cue: name_hash("mu_fac_pmc_explore_01"),
                record: 1,
                a: 0.5,
                seconds: -1.0,
            }],
            mixes: Vec::new(),
        }
    }

    fn index() -> StreamIndex {
        let s = |hash: u32, seconds: u32| Stream {
            hash,
            channels: 2,
            rate: 44_100,
            offset: 0,
            encoded_len: 0,
            decoded_len: seconds * 44_100 * 4,
        };
        StreamIndex {
            hash: 1,
            file: "music.pws".into(),
            streams: vec![s(10, 60), s(11, 125), s(12, 30), s(13, 5)],
        }
    }

    #[test]
    fn script_strings_name_the_cues_by_hash() {
        let names = names_from_strings([
            "mu_fac_pmc_explore_01",
            "MU_Shell_01",
            "pmc",
            "mu bad",
            "jack_03",
        ]);
        assert_eq!(names.len(), 2);
        assert_eq!(names[&name_hash("mu_shell_01")], "mu_shell_01");
    }

    #[test]
    fn every_stream_is_listed_once_with_its_cue() {
        let names = names_from_strings(["mu_fac_pmc_explore_01", "mu_shell_01"]);
        let t = tracks(&table(), &index(), &names);
        let rows: Vec<(usize, &str, &str)> = t
            .iter()
            .map(|t| (t.stream, t.label.as_str(), t.group.as_str()))
            .collect();
        assert_eq!(
            rows,
            [
                (2, "Menus 1", "Menus"),
                (1, "PMC: explore 1", "PMC"),
                (0, "Track 0", "Unnamed"),
            ]
        );
        // The plain cue beats the record's own cue; another bank's stream is
        // not music.
        assert_eq!(t[1].cue, name_hash("mu_fac_pmc_explore_01"));
        assert_eq!(length(t[1].seconds), "2:05");
    }

    #[test]
    fn readable_labels_follow_the_names() {
        assert_eq!(readable("mu_fac_ch_kickass_01").0, "China: kickass 1");
        assert_eq!(
            readable("mu_src_up_op_01"),
            (
                "Universal Petroleum: radio, op 1".into(),
                "Universal Petroleum radio".into()
            )
        );
        assert_eq!(readable("mu_src_civ").0, "Civilians: radio");
        assert_eq!(
            readable("mu_freeplay_city_explore"),
            ("Free play: city, explore".into(), "Free play".into())
        );
        assert_eq!(
            readable("mu_explore_01"),
            ("mu_explore_01".into(), "Other".into())
        );
        assert_eq!(readable("mu_fac_zz_win_02").0, "Zz: win 2");
    }

    #[test]
    fn samples_are_converted_to_the_output() {
        // Mono 2 frames at 1 Hz to stereo at 2 Hz.
        let out = convert(&[0, 16384], 1, 1, 2, 2);
        assert_eq!(out.len(), 8);
        assert_eq!(out[0..2], [0.0, 0.0]);
        assert!((out[2] - 0.25).abs() < 1e-6 && (out[3] - 0.25).abs() < 1e-6);
        // Stereo to 6 channels: the extra four silent.
        let out = convert(&[100, -100], 2, 48_000, 6, 48_000);
        assert_eq!(out.len(), 6);
        assert!(out[2..].iter().all(|&v| v == 0.0));
        assert!(convert(&[], 2, 1, 2, 1).is_empty());
    }
}

/// Reads the real install named by `M2_INSTALL` (no game is run, nothing
/// is written): `cargo test -p m2-launcher -- --ignored install_music`.
#[cfg(test)]
#[test]
#[ignore = "needs the game's install in M2_INSTALL"]
fn install_music() {
    let dir = std::env::var("M2_INSTALL").expect("M2_INSTALL");
    let install = m2_engine::Install::find(std::path::Path::new(&dir)).expect("an install");
    let m = load(&install).expect("the music");
    let named = m.tracks.iter().filter(|t| t.name.is_some()).count();
    eprintln!("{} pieces, {named} named", m.tracks.len());
    for t in m.tracks.iter().take(8) {
        eprintln!(
            "  {} | {} | {:?} | {}",
            t.group,
            t.label,
            t.name,
            length(t.seconds)
        );
    }
    assert!(!m.tracks.is_empty());
}
