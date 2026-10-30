//! Lists the effects a model's part states start (`StartEmitter`) and
//! stop, by state, each with how long the effect template runs
//! (`RedEffectComponent`'s `Duration` and `DurationVar`, and whether it
//! deletes itself, `AutoDelete`): a survey for how long a wreck's or a
//! broken building's fires burn.
//!
//! ```text
//! cargo run --release -p m2-engine --example emitters -- "<install dir>" <model hash or name>...
//! ```
//!
//! A name is hashed with the asset name hash. Prints names, hashes and
//! numbers only.

use std::collections::HashMap;
use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_assets::name_hash;
use m2_engine::Install;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        return Err("usage: emitters <install dir> <model hash or name>...".into());
    };
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
    // Template by name hash, and its effect's run.
    let by_hash: HashMap<u32, u32> = names.iter().map(|(&id, n)| (name_hash(n), id)).collect();
    let red = db
        .component("RedEffectComponent")
        .ok_or("no RedEffectComponent")?;
    let field = |n: &str| {
        red.fields
            .iter()
            .find(|f| f.name_hash == name_hash(n))
            .map(|f| usize::from(f.offset))
    };
    let (duration, var, auto) = (field("Duration"), field("DurationVar"), field("AutoDelete"));
    let mut runs: HashMap<u32, (f32, f32, u32)> = HashMap::new();
    for (id, r) in red.records().unwrap_or_default() {
        let word = |o: Option<usize>| {
            o.and_then(|o| r.get(o..o + 4))
                .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        runs.entry(id).or_insert((
            f32::from_bits(word(duration)),
            f32::from_bits(word(var)),
            word(auto),
        ));
    }
    let describe = |h: u32| {
        let Some(&t) = by_hash.get(&h) else {
            return format!("{h:08x} (no template)");
        };
        let name = names.get(&t).copied().unwrap_or("?");
        match runs.get(&t) {
            Some((d, v, a)) => format!("{name}: {d} s +-{v}, autodelete {a}"),
            None => format!("{name}: no RedEffectComponent"),
        }
    };
    for arg in &args[1..] {
        let hash = u32::from_str_radix(arg.trim_start_matches("0x"), 16)
            .ok()
            .filter(|_| arg.starts_with("0x"))
            .unwrap_or_else(|| name_hash(arg));
        let data = match source.model(hash) {
            Ok(d) => d,
            Err(e) => {
                println!("== {arg}: {e}");
                continue;
            }
        };
        let model = data.parse()?;
        println!("== {arg} ({hash:08x})");
        let Some(states) = &model.states else {
            println!("  no part states");
            continue;
        };
        for (k, m) in states.machines.iter().enumerate() {
            for ((state, _), acts) in m.states.iter().zip(&m.acts) {
                for (effect, at) in &acts.emitters {
                    println!(
                        "  machine {k} state {state:08x} starts {} at {at:?}",
                        describe(*effect)
                    );
                }
                for effect in &acts.stops {
                    println!(
                        "  machine {k} state {state:08x} stops {}",
                        describe(*effect)
                    );
                }
            }
        }
    }
    Ok(())
}
