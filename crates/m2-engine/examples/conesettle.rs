//! Measures each gun's `WeaponScatter` (all seven fields) and how its cone
//! settles after a burst ([`Bloom`]), over every weapon template with a
//! model (gap round 13's harness).
//!
//! For each gun it fires a burst of 30 at its rate of fire, then rests,
//! and prints the cone after the burst and 0.25, 0.5, 1 and 2 seconds
//! later, and how long the cone takes to come back within 1% of
//! `ScatterMin`.
//!
//! ```text
//! cargo run --release -p m2-engine --example conesettle -- "<install dir>"
//! ```
//!
//! Prints numbers and template names only.

use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_engine::Install;
use m2_engine::weapons::{Bloom, Feel};

/// The bloom after a burst of `n` shots, `gap` seconds apart.
fn burst(feel: &Feel, gap: f32, n: usize) -> Bloom {
    let mut bloom = Bloom::default();
    for i in 0..n {
        if i > 0 {
            bloom.settle(feel, gap);
        }
        bloom.shot(feel);
    }
    bloom
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(root) = std::env::args().nth(1) else {
        eprintln!("usage: conesettle <install dir>");
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
    let arms = m2_engine::weapons::read(&db);
    let mut guns: Vec<_> = arms
        .weapons
        .values()
        .filter(|w| w.model.is_some())
        .collect();
    guns.sort_by(|a, b| a.name.cmp(&b.name).then(a.template.cmp(&b.template)));

    println!("{} guns (weapon templates with a model)", guns.len());
    println!("name\trate\tf0\tf1\tf2\tf3\tmin\tmax\tper\tburst\t+0.25s\t+0.5s\t+1s\t+2s\tback s");
    let step = 1.0 / 60.0;
    let (mut grows, mut back_sum, mut back_n) = (0, 0.0, 0);
    for w in &guns {
        let f = &w.feel;
        let gap = w.gap();
        let mut bloom = burst(f, gap, 30);
        let peak = f.cone(bloom.added);
        let mut at = Vec::new();
        let mut back = None;
        let mut t = 0.0;
        while t < 10.0 {
            bloom.settle(f, step);
            t += step;
            for mark in [0.25, 0.5, 1.0, 2.0] {
                if (t - mark).abs() < step / 2.0 {
                    at.push(f.cone(bloom.added));
                }
            }
            if back.is_none() && f.cone(bloom.added) <= f.scatter_min() * 1.01 + 1e-4 {
                back = Some(t);
            }
        }
        if peak > f.scatter_min() + 1e-3 {
            grows += 1;
            if let Some(b) = back {
                back_sum += b;
                back_n += 1;
            }
        }
        let s = &f.scatter;
        println!(
            "{}\t{:.0}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{peak:.2}\t{}\t{}",
            w.name,
            w.rate_of_fire,
            s[0],
            s[1],
            s[2],
            s[3],
            s[4],
            s[5],
            s[6],
            at.iter()
                .map(|c| format!("{c:.2}"))
                .collect::<Vec<_>>()
                .join("\t"),
            back.map_or("-".into(), |b| format!("{b:.2}")),
        );
    }
    println!(
        "summary: {} guns, {grows} grow in a burst of 30, mean settle back {:.2} s over {back_n}",
        guns.len(),
        if back_n > 0 {
            back_sum / back_n as f32
        } else {
            0.0
        },
    );

    // Every component on the guns' templates: how many guns carry it, its
    // fields' hashes and the carbine's words, to find any other table a
    // settle rate could sit in.
    if std::env::args().nth(2).as_deref() == Some("components") {
        let ids: std::collections::HashSet<u32> = guns.iter().map(|w| w.template).collect();
        let carbine = guns
            .iter()
            .find(|w| w.name.contains("Carbine"))
            .map(|w| w.template);
        for c in &db.components {
            let Some(records) = c.records() else {
                continue;
            };
            let on = records
                .iter()
                .filter(|(id, _)| ids.contains(id))
                .map(|(id, _)| *id)
                .collect::<std::collections::HashSet<_>>()
                .len();
            if on == 0 {
                continue;
            }
            println!(
                "component {} on {on} guns, {} fields",
                c.name,
                c.fields.len()
            );
            let rec = carbine.and_then(|cb| records.iter().find(|(id, _)| *id == cb));
            for f in &c.fields {
                let word = rec.and_then(|(_, r)| {
                    let at = usize::from(f.offset);
                    r.get(at..at + 4)
                        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                });
                println!(
                    "  {:08x} ty {} {}",
                    f.name_hash,
                    f.ty,
                    word.map_or("-".into(), |w| format!("{w:08x} ({})", f32::from_bits(w))),
                );
            }
        }
    }
    Ok(())
}
