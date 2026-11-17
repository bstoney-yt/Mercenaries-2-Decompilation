//! Lists the material animations in your own installation's archive:
//! per animation its name hash, frames and rate, and per material its
//! tracks (parameter hash, divisor, flags, keys and the first and last
//! values), to work out what the parameters drive.
//!
//! ```text
//! cargo run -p m2-assets --example matanim -- <vz.wad> [name or hash...]
//! ```
//!
//! With names or hashes, only those animations. Prints hashes and numbers
//! only.

use std::fs::File;
use std::io::BufReader;

use m2_assets::animation::ANIMATION_TYPE;
use m2_assets::material_animation::MaterialAnimation;
use m2_assets::{BlockContents, WadArchive, name_hash};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        return Err("usage: matanim <vz.wad> [name or hash...]".into());
    };
    let wanted: Vec<u32> = args[1..]
        .iter()
        .map(|a| u32::from_str_radix(a, 16).unwrap_or_else(|_| name_hash(a)))
        .collect();
    let mut wad = WadArchive::open(BufReader::new(File::open(path)?))?;
    for block in 0..wad.index().blocks().len() {
        let decoded = wad.read_block(block)?;
        let contents = BlockContents::parse(&decoded)?;
        for (entry, data) in contents.iter() {
            if entry.type_hash != ANIMATION_TYPE
                || !MaterialAnimation::is_material(data)
                || (!wanted.is_empty() && !wanted.contains(&entry.name_hash))
            {
                continue;
            }
            let Ok(m) = MaterialAnimation::parse(data) else {
                continue;
            };
            println!(
                "anim {:08x}: {} frames at {} fps, {} materials",
                entry.name_hash,
                m.frames,
                m.fps,
                m.materials.len()
            );
            for mt in &m.materials {
                println!("  material {:08x}", mt.material_hash);
                for t in &mt.tracks {
                    let first = t.keys.first().map_or(0.0, |k| t.value(k.1));
                    let last = t.keys.last().map_or(0.0, |k| t.value(k.1));
                    println!(
                        "    param {:08x} div {} flags {} keys {} first {first:.3} last {last:.3} (last frame {})",
                        t.param_hash,
                        t.divisor,
                        t.flags,
                        t.keys.len(),
                        t.keys.last().map_or(0, |k| k.0)
                    );
                }
            }
        }
    }
    Ok(())
}
