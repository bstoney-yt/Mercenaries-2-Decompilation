//! Times the physics step with a town's worth of props as fixed collision
//! (the port today) against the same props as sleeping loose bodies (as
//! the 360 keeps them, gap P7), headless, on a flat ground:
//!
//! ```text
//! cargo run --release -p m2-engine --example propbench -- "<install dir>" [props]
//! ```
//!
//! 3,000 crates (1 m, 50 kg) by default, on a 600 m square, with a row of
//! 40 across the car's way. Each mode runs three phases: still (5 s), the
//! Phoenix driven at 15 m/s through the row (6 s), and a 20 m blast in
//! the thick of them (10 s; the loose ones get the 360's rigid push as
//! [`m2_engine::durability::BlastPush::throw`] sums it, the fixed
//! ones nothing). Prints, per mode and phase, the step's median and worst
//! milliseconds, the most bodies awake at once and, for the blast, how
//! long until every crate sleeps again. Prints numbers only.

use std::path::Path;
use std::time::Instant;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_engine::durability::BlastPush;
use m2_engine::physics::LoosePiece;
use m2_engine::tuning::{DAMAGE_TABLE, DAMAGE_TABLE_TYPE, DamageTable};
use m2_engine::{
    CatalogueEntry, CollisionShape, Install, Physics, StaticKey, VehicleControls, VehicleSpec,
};
use m2_math::glam::{Mat4, Quat, Vec3};

const DT: f32 = 1.0 / 60.0;
const HALF: f32 = 0.5;
const MASS: f32 = 50.0;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = args
        .first()
        .ok_or("usage: propbench <install dir> [props]")?;
    if args.get(1).is_some_and(|a| a == "town") {
        return town(root);
    }
    let count: usize = args.get(1).map_or(Ok(3000), |s| s.parse())?;
    let spec = phoenix(root)?;
    for loose in [false, true] {
        run(&spec, count, loose);
    }
    Ok(())
}

/// The Phoenix's spec, as the drive test builds it.
fn phoenix(root: &str) -> Result<VehicleSpec, Box<dyn std::error::Error>> {
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let index = source.archive().index();
    let db_hash = index
        .assets()
        .iter()
        .find(|a| index.type_hash(a) == TEMPLATE_DB_TYPE)
        .map(|a| a.name_hash)
        .ok_or("no template database")?;
    let data = source.asset(db_hash, TEMPLATE_DB_TYPE)?;
    let db = Layer::parse(&data)?;
    let names = db.names();
    let damage = source
        .asset(m2_assets::name_hash(DAMAGE_TABLE), DAMAGE_TABLE_TYPE)
        .ok()
        .and_then(|d| DamageTable::parse(&d));
    let tuning = m2_engine::tuning::read(&db, damage.as_ref());
    let models = db.hashes("ModelName");
    let id = names
        .iter()
        .find(|(_, n)| **n == "Phoenix")
        .map(|(i, _)| *i)
        .ok_or("no Phoenix")?;
    let model = *models.get(&id).ok_or("no model")?;
    let data = source.model(model)?;
    let m = data.parse()?;
    let entry = CatalogueEntry {
        model,
        kind: "car".to_owned(),
        label: "Phoenix".to_owned(),
    };
    let spec = VehicleSpec::from_model(&entry, &m)
        .ok_or("no spec")?
        .with_chassis(&m, &data.base)
        .tuned(tuning.get(&id).copied());
    Ok(spec)
}

/// Where the crates stand: a grid over a 600 m square, and a row of 40
/// across the car's way (x 0, z 30 to 70).
fn places(count: usize) -> Vec<Vec3> {
    let row = 40.min(count);
    let mut out: Vec<Vec3> = (0..row)
        .map(|k| Vec3::new(-0.6 + (k % 2) as f32 * 1.2, HALF, 30.0 + k as f32))
        .collect();
    let rest = count - row;
    let side = (rest as f32).sqrt().ceil() as usize;
    let step = 600.0 / side.max(1) as f32;
    for k in 0..rest {
        let (i, j) = (k % side, k / side);
        let p = Vec3::new(-300.0 + i as f32 * step, HALF, -300.0 + j as f32 * step);
        // Keep the car's lane clear but for the row.
        if p.x.abs() < 4.0 && p.z > -10.0 && p.z < 120.0 {
            continue;
        }
        out.push(p);
    }
    out
}

fn run(spec: &VehicleSpec, count: usize, loose: bool) {
    let mut p = Physics::new();
    // Every crate a body of its own (the port holds all but the newest
    // [`m2_engine::physics`]'s 160 loose pieces fixed).
    p.set_debris_cap(usize::MAX);
    let half = 1000.0;
    let v = [
        Vec3::new(-half, 0.0, -half),
        Vec3::new(half, 0.0, -half),
        Vec3::new(half, 0.0, half),
        Vec3::new(-half, 0.0, half),
    ];
    let ground = CollisionShape::mesh(&v, &[[0, 1, 2], [0, 2, 3]]).expect("plane");
    p.add_static(StaticKey::Cell(1), &ground, Vec3::ZERO, Quat::IDENTITY);
    let at = places(count);
    let crate_shape = CollisionShape::cuboid(Vec3::splat(HALF));
    let piece = LoosePiece::block(None, Vec3::ZERO, Vec3::splat(HALF), MASS);
    let mut pieces = Vec::new();
    for (k, c) in at.iter().enumerate() {
        if loose {
            pieces.push(p.add_prop_body(&piece, Mat4::from_translation(*c), false));
        } else {
            p.add_static(
                StaticKey::Entity(k as u32 + 1),
                &crate_shape,
                *c,
                Quat::IDENTITY,
            );
        }
    }
    // Settled and asleep before the clock starts.
    for _ in 0..(4.0 / DT) as usize {
        p.step(DT);
    }
    let rest = spec
        .rest_on(Vec3::new(0.0, 0.0, -20.0), 0.0, |_| Some(0.0))
        .expect("rests on a plane");
    let car = p.add_vehicle_turned(spec, rest.position + Vec3::Y * 0.2, rest.rotation);
    let mode = if loose { "loose" } else { "fixed" };
    let phase = |p: &mut Physics, name: &str, seconds: f32, drive: bool| {
        let mut ms = Vec::new();
        let mut most = 0;
        let mut asleep_at = None;
        for k in 0..(seconds / DT) as usize {
            let speed = p.vehicle_speed(car);
            p.drive(
                car,
                VehicleControls {
                    throttle: if drive && speed < 15.0 { 1.0 } else { 0.0 },
                    steer: 0.0,
                    brake: !drive,
                    parked: !drive,
                },
            );
            let t = Instant::now();
            p.step(DT);
            ms.push(t.elapsed().as_secs_f64() * 1000.0);
            let awake = p.debris_awake();
            most = most.max(awake);
            if awake == 0 && asleep_at.is_none() && most > 0 {
                asleep_at = Some(k as f32 * DT);
            }
            if awake > 0 {
                asleep_at = None;
            }
        }
        ms.sort_by(f64::total_cmp);
        let car_at = p
            .vehicle_transform(car)
            .map_or(Vec3::ZERO, |m| m.w_axis.truncate());
        println!(
            "{mode} {name}: car at z {:.1}, {:.1} m/s; step median {:.2} ms, worst {:.2} ms, most awake {most}{}",
            car_at.z,
            p.vehicle_speed(car),
            ms[ms.len() / 2],
            ms[ms.len() - 1],
            match (loose, asleep_at) {
                (true, Some(t)) => format!(", all asleep again {t:.1} s in"),
                (true, None) if most > 0 => ", still awake at the end".to_owned(),
                _ => String::new(),
            }
        );
    };
    phase(&mut p, "still", 5.0, false);
    phase(&mut p, "car through the row", 6.0, true);
    // A 20 m blast among the grid's crates (300 kg-m/s-ish: `fx_Explosion
    // _Large`'s push 300), a frame's push on each crate within it.
    let blast = BlastPush {
        at: Vec3::new(100.0, 0.5, 100.0),
        radius: 20.0,
        force: 300.0,
    };
    let mut reached = 0;
    if loose {
        for (k, c) in at.iter().enumerate() {
            if c.distance(blast.at) <= blast.radius {
                let lo = *c - Vec3::splat(HALF);
                let hi = *c + Vec3::splat(HALF);
                let v = blast.throw(lo, hi, 1.0).0;
                p.push_debris(pieces[k], v);
                reached += 1;
            }
        }
    }
    println!("{mode}: the blast reaches {reached} crates");
    phase(&mut p, "after a 20 m blast", 10.0, false);
}

/// One of the island's placed objects in the town square: where it is,
/// its whole collision and, if it comes loose, how.
struct Placed {
    at: Mat4,
    shape: CollisionShape,
    loose: Option<std::sync::Arc<m2_engine::physics::Loose>>,
}

/// `town`: the island's placed objects in its most crowded 600 m square,
/// each with its own collision and `PHY2` bodies. The fixed run has them
/// all as fixed collision (as the port today). The loose run has every
/// one that comes loose whole (a barricade, a chair: its moving bodies
/// shown intact) as sleeping bodies of its own from the start, and the
/// rest fixed. One that breaks apart keeps its fixed intact body, and its
/// pieces' bodies are not made until it breaks (the 360 rebuilds an
/// object's physics for the parts its state shows: 0x82182CE8 ->
/// 0x82152780 -> 0x821831B0). Then it settles, stands still and takes a
/// 20 m blast where the loose bodies are thickest.
fn town(root: &str) -> Result<(), Box<dyn std::error::Error>> {
    use m2_assets::Ucfx;
    use m2_assets::model::Model;
    use m2_engine::world::WorldIndex;
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let index = WorldIndex::build(&mut source)?;
    // The most crowded 600 m square, by 100 m cells.
    let mut cells: std::collections::HashMap<(i32, i32), usize> = Default::default();
    // The island's placed objects and every state layer's (all layers at
    // once: an upper bound, as the game loads only some).
    let all: Vec<&m2_engine::world::ObjectPlacement> = index
        .objects
        .iter()
        .chain(index.state_objects.values())
        .collect();
    for o in &all {
        let cell = (
            (o.position.x / 100.0).floor() as i32,
            (o.position.z / 100.0).floor() as i32,
        );
        *cells.entry(cell).or_default() += 1;
    }
    let (corner, _) = cells
        .keys()
        .map(|&(x, z)| {
            let n: usize = (0..6)
                .flat_map(|i| (0..6).map(move |j| (x + i, z + j)))
                .map(|c| cells.get(&c).copied().unwrap_or(0))
                .sum();
            ((x, z), n)
        })
        .max_by_key(|&(_, n)| n)
        .ok_or("no objects")?;
    let lo = Vec3::new(corner.0 as f32 * 100.0, 0.0, corner.1 as f32 * 100.0);
    let inside = |p: Vec3| p.x >= lo.x && p.x < lo.x + 600.0 && p.z >= lo.z && p.z < lo.z + 600.0;
    let mut shapes: std::collections::HashMap<u32, Option<CollisionShape>> = Default::default();
    let mut placed = Vec::new();
    for o in all.iter().filter(|o| inside(o.position)) {
        let shape = shapes.entry(o.model).or_insert_with(|| {
            let d = source.model(o.model).ok()?;
            let m = Model::parse(&d.base).ok()?;
            let phy2 = Ucfx::parse(&d.base)
                .ok()?
                .top_level(*b"PHY2")
                .map(<[u8]>::to_vec);
            CollisionShape::for_model(&m, phy2.as_deref(), 0)
        });
        if let Some(shape) = shape {
            placed.push(Placed {
                at: Mat4::from_rotation_translation(o.rotation, o.position),
                shape: shape.clone(),
                loose: shape.loose.clone(),
            });
        }
    }
    let whole = placed
        .iter()
        .filter(|p| p.loose.as_ref().is_some_and(|l| !l.breaks))
        .count();
    let breaks = placed
        .iter()
        .filter(|p| p.loose.as_ref().is_some_and(|l| l.breaks))
        .count();
    println!(
        "town: the square from {lo:.0}: {} placed objects with collision ({} models), {whole} \
         come loose whole, {breaks} break apart",
        placed.len(),
        shapes.len()
    );
    town_run(&placed, lo, false, false);
    town_run(&placed, lo, true, false);
    town_run(&placed, lo, true, true);
    Ok(())
}

/// Steps `p` for `seconds`, its step times into `ms`: the most loose
/// bodies awake at once, and when they all slept again (if they did).
fn timed(p: &mut Physics, seconds: f32, ms: &mut Vec<f64>) -> (usize, Option<f32>) {
    let (mut most, mut asleep) = (0, None);
    for k in 0..(seconds / DT) as usize {
        let t = Instant::now();
        p.step(DT);
        ms.push(t.elapsed().as_secs_f64() * 1000.0);
        let awake = p.debris_awake();
        most = most.max(awake);
        if awake > 0 {
            asleep = None;
        } else if asleep.is_none() {
            asleep = Some(k as f32 * DT);
        }
    }
    ms.sort_by(f64::total_cmp);
    (most, asleep)
}

fn town_run(placed: &[Placed], lo: Vec3, loose: bool, asleep: bool) {
    let mut p = Physics::new();
    p.set_debris_cap(usize::MAX);
    let mut pieces = Vec::new();
    for (k, o) in placed.iter().enumerate() {
        let (_, rotation, translation) = o.at.to_scale_rotation_translation();
        match (&o.loose, loose) {
            (Some(l), true) if !l.breaks => {
                for (piece, at) in l.pieces.iter().zip(l.starts(o.at)) {
                    pieces.push((p.add_prop_body(piece, at, asleep), at.w_axis.truncate()));
                }
            }
            _ => p.add_static(StaticKey::Object(k), &o.shape, translation, rotation),
        }
    }
    // A ground under it all (the town's terrain isn't read here).
    let c = lo + Vec3::new(300.0, 0.0, 300.0);
    let floor = placed
        .iter()
        .map(|o| o.at.w_axis.y)
        .fold(f32::MAX, f32::min)
        - 1.0;
    let half = 1000.0;
    let v = [
        Vec3::new(c.x - half, floor, c.z - half),
        Vec3::new(c.x + half, floor, c.z - half),
        Vec3::new(c.x + half, floor, c.z + half),
        Vec3::new(c.x - half, floor, c.z + half),
    ];
    let ground = CollisionShape::mesh(&v, &[[0, 1, 2], [0, 2, 3]]).expect("plane");
    p.add_static(StaticKey::Cell(1), &ground, Vec3::ZERO, Quat::IDENTITY);
    let mode = match (loose, asleep) {
        (false, _) => "fixed",
        (true, false) => "loose (placed awake)",
        (true, true) => "loose (placed asleep)",
    };
    let mut ms = Vec::new();
    let (woke, slept) = timed(&mut p, 8.0, &mut ms);
    println!(
        "town {mode}: {} bodies of their own; first 8 s: step median {:.2} ms, worst {:.2} ms, \
         most awake {woke}, {}",
        pieces.len(),
        ms[ms.len() / 2],
        ms[ms.len() - 1],
        slept.map_or("still awake at the end".to_owned(), |t| format!(
            "all asleep {t:.1} s in"
        ))
    );
    ms.clear();
    let (still, _) = timed(&mut p, 5.0, &mut ms);
    println!(
        "town {mode} still: step median {:.2} ms, worst {:.2} ms, most awake {still}",
        ms[ms.len() / 2],
        ms[ms.len() - 1]
    );
    if !loose || pieces.is_empty() {
        return;
    }
    // A 20 m blast where the loose bodies are thickest.
    let at = pieces
        .iter()
        .max_by_key(|(_, a)| pieces.iter().filter(|(_, b)| a.distance(*b) < 20.0).count())
        .map_or(c, |(_, a)| *a);
    let blast = BlastPush {
        at,
        radius: 20.0,
        force: 300.0,
    };
    let mut reached = 0;
    for (id, a) in &pieces {
        if a.distance(at) <= 20.0 {
            let v = blast
                .throw(*a - Vec3::splat(0.5), *a + Vec3::splat(0.5), 1.0)
                .0;
            p.push_debris(*id, v);
            reached += 1;
        }
    }
    ms.clear();
    let (most, slept) = timed(&mut p, 10.0, &mut ms);
    println!(
        "town {mode} after a 20 m blast ({reached} bodies in reach): step median {:.2} ms, worst \
         {:.2} ms, most awake {most}, {}",
        ms[ms.len() / 2],
        ms[ms.len() - 1],
        slept.map_or("still awake at the end".to_owned(), |t| format!(
            "all asleep again {t:.1} s in"
        ))
    );
}
