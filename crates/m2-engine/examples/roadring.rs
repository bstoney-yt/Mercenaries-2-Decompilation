//! Where a pursuit's cars and tanks would come in for her at a place on
//! the island ([`m2_engine::road_ring`]), from your own installation:
//! how the road network's ends meet its junctions, the flood out from
//! her, and the arrivals.
//!
//! ```text
//! cargo run --release -p m2-engine --example roadring -- "<install dir>" <x>,<z> [heading deg]
//! ```
//!
//! Prints numbers only.

use std::path::Path;

use m2_engine::Install;
use m2_engine::road_ring::{self, Flood, REACH, RoadNet};
use m2_engine::world::WorldIndex;
use m2_math::glam::Vec3;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(root), Some(at)) = (args.first(), args.get(1)) else {
        eprintln!("usage: roadring <install dir> <x>,<z> [heading deg]");
        std::process::exit(2);
    };
    let (x, z) = at.split_once(',').ok_or("a place is <x>,<z>")?;
    let her = Vec3::new(x.trim().parse()?, 0.0, z.trim().parse()?);
    let heading: f32 = args.get(2).map_or(Ok(0.0), |h| h.parse())?;
    let forward = Vec3::new(heading.to_radians().sin(), 0.0, heading.to_radians().cos());
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let world = WorldIndex::build(&mut source)?;
    let net = RoadNet::new(&world.roads, &world.junctions);
    let ends = |f: fn(&road_ring::NetRoad) -> bool| net.roads.iter().filter(|r| f(r)).count();
    println!(
        "{} drivable roads, {} junctions; roads with a junction at both ends {}, at one {}, at none {}",
        net.roads.len(),
        net.junctions.len(),
        ends(|r| r.start.is_some() && r.end.is_some()),
        ends(|r| r.start.is_some() != r.end.is_some()),
        ends(|r| r.start.is_none() && r.end.is_none()),
    );
    // RING_BUSY=1: from the junction joining the most roads instead.
    let her = if std::env::var("RING_BUSY").is_ok() {
        let k = (0..net.junctions.len())
            .max_by_key(|&k| net.junctions[k].len())
            .ok_or("no junctions")?;
        let p = world.junctions[k].position;
        println!(
            "from junction {k} ({} roads) at {:.0?}",
            net.junctions[k].len(),
            p.to_array()
        );
        p.with_y(0.0) + Vec3::new(1.0, 0.0, 1.0)
    } else {
        her
    };
    // Off the roads: from the nearest road point instead.
    let her = if net.nearest(her).is_some() {
        her
    } else {
        let p = net
            .roads
            .iter()
            .flat_map(|r| r.points.iter())
            .min_by(|a, b| a.distance(her).total_cmp(&b.distance(her)))
            .copied()
            .ok_or("no roads")?;
        println!(
            "no road within {} m: from the nearest road point, {:.0?} ({:.0} m off)",
            road_ring::NEAR,
            p.to_array(),
            p.with_y(0.0).distance(her)
        );
        p.with_y(0.0)
    };
    let Some((road, frac, off)) = net.nearest(her) else {
        return Ok(());
    };
    {
        let r = &net.roads[road];
        for (name, j) in [("start", r.start), ("end", r.end)] {
            println!(
                "  its {name} junction {j:?}: roads {:?}",
                j.map(|j| net.junctions[j].clone())
            );
        }
    }
    let r = &net.roads[road];
    let frac = frac.clamp(0.001, 0.999);
    let half = world
        .roads
        .iter()
        .filter(|w| w.drivable())
        .find(|w| w.points == r.points)
        .map_or(4.0, |w| w.width / 2.0);
    let dir = road_ring::facing_along(r, frac, her, forward, half);
    println!(
        "on road {road} ({:.0} m long) {frac:.3} along, {off:.1} m off, facing {dir:+}",
        r.length
    );
    if std::env::var("RING_DUMP").is_ok() {
        dump(&world, her);
    }
    components(&world);
    let Some(flood) = Flood::new(&net, road, frac, dir, REACH) else {
        println!("no flood");
        return Ok(());
    };
    let side = |s: i8| flood.met.iter().filter(|m| m.side == s).count();
    println!(
        "flood to {REACH} m: {} roads ({} ahead, {} behind), deepest {} junctions",
        flood.met.len(),
        side(1),
        side(-1),
        flood.met.iter().map(|m| m.depth).max().unwrap_or(0)
    );
    for ring in [
        50.0,
        road_ring::CAR_BEHIND,
        road_ring::CAR_AHEAD,
        road_ring::TANK_AHEAD,
    ] {
        let pts = flood.ring(ring);
        let n = |s: i8| pts.iter().filter(|p| p.side == s).count();
        println!("ring {ring} m: {} ahead, {} behind", n(1), n(-1));
    }
    let show = |name: &str, a: Option<road_ring::Arrival>| match a {
        Some(a) => println!(
            "{name}: at {:.1?} ({:.0} m from her), facing {:.2?}, {:.1} m/s",
            a.at.to_array(),
            a.at.with_y(0.0).distance(her),
            a.facing.to_array(),
            a.velocity.length()
        ),
        None => println!("{name}: none"),
    };
    show("car behind", road_ring::car_behind(&net, &flood));
    show("car ahead", road_ring::car_ahead(&net, &flood));
    show(
        "tank ahead",
        road_ring::tank_ahead(&net, &flood, false, 0.5),
    );
    Ok(())
}

#[allow(dead_code)]
fn dump(world: &WorldIndex, near: Vec3) {
    for j in world
        .junctions
        .iter()
        .filter(|j| j.position.with_y(0.0).distance(near.with_y(0.0)) < 400.0)
    {
        println!("junction {:#x} at {:.0?}", j.entity, j.position.to_array());
        for (e, port) in &j.ports {
            let roads: Vec<String> = world
                .roads
                .iter()
                .filter(|r| r.entity == *e)
                .map(|r| {
                    let f = r.points[0].distance(*port);
                    let l = r.points.last().unwrap().distance(*port);
                    format!(
                        "drivable {} ends {f:.1}/{l:.1} names {:x?}",
                        r.drivable(),
                        r.ends
                    )
                })
                .collect();
            println!("  port road {e:#x} at {:.0?}: {roads:?}", port.to_array());
        }
    }
}

#[allow(dead_code)]
fn components(world: &WorldIndex) {
    use std::collections::HashMap;
    // Junction entity -> drivable roads naming it (by the roads' ends), and
    // by the junction's own record.
    let mut by_ends: HashMap<u32, Vec<usize>> = HashMap::new();
    let drivable: Vec<&m2_engine::world::Road> =
        world.roads.iter().filter(|r| r.drivable()).collect();
    for (i, r) in drivable.iter().enumerate() {
        for e in r.ends {
            if e != 0 {
                by_ends.entry(e).or_default().push(i);
            }
        }
    }
    let multi = by_ends.values().filter(|v| v.len() >= 2).count();
    let own: usize = world
        .junctions
        .iter()
        .filter(|j| {
            j.ports
                .iter()
                .filter(|(e, _)| drivable.iter().any(|r| r.entity == *e))
                .count()
                >= 2
        })
        .count();
    println!(
        "junctions joining 2+ drivable roads: by the roads' ends {multi}, by their own records {own}"
    );
    // Components by the roads' ends.
    let mut seen = vec![false; drivable.len()];
    let mut sizes = Vec::new();
    for s in 0..drivable.len() {
        if seen[s] {
            continue;
        }
        let mut stack = vec![s];
        seen[s] = true;
        let mut n = 0;
        while let Some(i) = stack.pop() {
            n += 1;
            for e in drivable[i].ends {
                for &k in by_ends.get(&e).into_iter().flatten() {
                    if !seen[k] {
                        seen[k] = true;
                        stack.push(k);
                    }
                }
            }
        }
        sizes.push(n);
    }
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    println!(
        "components by the roads' ends: {} (largest {:?})",
        sizes.len(),
        &sizes[..sizes.len().min(8)]
    );
}
