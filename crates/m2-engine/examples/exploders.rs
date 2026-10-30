//! Counts the island's placed objects that blow up when destroyed, as
//! the template database says (what their template leaves on dying,
//! `SpawnOnDeath`, is a blast, `Explosive`), against those a list of
//! model names picks; and prints, by template, how much each takes
//! (`Health`), how it burns down once worn low (`TickDamage`) and its
//! blast.
//!
//! ```text
//! cargo run --release -p m2-engine --example exploders -- "<install dir>" [model name]...
//! ```
//!
//! Prints counts, template names and numbers only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_assets::name_hash;
use m2_engine::Install;
use m2_engine::world::WorldIndex;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        return Err("usage: exploders <install dir> [model name]...".into());
    };
    let models: Vec<u32> = args[1..].iter().map(|m| name_hash(m)).collect();
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let world = WorldIndex::build(&mut source)?;
    let db = &world.template_db;
    let blast = |t: u32| {
        db.leavings
            .spawns
            .get(&t)
            .and_then(|s| s.iter().find_map(|s| db.explosions.get(s).map(|e| (*s, e))))
    };
    let by_model = world
        .objects
        .iter()
        .filter(|o| models.contains(&o.model))
        .count();
    let mut by_template: BTreeMap<String, (u32, usize)> = BTreeMap::new();
    let mut listed_too = 0;
    for o in &world.objects {
        if blast(o.template).is_none() {
            continue;
        }
        listed_too += usize::from(models.contains(&o.model));
        let name = db.names.get(&o.template).cloned().unwrap_or_default();
        by_template.entry(name).or_insert((o.template, 0)).1 += 1;
    }
    let by_data: usize = by_template.values().map(|v| v.1).sum();
    println!("by model list: {by_model}; by template data: {by_data} ({listed_too} in both)");
    for (name, (t, n)) in &by_template {
        let d = db.durability.get(t);
        let (s, e) = blast(*t).unwrap();
        let blast_name = db.names.get(&s).map_or("?", String::as_str);
        // What one of its own blasts does to it: its `ModifierKey` row's
        // cell for the blast's `DamageKey` (none: all of it).
        let own = match (db.modifiers.get(t), db.damage.as_ref()) {
            (Some(&row), Some(table)) => table.scale(row, e.kind),
            _ => None,
        };
        println!(
            "{name}\t{n}\thealth {:?}\tburn {:?}\tblast {blast_name}: radius {} damage {} least {} key {:08x}\trow {:08x?} takes {own:?} of its own",
            d.map(|d| d.health),
            d.and_then(|d| d.burn),
            e.radius,
            e.damage,
            e.least,
            e.kind,
            db.modifiers.get(t),
        );
    }
    Ok(())
}
