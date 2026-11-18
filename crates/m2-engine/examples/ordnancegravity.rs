//! Lists every projectile template of your own installation with its
//! speed and `GravityScale` ([`m2_engine::ordnance`]): what the 360's
//! ordnance step multiplies the world's gravity by (0x82295DF0).
//!
//! ```text
//! cargo run -p m2-engine --example ordnancegravity -- "<install dir>"
//! ```
//!
//! Prints names and numbers only.

use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_engine::Install;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: ordnancegravity <install dir>")?;
    let install = Install::find(Path::new(&root))?;
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
    let o = m2_engine::ordnance::read(&db);
    let mut rows: Vec<(String, f32, f32, f32)> = o
        .projectiles
        .iter()
        .map(|(id, p)| {
            (
                names
                    .get(id)
                    .map_or(format!("{id:08x}"), |n| (*n).to_owned()),
                p.gravity,
                p.speed,
                p.max_speed,
            )
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, g, v, max) in rows {
        println!("{name:<48} GravityScale {g:5.2}  Velocity {v:7.1}  MaxVelocity {max:7.1}");
    }
    Ok(())
}
