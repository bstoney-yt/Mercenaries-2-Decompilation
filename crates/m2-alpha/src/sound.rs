//! The game's sound: an output device fed by the engine's mixer, the
//! island's ambient sounds, music on demand, and the fight's gunshots and
//! blasts.
//!
//! Which sound in a bank is a shot or a blast is not known (sounds are
//! named by hash), so they are picked by measuring, as in the viewer: the
//! rifle bank's loudest short mono sound, the RPG bank's loudest short one,
//! and the artillery bank's loudest 1.5-3 s mono sound.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use std::time::{Duration, Instant};

use m2_engine::audio::{EngineSounds, Falloff, Play, SoundLibrary};
use m2_engine::{Category, Install, Mixer, SoundBuffer, Streams, VoiceId, Voices};

use crate::combat::{Remark, Sound};
use crate::hero::Hero;
use crate::scene::Cue;
use m2_assets::name_hash;
use m2_math::glam::Vec3;

/// The mixer, and the device stream playing it (none where there is no
/// output: other platforms, or no sound device).
pub struct AudioOut {
    pub mixer: Arc<Mutex<Mixer>>,
    #[cfg(windows)]
    _stream: Option<cpal::Stream>,
}

impl std::fmt::Debug for AudioOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioOut").finish_non_exhaustive()
    }
}

impl AudioOut {
    #[cfg(windows)]
    pub fn start() -> Self {
        match open_device() {
            Ok((mixer, stream)) => Self {
                mixer,
                _stream: Some(stream),
            },
            Err(e) => {
                eprintln!("sound: {e:#}; playing silently");
                Self {
                    mixer: Arc::new(Mutex::new(Mixer::new(48_000))),
                    _stream: None,
                }
            }
        }
    }

    #[cfg(not(windows))]
    pub fn start() -> Self {
        eprintln!("sound: no output on this platform yet; playing silently");
        Self {
            mixer: Arc::new(Mutex::new(Mixer::new(48_000))),
        }
    }

    /// Runs `f` on the mixer (a poisoned lock is taken anyway: the mixer
    /// holds no invariants a panic could break).
    pub fn with<T>(&self, f: impl FnOnce(&mut Mixer) -> T) -> T {
        let mut m = self.mixer.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut m)
    }
}

#[cfg(windows)]
fn open_device() -> anyhow::Result<(Arc<Mutex<Mixer>>, cpal::Stream)> {
    use anyhow::Context as _;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let device = cpal::default_host()
        .default_output_device()
        .context("no sound output device")?;
    let supported = device.default_output_config()?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let channels = usize::from(config.channels.max(1));
    let mixer = Arc::new(Mutex::new(Mixer::new(config.sample_rate)));
    let shared = mixer.clone();
    let mut stereo = Vec::new();
    let mut fill = move |frames: usize, mut put: Box<dyn FnMut(usize, f32) + '_>| {
        stereo.resize(frames * 2, 0.0);
        shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .mix(&mut stereo);
        for f in 0..frames {
            for c in 0..channels {
                let s = if c < 2 { stereo[f * 2 + c] } else { 0.0 };
                put(f * channels + c, s);
            }
        }
    };
    let error = |e| eprintln!("sound: {e}");
    let stream = match format {
        cpal::SampleFormat::F32 => device.build_output_stream::<f32, _, _>(
            config,
            move |data, _| {
                let frames = data.len() / channels;
                fill(frames, Box::new(|i, s| data[i] = s));
            },
            error,
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_output_stream::<i16, _, _>(
            config,
            move |data, _| {
                let frames = data.len() / channels;
                fill(frames, Box::new(|i, s| data[i] = (s * 32767.0) as i16));
            },
            error,
            None,
        )?,
        other => anyhow::bail!("unsupported sample format {other:?}"),
    };
    stream.play()?;
    Ok((mixer, stream))
}

/// A script's change to a category of sounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CategoryChange {
    Fade {
        category: Category,
        level: f32,
        seconds: f32,
    },
    Pitch {
        category: Category,
        pitch: f32,
        seconds: f32,
    },
}

/// Ambience and music.
#[derive(Debug)]
pub struct GameAudio {
    pub out: AudioOut,
    streams: Option<Arc<Streams>>,
    music: Option<VoiceId>,
    /// The music stream asked for (by hash) and whether it loops: a
    /// decoded stream that is not it was overtaken and is dropped.
    music_wanted: Option<(u32, bool)>,
    next_track: usize,
    decoded: (Sender<Decoded>, Receiver<Decoded>),
    /// Rifle shot, rocket launch, explosion, tank gun, grenade launcher,
    /// a building coming down.
    fight: [Option<SoundBuffer>; 6],
    /// Short sounds of the town (`amb_citymed`) and the jungle
    /// (`amb_jungle`) played now and then around the listener.
    ambient: [Vec<SoundBuffer>; 2],
    /// Engine sounds by bank name hash, once read (`None`: the bank has
    /// none); banks are read on a thread the first time they're driven.
    engines: HashMap<u32, Option<Arc<EngineSounds>>>,
    engine_loads: (Sender<EngineLoad>, Receiver<EngineLoad>),
    install: Option<Install>,
    /// The engine running now.
    engine: Option<Engine>,
    /// The rotors heard, by helicopter: a voice per loop.
    rotors: HashMap<u32, Vec<VoiceId>>,
    /// Banks read and found to have no engine sounds.
    silent: HashSet<u32>,
    /// Seconds to the next one.
    ambient_wait: f32,
    seed: u64,
    /// The characters' voices, once read (on a thread: `English.wad`).
    voices: Option<Arc<Voices>>,
    voices_loading: Option<Receiver<Voices>>,
    /// Decoded lines waiting to play: where (`None` flat) and how.
    said: (Sender<Spoken>, Receiver<Spoken>),
    /// When each line now playing or decoding ends.
    speaking: Vec<Instant>,
    /// Who the player is: whose voice set their remarks come from.
    pub hero: Hero,
    /// When the hero may speak again.
    quiet_until: Instant,
    /// Scripted lines playing, by the mission's id for them.
    lines: HashMap<u32, VoiceId>,
    /// Interface sounds started, by cue in lower case.
    ui_playing: HashMap<String, VoiceId>,
    /// Scripted lines stopped before they were decoded.
    hushed: HashSet<u32>,
    /// The sounds the clips' events play, once read (on a thread).
    clip_sounds: Option<ClipSounds>,
    clip_sounds_loading: Option<Receiver<ClipSounds>>,
    /// The interface's sounds (`ui_hud`), once read (on a thread).
    ui: Option<UiSounds>,
    ui_loading: Option<Receiver<UiSounds>>,
}

/// The interface's sound bank, `ui_hud`: its cue table (cue name hash to
/// record, record to the bank's sound) and its sounds; and the music's cue
/// table (records name streams of `music.pws`).
#[derive(Debug, Default)]
pub struct UiSounds {
    cues: Option<m2_assets::CueTable>,
    sounds: Vec<SoundBuffer>,
    music: Option<m2_assets::CueTable>,
}

impl UiSounds {
    fn load(install: &Install) -> Self {
        let bank = m2_assets::name_hash("ui_hud");
        let Ok(mut source) = install.open_world() else {
            return Self::default();
        };
        let cues = source
            .asset(bank, m2_assets::voice::CUE_TABLE_TYPE)
            .ok()
            .and_then(|d| m2_assets::CueTable::parse(&d).ok());
        let music = source
            .asset(
                m2_assets::name_hash("music"),
                m2_assets::voice::CUE_TABLE_TYPE,
            )
            .ok()
            .and_then(|d| m2_assets::CueTable::parse(&d).ok());
        let library = SoundLibrary::load_banks(&mut source, &[bank]);
        let sounds = library
            .bank_hash(bank)
            .map(|(_, s)| s.to_vec())
            .unwrap_or_default();
        Self {
            cues,
            sounds,
            music,
        }
    }

    /// The music stream cue `cue` plays, and whether it loops.
    fn music(&self, cue: &str) -> Option<(usize, bool)> {
        let table = self.music.as_ref()?;
        let plain = table.plain_cue(m2_assets::name_hash(cue))?;
        let record = table.records.get(plain.record as usize)?;
        let &(_, index, _) = record.variants.iter().find(|v| v.0 == table.hash)?;
        Some((index as usize, plain.seconds < 0.0))
    }

    /// The sound cue `cue` plays and its volume (the first of its
    /// record's variants that is this bank's).
    /// and whether it loops.
    fn find(&self, cue: &str) -> Option<(SoundBuffer, f32, bool)> {
        let table = self.cues.as_ref()?;
        let hash = m2_assets::name_hash(cue);
        let record = table.cue_record(hash)?;
        let &(_, index, _) = record.variants.iter().find(|v| v.0 == table.hash)?;
        let looped = table.plain_cue(hash).is_some_and(|c| c.seconds < 0.0);
        Some((
            self.sounds.get(index as usize)?.clone(),
            record.volume,
            looped,
        ))
    }
}

/// How loud the music plays.
const MUSIC_GAIN: f32 = 0.6;

/// Banks the clips' sound events are found in, by name: the resident
/// foley (`fol_rustle_human_02`, `fol_grenade_sidethrow_01`, ...).
const CUE_BANKS: [&str; 1] = ["fol_shared"];
/// Endings a cue's sounds carry besides none: its variants (round 91's
/// survey found `_01` to `_04`).
const CUE_VARIANTS: usize = 12;
/// How loud the clips' sounds play, and how far they carry.
const CUE_GAIN: f32 = 0.55;
const CUE_FALLOFF: Falloff = Falloff {
    near: 2.0,
    far: 35.0,
};
/// Most clip sounds started in one tick.
const CUES_AT_ONCE: usize = 6;

/// The sounds the clips' sound events play: an event's name, bare or with
/// a variant's ending (`fol_rustle_human_02`), among [`CUE_BANKS`]'
/// sounds.
#[derive(Debug, Default)]
pub struct ClipSounds {
    by_hash: HashMap<u32, SoundBuffer>,
    /// Each name's sounds, by its hash, once looked for.
    found: HashMap<u32, Vec<SoundBuffer>>,
}

impl ClipSounds {
    /// Reads [`CUE_BANKS`] from the install's world archive.
    pub fn load(install: &Install) -> Self {
        let hashes: Vec<u32> = CUE_BANKS.iter().map(|b| name_hash(b)).collect();
        let library = install
            .open_world()
            .map(|mut source| SoundLibrary::load_banks(&mut source, &hashes))
            .unwrap_or_default();
        let by_hash = hashes
            .iter()
            .filter_map(|&h| library.bank_hash(h))
            .flat_map(|(_, sounds)| sounds.iter().map(|s| (s.hash, s.clone())))
            .collect();
        Self::new(by_hash)
    }

    fn new(by_hash: HashMap<u32, SoundBuffer>) -> Self {
        Self {
            by_hash,
            found: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.by_hash.len()
    }

    /// The sounds event `name` plays: itself and its variants (`_01`,
    /// `_1`, `01`, ...) that the banks hold. Empty when none.
    pub fn sounds(&mut self, name: &str) -> &[SoundBuffer] {
        let by_hash = &self.by_hash;
        self.found.entry(name_hash(name)).or_insert_with(|| {
            let mut names = vec![name.to_owned()];
            for n in 1..=CUE_VARIANTS {
                names.push(format!("{name}_{n:02}"));
                names.push(format!("{name}_{n}"));
                names.push(format!("{name}{n:02}"));
            }
            let mut out: Vec<SoundBuffer> = names
                .iter()
                .filter_map(|n| by_hash.get(&name_hash(n)).cloned())
                .collect();
            out.dedup_by_key(|s| s.hash);
            out
        })
    }
}

/// A decoded voice line: the sound, where it is said (`None` flat: the
/// player's own), its falloff and volume, and the scripted line it is.
type Spoken = (SoundBuffer, Option<Vec3>, Falloff, f32, Option<u32>);

/// Voice sets, by the name hash of their name: two of Venezuelan army
/// soldiers, `vo_vzSoldier_01` and `_02` (so not everyone sounds alike),
/// and two each of civilian men and women, `vo_vzCiv_01`, `_02`,
/// `vo_vzCiv_female_01` and `_02`.
pub const VZ_SOLDIERS: [u32; 2] = [0x5078_7bff, 0x467a_aad8];
pub const CIVILIANS: [u32; 2] = [0x6b0c_3b0b, 0x910e_b574];
pub const CIVILIAN_WOMEN: [u32; 2] = [0x3a6e_c728, 0xc46b_cecf];
/// The hero's lines for what happens to them, by cue (a line, or a group
/// of variants to pick from) in their voice set ([`Hero::voice_set`]),
/// chosen by reading their subtitles.
pub fn hero_cues(hero: Hero, remark: Remark) -> &'static [u32] {
    match hero {
        Hero::Jennifer => jennifer_cues(remark),
        Hero::Mattias => mattias_cues(remark),
        Hero::Chris => chris_cues(remark),
    }
}

/// Mattias's, in `vo_mattias` (heroes round 03, by their subtitles).
fn mattias_cues(remark: Remark) -> &'static [u32] {
    match remark {
        // Two pairs about dead Venezuelan troops, two said to anyone.
        Remark::Kill => &[
            0xfac0_4d90,
            0xe4c2_6985,
            0xdab1_10a6,
            0xe4bd_ec57,
            0xe7d3_c09f,
            0x05d1_b142,
            0xdfcf_36d9,
            0xd488_2edc,
        ],
        // He has a line for wrecking a vehicle.
        Remark::Wreck => &[0x1da2_d5ed],
        // His group of pain grunts, and short lines about being hit.
        Remark::Hurt => &[
            0x67f1_8134,
            0x01d7_f391,
            0x7fd0_6b26,
            0xa1d2_df43,
            0x9fdf_a810,
            0x89e1_c405,
            0x27da_6dfa,
            0x89dd_46d7,
        ],
        // No group of his is labelled as death cries; not chosen yet.
        Remark::Down => &[],
        Remark::Vehicle => &[0xdf7d_d41a, 0x4185_2a25],
    }
}

/// Chris's, in `vo_Chris` (heroes round 03, by their subtitles).
fn chris_cues(remark: Remark) -> &'static [u32] {
    match remark {
        Remark::Kill => &[
            0x5f13_cf48,
            0xc916_b4bd,
            0x6911_a06f,
            0xbf05_5bde,
            0x05db_41c3,
            0xf899_1c61,
            0x813f_807f,
            0xedea_2685,
        ],
        // No line for wrecking a vehicle: a kill line said to anyone.
        Remark::Wreck => &[0xedea_2685],
        Remark::Hurt => &[
            0x2707_1f3c,
            0x8598_39a3,
            0xb5c0_ed36,
            0x37c8_75a1,
            0xbdc5_76fc,
            0x3fcc_ff67,
            0x1dca_8b4a,
            0xbfd2_4615,
        ],
        // His group of death cries.
        Remark::Down => &[0x5650_1137],
        Remark::Vehicle => &[0x151f_3354, 0x9524_7a02],
    }
}

/// Jennifer's, in `vo_Jen`.
fn jennifer_cues(remark: Remark) -> &'static [u32] {
    match remark {
        // Two about dead Venezuelan troops (everyone she fights here) and
        // two said to anyone.
        Remark::Kill => &[0xdcad_44ab, 0x02af_bf14, 0x5157_a743, 0x8eee_b3db],
        // There is no line for wrecking a vehicle: one of the kill lines
        // that suits it.
        Remark::Wreck => &[0x8eee_b3db],
        // Her group of pain grunts, and short lines about being hit.
        Remark::Hurt => &[
            0x559f_e5ef,
            0x3aab_17ee,
            0x8dae_e9a4,
            0x85aa_5fde,
            0x8fbb_b8bd,
            0x339d_71d2,
            0x0db4_3052,
        ],
        // Her group of death cries.
        Remark::Down => &[0x33e7_b94d],
        // Two of her lines for taking someone's car.
        Remark::Vehicle => &[0x84b6_cf3b, 0x88d4_409c],
    }
}
/// How often she says something about it (she doesn't remark on every
/// hit), and the least time between her remarks, seconds.
fn remark_chance(remark: Remark) -> f32 {
    match remark {
        Remark::Kill => 0.3,
        Remark::Wreck | Remark::Vehicle => 0.8,
        Remark::Hurt => 0.3,
        Remark::Down => 1.0,
    }
}
const REMARK_GAP: f32 = 10.0;
/// Shouts are the sets' short lines said where the speaker stands.
const SHOUT_LONGEST: f32 = 3.5;
/// Most lines said at once.
const MAX_SPEAKING: usize = 2;

/// Picks the loudest sound in `bank` passing `keep`.
fn pick(
    library: &SoundLibrary,
    bank: &str,
    keep: impl Fn(&SoundBuffer) -> bool,
) -> Option<SoundBuffer> {
    library
        .bank(bank)
        .iter()
        .filter(|s| s.frames() > 0 && keep(s))
        .max_by(|a, b| a.loudness().total_cmp(&b.loudness()))
        .cloned()
}

/// Ambient one-shots no longer than this, seconds (the banks' longer
/// sounds are loops).
const AMBIENT_LONGEST: f32 = 8.0;

/// Engine banks for vehicles whose template doesn't name one (the
/// template database's `SoundEffect` does for most), by kind and label
/// words, first match: the banks the templates give such vehicles.
const ENGINES: [(&str, &str); 18] = [
    // Boats first: their labels may hold other kinds' words. The
    // playthroughs' start boat is a racing speedboat.
    ("jetski", "veh_boatjetski"),
    ("fishing|trawler", "veh_boatbigfishing"),
    ("cruiser|yacht", "veh_boatcruiser"),
    ("outboard|skiff", "veh_boatoutboard"),
    ("boat", "veh_boatracing"),
    ("helicopter", "veh_helicoptermedium"),
    ("hmmwv|humvee|valiant", "veh_largegascrappy"),
    ("m151|jeep", "veh_jeep"),
    ("amx", "veh_tankmed"),
    // The Stryker is the game's LAV III.
    ("stryker|lav", "veh_largedieselold"),
    ("m113|apc", "veh_tankdiesel"),
    ("tank", "veh_tankbig"),
    ("semi", "veh_semi"),
    ("bus", "veh_bus"),
    ("pickup|truck", "veh_8cylindertruck"),
    ("honda|crx", "veh_4cylinderold"),
    ("van", "veh_largegasnew"),
    ("", "veh_4cylindernew"),
];

/// The engine bank for a vehicle of `kind` named `label`, when its
/// template doesn't say.
pub fn engine_bank(kind: &str, label: &str) -> &'static str {
    let key = format!("{kind} {label}").to_lowercase();
    ENGINES
        .iter()
        .find(|(words, _)| words.is_empty() || words.split('|').any(|w| key.contains(w)))
        .map_or("veh_4cylindernew", |e| e.1)
}

/// The engine of the vehicle being driven, as the game sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EngineInput {
    /// The name hash of its sound bank (`veh_jeep`).
    pub bank: u32,
    /// Speed and top speed, m/s.
    pub speed: f32,
    pub top: f32,
    /// The driver's throttle, -1 (reverse) to 1.
    pub throttle: f32,
}

/// A bank's engine sounds, read on a thread.
type EngineLoad = (u32, Option<EngineSounds>);

/// An engine running: its sounds, a voice per loop, and how hard it runs.
#[derive(Debug)]
struct Engine {
    bank: u32,
    sounds: Arc<EngineSounds>,
    voices: Vec<VoiceId>,
    /// 0 idling to 1 flat out, eased.
    rpm: f32,
    /// Seconds since it started.
    running: f32,
}

/// How loud the engine is, and how long its loops take to come in (under
/// the start sound, if it has one).
const ENGINE_GAIN: f32 = 0.45;
const ENGINE_FADE_IN: f32 = 0.6;
/// How quickly the engine's revs follow speed and throttle, per second.
const ENGINE_EASE: f32 = 3.0;
/// The most a loop is pitched from as recorded.
const ENGINE_BEND: (f32, f32) = (0.7, 1.45);

impl Engine {
    /// Each loop's gain and pitch with the engine at `rpm`.
    fn mix(sounds: &EngineSounds, rpm: f32) -> Vec<(f32, f32)> {
        let n = sounds.loops.len();
        if n == 1 {
            let (idle, top) = sounds.heard.unwrap_or((0.8, 1.5));
            return vec![(1.0, idle + (top - idle) * rpm)];
        }
        let at = rpm.clamp(0.0, 1.0) * (n - 1) as f32;
        let k = (at.floor() as usize).min(n - 2);
        let t = at - k as f32;
        // Measured pitches, if every loop has one and they rise sensibly:
        // the engine's note runs smoothly from the lowest loop's to the
        // highest's and each loop is pitched to it.
        let pitches: Option<Vec<f32>> = sounds.loops.iter().map(|l| l.1).collect();
        let rising = pitches.as_ref().filter(|p| {
            p.windows(2)
                .all(|w| (1.05..=3.0).contains(&(w[1] / w[0].max(1.0))))
        });
        (0..n)
            .map(|j| {
                let gain = if j == k {
                    (t * std::f32::consts::FRAC_PI_2).cos()
                } else if j == k + 1 {
                    (t * std::f32::consts::FRAC_PI_2).sin()
                } else {
                    0.0
                };
                let pitch = match rising {
                    Some(p) => {
                        // The note between the two loops either side.
                        let (lo, hi) = (p[k], p[k + 1]);
                        lo * (hi / lo).powf(t) / p[j]
                    }
                    None => 1.0 + (at - j as f32) * 0.25,
                };
                (gain, pitch.clamp(ENGINE_BEND.0, ENGINE_BEND.1))
            })
            .collect()
    }
}

/// The bank a helicopter's rotors are heard from, how loud its blades
/// and its turbine are, and how far off they carry.
const ROTOR_BANK: &str = "veh_helicoptermedium";
const ROTOR_GAIN: f32 = 0.8;
const ROTOR_WHINE: f32 = 0.3;
const ROTOR_FALLOFF: Falloff = Falloff {
    near: 20.0,
    far: 800.0,
};

/// A tank's main gun: the loudest of its bank's sounds 0.4 to 4 s long.
fn tank_gun(library: &SoundLibrary) -> Option<SoundBuffer> {
    pick(library, "wpn_tankgun", |s| {
        (0.4..=4.0).contains(&s.seconds())
    })
    .inspect(|s| eprintln!("sound: tank gun {:08x} ({:.2} s)", s.hash, s.seconds()))
}

/// Reads and logs which sound a tank's gun plays (for the drive test,
/// whose picture has no sound).
pub fn log_tank_gun(install: &Install) {
    let library = install
        .open_world()
        .and_then(|mut source| SoundLibrary::load(&mut source, &["wpn_tankgun"]));
    match library {
        Ok(l) => {
            if tank_gun(&l).is_none() {
                eprintln!("sound: no tank gun shot in wpn_tankgun");
            }
        }
        Err(e) => eprintln!("sound: no tank gun ({e})"),
    }
}

/// Sounds read from the banks: the fight's and the ambient one-shots.
type BankSounds = ([Option<SoundBuffer>; 6], [Vec<SoundBuffer>; 2]);

/// A building coming down (a mission's gate the bomb brings down): the
/// rumble of the building destruction bank (`bulding_destruct`, the
/// game's spelling), `sfx_dest_dist_rumble_genericd`, found by its name's
/// hash with its bank's debris sounds (`sfx_debris_distant_hut_mid_03a`
/// and `_03b`). The gate's own script names its sounds through emitters
/// not yet read.
const COLLAPSE_SOUND: u32 = 0x9204_a89a;

/// How many of the fight's sounds the banks have, read as play reads
/// them (each logs what it picked, the collapse's rumble among them): a
/// check for headless runs, which play no sound.
pub fn fight_sounds_found(install: &Install) -> usize {
    bank_sounds(install).0.iter().flatten().count()
}

/// The fight's sounds and the ambient one-shots.
fn bank_sounds(install: &Install) -> BankSounds {
    let library = install.open_world().and_then(|mut source| {
        SoundLibrary::load(
            &mut source,
            &[
                "wpn_assaultrifle",
                "wpn_rpg",
                "support_artillery",
                "wpn_tankgun",
                "wpn_grenadelauncher",
                "amb_citymed",
                "amb_jungle",
                "bulding_destruct",
            ],
        )
    });
    match library {
        Ok(l) => {
            let short = |bank: &str| -> Vec<SoundBuffer> {
                l.bank(bank)
                    .iter()
                    .filter(|s| s.frames() > 0 && s.seconds() <= AMBIENT_LONGEST)
                    .cloned()
                    .collect()
            };
            (
                [
                    pick(&l, "wpn_assaultrifle", |s| {
                        s.channels == 1 && s.seconds() < 0.6
                    }),
                    pick(&l, "wpn_rpg", |s| s.seconds() < 1.5),
                    pick(&l, "support_artillery", |s| {
                        s.channels == 1 && (1.5..=3.0).contains(&s.seconds())
                    }),
                    tank_gun(&l),
                    // The launcher's thump: its loudest short sound.
                    pick(&l, "wpn_grenadelauncher", |s| s.seconds() < 1.2).inspect(|s| {
                        eprintln!(
                            "sound: grenade launcher {:08x} ({:.2} s)",
                            s.hash,
                            s.seconds()
                        )
                    }),
                    pick(&l, "bulding_destruct", |s| s.hash == COLLAPSE_SOUND).inspect(|s| {
                        eprintln!("sound: collapse {:08x} ({:.2} s)", s.hash, s.seconds())
                    }),
                ],
                [short("amb_citymed"), short("amb_jungle")],
            )
        }
        Err(e) => {
            eprintln!("sound: no weapon or ambient sounds ({e})");
            Default::default()
        }
    }
}

/// Every engine bank, to name a template's bank (a name hash) in logs.
const ENGINE_BANKS: [&str; 30] = [
    "veh_boatbigfishing",
    "veh_boatoutboard",
    "veh_boatoutboard2",
    "veh_boatrib",
    "veh_boatcruiser",
    "veh_boatracing",
    "veh_boatjetski",
    "veh_boatcrappy",
    "veh_semi",
    "veh_bus",
    "veh_jeep",
    "veh_tankbig",
    "veh_tankmed",
    "veh_tanksmall",
    "veh_tankdiesel",
    "veh_8cylinderold",
    "veh_8cylinderold_cobra",
    "veh_8cylindertruck",
    "veh_8cylindernew",
    "veh_8cylinderluxury",
    "veh_8cylinderjapanese",
    "veh_4cylindernew",
    "veh_4cylinderold",
    "veh_4cylinderracing",
    "veh_largedieselnew",
    "veh_largedieselold",
    "veh_largegasnew",
    "veh_largegasold",
    "veh_largegascrappy",
    "veh_helicoptermedium",
];

/// The engine bank whose name hashes to `hash`.
pub fn bank_name(hash: u32) -> Option<&'static str> {
    ENGINE_BANKS
        .iter()
        .copied()
        .find(|n| m2_assets::name_hash(n) == hash)
}

/// Reads bank `bank`'s engine sounds (from an install's world archive),
/// and logs what it picked.
pub fn load_engine(install: &Install, bank: u32) -> Option<EngineSounds> {
    let library = install
        .open_world()
        .and_then(|mut source| SoundLibrary::load_hashes(&mut source, &[bank]));
    let library = match library {
        Ok(l) => l,
        Err(e) => {
            eprintln!("engine: bank {bank:08x}: {e}");
            return None;
        }
    };
    let Some((name, sounds)) = library.bank_hash(bank) else {
        eprintln!("engine: no bank {bank:08x}");
        return None;
    };
    let picked = EngineSounds::pick(name, sounds);
    let loops: Vec<String> = picked
        .loops
        .iter()
        .map(|(s, f)| {
            format!(
                "{:08x} {:.2} s {}",
                s.hash,
                s.seconds(),
                f.map_or("no pitch".to_owned(), |f| format!("{f:.0} Hz"))
            )
        })
        .collect();
    eprintln!(
        "engine: {name}, {} sounds: {} loops{} [{}], start {}, stop {}",
        sounds.len(),
        picked.loops.len(),
        if picked.named {
            " by name"
        } else if picked.heard.is_some() {
            " as heard in a playthrough"
        } else {
            " measured"
        },
        loops.join(", "),
        picked.start.is_some(),
        picked.stop.is_some()
    );
    (!picked.loops.is_empty()).then_some(picked)
}

/// A finished decode: which kind of stream, and its samples.
type Decoded = (&'static str, SoundBuffer);

impl GameAudio {
    /// Opens the output and, with an install, starts the ambience.
    pub fn start(install: Option<&Install>) -> Self {
        let out = AudioOut::start();
        let streams = install.and_then(|install| {
            let loaded = install
                .open_world()
                .and_then(|mut source| Streams::load(install, &mut source));
            match loaded {
                Ok(s) => Some(Arc::new(s)),
                Err(e) => {
                    eprintln!("sound: no music or ambience ({e})");
                    None
                }
            }
        });
        let (fight, ambient) = install.map(bank_sounds).unwrap_or_default();
        // The recorded ambience (ambience.pws) isn't played: which of its
        // tracks is which place is not known, and the first is a tank
        // rolling by. The banks' short sounds stand in, as in the viewer.
        Self {
            out,
            streams,
            music: None,
            music_wanted: None,
            next_track: 0,
            decoded: channel(),
            fight,
            ambient,
            engines: HashMap::new(),
            engine_loads: channel(),
            install: install.cloned(),
            engine: None,
            rotors: HashMap::new(),
            silent: HashSet::new(),
            ambient_wait: 3.0,
            seed: 0x9e37_79b9_7f4a_7c15,
            voices: None,
            voices_loading: install.map(|install| {
                let install = install.clone();
                let (tx, rx) = channel();
                std::thread::spawn(move || match Voices::load(&install) {
                    Ok(v) => {
                        eprintln!(
                            "voices: {} sets, {} lines",
                            v.sets.len(),
                            v.index.streams.len()
                        );
                        let _ = tx.send(v);
                    }
                    Err(e) => eprintln!("voices: {e}"),
                });
                rx
            }),
            said: channel(),
            lines: HashMap::new(),
            ui_playing: HashMap::new(),
            hushed: HashSet::new(),
            speaking: Vec::new(),
            hero: Hero::default(),
            quiet_until: Instant::now(),
            clip_sounds: None,
            clip_sounds_loading: install.map(|install| {
                let install = install.clone();
                let (tx, rx) = channel();
                std::thread::spawn(move || {
                    let sounds = ClipSounds::load(&install);
                    eprintln!("clip sounds: {} foley sounds", sounds.len());
                    let _ = tx.send(sounds);
                });
                rx
            }),
            ui: None,
            ui_loading: install.map(|install| {
                let install = install.clone();
                let (tx, rx) = channel();
                std::thread::spawn(move || {
                    let ui = UiSounds::load(&install);
                    eprintln!(
                        "ui sounds: {} cues, {} sounds; music: {} cues",
                        ui.cues.as_ref().map_or(0, |c| c.cues.len()),
                        ui.sounds.len(),
                        ui.music.as_ref().map_or(0, |c| c.cues.len())
                    );
                    let _ = tx.send(ui);
                });
                rx
            }),
        }
    }

    fn ui_loaded(&mut self) {
        if let Some(rx) = &self.ui_loading
            && let Ok(u) = rx.try_recv()
        {
            self.ui = Some(u);
            self.ui_loading = None;
        }
    }

    /// Plays the scripts' music cue `cue` (`mu_fac_pmc_explore_01`) in
    /// place of what plays, looped unless it is a one-shot (a fanfare);
    /// `None` stops the music.
    pub fn music_cue(&mut self, cue: Option<&str>) {
        self.ui_loaded();
        if let Some(id) = self.music.take() {
            self.out.with(|m| m.stop(id));
        }
        self.music_wanted = None;
        let Some(cue) = cue else {
            eprintln!("music: stopped");
            return;
        };
        let Some((stream, looped)) = self.ui.as_ref().and_then(|u| u.music(cue)) else {
            eprintln!("music: {cue} not found");
            return;
        };
        let hash = self
            .streams
            .as_ref()
            .and_then(|s| s.music.streams.get(stream))
            .map(|s| s.hash);
        eprintln!(
            "music: {cue}, stream {stream}{}",
            if looped { ", looped" } else { "" }
        );
        self.music_wanted = hash.map(|h| (h, looped));
        self.decode("music", stream);
    }

    /// Plays the interface's sound cue `cue` (`ui_HUD_Objective_New`),
    /// flat.
    pub fn ui_sound(&mut self, cue: &str) {
        self.ui_loaded();
        match self.ui.as_ref().and_then(|u| u.find(cue)) {
            Some((buffer, volume, looped)) => {
                eprintln!(
                    "ui sound: {cue} ({:.2} s{})",
                    buffer.seconds(),
                    if looped { ", looped" } else { "" }
                );
                let mut play = Play::flat(buffer).gain(volume.clamp(0.0, 2.0)).ui();
                if looped {
                    play = play.looped();
                }
                let voice = self.out.with(|m| m.play(play));
                // Started again: the one before stops, as a cue plays once
                // per emitter.
                if let Some(old) = self.ui_playing.insert(cue.to_ascii_lowercase(), voice) {
                    self.out.with(|m| m.stop(old));
                }
            }
            None => eprintln!("ui sound: {cue} not found"),
        }
    }

    /// Fades or pitches a category of sounds as the scripts' sound sets
    /// ask (the world ducks under conversations and slows in the survival
    /// rescue).
    pub fn change_category(&mut self, change: CategoryChange) {
        match change {
            CategoryChange::Fade {
                category,
                level,
                seconds,
            } => {
                eprintln!("{category:?} sounds fade to {level:.2} over {seconds:.2} s");
                self.out.with(|m| m.fade(category, level, seconds));
            }
            CategoryChange::Pitch {
                category,
                pitch,
                seconds,
            } => {
                eprintln!("{category:?} sounds pitch to {pitch:.2} over {seconds:.2} s");
                self.out.with(|m| m.pitch(category, pitch, seconds));
            }
        }
    }

    /// Stops interface sound `cue` if it plays (`Sound.StopSound`).
    pub fn stop_ui_sound(&mut self, cue: &str) {
        if let Some(voice) = self.ui_playing.remove(&cue.to_ascii_lowercase()) {
            eprintln!("ui sound: {cue} stopped");
            self.out.with(|m| m.stop(voice));
        }
    }

    /// Plays what the clips' sound events ask for, where they happen (a
    /// few a tick at most); events no sound answers stay silent.
    pub fn play_cues(&mut self, cues: &[Cue]) {
        if let Some(rx) = &self.clip_sounds_loading
            && let Ok(c) = rx.try_recv()
        {
            self.clip_sounds = Some(c);
            self.clip_sounds_loading = None;
        }
        let mut started = 0;
        for cue in cues {
            if started >= CUES_AT_ONCE {
                break;
            }
            let pick = self.random();
            let Some(sounds) = self.clip_sounds.as_mut() else {
                return;
            };
            let list = sounds.sounds(&cue.name);
            if list.is_empty() {
                continue;
            }
            let buffer = list[((pick * list.len() as f32) as usize).min(list.len() - 1)].clone();
            let play = Play {
                falloff: CUE_FALLOFF,
                ..Play::at(buffer, cue.at)
            };
            self.out.with(|m| m.play(play.gain(CUE_GAIN)));
            started += 1;
        }
    }

    /// Says a short line of voice set `set` where `at` is (`None`: flat,
    /// the player's own), unless enough people are talking already.
    pub fn shout(&mut self, set: u32, at: Option<Vec3>) {
        let now = Instant::now();
        self.speaking.retain(|end| *end > now);
        if self.speaking.len() >= MAX_SPEAKING {
            return;
        }
        let Some(voices) = self.voices.clone() else {
            return;
        };
        let Some(table) = voices.set(set) else {
            return;
        };
        let lines: Vec<_> = table
            .lines
            .iter()
            .filter(|l| {
                l.is_positional()
                    && voices
                        .index
                        .streams
                        .get(l.stream as usize)
                        .is_some_and(|s| s.seconds() <= SHOUT_LONGEST)
            })
            .cloned()
            .collect();
        if lines.is_empty() {
            return;
        }
        let pick = ((self.random() * lines.len() as f32) as usize).min(lines.len() - 1);
        let line = lines[pick].clone();
        let falloff = Falloff {
            near: line.min_distance.max(1.0),
            far: line.max_distance.max(line.min_distance + 1.0),
        };
        self.speak(voices, line.stream, at, falloff, line.volume, None);
    }

    /// The hero says something about `remark`, now and then: a cue from
    /// [`hero_cues`] (one of a group's variants), flat, as their own.
    pub fn remark(&mut self, remark: Remark) {
        let now = Instant::now();
        // Going down cuts in; anything else waits its turn.
        if (now < self.quiet_until && remark != Remark::Down)
            || self.random() >= remark_chance(remark)
        {
            return;
        }
        let Some(voices) = self.voices.clone() else {
            return;
        };
        let Some(table) = voices.set(self.hero.voice_set()) else {
            return;
        };
        let streams: Vec<(u32, f32)> = hero_cues(self.hero, remark)
            .iter()
            .filter_map(|&cue| {
                let volume = table
                    .lines
                    .iter()
                    .find(|l| l.cue == cue)
                    .map(|l| l.volume)
                    .or_else(|| table.groups.iter().find(|g| g.cue == cue).map(|g| g.volume))?;
                Some((table.streams(cue)?, volume))
            })
            .flat_map(|(streams, volume)| streams.into_iter().map(move |s| (s, volume)))
            .collect();
        if streams.is_empty() {
            return;
        }
        let pick = ((self.random() * streams.len() as f32) as usize).min(streams.len() - 1);
        let (stream, volume) = streams[pick];
        let seconds = voices
            .index
            .streams
            .get(stream as usize)
            .map_or(0.0, |s| s.seconds());
        self.quiet_until = now + Duration::from_secs_f32(seconds + REMARK_GAP);
        eprintln!("voice: {}, {remark:?}, stream {stream}", self.hero);
        self.speak(voices, stream, None, Falloff::default(), volume, None);
    }

    /// Plays the scripts' line `cue` (`Fiona-In-Mission-Contract-Vz01-146`,
    /// found by its name hash in any voice set) as line `id`: flat, or
    /// where `at` is if the line is heard from where it is said. Returns
    /// how long it lasts, or `None` if it cannot be played.
    pub fn say(&mut self, id: u32, cue: &str, at: Option<Vec3>) -> Option<f32> {
        let voices = self.voices.clone()?;
        let hash = m2_assets::name_hash(cue);
        let (streams, volume, near, far, positional) = voices.sets.iter().find_map(|t| {
            let (volume, near, far, positional) = t
                .lines
                .iter()
                .find(|l| l.cue == hash)
                .map(|l| (l.volume, l.min_distance, l.max_distance, l.is_positional()))
                .or_else(|| {
                    t.groups
                        .iter()
                        .find(|g| g.cue == hash)
                        .map(|g| (g.volume, g.min_distance, g.max_distance, g.is_positional()))
                })?;
            Some((t.streams(hash)?, volume, near, far, positional))
        })?;
        if streams.is_empty() {
            return None;
        }
        let pick = ((self.random() * streams.len() as f32) as usize).min(streams.len() - 1);
        let stream = streams[pick];
        let seconds = voices.index.streams.get(stream as usize)?.seconds();
        let falloff = Falloff {
            near: near.max(1.0),
            far: far.max(near + 1.0),
        };
        // Jennifer keeps quiet while she is spoken to.
        let now = Instant::now();
        self.quiet_until = self
            .quiet_until
            .max(now + Duration::from_secs_f32(seconds + REMARK_GAP));
        eprintln!("voice: line {cue}, stream {stream}, {seconds:.1} s");
        self.speak(
            voices,
            stream,
            at.filter(|_| positional),
            falloff,
            volume,
            Some(id),
        );
        Some(seconds)
    }

    /// Stops scripted line `id`.
    pub fn hush(&mut self, id: u32) {
        match self.lines.remove(&id) {
            Some(voice) => self.out.with(|m| m.stop(voice)),
            None => {
                self.hushed.insert(id);
            }
        }
    }

    /// Decodes voice stream `stream` on a thread and plays it at `at`.
    fn speak(
        &mut self,
        voices: Arc<Voices>,
        stream: u32,
        at: Option<Vec3>,
        falloff: Falloff,
        volume: f32,
        line: Option<u32>,
    ) {
        let Some(seconds) = voices
            .index
            .streams
            .get(stream as usize)
            .map(|s| s.seconds())
        else {
            return;
        };
        self.speaking
            .push(Instant::now() + Duration::from_secs_f32(seconds + 0.3));
        let tx = self.said.0.clone();
        std::thread::spawn(move || match voices.decode(stream as usize) {
            Ok(b) => drop(tx.send((b, at, falloff, volume.clamp(0.0, 2.0), line))),
            Err(e) => eprintln!("voice line {stream}: {e}"),
        });
    }

    fn random(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Counts down to the next ambient sound: true when one is due.
    pub fn ambient_due(&mut self, dt: f32) -> bool {
        self.ambient_wait -= dt;
        self.ambient_wait <= 0.0
    }

    /// Plays a town or jungle sound 10 to 40 m from `listener`, and waits
    /// 3 to 9 s for the next.
    pub fn play_ambient(&mut self, town: bool, listener: Vec3) {
        self.ambient_wait = 3.0 + self.random() * 6.0;
        let n = self.ambient[usize::from(!town)].len();
        if n == 0 {
            return;
        }
        let pick = ((self.random() * n as f32) as usize).min(n - 1);
        let angle = self.random() * std::f32::consts::TAU;
        let distance = 10.0 + self.random() * 30.0;
        let at = listener + Vec3::new(angle.cos() * distance, 2.0, angle.sin() * distance);
        let buffer = self.ambient[usize::from(!town)][pick].clone();
        self.out.with(|m| m.play(Play::at(buffer, at).gain(0.5)));
    }

    /// Decodes stream `i` of `which` ("music" or "ambience") on a thread.
    fn decode(&mut self, which: &'static str, i: usize) {
        let Some(streams) = self.streams.clone() else {
            return;
        };
        let tx = self.decoded.0.clone();
        std::thread::spawn(move || {
            let index = if which == "music" {
                &streams.music
            } else {
                &streams.ambience
            };
            match streams.decode(index, i) {
                Ok(b) => drop(tx.send((which, b))),
                Err(e) => eprintln!("sound: {which} {i}: {e}"),
            }
        });
    }

    /// Plays the fight's sounds where they happened.
    pub fn play_fight(&mut self, sounds: &[Sound]) {
        for sound in sounds {
            let (i, at, gain) = match *sound {
                Sound::Shot(at) => (0, at, 0.9),
                Sound::Launch(at) => (1, at, 1.0),
                Sound::Explosion(at) => (2, at, 1.4),
                Sound::Bomb(at) => (2, at, 2.5),
                Sound::Cannon(at) => (3, at, 1.3),
                Sound::Lob(at) => (4, at, 1.0),
                Sound::Collapse(at) => (5, at, 2.0),
            };
            if let Some(buffer) = self.fight[i].clone() {
                self.out.with(|m| m.play(Play::at(buffer, at).gain(gain)));
            }
        }
    }

    /// Runs the engine of the vehicle being driven (`None`: on foot) for
    /// `dt` seconds: its bank's steady loops crossfaded and pitched as it
    /// revs with speed and throttle, its start and stop sounds as it is
    /// got in and out of. A bank is read the first time it's driven.
    pub fn engine(&mut self, driving: Option<EngineInput>, dt: f32) {
        if self
            .engine
            .as_ref()
            .is_some_and(|e| Some(e.bank) != driving.map(|d| d.bank))
            && let Some(e) = self.engine.take()
        {
            self.out.with(|m| {
                for id in &e.voices {
                    m.stop(*id);
                }
                if let Some(stop) = &e.sounds.stop {
                    m.play(Play::flat(stop.clone()).gain(ENGINE_GAIN));
                }
            });
        }
        let Some(input) = driving else {
            return;
        };
        if self.engine.is_none() {
            // Being read, or without engine sounds.
            let Some(sounds) = self.engine_sounds(input.bank) else {
                return;
            };
            let voices = self.out.with(|m| {
                if let Some(start) = &sounds.start {
                    m.play(Play::flat(start.clone()).gain(ENGINE_GAIN));
                }
                sounds
                    .loops
                    .iter()
                    .map(|(buffer, _)| m.play(Play::flat(buffer.clone()).looped().gain(0.0)))
                    .collect()
            });
            self.engine = Some(Engine {
                bank: input.bank,
                sounds,
                voices,
                rpm: 0.0,
                running: 0.0,
            });
        }
        let Some(e) = &mut self.engine else {
            return;
        };
        e.running += dt;
        let load = input.throttle.abs().min(1.0);
        let want = (input.speed.abs() / input.top.max(1.0)).min(1.0) * 0.8 + load * 0.2;
        e.rpm += (want - e.rpm) * (dt * ENGINE_EASE).min(1.0);
        let fade = (e.running / ENGINE_FADE_IN).min(1.0);
        let gain = ENGINE_GAIN * fade * (0.85 + 0.15 * load);
        let mix = Engine::mix(&e.sounds, e.rpm);
        self.out.with(|m| {
            for (id, (g, pitch)) in e.voices.iter().zip(mix) {
                m.set_gain(*id, gain * g);
                m.set_pitch(*id, pitch);
            }
        });
    }

    /// Bank `bank`'s engine sounds, once read (`None` while they're read,
    /// or if it has none); the first ask reads them on a thread.
    fn engine_sounds(&mut self, bank: u32) -> Option<Arc<EngineSounds>> {
        while let Ok((bank, sounds)) = self.engine_loads.1.try_recv() {
            if sounds.is_none() {
                self.silent.insert(bank);
            }
            self.engines.insert(bank, sounds.map(Arc::new));
        }
        if let Some(sounds) = self.engines.get(&bank) {
            return sounds.clone();
        }
        self.engines.insert(bank, None);
        if let Some(install) = self.install.clone() {
            let tx = self.engine_loads.0.clone();
            std::thread::spawn(move || drop(tx.send((bank, load_engine(&install, bank)))));
        }
        None
    }

    /// Keeps the rotors of the helicopters in the air (`flying`: each
    /// one's guid, where it is and the sound bank its template names)
    /// heard where they are: the bank's lowest steady loop (the blades'
    /// beat) with its highest (the turbine) under it, from
    /// [`ROTOR_BANK`] where it names none (or one without them). One gone
    /// from the list falls silent.
    pub fn rotors(&mut self, flying: &[(u32, Vec3, Option<u32>)]) {
        let gone: Vec<u32> = self
            .rotors
            .keys()
            .filter(|g| !flying.iter().any(|f| f.0 == **g))
            .copied()
            .collect();
        for guid in gone {
            if let Some(voices) = self.rotors.remove(&guid) {
                self.out
                    .with(|m| voices.into_iter().for_each(|id| m.stop(id)));
            }
        }
        for &(guid, at, bank) in flying {
            if let Some(voices) = self.rotors.get(&guid) {
                self.out
                    .with(|m| voices.iter().for_each(|id| m.set_position(*id, at)));
                continue;
            }
            let bank = bank
                .filter(|b| !self.silent.contains(b))
                .unwrap_or_else(|| m2_assets::name_hash(ROTOR_BANK));
            let Some(sounds) = self.engine_sounds(bank) else {
                continue;
            };
            let n = sounds.loops.len();
            let voices = self.out.with(|m| {
                sounds
                    .loops
                    .iter()
                    .enumerate()
                    .filter(|(k, _)| *k == 0 || *k + 1 == n)
                    .map(|(k, (buffer, _))| {
                        let play = Play {
                            falloff: ROTOR_FALLOFF,
                            ..Play::at(buffer.clone(), at)
                        };
                        m.play(
                            play.looped()
                                .gain(if k == 0 { ROTOR_GAIN } else { ROTOR_WHINE }),
                        )
                    })
                    .collect()
            });
            self.rotors.insert(guid, voices);
        }
    }

    /// Starts the next music track, or stops the music.
    pub fn toggle_music(&mut self) {
        if let Some(id) = self.music.take() {
            self.out.with(|m| m.stop(id));
            return;
        }
        let Some(count) = self.streams.as_ref().map(|s| s.music.streams.len()) else {
            return;
        };
        if count == 0 {
            return;
        }
        let track = self.next_track % count;
        self.next_track += 1;
        self.music_wanted = self
            .streams
            .as_ref()
            .and_then(|s| s.music.streams.get(track))
            .map(|s| (s.hash, false));
        eprintln!("music: track {track} of {count}");
        self.decode("music", track);
    }

    /// Starts what finished decoding and moves the listener.
    pub fn update(&mut self, listener: Vec3, forward: Vec3) {
        if let Some(rx) = &self.voices_loading
            && let Ok(v) = rx.try_recv()
        {
            self.voices = Some(Arc::new(v));
            self.voices_loading = None;
        }
        while let Ok((buffer, at, falloff, gain, line)) = self.said.1.try_recv() {
            if line.is_some_and(|l| self.hushed.remove(&l)) {
                continue;
            }
            // Scripted lines (the radio's too) are dialogue; the rest,
            // hers included, are people's barks.
            let category = if line.is_some() {
                Category::Vo
            } else {
                Category::Chatter
            };
            let play = match at {
                Some(at) => Play {
                    falloff,
                    ..Play::at(buffer, at)
                },
                None => Play::flat(buffer),
            }
            .category(category);
            let voice = self.out.with(|m| m.play(play.gain(gain)));
            if let Some(l) = line {
                self.lines.insert(l, voice);
            }
        }
        self.lines
            .retain(|_, v| self.out.with(|m| m.is_playing(*v)));
        while let Ok((which, buffer)) = self.decoded.1.try_recv() {
            if which == "music" {
                let Some((hash, looped)) = self.music_wanted else {
                    continue;
                };
                if buffer.hash != hash {
                    continue;
                }
                self.music_wanted = None;
                let play = Play::flat(buffer)
                    .gain(MUSIC_GAIN)
                    .category(Category::Music);
                let play = if looped { play.looped() } else { play };
                if let Some(old) = self.music.take() {
                    self.out.with(|m| m.stop(old));
                }
                self.music = Some(self.out.with(|m| m.play(play)));
                continue;
            }
            self.out
                .with(|m| m.play(Play::flat(buffer).looped().gain(0.5)));
        }
        self.out.with(|m| m.set_listener(listener, forward));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_heard_engine_runs_between_its_heard_pitches() {
        let mut e = engine(&[Some(125.0)]);
        e.heard = Some((0.84, 1.0));
        assert_eq!(Engine::mix(&e, 0.0), vec![(1.0, 0.84)]);
        assert_eq!(Engine::mix(&e, 1.0), vec![(1.0, 1.0)]);
    }

    #[test]
    fn engines_go_by_kind_and_name() {
        assert_eq!(engine_bank("tank", "al_veh_tank_m1"), "veh_tankbig");
        assert_eq!(
            engine_bank("car", "al_veh_apc_stryker"),
            "veh_largedieselold"
        );
        assert_eq!(engine_bank("apc", "vz_veh_apc_m113"), "veh_tankdiesel");
        assert_eq!(engine_bank("truck", "semi_orange"), "veh_semi");
        assert_eq!(
            engine_bank("car", "global_veh_68valiant4door"),
            "veh_largegascrappy"
        );
        assert_eq!(
            engine_bank("truck", "al_veh_truck_hmmwv"),
            "veh_largegascrappy"
        );
        assert_eq!(
            engine_bank("car", "global_veh_hondacrx"),
            "veh_4cylinderold"
        );
        assert_eq!(engine_bank("truck", "vz_veh_truck_m151"), "veh_jeep");
        assert_eq!(engine_bank("boat", "Speedboat"), "veh_boatracing");
        assert_eq!(engine_bank("boat", "Jetski (CIV)"), "veh_boatjetski");
        assert_eq!(engine_bank("car", "civ_veh_car_sedan"), "veh_4cylindernew");
        // The template database's banks, by name hash.
        assert_eq!(bank_name(0x3714_581c), Some("veh_largegascrappy"));
        assert_eq!(bank_name(0x3621_dfe1), Some("veh_jeep"));
        // Round 88's boat banks, by the hashes in the world archive.
        assert_eq!(bank_name(0xa3ca_3b95), Some("veh_boatracing"));
        assert_eq!(bank_name(0xad17_2ebf), Some("veh_boatjetski"));
    }

    #[test]
    fn a_cue_plays_its_variants() {
        let mut c = ClipSounds::new(
            [
                "fol_rustle_human_02",
                "fol_grenade_sidethrow_01",
                "fol_typing_01",
                "fol_typing_02",
            ]
            .map(|n| (name_hash(n), sound(name_hash(n))))
            .into(),
        );
        assert_eq!(c.sounds("fol_rustle_human").len(), 1);
        assert_eq!(c.sounds("FOL_Typing").len(), 2);
        assert!(c.sounds("footstep_walk").is_empty());
        assert_eq!(c.found.len(), 3);
    }

    fn sound(hash: u32) -> SoundBuffer {
        SoundBuffer {
            hash,
            channels: 1,
            rate: 100,
            samples: Arc::from(vec![0i16; 300]),
        }
    }

    fn engine(pitches: &[Option<f32>]) -> EngineSounds {
        EngineSounds {
            loops: pitches
                .iter()
                .enumerate()
                .map(|(i, &p)| (sound(i as u32), p))
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn an_engine_idles_on_its_lowest_loop_and_crossfades_up() {
        let e = engine(&[Some(40.0), Some(80.0), Some(160.0)]);
        let idle = Engine::mix(&e, 0.0);
        assert_eq!(idle[0], (1.0, 1.0));
        assert!(idle[1].0.abs() < 1e-6 && idle[2].0 == 0.0);
        // Halfway between the first two loops: both, at one note.
        let mid = Engine::mix(&e, 0.25);
        assert!((mid[0].0 - mid[1].0).abs() < 1e-5);
        assert!((40.0 * mid[0].1 - 80.0 * mid[1].1).abs() < 0.5, "{mid:?}");
        let top = Engine::mix(&e, 1.0);
        assert!(top[2].0 > 0.999 && (top[2].1 - 1.0).abs() < 1e-5);
        // Gains keep their power across the crossfade.
        for rpm in [0.1, 0.4, 0.6, 0.9] {
            let p: f32 = Engine::mix(&e, rpm).iter().map(|(g, _)| g * g).sum();
            assert!((p - 1.0).abs() < 1e-4, "{rpm}: {p}");
        }
    }

    #[test]
    fn unmeasured_loops_bend_a_little_and_one_loop_bends_with_revs() {
        let e = engine(&[Some(40.0), None]);
        let mid = Engine::mix(&e, 0.5);
        assert!((mid[0].1 - 1.125).abs() < 1e-5 && (mid[1].1 - 0.875).abs() < 1e-5);
        let one = engine(&[None]);
        assert_eq!(Engine::mix(&one, 0.0), vec![(1.0, 0.8)]);
        assert_eq!(Engine::mix(&one, 1.0), vec![(1.0, 1.5)]);
    }
}
