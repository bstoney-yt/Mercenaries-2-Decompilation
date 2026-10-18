//! Scores where the island's water heights ([`m2_assets::water`]) lie
//! against the terrain: for each candidate reading of the `watr` grid
//! (where its heights start, and where each point's square sits), how
//! much of the ground under the water drawn above the sea lies below the
//! water's surface, and how much of the ground beside it (the next square
//! out, with no water above the sea) lies below a neighbour's surface.
//!
//! ```text
//! cargo run --release -p m2-engine --example waterfit -- "<install dir>" [x,z]
//! ```
//!
//! A right reading puts water over hollows: many terrain vertices under a
//! square sit below its surface, and the dry squares around a lake rise
//! above it. With `x,z` it also prints the grid around that spot (as the
//! current parser reads it). Prints numbers only.

use std::collections::HashMap;
use std::path::Path;

use m2_assets::water::{NO_WATER, WATER_TYPE, WaterHeights};
use m2_engine::world::terrain_cell;
use m2_engine::{Install, WorldIndex};

type Error = Box<dyn std::error::Error>;

/// Terrain vertices binned by 32 m square (keyed by the square's lower
/// corner in metres), as heights.
type Bins = HashMap<(i32, i32), Vec<f32>>;

fn main() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: waterfit <install dir> [x,z]");
        std::process::exit(2);
    };
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let world = WorldIndex::build(&mut source)?;
    let index = source.archive().index();
    let hash = index
        .assets()
        .iter()
        .find(|a| index.type_hash(a) == WATER_TYPE)
        .map(|a| a.name_hash)
        .ok_or("no water heights")?;
    let raw = source.asset(hash, WATER_TYPE)?;
    let ucfx = m2_assets::Ucfx::parse(&raw)?;
    let section = ucfx.top_level(*b"watr").ok_or("no watr")?.to_vec();
    let water = WaterHeights::parse_section(&section)?;
    println!(
        "grid {} x {} at {} m, heights from 0x{:x}, mask points with water {}",
        water.columns,
        water.rows,
        water.cell,
        WaterHeights::HEIGHTS,
        water.wet.iter().filter(|&&w| w).count()
    );

    // Every full-detail terrain vertex, in 4 m bins.
    let mut verts: Vec<[f32; 3]> = Vec::new();
    let mut cells = 0;
    for (&cell, &centre) in &world.cells {
        let Ok(mesh) = terrain_cell(&mut source, cell, centre) else {
            continue;
        };
        cells += 1;
        for p in mesh.positions() {
            let w = p + mesh.origin;
            verts.push([w.x, w.y, w.z]);
        }
    }
    println!("{cells} terrain cells, {} vertices", verts.len());

    // Candidate readings: where the heights start (0x24 as before, 0x2c),
    // and the square's corner relative to its point, in cells.
    let sea = -36.0 + 0.5;
    for start in [0x24usize, 0x2c] {
        let heights = read_heights(&section, start, water.columns * water.rows);
        for (sx, sz) in [
            (-0.5f32, -0.5f32),
            (0.0, 0.0),
            (-1.0, -1.0),
            (-0.5, 0.0),
            (0.0, -0.5),
        ] {
            let (wet, under, dry, dry_under) = score(&water, &heights, &verts, [sx, sz], sea);
            println!(
                "start 0x{start:x} square at point{:+.1},{:+.1} cells: ground under water below its surface {:.1}% of {under}; dry squares beside water with ground below the neighbour's surface {:.1}% of {dry}",
                sx,
                sz,
                100.0 * wet as f64 / under.max(1) as f64,
                100.0 * dry_under as f64 / dry.max(1) as f64,
            );
        }
    }

    if let Some(at) = args.get(1) {
        let xz: Vec<f32> = at
            .split(',')
            .filter_map(|v| v.trim().parse().ok())
            .collect();
        let c0 = ((xz[0] - water.origin[0]) / water.cell).round() as i64;
        let r0 = ((xz[1] - water.origin[1]) / water.cell).round() as i64;
        for r in (r0 - 4..=r0 + 4).rev() {
            let row: Vec<String> = (c0 - 4..=c0 + 4)
                .map(|c| {
                    water
                        .get(c as usize, r as usize)
                        .map_or("   -  ".into(), |h| format!("{h:6.1}"))
                })
                .collect();
            println!(
                "row {r} (z {:.0}): {}",
                water.origin[1] + r as f32 * water.cell,
                row.join(" ")
            );
        }
        println!(
            "columns {}..={} (x {:.0}..)",
            c0 - 4,
            c0 + 4,
            water.origin[0] + (c0 - 4) as f32 * water.cell
        );
        // The ground round the spot: the lowest vertex per 8 m, as metres
        // above (+) or below (-) the nearest water point's height there.
        let level = water.at(xz[0], xz[1]).unwrap_or(0.0);
        let mut low: HashMap<(i32, i32), f32> = HashMap::new();
        for v in &verts {
            let (dx, dz) = (v[0] - xz[0], v[2] - xz[1]);
            if dx.abs() < 96.0 && dz.abs() < 96.0 {
                let k = ((dx / 8.0).floor() as i32, (dz / 8.0).floor() as i32);
                let e = low.entry(k).or_insert(f32::MAX);
                *e = e.min(v[1]);
            }
        }
        println!(
            "ground per 8 m round ({:.0}, {:.0}) less {level:.1} (x across, z up; '.' none):",
            xz[0], xz[1]
        );
        for k in (-12..12).rev() {
            let row: String = (-12..12)
                .map(|i| match low.get(&(i, k)) {
                    Some(&y) => format!("{:4.0}", y - level),
                    None => "   .".into(),
                })
                .collect();
            println!("z {:+4}: {row}", k * 8);
        }
    }
    Ok(())
}

fn read_heights(section: &[u8], start: usize, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| {
            let o = start + i * 4;
            f32::from_le_bytes(section[o..o + 4].try_into().expect("4 bytes"))
        })
        .collect()
}

/// (vertices under inland water below its surface, vertices under inland
/// water, dry squares next to inland water with vertices, those of their
/// vertices below the neighbour's surface).
fn score(
    water: &WaterHeights,
    heights: &[f32],
    verts: &[[f32; 3]],
    shift: [f32; 2],
    sea: f32,
) -> (usize, usize, usize, usize) {
    let (cols, rows, cell) = (water.columns as i64, water.rows as i64, water.cell);
    // Square of a vertex: the point whose square holds it.
    let square = |x: f32, z: f32| -> Option<(i64, i64)> {
        let c = ((x - water.origin[0]) / cell - shift[0]).floor() as i64;
        let r = ((z - water.origin[1]) / cell - shift[1]).floor() as i64;
        (c >= 0 && r >= 0 && c < cols && r < rows).then_some((c, r))
    };
    let h = |c: i64, r: i64| -> Option<f32> {
        (c >= 0 && r >= 0 && c < cols && r < rows)
            .then(|| heights[(r * cols + c) as usize])
            .filter(|&h| h > NO_WATER + 1.0)
    };
    let mut bins: Bins = HashMap::new();
    for v in verts {
        if let Some((c, r)) = square(v[0], v[2]) {
            bins.entry((c as i32, r as i32)).or_default().push(v[1]);
        }
    }
    let (mut wet, mut under, mut dry, mut dry_under) = (0, 0, 0, 0);
    for (&(c, r), ys) in &bins {
        let (c, r) = (c as i64, r as i64);
        match h(c, r) {
            Some(level) if level > sea => {
                under += ys.len();
                wet += ys.iter().filter(|&&y| y < level).count();
            }
            _ => {
                let near = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .filter_map(|&(dc, dr)| h(c + dc, r + dr))
                    .filter(|&l| l > sea)
                    .fold(f32::NAN, f32::max);
                if near.is_finite() {
                    dry += ys.len();
                    dry_under += ys.iter().filter(|&&y| y < near).count();
                }
            }
        }
    }
    (wet, under, dry, dry_under)
}
