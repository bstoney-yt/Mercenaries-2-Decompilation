//! Lists, for every vehicle model in the catalogue, the drawn nodes that
//! turn with a wheel and whether each is round about the wheel's axle
//! (`m2_assets::vehicle::rolls_with_wheel`) or reaches past the tyre (a
//! fender, a fork, a suspension arm), and whether the spec spins it
//! (playtest 5: the Harley's fender turned with its tyre).
//!
//! ```text
//! cargo run --release -p m2-engine --example wheelspin -- "<install dir>" [model hash]
//! ```
//!
//! Prints model hashes, labels, node numbers and distances only.

use std::path::Path;

use m2_assets::vehicle::{AxleReach, node_reach, rolls_with_wheel};
use m2_engine::{Install, VehicleSpec};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: wheelspin <install dir> [model hash]");
        std::process::exit(2);
    };
    let only = args
        .get(1)
        .and_then(|h| u32::from_str_radix(h.trim_start_matches("0x"), 16).ok());
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let catalogue = m2_engine::vehicle::catalogue(&mut source)?;
    // (models, round spun, other spun, round still, other steered only)
    let mut totals = [0usize; 5];
    let mut odd_models = 0;
    let mut bald = 0;
    // The widest round node (and the one most off its axle) and the
    // narrowest other, in wheel radii.
    let (mut widest, mut narrowest, mut widest_off) = (0.0f32, f32::INFINITY, 0.0f32);
    for entry in &catalogue.vehicles {
        if only.is_some_and(|h| h != entry.model) {
            continue;
        }
        let Some(model) = source.model(entry.model).ok() else {
            continue;
        };
        let Ok(m) = model.parse() else { continue };
        let Some(spec) = VehicleSpec::from_model(entry, &m) else {
            continue;
        };
        let centres: Vec<([f32; 3], f32)> = if spec.tracked {
            spec.road_wheels
                .iter()
                .map(|&(c, r)| (c.to_array(), r))
                .collect()
        } else {
            spec.wheels
                .iter()
                .map(|w| (w.centre.to_array(), w.radius))
                .collect()
        };
        if centres.is_empty() {
            continue;
        }
        totals[0] += 1;
        let reaches: Vec<Vec<AxleReach>> =
            centres.iter().map(|&(c, _)| node_reach(&m, c)).collect();
        let mut lines = Vec::new();
        for (node, w) in spec.node_wheel.iter().enumerate() {
            let Some(w) = *w else { continue };
            let Some(at) = reaches[w].iter().find(|a| usize::from(a.node) == node) else {
                continue;
            };
            let (reach, off) = (at.reach, at.off);
            let r = centres[w].1;
            let round = rolls_with_wheel(at, r);
            let spun = spec.spins(node);
            let slot = match (round, spun) {
                (true, true) => 1,
                (false, true) => 2,
                (true, false) => 3,
                (false, false) => 4,
            };
            totals[slot] += 1;
            if round {
                widest = widest.max(reach / r);
                widest_off = widest_off.max(off / r);
            } else {
                narrowest = narrowest.min(reach / r);
            }
            if slot != 1 || only.is_some() || reach > 1.04 * r || off > 0.05 * r {
                lines.push(format!(
                    "  node {node:3} {:08x} wheel {w} r {r:.2} reach {reach:.2} ({:.2} r) \
                     off {off:.2} ({:.2} r) {}",
                    m.nodes[node].name_hash,
                    reach / r,
                    off / r,
                    [
                        "",
                        "round, spins",
                        "NOT ROUND, SPINS",
                        "ROUND, STILL",
                        "not round, still"
                    ][slot],
                ));
            }
        }
        // Each wheel's tyre (a spun node reaching at least 0.85 of its
        // radius) still spins.
        for (w, &(_, r)) in centres.iter().enumerate() {
            let tyre = spec.node_wheel.iter().enumerate().any(|(node, &x)| {
                x == Some(w)
                    && spec.spins(node)
                    && reaches[w]
                        .iter()
                        .any(|a| usize::from(a.node) == node && a.reach >= 0.85 * r)
            });
            if !tyre {
                bald += 1;
                lines.push(format!("  wheel {w} r {r:.2}: NO SPINNING TYRE"));
            }
        }
        if !lines.is_empty() {
            if lines
                .iter()
                .any(|l| l.contains("SPINS") || l.contains("STILL") || l.contains("TYRE"))
            {
                odd_models += 1;
            }
            println!(
                "{:08x} {} {} ({} wheels{})",
                entry.model,
                entry.kind,
                entry.label,
                centres.len(),
                if spec.tracked { ", road" } else { "" }
            );
            for l in lines {
                println!("{l}");
            }
        }
    }
    println!(
        "models {}, round spun {}, NOT ROUND SPUN {}, ROUND STILL {}, not round still {}; \
         models with odd nodes {odd_models}; wheels without a spinning tyre {bald}; widest round {widest:.2} r (off {widest_off:.2} r), narrowest other {narrowest:.2} r",
        totals[0], totals[1], totals[2], totals[3], totals[4]
    );
    Ok(())
}
