//! Lists the emitters of the effects the satellite bomb's templates show
//! (`Explosion (Airstike Bomb)`, `Explosion (Smart Bomb)`, or the
//! templates named on the command line): each one's size, life, rate,
//! speed, spread, forces, blending and how its colour keys' brightness
//! and alpha run, to find what draws hard-edged or out-of-place sprites.
//!
//! ```text
//! cargo run --release -p m2-engine --example bombfx -- "<install dir>" [template...]
//! ```
//!
//! Prints numbers, hashes and names only.

use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_engine::Install;
use m2_engine::particles::{Library, template_effects};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let Some(root) = args.next() else {
        eprintln!("usage: bombfx <install dir> [template...]");
        std::process::exit(2);
    };
    let mut wanted: Vec<String> = args.collect();
    if wanted.is_empty() {
        wanted = vec![
            "Explosion (Airstike Bomb)".into(),
            "Explosion (Smart Bomb)".into(),
        ];
    }
    let install = Install::find(Path::new(&root))?;
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
    let lib = Library::load(&mut source, template_effects(&db))?;
    for want in &wanted {
        let ids: Vec<u32> = names
            .iter()
            .filter(|(_, n)| **n == want.as_str())
            .map(|(&id, _)| id)
            .collect();
        for id in ids {
            println!("{want} ({id:08x}): effects {:08x?}", lib.of_template(id));
            for h in lib.of_template(id) {
                let Some(fx) = lib.effects.get(h) else {
                    println!("  {h:08x}: not read");
                    continue;
                };
                for (k, e) in fx.emitters.iter().enumerate() {
                    let ints: Vec<f32> = e.colour.iter().map(|c| c.intensity).collect();
                    let below = ints.iter().filter(|&&i| i < 0.0).count();
                    let above = ints.iter().filter(|&&i| i > 0.0).count();
                    let alpha: Vec<u8> = e.colour.iter().map(|c| c.rgba()[3]).collect();
                    let (amin, amax) = alpha
                        .iter()
                        .fold((255u8, 0u8), |(lo, hi), &a| (lo.min(a), hi.max(a)));
                    let (imin, imax) = ints
                        .iter()
                        .fold((f32::MAX, f32::MIN), |(lo, hi), &i| (lo.min(i), hi.max(i)));
                    println!(
                        "  {h:08x}#{k}: glows {} bursts {} frames {}/{} off {:.1?} axis {:.2?}",
                        e.glows(),
                        e.bursts(),
                        e.found,
                        e.textures,
                        e.offset.to_array(),
                        e.axis.to_array()
                    );
                    println!(
                        "    size {:?} = {:.2}..{:.2} var {:.2}; life {:.2} var {:.2}; rate {:?} var {:.1}",
                        e.size.keys,
                        e.size.at(0.0),
                        e.size.at(99.0),
                        e.size_var,
                        e.life.at(0.0),
                        e.life_var,
                        e.rate.keys,
                        e.rate_var
                    );
                    println!(
                        "    rate0 {:.1} speed {:.1} var {:.1} spread {:.0} var {:.0} grav {:.2} drag {:.2} turn {:.0} spin {:.0}",
                        e.rate.at(0.0),
                        e.speed.at(0.0),
                        e.speed_var,
                        e.spread.at(0.0),
                        e.spread_var,
                        e.gravity.value,
                        e.drag.value,
                        e.turn,
                        e.spin
                    );
                    println!(
                        "    colour keys {} (below 0: {below}, above: {above}) intensity {imin:.1}..{imax:.1} alpha {amin}..{amax}; first/last rgba {:?} {:?}",
                        e.colour.len(),
                        e.colour.first().map(|c| (c.rgba(), c.intensity)),
                        e.colour.last().map(|c| (c.rgba(), c.intensity)),
                    );
                    let curve: Vec<String> = e
                        .colour
                        .iter()
                        .step_by(10)
                        .map(|c| {
                            let [r, g, b, a] = c.rgba();
                            format!("{r},{g},{b},{a}x{}", c.intensity)
                        })
                        .collect();
                    println!("    colour every 10th key: {}", curve.join(" "));
                }
            }
        }
    }
    Ok(())
}
