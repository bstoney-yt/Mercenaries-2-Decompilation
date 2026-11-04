//! Lists what every blast template (`Explosion (...)`) says it sounds
//! like: its `SoundEffect` record's sound set, bank and other words, the
//! `Sounds` table's rows for that set (event, param, cue), and whether
//! each cue finds a sound in the world's cue tables.
//!
//! ```text
//! cargo run --release -p m2-engine --example blastsounds -- "<install dir>" [names.txt]
//! ```
//!
//! `names.txt` holds guesses for event names, one a line; an event whose
//! hash one makes prints by that name. Prints hashes, names and numbers
//! only.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use m2_assets::animation_table::{ANIMATION_TABLE_TYPE, AnimationTable};
use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_assets::name_hash;
use m2_engine::Install;
use m2_engine::audio::{KeySounds, world_cue_tables};
use m2_engine::gun_sounds::SOUNDS_TABLE;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = args
        .first()
        .ok_or("usage: blastsounds <install dir> [names.txt]")?;
    let mut dict: HashMap<u32, String> = HashMap::new();
    if let Some(f) = args.get(1) {
        for l in std::fs::read_to_string(f)?.lines() {
            let n = l.trim();
            if !n.is_empty() {
                dict.insert(name_hash(n), n.to_owned());
            }
        }
    }
    let name = |h: u32| dict.get(&h).cloned().unwrap_or(format!("{h:08x}"));
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
    let names = db.names();
    let c = db.component("SoundEffect").ok_or("no SoundEffect")?;
    let field = |h: u32| {
        c.fields
            .iter()
            .find(|f| f.name_hash == h)
            .map(|f| usize::from(f.offset))
    };
    let word = |r: &[u8], at: Option<usize>| {
        at.and_then(|a| r.get(a..a + 4))
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let set_at = field(name_hash("SoundSet"));
    let others: Vec<(u32, Option<usize>)> = c
        .fields
        .iter()
        .filter(|f| f.name_hash != name_hash("SoundSet"))
        .map(|f| (f.name_hash, Some(usize::from(f.offset))))
        .collect();

    let table = AnimationTable::parse(&source.asset(SOUNDS_TABLE, ANIMATION_TABLE_TYPE)?)?;
    let cols = [
        table.column("SoundSet").ok_or("col")?,
        table.column("EventHandle").ok_or("col")?,
        table.column("Param").ok_or("col")?,
        table.column("SoundID").ok_or("col")?,
    ];
    let mut rows: HashMap<u32, Vec<[u32; 3]>> = HashMap::new();
    for r in 0..table.rows() {
        let v: Vec<u32> = cols.iter().filter_map(|&k| table.cell(r, k)).collect();
        if let [s, e, p, q] = v[..] {
            rows.entry(s).or_default().push([e, p, q]);
        }
    }
    let tables = world_cue_tables(&mut source);

    let mut by_set: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for (id, r) in c.records().unwrap_or_default() {
        let n = names.get(&id).copied().unwrap_or("?");
        if !n.starts_with("Explosion") {
            continue;
        }
        let set = word(r, set_at).unwrap_or(0);
        let rest: Vec<String> = others
            .iter()
            .map(|(h, a)| {
                let w = word(r, *a).unwrap_or(0);
                let f = f32::from_bits(w);
                if f.is_normal() && f.abs() < 1e6 {
                    format!("{h:08x}={f}")
                } else {
                    format!("{h:08x}=#{w:08x}")
                }
            })
            .collect();
        by_set
            .entry(set)
            .or_default()
            .push(format!("{n} [{}]", rest.join(" ")));
    }
    let mut blasts = 0;
    let mut heard = 0;
    for (set, ts) in &by_set {
        let rs = rows.get(set).cloned().unwrap_or_default();
        println!(
            "== set {set:08x} ({} templates, {} rows)",
            ts.len(),
            rs.len()
        );
        for [e, p, q] in &rs {
            let found = KeySounds::resolve(&mut source, &tables, *q);
            println!(
                "   row event {} param {p:08x} cue {q:08x}: {}",
                name(*e),
                found.as_ref().map_or("no sound".to_owned(), |k| format!(
                    "{} sounds, volume {:.2}, falloff {:?}, {}",
                    k.sounds.len(),
                    k.volume,
                    k.falloff,
                    k.how
                ))
            );
        }
        for t in ts {
            println!("   {t}");
        }
        blasts += ts.len();
        if !rs.is_empty() {
            heard += ts.len();
        }
    }
    println!("{heard} of {blasts} blast templates' sound sets have Sounds rows");
    Ok(())
}
