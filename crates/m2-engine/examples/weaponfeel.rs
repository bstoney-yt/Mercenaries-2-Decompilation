//! Measures what firing each gun sounds and looks like, as the game's data
//! gives it: every gun template with a model, its `SoundEffect` bank and
//! `SoundSet`, `WeaponEffects` muzzle flash and `WeaponScatter`; then, as
//! the alpha plays them, the cue each gun fires with (the hero's and
//! anyone else's, [`GunCues`]) and the sounds that cue plays
//! ([`KeySounds::resolve`]), and the particle effects its flash template
//! shows (emitters, how many on the atlas, their sprite size).
//!
//! ```text
//! cargo run --release -p m2-engine --example weaponfeel -- "<install dir>" [banks]
//! ```
//!
//! `banks` adds each gun bank's sounds (length, channels, level). Prints
//! numbers, hashes and names only.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_assets::name_hash;
use m2_engine::Install;
use m2_engine::audio::{KeySounds, SoundLibrary, world_cue_tables};
use m2_engine::gun_sounds::GunCues;
use m2_engine::particles::{Library, Particles, template_effects};
use m2_engine::weapons::GUN_BANKS;
use m2_math::glam::Vec3;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: weaponfeel <install dir> [banks]");
        std::process::exit(2);
    };
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let index = source.archive().index();
    let db_hash = index
        .assets()
        .iter()
        .find(|a| index.type_hash(a) == TEMPLATE_DB_TYPE)
        .map(|a| a.name_hash)
        .ok_or("no template database")?;
    let data = source.asset(db_hash, TEMPLATE_DB_TYPE)?;
    let db = Layer::parse(&data)?;
    let names = db.names();
    let name = |t: u32| {
        names
            .get(&t)
            .map_or(format!("{t:08x}"), |n| (*n).to_owned())
    };
    let arms = m2_engine::weapons::read(&db);
    let bank_name = |b: u32| {
        GUN_BANKS
            .iter()
            .find(|f| name_hash(f) == b)
            .map_or_else(|| format!("{b:08x}"), |f| (*f).to_owned())
    };

    let mut guns: Vec<_> = arms
        .weapons
        .values()
        .filter(|w| w.model.is_some())
        .collect();
    guns.sort_by(|a, b| a.name.cmp(&b.name).then(a.template.cmp(&b.template)));
    println!("{} guns (weapon templates with a model)", guns.len());
    println!("name\trate/min\tsemi\tbank\tset\tflash\tshell hp\tshell\tscatter[7]");
    for w in &guns {
        let f = &w.feel;
        println!(
            "{}\t{:.0}\t{}\t{}\t{}\t{}\t{}\t{}\t{:?}",
            w.name,
            w.rate_of_fire,
            u8::from(w.semi),
            f.sound_bank.map_or("-".to_owned(), bank_name),
            f.sound_set.map_or("-".to_owned(), |s| format!("{s:08x}")),
            f.flash.map_or("-".to_owned(), name),
            f.shell_hardpoint
                .map_or("-".to_owned(), |h| format!("{h:08x}")),
            f.shell.map_or("-".to_owned(), name),
            f.scatter
        );
    }
    let count =
        |p: &dyn Fn(&m2_engine::weapons::WeaponSpec) -> bool| guns.iter().filter(|w| p(w)).count();
    println!(
        "templates: a bank {}, a sound set {}, a flash {}, a shell {}, a scatter {}",
        count(&|w| w.feel.sound_bank.is_some()),
        count(&|w| w.feel.sound_set.is_some()),
        count(&|w| w.feel.flash.is_some()),
        count(&|w| w.feel.shell.is_some()),
        count(&|w| w.feel.scatter.iter().any(|&s| s != 0.0)),
    );

    // Sounds, as the alpha plays them.
    let cues = GunCues::read(&install)?;
    let tables = world_cue_tables(&mut source);
    println!(
        "Sounds table: {} sets fire, {} fire as others; {} world cue tables",
        cues.sets_with(m2_engine::gun_sounds::FIRE),
        cues.sets_with(m2_engine::gun_sounds::FIRE_NPC),
        tables.len()
    );
    let mut resolved: HashMap<u32, Option<KeySounds>> = HashMap::new();
    let mut heard = |cue: u32, source: &mut _| {
        resolved
            .entry(cue)
            .or_insert_with(|| KeySounds::resolve(source, &tables, cue))
            .clone()
    };
    println!("name\tcue\tsounds\tnpc cue\tsounds");
    let (mut own, mut npc_own) = (0, 0);
    let mut by_sound: BTreeSet<Vec<u32>> = BTreeSet::new();
    let mut unheard: BTreeMap<String, String> = BTreeMap::new();
    for w in &guns {
        let set = w.feel.sound_set;
        let mut cells = Vec::new();
        for npc in [false, true] {
            let cue = set.and_then(|s| cues.shot(s, npc));
            let found = cue.and_then(|c| heard(c, &mut source));
            match (&cue, &found) {
                (Some(c), Some(k)) => {
                    if npc {
                        npc_own += 1;
                    } else {
                        own += 1;
                    }
                    let mut s: Vec<u32> = k.sounds.iter().map(|(b, _)| b.hash).collect();
                    s.sort_unstable();
                    cells.push(format!(
                        "{c:08x}\t{} ({:.2} s)",
                        s.len(),
                        k.sounds[0].0.seconds()
                    ));
                    by_sound.insert(s);
                }
                (Some(c), None) => {
                    cells.push(format!("{c:08x}\tnone"));
                    unheard.insert(w.name.clone(), format!("{c:08x} (npc {npc})"));
                }
                (None, _) => {
                    cells.push("-\t-".to_owned());
                    unheard.insert(w.name.clone(), format!("no cue (npc {npc})"));
                }
            }
        }
        println!("{}\t{}", w.name, cells.join("\t"));
    }
    println!(
        "guns heard with their own cue: hero {own} of {n}, others {npc_own} of {n}; \
         distinct shot sounds {}",
        by_sound.len(),
        n = guns.len()
    );
    for (g, why) in &unheard {
        println!("  not heard: {g}: {why}");
    }

    // Muzzle flashes: the effects each flash template shows.
    let lib = Library::load(&mut source, template_effects(&db))?;
    let mut flashes: BTreeMap<u32, usize> = BTreeMap::new();
    for w in &guns {
        if let Some(f) = w.feel.flash {
            *flashes.entry(f).or_default() += 1;
        }
    }
    let mut drawn = 0;
    for (f, users) in &flashes {
        let specs: Vec<_> = lib
            .of_template(*f)
            .iter()
            .filter_map(|h| lib.effects.get(h))
            .collect();
        let emitters: Vec<_> = specs.iter().flat_map(|s| &s.emitters).collect();
        let textured = emitters
            .iter()
            .filter(|e| e.textures > 0 && e.found == e.textures)
            .count();
        let size = emitters
            .iter()
            .map(|e| e.size.at(0.0))
            .fold(0.0_f32, f32::max);
        let life = emitters
            .iter()
            .map(|e| e.life.at(0.0))
            .fold(0.0_f32, f32::max);
        // Set off once and run a frame (60 a second), as the alpha does:
        // the sprites the first frame draws, and their biggest.
        let mut sim = Particles::default();
        sim.spawn_template(&lib, *f, Vec3::ZERO, 0.0, 1.0);
        sim.step(&lib, 1.0 / 60.0);
        let mut sprites = Vec::new();
        sim.sprites(&lib, &mut sprites);
        let widest = 2.0 * sprites.iter().map(|s| s.size).fold(0.0_f32, f32::max);
        if textured > 0 && !sprites.is_empty() {
            drawn += users;
        }
        println!(
            "flash {}: {users} guns, {} effects, {} emitters, {textured} textured, \
             biggest sprite {size:.2} m, longest life {life:.2} s; first frame {} sprites, \
             widest {:.2} m",
            name(*f),
            specs.len(),
            emitters.len(),
            sprites.len(),
            widest
        );
    }
    println!(
        "guns whose muzzle flash is a drawn game effect: {drawn} of {}",
        guns.len()
    );

    if args.iter().any(|a| a == "banks") {
        let mut banks: Vec<u32> = guns.iter().filter_map(|w| w.feel.sound_bank).collect();
        banks.sort_unstable();
        banks.dedup();
        let library = SoundLibrary::load_banks(&mut source, &banks);
        for b in banks {
            let Some((_, sounds)) = library.bank_hash(b) else {
                println!("bank {}: not read", bank_name(b));
                continue;
            };
            println!("bank {} ({} sounds)", bank_name(b), sounds.len());
            for (i, s) in sounds.iter().enumerate() {
                println!(
                    "  {i:>2} {:08x} {:>5.2} s {}ch {:>6.1} dB",
                    s.hash,
                    s.seconds(),
                    s.channels,
                    20.0 * s.loudness().max(1e-6).log10()
                );
            }
        }
    }
    Ok(())
}
