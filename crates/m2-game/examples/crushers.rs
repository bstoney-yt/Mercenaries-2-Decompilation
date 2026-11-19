//! Lists the templates that carry the `Crusher` component
//! ([`m2_game::crusher`]), from your own installation.
//!
//! ```text
//! cargo run --release -p m2-game --example crushers -- <vz.wad>
//! ```
//!
//! Prints template names and numbers only.

use std::fs::File;
use std::io::BufReader;

use m2_assets::WadArchive;
use m2_game::mission::GameData;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("usage: crushers <vz.wad>")?;
    let mut wad = WadArchive::open(BufReader::new(File::open(path)?))?;
    let data = GameData::read(&mut wad)?;
    let mut all: Vec<(&String, &f32)> = data.template_crushers.iter().collect();
    all.sort_by(|a, b| a.0.cmp(b.0));
    for (name, v) in &all {
        println!("crusher: {name} ({v})");
    }
    println!("crushers: {} templates", all.len());
    Ok(())
}
