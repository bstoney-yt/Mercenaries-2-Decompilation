//! Lists the `Sounds` table's rows keyed by `Param` (a value other than
//! the table's wildcard) and tries names for those values.
//!
//! ```text
//! cargo run --release -p m2-engine --example soundparams -- "<install dir>" [names.txt]
//! ```
//!
//! `names.txt` holds one guess per line; a guess whose hash is a `Param`
//! value is printed beside it. Then, over every gun sound set, the shot
//! cues one shooter hears against those 64 shooters hear (the sets with
//! `variant_N` rows). Prints hashes and names only.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use m2_assets::animation_table::{ANIMATION_TABLE_TYPE, ANY, AnimationTable};
use m2_assets::name_hash;
use m2_engine::Install;
use m2_engine::gun_sounds::{GunCues, SOUNDS_TABLE};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: soundparams <install dir> [names.txt]");
        std::process::exit(2);
    };
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let data = source.asset(SOUNDS_TABLE, ANIMATION_TABLE_TYPE)?;
    let table = AnimationTable::parse(&data)?;
    let cols: Vec<usize> = ["SoundSet", "EventHandle", "Param", "SoundID"]
        .iter()
        .map(|c| table.column(c).expect("column"))
        .collect();
    let mut names: HashMap<u32, String> = HashMap::new();
    if let Some(path) = args.get(1) {
        for line in std::fs::read_to_string(path)?.lines() {
            let w = line.trim();
            if !w.is_empty() {
                names.insert(name_hash(w), w.to_owned());
            }
        }
    }
    let name = |h: u32| names.get(&h).cloned().unwrap_or_default();
    println!("rows {}", table.rows());
    let mut keyed: BTreeMap<(u32, u32), Vec<(u32, u32)>> = BTreeMap::new();
    let mut params: BTreeMap<u32, usize> = BTreeMap::new();
    for r in 0..table.rows() {
        let c: Vec<u32> = cols
            .iter()
            .map(|&k| table.cell(r, k).unwrap_or(0))
            .collect();
        if c[2] != ANY {
            keyed.entry((c[0], c[1])).or_default().push((c[2], c[3]));
            *params.entry(c[2]).or_default() += 1;
        }
    }
    println!("keyed set/event pairs {}", keyed.len());
    for ((set, ev), rows) in &keyed {
        println!("set {set:08x} {} event {ev:08x} {}", name(*set), name(*ev));
        for (p, cue) in rows {
            println!("  param {p:08x} {:<24} cue {cue:08x}", name(*p));
        }
    }
    println!("distinct params {}", params.len());
    for (p, n) in &params {
        println!("param {p:08x} rows {n} {}", name(*p));
    }
    // Every gun set's shot, the hero's and others': the cues one shooter
    // plays ([`GunCues::shot`]) against those 64 shooters play
    // ([`GunCues::shot_by`]).
    let cues = GunCues::from_table(&table).ok_or("a column is missing")?;
    let sets: BTreeSet<u32> = (0..table.rows())
        .filter_map(|r| table.cell(r, cols[0]))
        .filter(|&s| s != ANY)
        .collect();
    let (mut one, mut many, mut shooting) = (0, 0, 0);
    for &set in &sets {
        for npc in [false, true] {
            let Some(first) = cues.shot(set, npc) else {
                continue;
            };
            let by: BTreeSet<u32> = (0..64).filter_map(|s| cues.shot_by(set, npc, s)).collect();
            shooting += 1;
            one += 1;
            many += by.len();
            if by.len() > 1 || !by.contains(&first) {
                println!(
                    "shot set {set:08x} {}{}: one shooter {first:08x}, 64 shooters {}",
                    name(set),
                    if npc { " (not the hero)" } else { "" },
                    by.iter()
                        .map(|c| format!("{c:08x}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                );
            }
        }
    }
    println!(
        "gun shots (set x hero/others) {shooting}: cues heard {one} -> {many}; \
         set/event pairs with variants {}",
        cues.varied()
    );
    Ok(())
}
