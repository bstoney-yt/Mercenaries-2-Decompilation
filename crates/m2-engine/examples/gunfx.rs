//! Measures what the game's data says about each gun's feel beyond its
//! shot (round 19's harness): its cone as it fires (`ScatterMin`,
//! `ScatterMax`, `ScatterPerShot`, [`Feel::cone`]), its spent cases
//! (`ShellEjectTemplate`'s effects, and whether its model has the
//! `ShellEjectHardpoint` node), how far its shot carries (its cue's
//! record distances, [`KeySounds::falloff`]) and whether a spinning gun
//! has a loop ([`GunCues::spin`]); then the bullet impact effects
//! (`global_particle_impact_bullet_*`) and their emitters, to find any
//! that last as a mark.
//!
//! ```text
//! cargo run --release -p m2-engine --example gunfx -- "<install dir>"
//! ```
//!
//! Prints numbers, hashes and names only.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_engine::Install;
use m2_engine::audio::{KeySounds, world_cue_tables};
use m2_engine::gun_sounds::GunCues;
use m2_engine::particles::{Library, template_effects};
use m2_engine::weapons::{Bloom, Feel};

/// The cone after `n` shots in a row at its rate of fire.
fn cone_after(feel: &Feel, gap: f32, n: usize) -> f32 {
    let mut bloom = Bloom::default();
    for _ in 1..n {
        bloom.shot(feel);
        bloom.settle(feel, gap);
    }
    feel.cone(bloom.added)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(root) = std::env::args().nth(1) else {
        eprintln!("usage: gunfx <install dir>");
        std::process::exit(2);
    };
    let install = Install::find(Path::new(&root))?;
    let mut source = install.open_world()?;
    let db_hash = {
        let index = source.archive().index();
        index
            .assets()
            .iter()
            .find(|a| index.type_hash(a) == TEMPLATE_DB_TYPE)
            .map(|a| a.name_hash)
            .ok_or("no template database")?
    };
    let data = source.asset(db_hash, TEMPLATE_DB_TYPE)?;
    let db = Layer::parse(&data)?;
    let names = db.names();
    let name = |t: u32| {
        names
            .get(&t)
            .map_or(format!("{t:08x}"), |n| (*n).to_owned())
    };
    let arms = m2_engine::weapons::read(&db);
    let mut guns: Vec<_> = arms
        .weapons
        .values()
        .filter(|w| w.model.is_some())
        .collect();
    guns.sort_by(|a, b| a.name.cmp(&b.name).then(a.template.cmp(&b.template)));
    let lib = Library::load(&mut source, template_effects(&db))?;
    let cues = GunCues::read(&install)?;
    let tables = world_cue_tables(&mut source);
    let mut resolved: HashMap<u32, Option<KeySounds>> = HashMap::new();

    println!("{} guns (weapon templates with a model)", guns.len());
    println!(
        "name\tfirst field\tmin\tmax\tper shot\tcone 1st\t5th\t10th\t30th\t\
         shell\tshell effects\teject node\tcue\tnear-far m\tspin"
    );
    let (mut named, mut blooms, mut shells, mut eject, mut carried, mut spins) = (0, 0, 0, 0, 0, 0);
    let mut fars: BTreeMap<String, usize> = BTreeMap::new();
    for w in &guns {
        let f = &w.feel;
        let gap = w.gap();
        if f.scatter_min() > 0.0 || f.scatter_max() > 0.0 {
            named += 1;
        }
        let cones = [1, 5, 10, 30].map(|n| cone_after(f, gap, n));
        if cones[3] > cones[0] + 1e-3 {
            blooms += 1;
        }
        let shell_fx = f.shell.map_or(0, |s| lib.of_template(s).len());
        if shell_fx > 0 {
            shells += 1;
        }
        // The eject node on the gun's own model.
        let node = f.shell_hardpoint.and_then(|hp| {
            let data = source.model(w.model?).ok()?;
            let model = data.parse().ok()?;
            m2_assets::vehicle::marker(&model, hp)
        });
        if node.is_some() {
            eject += 1;
        }
        let set = f.sound_set;
        let shot = set.and_then(|s| cues.shot(s, false));
        let spin = set.and_then(|s| cues.spin(s, false));
        if spin.is_some() {
            spins += 1;
        }
        let cue = shot.or(spin);
        let found = cue.and_then(|c| {
            resolved
                .entry(c)
                .or_insert_with(|| KeySounds::resolve(&mut source, &tables, c))
                .clone()
        });
        let reach = found.as_ref().and_then(|k| k.falloff);
        if let Some(r) = reach {
            carried += 1;
            *fars
                .entry(format!("{:.0}-{:.0}", r.near, r.far))
                .or_default() += 1;
        }
        println!(
            "{}\t{}\t{}\t{}\t{}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{}\t{shell_fx}\t{}\t{}\t{}\t{}",
            w.name,
            f.scatter[0],
            f.scatter_min(),
            f.scatter_max(),
            f.scatter_per_shot(),
            cones[0],
            cones[1],
            cones[2],
            cones[3],
            f.shell.map_or("-".to_owned(), name),
            node.map_or("-".to_owned(), |p| format!(
                "({:.2},{:.2},{:.2})",
                p[0], p[1], p[2]
            )),
            cue.map_or("-".to_owned(), |c| format!("{c:08x}")),
            reach.map_or("-".to_owned(), |r| format!("{:.0}-{:.0}", r.near, r.far)),
            match (spin, &found) {
                (Some(_), Some(k)) => format!("loop {:.2} s", k.sounds[0].0.seconds()),
                (Some(_), None) => "loop unheard".to_owned(),
                _ => "-".to_owned(),
            }
        );
        if let Some(k) = &found
            && k.how.starts_with("mixed")
        {
            println!("  {}", k.how);
        }
    }
    let n = guns.len();
    println!(
        "summary: named scatter {named}/{n}, cone grows when held {blooms}/{n}; \
         shell effect {shells}/{n}, eject node on the model {eject}/{n}; \
         shot cue with record distances {carried}/{n} ({}); spinning guns with a loop {spins}",
        fars.iter()
            .map(|(k, v)| format!("{k} m x{v}"))
            .collect::<Vec<_>>()
            .join(", ")
    );

    // Bullet impacts: each impact template's effects, and the emitters that
    // last (a mark would) or stand still.
    println!("impact\teffects\temitters\tlongest life s\tslowest speed\tbiggest size m\ttextures");
    let mut impacts: Vec<(u32, String)> = names
        .iter()
        .filter(|(_, n)| {
            n.starts_with("global_particle_impact_")
                || n.starts_with("3ddecal_")
                || n.contains("decal")
        })
        .map(|(&t, n)| (t, (*n).to_owned()))
        .collect();
    impacts.sort_by(|a, b| a.1.cmp(&b.1));
    for (t, n) in &impacts {
        let specs: Vec<_> = lib
            .of_template(*t)
            .iter()
            .filter_map(|h| lib.effects.get(h))
            .collect();
        let emitters: Vec<_> = specs.iter().flat_map(|s| &s.emitters).collect();
        let life = emitters
            .iter()
            .map(|e| e.life.at(0.0) + e.life_var)
            .fold(0.0_f32, f32::max);
        let slowest = emitters
            .iter()
            .map(|e| e.speed.at(0.0))
            .fold(f32::INFINITY, f32::min);
        let size = emitters
            .iter()
            .map(|e| e.size.at(0.0))
            .fold(0.0_f32, f32::max);
        let textures: usize = emitters.iter().map(|e| e.textures).sum();
        println!(
            "{n}\t{}\t{}\t{life:.2}\t{}\t{size:.2}\t{textures}",
            specs.len(),
            emitters.len(),
            if slowest.is_finite() {
                format!("{slowest:.2}")
            } else {
                "-".to_owned()
            }
        );
    }
    // The `global_impact_*` effects, each emitter.
    for m in IMPACT_MATERIALS {
        let n = format!("global_impact_{m}");
        let Some(spec) = lib.effects.get(&m2_assets::name_hash(&n)) else {
            continue;
        };
        let users: Vec<String> = lib
            .by_template
            .iter()
            .filter(|(_, fx)| fx.contains(&spec.hash))
            .map(|(&t, _)| name(t))
            .collect();
        println!(
            "effect {n}: {} emitters; templates {}",
            spec.emitters.len(),
            users.join(", ")
        );
        for e in &spec.emitters {
            println!(
                "  life {:.2}+-{:.2} s, speed {:.2}, size {:.3} m, rate {:.1}, spread {:.0},                  textures {}/{}, glows {}",
                e.life.at(0.0),
                e.life_var,
                e.speed.at(0.0),
                e.size.at(0.0),
                e.rate.at(0.0),
                e.spread.at(0.0),
                e.found,
                e.textures,
                e.glows()
            );
        }
    }

    // A bullet hole's texture, by guessed names: in the particle atlas or
    // as any asset of the world archive.
    let index = source.archive().index();
    let assets: std::collections::HashSet<u32> =
        index.assets().iter().map(|a| a.name_hash).collect();
    let mut guesses = 0;
    let mut found = Vec::new();
    for stem in DECAL_GUESSES {
        for material in [
            "",
            "_metal",
            "_wood",
            "_concrete",
            "_glass",
            "_brick",
            "_dirt",
        ] {
            for tail in ["", "01", "_01", "_a"] {
                for prefix in ["", "fx_", "global_", "global_decal_", "decal_", "particle_"] {
                    let n = format!("{prefix}{stem}{material}{tail}");
                    let h = m2_assets::name_hash(&n);
                    guesses += 1;
                    if lib.atlas.rects.contains_key(&h) || assets.contains(&h) {
                        found.push(format!(
                            "{n} ({h:08x}{})",
                            if lib.atlas.rects.contains_key(&h) {
                                ", atlas"
                            } else {
                                ""
                            }
                        ));
                    }
                }
            }
        }
    }
    println!(
        "bullet-hole names: {guesses} guesses, {} found: {}",
        found.len(),
        found.join(", ")
    );
    Ok(())
}

const IMPACT_MATERIALS: [&str; 10] = [
    "metal", "concrete", "glass", "brick", "dirt", "wood", "stone", "leaves", "water", "blood",
];

const DECAL_GUESSES: [&str; 12] = [
    "bullethole",
    "bullet_hole",
    "bulletholes",
    "bullet_holes",
    "hole",
    "bullet",
    "impact",
    "decal_bullet",
    "glasscrack",
    "glass_crack",
    "crack",
    "bulletdecal",
];
