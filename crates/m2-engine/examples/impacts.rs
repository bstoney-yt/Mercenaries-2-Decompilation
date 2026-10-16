//! Measures what the island's surfaces are made of and which bullet
//! impact each throws up ([`m2_engine::surface`]): every model
//! material's surface word, whether any `PHY2` mesh triangle carries one
//! of its own, and over every terrain cell's ground and every placed
//! model's solid drawn area, how much strikes an impact its word names
//! and how much the generic one.
//!
//! ```text
//! cargo run --release -p m2-engine --example impacts -- "<install dir>"
//! ```
//!
//! Prints numbers, hashes and names only.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use m2_assets::model::MODEL_TYPE;
use m2_assets::physics::{Geometry, Physics};
use m2_assets::terrain::{TERRAIN_TYPE, TerrainCell};
use m2_engine::physics::drawn_surface;
use m2_engine::surface::{bullet_impact, word};
use m2_engine::world::WorldIndex;
use m2_engine::{AssetSource, Install};
use m2_math::glam::Vec3;

fn show(surface: u32) -> String {
    word(surface).map_or(format!("{surface:08x}"), str::to_owned)
}

fn area(p: &[Vec3], t: [u16; 3]) -> f32 {
    let g = |i: u16| p.get(usize::from(i)).copied().unwrap_or_default();
    (g(t[1]) - g(t[0])).cross(g(t[2]) - g(t[0])).length() * 0.5
}

/// Area by impact template, and how much of it its surface's word chose.
#[derive(Default)]
struct Struck {
    by_impact: BTreeMap<&'static str, f64>,
    by_surface: BTreeMap<u32, f64>,
    named: f64,
    all: f64,
}

impl Struck {
    fn add(&mut self, surface: u32, a: f64) {
        let (template, named) = bullet_impact(surface);
        *self.by_impact.entry(template).or_default() += a;
        *self.by_surface.entry(surface).or_default() += a;
        self.all += a;
        if named {
            self.named += a;
        }
    }

    fn print(&self, what: &str) {
        let pc = |a: f64| 100.0 * a / self.all.max(1.0);
        println!(
            "{what}: {:.1}% strikes the impact its surface names, {:.1}% the generic one",
            pc(self.named),
            pc(self.all - self.named)
        );
        for (t, a) in &self.by_impact {
            println!("  {t}\t{:.1}%", pc(*a));
        }
        let mut v: Vec<_> = self.by_surface.iter().collect();
        v.sort_by(|a, b| b.1.total_cmp(a.1));
        let s: Vec<String> = v
            .iter()
            .map(|(s, a)| format!("{} {:.1}%", show(**s), pc(**a)))
            .collect();
        println!("  by surface: {}", s.join(", "));
    }
}

fn models<R: std::io::Read + std::io::Seek>(source: &AssetSource<R>) -> Vec<u32> {
    let index = source.archive().index();
    index
        .assets()
        .iter()
        .filter(|a| index.type_hash(a) == MODEL_TYPE)
        .map(|a| a.name_hash)
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(root) = std::env::args().nth(1) else {
        eprintln!("usage: impacts <install dir>");
        std::process::exit(2);
    };
    let install = Install::find(Path::new(&root))?;
    let mut source = install.open_world()?;

    // Every model's materials, and its PHY2 mesh triangles' own word.
    let mut materials: BTreeMap<u32, (usize, usize)> = BTreeMap::new();
    let (mut read, mut phy2, mut triangles, mut marked) = (0, 0, 0usize, 0usize);
    for h in models(&source) {
        let Ok(data) = source.model(h) else { continue };
        let Ok(m) = data.parse() else { continue };
        read += 1;
        let mut seen = Vec::new();
        for x in &m.materials {
            let e = materials.entry(x.surface).or_default();
            e.0 += 1;
            if !seen.contains(&x.surface) {
                seen.push(x.surface);
                e.1 += 1;
            }
        }
        let Some(p) = m2_assets::Ucfx::parse(&data.base)
            .ok()
            .and_then(|u| u.top_level(*b"PHY2").map(<[u8]>::to_vec))
            .and_then(|d| Physics::parse(&d).ok())
        else {
            continue;
        };
        phy2 += 1;
        for s in &p.shapes {
            if let Geometry::Mesh { flags, .. } = &s.geometry {
                triangles += flags.len();
                marked += flags.iter().filter(|&&f| f != 0).count();
            }
        }
    }
    let named = materials.keys().filter(|&&s| word(s).is_some()).count();
    println!(
        "{read} models: {} material surfaces, {named} named; {phy2} with PHY2, \
         {marked} of {triangles} mesh triangles with a word of their own",
        materials.len()
    );
    println!("surface\tmaterials\tmodels\timpact");
    for (s, (n, m)) in &materials {
        println!("{}\t{n}\t{m}\t{}", show(*s), bullet_impact(*s).0);
    }

    // The island: terrain cells, then the placed models.
    let world = WorldIndex::build(&mut source)?;
    let mut ground = Struck::default();
    let cells: Vec<u32> = world.cells.keys().copied().collect();
    for h in cells {
        let Ok(data) = source.asset(h, TERRAIN_TYPE) else {
            continue;
        };
        let Ok(cell) = TerrainCell::parse(&data) else {
            continue;
        };
        for patch in &cell.patches {
            for g in &patch.groups {
                let pos: Vec<Vec3> = g.positions().into_iter().map(Vec3::from).collect();
                for r in g.lod_ranges(0) {
                    let s = cell
                        .materials
                        .get(r.material as usize)
                        .map_or(0, |m| m.material.surface);
                    let a: f32 = g.triangles(r).into_iter().map(|t| area(&pos, t)).sum();
                    ground.add(s, f64::from(a));
                }
            }
        }
    }
    ground.print(&format!("terrain, {} cells", world.cells.len()));

    let mut placed: HashMap<u32, usize> = HashMap::new();
    for o in world.objects.iter().chain(world.state_objects.values()) {
        *placed.entry(o.model).or_default() += 1;
    }
    // A model with PHY2 strikes as its largest surface; one without as
    // each drawn triangle's own. Area here is the solid drawn area.
    let mut objects = Struck::default();
    let mut with_phy2 = Struck::default();
    for (&h, &n) in &placed {
        let Ok(data) = source.model(h) else { continue };
        let Ok(m) = data.parse() else { continue };
        let has_phy2 = m2_assets::Ucfx::parse(&data.base)
            .ok()
            .is_some_and(|u| u.top_level(*b"PHY2").is_some());
        let whole = drawn_surface(&m, 0);
        for (_, geometry) in m.lod(0) {
            for group in &geometry.groups {
                let Some(pos) = group
                    .decl
                    .find(m2_assets::vertex::DeclUsage::Position, 0)
                    .and_then(|e| group.vertices.read(e))
                else {
                    continue;
                };
                let pos: Vec<Vec3> = pos.iter().map(|p| Vec3::new(p[0], p[1], p[2])).collect();
                for range in &group.material_ranges {
                    let Some(x) = m.materials.get(range.material as usize) else {
                        continue;
                    };
                    // Leaves, glows and unseen surfaces stand in the way
                    // of nothing.
                    if x.flags & (0x0002 | 0x0400 | 0x0800) != 0 {
                        continue;
                    }
                    let a: f32 = group
                        .triangles(range)
                        .into_iter()
                        .map(|t| area(&pos, t))
                        .sum();
                    let a = f64::from(a) * n as f64;
                    if has_phy2 {
                        objects.add(whole, a);
                        with_phy2.add(x.surface, a);
                    } else {
                        objects.add(x.surface, a);
                    }
                }
            }
        }
    }
    objects.print(&format!(
        "placed models, {} kinds, area times placements",
        placed.len()
    ));
    with_phy2.print("  of which with PHY2, by each triangle's own material");
    Ok(())
}
