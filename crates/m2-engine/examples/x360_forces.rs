//! Compares every effect's forces between the PC install and the Xbox 360
//! disc's archive (read-only; both from the user's own copies): matches
//! effects by name hash and forces by order, checks their settings agree,
//! and scores the PC kind inference ([`ForceKind::infer`]) against the
//! 360's kind tags, as are the kind tags the PC parser reads
//! ([`ForceHead`]). Usage: `x360_forces <PC install folder> <360 vz.wad>`.
use std::collections::{BTreeMap, HashMap};
use std::io::BufReader;

use m2_assets::effect::{self, Attr, EFFECT_TYPE, ForceHead, ForceKind};
use m2_assets::x360;
use m2_engine::AssetSource;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (pc, xb) = match (args.next(), args.next()) {
        (Some(a), Some(b)) => (a, b),
        _ => return Err("usage: x360_forces <PC install folder> <360 vz.wad>".into()),
    };

    // The 360 side.
    let mut ar = x360::Archive::open(BufReader::new(std::fs::File::open(&xb)?))?;
    let h = ar.header;
    println!(
        "360 archive: {} blocks, {} assets, {} types",
        h.block_count, h.asset_count, h.type_count
    );
    let mut fx_blocks: Vec<usize> = ar
        .assets
        .iter()
        .filter(|a| ar.type_hash(a) == Some(EFFECT_TYPE))
        .map(|a| usize::from(a.block))
        .collect();
    fx_blocks.sort_unstable();
    fx_blocks.dedup();
    let mut xfx: HashMap<u32, Vec<x360::Force>> = HashMap::new();
    let mut xbad = 0;
    for b in fx_blocks {
        let data = ar.read_block(b)?;
        for e in x360::block_entries(&data)? {
            if e.type_hash != EFFECT_TYPE {
                continue;
            }
            match x360::effect_forces(e.data) {
                Ok(f) => {
                    xfx.insert(e.name_hash, f);
                }
                Err(_) => xbad += 1,
            }
        }
    }
    let xforces: usize = xfx.values().map(Vec::len).sum();
    let mut tags: BTreeMap<String, usize> = BTreeMap::new();
    for f in xfx.values().flatten() {
        *tags.entry(format!("{:?}", f.kind)).or_default() += 1;
    }
    println!(
        "360 effects {} (unreadable {xbad}), forces {xforces}, by tag {tags:?}",
        xfx.len()
    );

    // The PC side.
    let f = std::fs::File::open(std::path::Path::new(&pc).join("data").join("vz.wad"))?;
    let mut source = AssetSource::new(BufReader::new(f))?;
    let index = source.archive().index();
    let mut hashes: Vec<u32> = index
        .assets()
        .iter()
        .filter(|a| index.type_hash(a) == EFFECT_TYPE)
        .map(|a| a.name_hash)
        .collect();
    hashes.sort_unstable();
    hashes.dedup();
    let mut pfx: HashMap<u32, Vec<Vec<Attr>>> = HashMap::new();
    let mut pheads: HashMap<u32, Vec<ForceHead>> = HashMap::new();
    for h in hashes {
        if let Ok(fx) = effect::parse(&source.asset(h, EFFECT_TYPE)?) {
            pheads.insert(h, fx.force_heads);
            pfx.insert(h, fx.forces);
        }
    }
    let pforces: usize = pfx.values().map(Vec::len).sum();
    let psettings: usize = pfx.values().flatten().map(Vec::len).sum();
    println!(
        "PC effects {}, forces {pforces}, force settings {psettings}",
        pfx.len()
    );

    // Match.
    let both: Vec<u32> = pfx
        .keys()
        .filter(|h| xfx.contains_key(h))
        .copied()
        .collect();
    let (mut same_count, mut diff_count) = (0, 0);
    let (mut forces_matched, mut names_same, mut values_same, mut settings_compared) =
        (0, 0, 0, 0usize);
    let mut settings_same = 0usize;
    let mut confusion: BTreeMap<(String, String), usize> = BTreeMap::new();
    let (mut pc_tagged, mut tag_same, mut params_same) = (0, 0, 0);
    let mut signatures: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for h in &both {
        let (p, x) = (&pfx[h], &xfx[h]);
        if p.len() != x.len() {
            diff_count += 1;
            println!("  {h:08x}: PC {} forces, 360 {}", p.len(), x.len());
            continue;
        }
        same_count += 1;
        for (k, (pf, xf)) in p.iter().zip(x).enumerate() {
            forces_matched += 1;
            let head = pheads[h].get(k).cloned().unwrap_or_default();
            if head.kind().is_some() {
                pc_tagged += 1;
            }
            if head.kind() == xf.kind {
                tag_same += 1;
            }
            if head.params == xf.params {
                params_same += 1;
            }
            let pn: Vec<Option<u32>> = pf.iter().map(|a| a.name).collect();
            let xn: Vec<Option<u32>> = xf.settings.iter().map(|s| Some(s.name)).collect();
            if pn == xn {
                names_same += 1;
            }
            let mut all = true;
            for (a, s) in pf.iter().zip(&xf.settings) {
                settings_compared += 1;
                // A keyed setting by its keys (the PC stores no plain value
                // for one, the 360 its last key's), a plain one by its bits.
                let same = a.name == Some(s.name)
                    && a.keyed() == s.keyed
                    && if s.keyed {
                        a.keys == s.keys
                    } else {
                        u32::from_le_bytes(a.value) == s.value
                    };
                if same {
                    settings_same += 1;
                } else {
                    all = false;
                    if std::env::var_os("X360_DIFFS").is_some() {
                        println!(
                            "  diff {h:08x} {:08x}: PC {:?} keyed {} keys {:?}, 360 {:?} keyed {}",
                            s.name,
                            f32::from_le_bytes(a.value),
                            a.keyed(),
                            a.keys,
                            s.float(),
                            s.keyed
                        );
                    }
                }
            }
            if all && pf.len() == xf.settings.len() {
                values_same += 1;
            }
            let inferred = format!("{:?}", ForceKind::infer(pf));
            let tagged = format!("{:?}", xf.kind);
            *confusion.entry((tagged.clone(), inferred)).or_default() += 1;
            let sig: Vec<String> = xf
                .settings
                .iter()
                .map(|s| format!("{:08x}", s.name))
                .collect();
            *signatures
                .entry(tagged)
                .or_default()
                .entry(sig.join(" "))
                .or_default() += 1;
        }
    }
    println!(
        "effects in both {}: same force count {same_count}, different {diff_count}; only PC {}, only 360 {}",
        both.len(),
        pfx.len() - both.len(),
        xfx.len() - both.len()
    );
    println!(
        "forces matched by order {forces_matched}: same setting names {names_same}, every setting identical {values_same}; settings compared {settings_compared}, identical {settings_same}"
    );
    println!(
        "PC forces whose own bytes the parser reads a tag from {pc_tagged}; same kind as the 360 {tag_same}, same kind settings {params_same}"
    );
    println!("360 tag -> PC inferred kind:");
    for ((t, i), n) in &confusion {
        println!("  {t:>16} -> {i:<10} {n}");
    }
    println!("setting lists by 360 kind (count):");
    for (k, sigs) in &signatures {
        for (s, n) in sigs {
            println!("  {k:>16} x{n:<4} {s}");
        }
    }
    Ok(())
}
