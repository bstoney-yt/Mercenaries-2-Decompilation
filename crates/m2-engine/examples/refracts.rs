//! Lists every model of `vz.wad` with a material drawn by a refracting
//! pixel shader (`PgDiffRefractFP`, `PgDiffRefractNormFP`): the model's
//! hash, the material's index, technique, flags, textures and constants.
//! A survey for the heat haze over vehicles' engines.
//!
//! ```text
//! cargo run --release -p m2-engine --example refracts -- "<install dir>"
//! ```
//!
//! Prints hashes and numbers only.

use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_assets::model::MODEL_TYPE;
use m2_assets::name_hash;
use m2_engine::Install;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: refracts <install dir>")?;
    let install = Install::find(Path::new(&root))?;
    let mut source = install.open_world()?;
    let shaders = [
        (name_hash("PgDiffRefractFP"), "PgDiffRefractFP"),
        (name_hash("PgDiffRefractNormFP"), "PgDiffRefractNormFP"),
    ];
    let models: Vec<u32> = {
        let index = source.archive().index();
        index
            .assets()
            .iter()
            .filter(|a| index.type_hash(a) == MODEL_TYPE)
            .map(|a| a.name_hash)
            .collect()
    };
    let db_hash = {
        let index = source.archive().index();
        index
            .assets()
            .iter()
            .find(|a| index.type_hash(a) == TEMPLATE_DB_TYPE)
            .map(|a| a.name_hash)
            .ok_or("no template database")?
    };
    let db_data = source.asset(db_hash, TEMPLATE_DB_TYPE)?;
    let db = Layer::parse(&db_data)?;
    let names = db.names();
    let model_of = db.hashes("ModelName");
    let (mut read, mut with) = (0, 0);
    for h in models {
        let Ok(data) = source.model(h) else { continue };
        let Ok(m) = data.parse() else { continue };
        read += 1;
        let mut any = false;
        for (i, x) in m.materials.iter().enumerate() {
            let Some((_, shader)) = shaders.iter().find(|s| s.0 == x.technique) else {
                continue;
            };
            any = true;
            let textures: Vec<String> = x.textures.iter().map(|t| format!("{t:08x}")).collect();
            let constants: Vec<String> = x.constants.iter().map(|c| format!("{c:.3}")).collect();
            println!(
                "model {h:08x} material {i} ({:08x}) {shader} flags {:04x} textures [{}] constants [{}]",
                x.name_hash,
                x.flags,
                textures.join(" "),
                constants.join(" ")
            );
        }
        if any {
            let mut by: Vec<String> = model_of
                .iter()
                .filter(|(_, m)| **m == h)
                .map(|(id, _)| format!("{id:08x} {}", names.get(id).copied().unwrap_or("?")))
                .collect();
            by.sort_unstable();
            println!("  model {h:08x}: templates {by:?}");
        }
        with += usize::from(any);
    }
    println!("{with} of {read} models have a refracting material");
    Ok(())
}
