//! Measures how many different looks people can have: every model's mixer
//! (`MIXR`: its interchangeable parts and profiles), then every template
//! that names a `ModelMixerProfile`, with the looks its profile allows in
//! its own model.
//!
//! ```text
//! cargo run --release -p m2-engine --example looks -- "<install dir>" [word]...
//! ```
//!
//! Templates whose name holds a word (all, without words) are listed one
//! a line. Prints names, hashes and counts only.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use m2_assets::layer::TEMPLATE_DB_TYPE;
use m2_assets::model::MODEL_TYPE;
use m2_assets::{BlockContents, Layer, Model, Ucfx, name_hash};
use m2_engine::Install;

/// Looks are counted up to this many per template.
const CAP: usize = 100_000;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: looks <install dir> [word]...");
        std::process::exit(2);
    };
    let words: Vec<String> = args[1..].iter().map(|w| w.to_ascii_lowercase()).collect();
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;

    // Every model's mixer, block by block (each base asset once).
    let blocks = source.archive().index().blocks().len();
    let mut mixers = HashMap::new();
    let (mut models, mut with_mixr, mut unread) = (0usize, 0usize, Vec::new());
    let mut seen = HashSet::new();
    for b in 0..blocks {
        let Ok(decoded) = source.block(b) else {
            continue;
        };
        let Ok(contents) = BlockContents::parse(&decoded) else {
            continue;
        };
        for (e, data) in contents.iter() {
            if e.type_hash != MODEL_TYPE || e.unk_0x08 != 0 || !seen.insert(e.name_hash) {
                continue;
            }
            models += 1;
            let has_mixr = Ucfx::parse(data).is_ok_and(|u| {
                u.sections()
                    .iter()
                    .any(|s| s.depth == 0 && &s.fourcc == b"MIXR")
            });
            if !has_mixr {
                continue;
            }
            with_mixr += 1;
            match Model::parse(data) {
                Ok(m) => match m.mixer {
                    Some(mx) => {
                        mixers.insert(e.name_hash, (mx, m.nodes));
                    }
                    None => unread.push(e.name_hash),
                },
                Err(_) => unread.push(e.name_hash),
            }
        }
    }
    let symmetric = mixers.values().filter(|(m, _)| m.is_symmetric()).count();
    let parts: usize = mixers.values().map(|(m, _)| m.parts.len()).sum();
    let nodeless: usize = mixers
        .values()
        .map(|(m, _)| m.parts.iter().filter(|p| p.node.is_none()).count())
        .sum();
    let profiles: usize = mixers.values().map(|(m, _)| m.profiles.len()).sum();
    let versions: BTreeMap<u16, usize> = mixers.values().fold(BTreeMap::new(), |mut a, (m, _)| {
        *a.entry(m.version).or_default() += 1;
        a
    });
    println!(
        "models: {models}; with MIXR {with_mixr}, read {} (unread {:08x?}); \
         symmetric {symmetric}; parts {parts} ({nodeless} with no node); \
         profiles {profiles}; versions {versions:?}",
        mixers.len(),
        unread
    );
    // Per mixer: what a profile-less look (every part allowed) gives, and
    // the pairs that go together only one way round.
    let mut by_looks: BTreeMap<usize, usize> = BTreeMap::new();
    let mut hashes: Vec<&u32> = mixers.keys().collect();
    hashes.sort();
    for h in hashes {
        let (m, _) = &mixers[h];
        let looks = m.looks(&m.allowed(None), CAP);
        *by_looks.entry(looks).or_default() += 1;
        let n = m.parts.len();
        let one_way: Vec<(usize, usize)> = (0..n)
            .flat_map(|a| (0..n).map(move |b| (a, b)))
            .filter(|&(a, b)| m.together(a, b) && !m.together(b, a))
            .collect();
        let empty: Vec<usize> = (0..n).filter(|&i| m.parts[i].node.is_none()).collect();
        println!(
            "  model {h:08x}: {n} parts, empty pick {empty:?}, {} profiles, {looks} looks; \
             together one way only (row, column): {one_way:?}",
            m.profiles.len()
        );
    }
    println!("looks per model, every part allowed (looks: models): {by_looks:?}");

    // The templates.
    let index = source.archive().index();
    let Some(db_hash) = index
        .assets()
        .iter()
        .find(|a| index.type_hash(a) == TEMPLATE_DB_TYPE)
        .map(|a| a.name_hash)
    else {
        println!("no template database");
        return Ok(());
    };
    let data = source.asset(db_hash, TEMPLATE_DB_TYPE)?;
    let db = Layer::parse(&data)?;
    let names = db.names();
    let profile_of = db.hashes("ModelMixerProfile");
    let model_of = db.hashes("ModelName");
    let minds: HashSet<u32> = db
        .component("Ai")
        .and_then(|c| c.records())
        .unwrap_or_default()
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    let by_hash: HashMap<u32, &str> = names.values().map(|n| (name_hash(n), *n)).collect();
    let label = |h: u32| by_hash.get(&h).copied().unwrap_or("?");

    let mut ids: Vec<u32> = profile_of.keys().copied().collect();
    ids.sort_by_key(|id| names.get(id).copied().unwrap_or_default());
    let (mut found, mut zero, mut missing, mut no_mixer) = (0, 0, 0, 0);
    let (mut looks_after, mut ai_templates, mut ai_looks) = (0usize, 0usize, 0usize);
    let mut lines = Vec::new();
    for id in &ids {
        let name = names.get(id).copied().unwrap_or("?");
        let profile = profile_of[id];
        let model = model_of.get(id).copied().unwrap_or(0);
        let (looks, how) = match mixers.get(&model) {
            None => {
                no_mixer += 1;
                (1, "model has no mixer".to_owned())
            }
            Some((m, _)) => {
                let has = m.profiles.iter().any(|p| p.0 == profile);
                let (key, how) = match (profile, has) {
                    (0, _) => {
                        zero += 1;
                        (None, "no profile: every part".to_owned())
                    }
                    (_, true) => {
                        found += 1;
                        (
                            Some(profile),
                            format!("profile {profile:08x} {}", label(profile)),
                        )
                    }
                    (_, false) => {
                        missing += 1;
                        (
                            None,
                            format!(
                                "profile {profile:08x} {} not in model: every part",
                                label(profile)
                            ),
                        )
                    }
                };
                let allowed = m.allowed(key);
                let n = allowed.iter().filter(|&&a| a).count();
                (
                    m.looks(&allowed, CAP),
                    format!("{how}; {n}/{} parts", m.parts.len()),
                )
            }
        };
        looks_after += looks;
        if minds.contains(id) {
            ai_templates += 1;
            ai_looks += looks;
        }
        let lower = name.to_ascii_lowercase();
        if words.is_empty() || words.iter().any(|w| lower.contains(w)) {
            lines.push(format!(
                "  {id:08x} {name:?}: model {model:08x}, {how}: {looks} looks"
            ));
        }
    }
    println!(
        "templates with ModelMixerProfile: {}; profile in its model {found}, none named {zero}, \
         named but not in model {missing}, model without mixer {no_mixer}",
        ids.len()
    );
    println!(
        "looks before (one shared model, every part drawn): {} templates x 1 = {}",
        ids.len(),
        ids.len()
    );
    println!("looks after: {looks_after} over all; {ai_looks} over the {ai_templates} with Ai");
    for l in lines {
        println!("{l}");
    }
    Ok(())
}
