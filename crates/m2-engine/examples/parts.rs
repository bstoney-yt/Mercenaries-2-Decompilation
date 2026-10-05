//! Lists a model's parts, to find which belong to its wrecked state: the
//! node tree with each node's geometry, and where the state machine
//! (`STAM`) and mixer (`MIXR`) sections mention a node.
//!
//! ```text
//! cargo run --release -p m2-engine --example parts -- "<install dir>" <model hash or name>...
//! ```
//!
//! A name is hashed with the asset name hash. Prints names, hashes, counts
//! and offsets only.

use std::path::Path;

use m2_assets::name_hash;
use m2_assets::ucfx::Ucfx;
use m2_engine::Install;

/// Words a node or state might be named; their hashes are matched against
/// the model's node names and the state machine's words.
const WORDS: &[&str] = &[
    "damage",
    "damaged",
    "destroyed",
    "destroy",
    "dead",
    "death",
    "wreck",
    "wrecked",
    "ruin",
    "ruined",
    "intact",
    "normal",
    "default",
    "broken",
    "burnt",
    "burned",
    "dmg",
    "state",
    "alive",
    "healthy",
    "health",
    "body",
    "chassis",
    "hull",
    "turret",
    "barrel",
    "gun",
    "cannon",
    "wheel",
    "door",
    "hatch",
    "root",
    "base",
    "pristine",
    "clean",
    "whole",
    "good",
    "bad",
    "live",
    "off",
    "on",
    "visible",
    "hidden",
    "show",
    "hide",
    "explode",
    "exploded",
    "fire",
    "smoke",
    "debris",
    "parts",
    "lod0",
    "lod1",
    "lod2",
    "shadow",
    "collision",
    // The part states and verbs a gate's machine was found to use (round
    // 99's names, by hash).
    "InitState",
    "PristineState",
    "CollapseState",
    "DebrisState",
    "DestroyedState",
    "InitDestroyedState",
    "SetState",
    "SetRootNode",
    "SendMessage",
    // And in its collapse: the message that starts it, each piece made an
    // object of its own, the emitters at its locators, the camera's shake.
    "DestroyMsg",
    "CreateObject",
    "StartEmitter",
    "CameraShake",
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: parts <install dir> <model hash or name>...");
        std::process::exit(2);
    };
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let words: Vec<(u32, &str)> = WORDS.iter().map(|w| (name_hash(w), *w)).collect();
    let word = |h: u32| words.iter().find(|w| w.0 == h).map(|w| w.1);
    for arg in &args[1..] {
        let hash = u32::from_str_radix(arg.trim_start_matches("0x"), 16)
            .ok()
            .filter(|_| arg.len() == 8 || arg.starts_with("0x"))
            .unwrap_or_else(|| name_hash(arg));
        let data = match source.model(hash) {
            Ok(d) => d,
            Err(e) => {
                println!("{arg} ({hash:08x}): {e}");
                continue;
            }
        };
        let model = data.parse()?;
        println!(
            "== {arg} ({hash:08x}): {} nodes, {} segments, {} slots, {} lods, flags {:#x}",
            model.nodes.len(),
            model.segments.len(),
            model.info.slot_count,
            model.info.lod_count,
            model.info.flags
        );
        let hidden = model.states.as_ref().map(|st| {
            println!("  switchable {:?}", st.switchable);
            for m in &st.machines {
                println!("  part {:?}: {} states", m.node, m.states.len());
                for (name, commands) in &m.states {
                    println!("    state {name:08x}: {commands:?}");
                }
            }
            let chunks = st.chunks(&model.nodes);
            println!(
                "  other verbs' points {:?}; breaks into {:?}, naming {:?} as it does",
                st.points,
                chunks,
                st.breaking_points(&chunks)
            );
            [
                st.hidden(&model.nodes, false),
                st.hidden(&model.nodes, true),
            ]
        });
        for (i, n) in model.nodes.iter().enumerate() {
            let state =
                hidden
                    .as_ref()
                    .map_or("", |[intact, wrecked]| match (intact[i], wrecked[i]) {
                        (false, false) => "both ",
                        (false, true) => "INTACT ",
                        (true, false) => "WRECK ",
                        (true, true) => "never ",
                    });
            let segs: Vec<String> = model
                .segments
                .iter()
                .filter(|s| usize::from(s.node) == i)
                .map(|s| {
                    let tris: u32 = model
                        .geometry(s.slot)
                        .map(|g| {
                            g.groups
                                .iter()
                                .map(|p| (p.indices.len() as u32).saturating_sub(2))
                                .sum()
                        })
                        .unwrap_or(0);
                    format!("slot {} lods {:#04b} ~{tris} tris", s.slot, s.lod_mask)
                })
                .collect();
            let size = [0, 1, 2].map(|k| n.bounds_max[k] - n.bounds_min[k]);
            let mid = [0, 1, 2].map(|k| (n.bounds_max[k] + n.bounds_min[k]) * 0.5);
            println!(
                "  {state}node {i:3} parent {:>4} {:08x}{} flags {:#06x} unk {:#010x} size {:.1}x{:.1}x{:.1} at {:.1},{:.1},{:.1}  {}",
                n.parent.map_or("-".into(), |p| p.to_string()),
                n.name_hash,
                word(n.name_hash).map_or(String::new(), |w| format!(" ({w})")),
                n.flags,
                n.unk_0x0c,
                size[0],
                size[1],
                size[2],
                mid[0],
                mid[1],
                mid[2],
                segs.join("; ")
            );
        }
        let u = Ucfx::parse(&data.base)?;
        for (i, s) in u.sections().iter().enumerate() {
            let tag = String::from_utf8_lossy(&s.fourcc).into_owned();
            let top = u.sections()[..=i]
                .iter()
                .rev()
                .find(|t| t.depth == 0)
                .map(|t| t.fourcc);
            if !matches!(top, Some(t) if &t == b"STAM" || &t == b"MIXR") {
                continue;
            }
            let indent = "  ".repeat(s.depth + 1);
            match s.data {
                None => println!("{indent}{tag}/ ({} records under it)", s.descendants),
                Some(d) => {
                    let mut notes = Vec::new();
                    for (k, w) in d.chunks_exact(4).enumerate() {
                        let v = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
                        if let Some(n) = model.nodes.iter().position(|n| n.name_hash == v) {
                            notes.push(format!("@{:#x}=node {n}", k * 4));
                        } else if let Some(w) = word(v) {
                            notes.push(format!("@{:#x}=\"{w}\"", k * 4));
                        }
                    }
                    if d.len() <= 64 {
                        // Small records in full: nodes, words, else hex.
                        let all: Vec<String> = d
                            .chunks_exact(4)
                            .map(|w| {
                                let v = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
                                match model.nodes.iter().position(|n| n.name_hash == v) {
                                    Some(n) => format!("n{n}"),
                                    None => word(v).map_or(format!("{v:#x}"), str::to_owned),
                                }
                            })
                            .collect();
                        println!("{indent}{tag} [{}]", all.join(" "));
                    } else if tag == "CEXE" {
                        // Commands in full, as (kind, word) pairs: 1 an
                        // operand, 2 a verb, 3 the end, others as they are.
                        let pairs: Vec<String> = d
                            .chunks_exact(8)
                            .map(|p| {
                                let k = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
                                let v = u32::from_le_bytes([p[4], p[5], p[6], p[7]]);
                                let what = match model.nodes.iter().position(|n| n.name_hash == v) {
                                    Some(n) => format!("n{n}"),
                                    None => word(v).map_or_else(
                                        || {
                                            let f = f32::from_bits(v);
                                            if v != 0
                                                && f.is_finite()
                                                && f.abs() > 1e-3
                                                && f.abs() < 1e5
                                            {
                                                format!("{v:#x}({f})")
                                            } else {
                                                format!("{v:#x}")
                                            }
                                        },
                                        str::to_owned,
                                    ),
                                };
                                format!("{k}:{what}")
                            })
                            .collect();
                        println!("{indent}{tag} {} bytes: {}", d.len(), pairs.join(" "));
                    } else {
                        println!("{indent}{tag} {} bytes  {}", d.len(), notes.join(" "));
                    }
                }
            }
        }
    }
    Ok(())
}
