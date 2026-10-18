//! Measures how the game's data says soldiers fire each gun
//! (`WeaponHint`, gap round 13's harness): its fire ranges and its bursts,
//! far and close (`weapons::FireHint`), and what those give in rounds a burst
//! and share of the time spent firing, beside the alpha's old invented
//! bursts (3 to 5 rounds, 0.9 to 1.7 s apart).
//!
//! ```text
//! cargo run --release -p m2-engine --example weaponhint -- "<install dir>" [raw]
//! ```
//!
//! `raw` adds every `WeaponHint` field's word per gun. Prints numbers and
//! template names only.

use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_engine::Install;

/// The old invented bursts: 3 rounds plus up to 2, then 0.9 s plus up to
/// 0.8 s; on average 4 rounds and 1.3 s.
const OLD_ROUNDS: f32 = 4.0;
const OLD_GAP: f32 = 1.3;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: weaponhint <install dir> [raw]");
        std::process::exit(2);
    };
    let raw = args.iter().any(|a| a == "raw");
    let install = Install::find(Path::new(root))?;
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
    println!(
        "name\trate\tmin m\trange m\tangle\ton s\toff s\ton close\toff close\t\
         rounds\tfiring share\told rounds\told share"
    );
    let old_share = |gap: f32| OLD_ROUNDS * gap / (OLD_ROUNDS * gap + OLD_GAP);
    let (mut with, mut rounds_sum, mut share_sum, mut old_share_sum) = (0, 0.0, 0.0, 0.0);
    let mut range_70 = 0;
    for w in &guns {
        let gap = w.gap();
        let Some(h) = w.hint else {
            println!("{}\t{:.0}\t(no WeaponHint)", w.name, w.rate_of_fire);
            continue;
        };
        with += 1;
        let rounds = h.rounds(gap, false);
        let share = h.firing_share(gap, false);
        rounds_sum += rounds as f32;
        share_sum += share;
        old_share_sum += old_share(gap);
        if (h.range - 70.0).abs() < 0.5 {
            range_70 += 1;
        }
        println!(
            "{}\t{:.0}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{rounds}\t{share:.2}\t{OLD_ROUNDS}\t{:.2}",
            w.name,
            w.rate_of_fire,
            h.min_range,
            h.range,
            h.angle,
            h.on,
            h.off,
            h.on_close,
            h.off_close,
            old_share(gap),
        );
    }
    let n = with.max(1) as f32;
    println!(
        "summary: {with}/{} guns have a WeaponHint; FireRange 70 on {range_70}; \
         mean rounds a burst {:.1} (old {OLD_ROUNDS}); mean firing share {:.2} (old {:.2})",
        guns.len(),
        rounds_sum / n,
        share_sum / n,
        old_share_sum / n,
    );

    if raw && let Some(c) = db.component("WeaponHint") {
        let records = c.records().unwrap_or_default();
        for w in &guns {
            let Some((_, r)) = records.iter().find(|(id, _)| *id == w.template) else {
                continue;
            };
            let words: Vec<String> = c
                .fields
                .iter()
                .map(|f| {
                    let at = usize::from(f.offset);
                    r.get(at..at + 4).map_or("-".into(), |b| {
                        let v = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                        if f.ty == 7 {
                            format!("{}", f32::from_bits(v))
                        } else {
                            format!("{v:x}")
                        }
                    })
                })
                .collect();
            println!("raw {}\t{}", w.name, words.join("\t"));
        }
        println!(
            "raw fields\t{}",
            c.fields
                .iter()
                .map(|f| format!("{:08x}/{}", f.name_hash, f.ty))
                .collect::<Vec<_>>()
                .join("\t")
        );
    }
    Ok(())
}
