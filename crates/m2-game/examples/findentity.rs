//! Finds layer entities of your own installation by guid or name: which
//! layer each is in, where it stands and what it is called.
//!
//! ```text
//! cargo run -p m2-game --example findentity -- <vz.wad> <0xguid|name part>...
//! ```
//!
//! A query starting `0x` is a guid; anything else matches names containing
//! it (any case). `--near x,z,r` lists every entity within `r` m of
//! (`x`, `z`) instead. Prints one line per entity.

use std::fs::File;
use std::io::BufReader;

use m2_assets::WadArchive;
use m2_game::mission::GameData;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let wad = args
        .first()
        .ok_or("usage: findentity <vz.wad> <0xguid|name part>... | --near x,z,r")?;
    let mut archive = WadArchive::open(BufReader::new(File::open(wad)?))?;
    let data = GameData::read(&mut archive)?;
    let near = args
        .iter()
        .position(|a| a == "--near")
        .and_then(|i| args.get(i + 1))
        .map(|s| {
            s.split(',')
                .filter_map(|v| v.parse::<f32>().ok())
                .collect::<Vec<_>>()
        });
    for (layer, entities) in &data.layers {
        for e in entities {
            let hit = match &near {
                Some(v) if v.len() == 3 => {
                    let (dx, dz) = (e.at[0] - v[0], e.at[2] - v[1]);
                    (dx * dx + dz * dz).sqrt() <= v[2]
                }
                Some(_) => false,
                None => args[1..].iter().any(|q| match q.strip_prefix("0x") {
                    Some(h) => u32::from_str_radix(h, 16).is_ok_and(|g| g == e.guid),
                    None => e
                        .name
                        .to_ascii_lowercase()
                        .contains(&q.to_ascii_lowercase()),
                }),
            };
            if hit {
                println!(
                    "{layer}: {:#x} {:?} at [{:.1}, {:.1}, {:.1}] yaw {:.2} labels {} script {:x?} template {:?} crew {:?}",
                    e.guid,
                    e.name,
                    e.at[0],
                    e.at[1],
                    e.at[2],
                    e.yaw,
                    e.labels.len(),
                    e.script,
                    e.template,
                    e.crew
                );
                if let Some(r) = &e.region {
                    println!("  region corners (x, z) {:?}", r.corners);
                }
            }
        }
    }
    Ok(())
}
