//! Measures how loud each gun's shot plays in the alpha (playtest 5's
//! "the carbine is far louder than the other guns"): over every weapon
//! template with a sound set, the hero's shot cue (`weapon_fire`, or a
//! spinning gun's loop), its record's volume and distances, and its
//! sounds' levels (peak and RMS, dBFS); then what the hero hears at the
//! camera's distance, the level times the cue's volume times the falloff
//! there, with the alpha's levelling applied when `level` is given
//! ([`m2_engine::gun_sounds::shot_gain`]).
//!
//! ```text
//! cargo run --release -p m2-engine --example shotlevels -- "<install dir>" [level]
//! ```
//!
//! Prints numbers, hashes and names only.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::time::Instant;

use m2_engine::Install;
use m2_engine::audio::{Falloff, KeySounds, world_cue_tables};
use m2_engine::gun_sounds::{GunCues, shot_gain};
use m2_engine::world::WorldIndex;

/// How far the hero's gun is from the listener (the camera behind her
/// shoulder), metres: the alpha's on-foot camera distance, about.
const CAMERA: f32 = 3.5;

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-6).log10()
}

/// The loudest 100 ms of a sound, RMS, 0 to 1.
fn window_rms(s: &m2_engine::SoundBuffer) -> f32 {
    let ch = usize::from(s.channels.max(1));
    let n = (s.rate as usize / 10).max(1) * ch;
    s.samples
        .chunks(n)
        .map(|w| {
            (w.iter().map(|&v| (v as f32 / 32768.0).powi(2)).sum::<f32>() / w.len() as f32).sqrt()
        })
        .fold(0.0, f32::max)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: shotlevels <install dir> [level]");
        std::process::exit(2);
    };
    let level = args.iter().any(|a| a == "level");
    let npc = args.iter().any(|a| a == "npc");
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let world = WorldIndex::build(&mut source)?;
    let db_ = &world.template_db;
    let cues = GunCues::read(&install)?;
    let t0 = Instant::now();
    let tables = world_cue_tables(&mut source);
    println!("cue tables read in {:.2} s", t0.elapsed().as_secs_f32());

    // Cue -> (weapon names, sound set).
    let mut by_cue: BTreeMap<u32, (Vec<String>, u32)> = BTreeMap::new();
    let mut rof: HashMap<String, (f32, bool)> = HashMap::new();
    for w in db_.arms.weapons.values() {
        let Some(set) = w.feel.sound_set else {
            continue;
        };
        let Some(cue) = cues.spin(set, npc).or_else(|| cues.shot(set, npc)) else {
            continue;
        };
        rof.insert(w.name.clone(), (w.rate_of_fire, w.semi));
        let e = by_cue.entry(cue).or_insert_with(|| (Vec::new(), set));
        if !e.0.contains(&w.name) {
            e.0.push(w.name.clone());
        }
    }
    println!(
        "cue\tset\tvolume\tnear\tfar\tvariants\tpeak dB\trms dB\t100ms dB\tlong s\theard dB\tload s\trof\tsustained dB\thow\tweapons"
    );
    let mut heard: HashMap<u32, f32> = HashMap::new();
    for (&cue, (names, set)) in &by_cue {
        let t = Instant::now();
        let Some(k) = KeySounds::resolve(&mut source, &tables, cue) else {
            println!(
                "{cue:08x}\t{set:08x}\t-\t-\t-\t0\t-\t-\t-\t-\t{}",
                names.join(",")
            );
            continue;
        };
        let secs = t.elapsed().as_secs_f32();
        let (mut peak, mut rms, mut win, mut len, mut energy) =
            (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for (s, _) in &k.sounds {
            let p = s
                .samples
                .iter()
                .map(|&v| (v as f32 / 32768.0).abs())
                .fold(0.0, f32::max);
            peak = peak.max(p);
            rms = rms.max(s.loudness());
            win = win.max(window_rms(s));
            len = len.max(s.seconds());
            energy = energy.max(s.loudness().powi(2) * s.seconds());
        }
        let falloff = k.falloff.unwrap_or_default();
        let gain = if level {
            shot_gain(k.volume, win)
        } else {
            k.volume
        };
        let at = gain * win * Falloff::gain(&falloff, CAMERA);
        heard.insert(cue, db(at));
        let r = names
            .first()
            .and_then(|n| rof.get(n))
            .copied()
            .unwrap_or((0.0, false));
        println!(
            "{cue:08x}\t{set:08x}\t{:.2}\t{:.0}\t{:.0}\t{}\t{:.1}\t{:.1}\t{:.1}\t{len:.2}\t{:.1}\t{secs:.2}\t{:.1}{}\t{:.1}\t{}\t{}",
            k.volume,
            falloff.near,
            falloff.far,
            k.sounds.len(),
            db(peak),
            db(rms),
            db(win),
            db(at),
            r.0,
            if r.1 { " semi" } else { "" },
            10.0 * (gain.powi(2) * energy * r.0.max(0.1) / 60.0)
                .max(1e-9)
                .log10(),
            k.how,
            names.first().map_or("", String::as_str)
        );
    }
    let mut v: Vec<f32> = heard.values().copied().collect();
    v.sort_by(f32::total_cmp);
    if !v.is_empty() {
        let median = v[v.len() / 2];
        let spread: Vec<f32> = v.iter().map(|x| (x - median).abs()).collect();
        let worst = spread.iter().copied().fold(0.0, f32::max);
        let mean = spread.iter().sum::<f32>() / spread.len() as f32;
        println!(
            "{} shot cues: heard at {CAMERA} m, median {median:.1} dB, min {:.1}, max {:.1}; \
             off the median by {mean:.1} dB on average, {worst:.1} at worst",
            v.len(),
            v[0],
            v[v.len() - 1]
        );
    }
    Ok(())
}
