//! Lists how the terrain cell at a spot is split into patches, groups and
//! material ranges, to find which groups overlap (levels of detail?) and
//! how ground layers meet.
//!
//! ```text
//! cargo run --release -p m2-engine --example terrain_groups -- "<install dir>" <x>,<z> [patches]
//! ```
//!
//! Per patch: each group's vertex count, vertex elements, unknown INFO
//! words (as hex and as floats), bounds of its positions, its vertex
//! colour sums, and each material range (material, technique name,
//! triangles, and the area its triangles cover). `patches` limits how many
//! patches are listed (default 4); a summary of every patch follows.
//! Prints numbers and names only.

use std::collections::HashMap;
use std::path::Path;

use m2_assets::TerrainCell;
use m2_assets::terrain::TERRAIN_TYPE;
use m2_assets::vertex::DeclUsage;
use m2_engine::{Install, Technique, WorldIndex};
use m2_math::glam::Vec3;

type Error = Box<dyn std::error::Error>;

fn main() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(root), Some(at)) = (args.first(), args.get(1)) else {
        eprintln!("usage: terrain_groups <install dir> <x>,<z> [patches]");
        std::process::exit(2);
    };
    let xz: Vec<f32> = at
        .split(',')
        .filter_map(|v| v.trim().parse().ok())
        .collect();
    let (x, z) = (xz[0], xz[1]);
    let shown: usize = args.get(2).and_then(|n| n.parse().ok()).unwrap_or(4);
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let world = WorldIndex::build(&mut source)?;
    let Some((&cell, &centre)) = world.cells.iter().min_by(|a, b| {
        let d = |c: &Vec3| (c.x - x).powi(2) + (c.z - z).powi(2);
        d(a.1).total_cmp(&d(b.1))
    }) else {
        println!("no cells");
        return Ok(());
    };
    let data = source.asset(cell, TERRAIN_TYPE)?;
    let parsed = TerrainCell::parse(&data)?;
    println!(
        "cell {cell:08x} centre ({:.0}, {:.0}), {} materials, {} patches",
        centre.x,
        centre.z,
        parsed.materials.len(),
        parsed.patches.len()
    );
    let tech = |m: u32| {
        parsed
            .materials
            .get(m as usize)
            .map(|m| {
                Technique::from_hash(m.material.technique).map_or_else(
                    || format!("{:08x}", m.material.technique),
                    |t| t.name.to_owned(),
                )
            })
            .unwrap_or_else(|| "?".into())
    };
    // Spot's patch first.
    let mut order: Vec<usize> = (0..parsed.patches.len()).collect();
    order.sort_by(|&a, &b| {
        let d = |i: usize| {
            let o = parsed.patches[i].offset;
            (centre.x + o[0] - x).powi(2) + (centre.z + o[2] - z).powi(2)
        };
        d(a).total_cmp(&d(b))
    });
    let mut summary: HashMap<String, usize> = HashMap::new();
    for (rank, &pi) in order.iter().enumerate() {
        let p = &parsed.patches[pi];
        let verbose = rank < shown;
        if verbose {
            println!(
                "\npatch {pi} offset ({:.0}, {:.0}, {:.0}), {} groups",
                p.offset[0],
                p.offset[1],
                p.offset[2],
                p.groups.len()
            );
        }
        let mut kinds = Vec::new();
        for (gi, g) in p.groups.iter().enumerate() {
            let pos = g.positions();
            let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            for q in &pos {
                lo = lo.min(Vec3::from(*q));
                hi = hi.max(Vec3::from(*q));
            }
            let sig: Vec<String> = g
                .decl
                .elements()
                .iter()
                .map(|e| format!("{:?}{}", e.usage, e.usage_index))
                .collect();
            let colours = g
                .decl
                .find(DeclUsage::Color, 0)
                .and_then(|e| g.vertices.read(e))
                .unwrap_or_default();
            let (mut zero, mut one, mut mean) = (0, 0, [0f32; 4]);
            for c in &colours {
                let s: f32 = c.iter().sum();
                if s < 0.02 {
                    zero += 1;
                }
                if (s - 1.0).abs() < 0.03 {
                    one += 1;
                }
                for k in 0..4 {
                    mean[k] += c[k] / colours.len().max(1) as f32;
                }
            }
            let techs: Vec<String> = g.material_ranges.iter().map(|r| tech(r.material)).collect();
            kinds.push(format!(
                "{}v/{}r/{}f[{}]",
                pos.len(),
                g.material_ranges.len(),
                g.full_ranges.len(),
                techs
                    .iter()
                    .map(|t| t.trim_start_matches("Pg"))
                    .collect::<Vec<_>>()
                    .join(",")
            ));
            if verbose {
                let floats: Vec<String> = g
                    .unk_info
                    .iter()
                    .map(|&w| format!("{w:08x}({:.3})", f32::from_bits(w)))
                    .collect();
                println!(
                    "  group {gi}: {} verts [{}] bounds ({:.1},{:.1},{:.1})..({:.1},{:.1},{:.1})",
                    pos.len(),
                    sig.join(" "),
                    lo.x,
                    lo.y,
                    lo.z,
                    hi.x,
                    hi.y,
                    hi.z
                );
                println!("    info {}", floats.join(" "));
                if !colours.is_empty() {
                    println!(
                        "    colour: {} zero-sum, {} unit-sum of {}; mean [{:.2} {:.2} {:.2} {:.2}]",
                        zero,
                        one,
                        colours.len(),
                        mean[0],
                        mean[1],
                        mean[2],
                        mean[3]
                    );
                }
                for (label, ranges) in [("range", &g.material_ranges), ("full", &g.full_ranges)] {
                    for r in ranges.iter() {
                        let tris = g.triangles(r);
                        let mut area = 0.0;
                        let (mut used_lo, mut used_hi) = (u16::MAX, 0u16);
                        for t in &tris {
                            if t.iter().any(|&i| usize::from(i) >= pos.len()) {
                                continue;
                            }
                            let [a, b, c] = t.map(|i| Vec3::from(pos[usize::from(i)]));
                            area += (b - a).cross(c - a).length() / 2.0;
                            for &i in t {
                                used_lo = used_lo.min(i);
                                used_hi = used_hi.max(i);
                            }
                        }
                        println!(
                            "    {label} material {} {} shader {:08x}: {} tris, {:.0} m², vertices {}..{}",
                            r.material,
                            tech(r.material),
                            r.shader_hash,
                            tris.len(),
                            area,
                            used_lo,
                            used_hi
                        );
                    }
                }
            }
        }
        *summary.entry(kinds.join(" | ")).or_default() += 1;
    }
    println!("\npatch shapes (groups: verts/material ranges/full ranges[techniques]):");
    let mut s: Vec<_> = summary.into_iter().collect();
    s.sort_by_key(|e| std::cmp::Reverse(e.1));
    for (k, n) in s.iter().take(20) {
        println!("  {n} x {k}");
    }
    Ok(())
}
