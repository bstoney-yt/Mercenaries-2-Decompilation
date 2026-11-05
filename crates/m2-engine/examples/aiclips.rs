//! Lists the clip of every weapon a vehicle template mounts: its
//! `iClipSize`, `MaxAmmoReserve` (255: the 360's -1, never runs out),
//! `RateOfFire` and how long a clip lasts with the trigger held, so the
//! AI's clip and reload ([`m2_engine::weapons::AiClip`]) can be checked
//! against the data.
//!
//! ```text
//! cargo run --release -p m2-engine --example aiclips -- "<install dir>"
//! ```
//!
//! Prints numbers and template names only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_engine::Install;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(root) = std::env::args().nth(1) else {
        eprintln!("usage: aiclips <install dir>");
        std::process::exit(2);
    };
    let install = Install::find(Path::new(&root))?;
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
    let arms = m2_engine::weapons::read(&db);
    let mut mounted: BTreeMap<String, (usize, u32, u32, f32, u32)> = BTreeMap::new();
    for a in arms.armaments.values() {
        for m in &a.mounts {
            if let Some(w) = arms.weapons.get(&m.weapon) {
                let e = mounted.entry(w.name.clone()).or_insert((
                    0,
                    w.clip,
                    w.reserve_clips,
                    w.rate_of_fire,
                    w.pellets,
                ));
                e.0 += 1;
            }
        }
    }
    println!("weapon\tmounts\tclip\treserve\trate/min\tclip s");
    for (name, (n, clip, reserve, rate, _)) in &mounted {
        let secs = *clip as f32 * 60.0 / rate.max(1.0);
        println!("{name}\t{n}\t{clip}\t{reserve}\t{rate:.0}\t{secs:.1}");
    }
    Ok(())
}
