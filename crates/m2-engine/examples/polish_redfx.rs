//! Throwaway: the fields of RedEffectComponent and RedEffectTweak, and the satellite bomb templates' records.
use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_engine::Install;
use std::path::Path;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args().nth(1).unwrap();
    let install = Install::find(Path::new(&root))?;
    let mut source = install.open_world()?;
    let db_hash = {
        let index = source.archive().index();
        index
            .assets()
            .iter()
            .find(|a| index.type_hash(a) == TEMPLATE_DB_TYPE)
            .map(|a| a.name_hash)
            .unwrap()
    };
    let data = source.asset(db_hash, TEMPLATE_DB_TYPE)?;
    let db = Layer::parse(&data)?;
    let names = db.names();
    for comp in ["RedEffectComponent", "RedEffectTweak"] {
        let c = db.component(comp).unwrap();
        println!(
            "{comp}: fields {:?}",
            c.fields
                .iter()
                .map(|f| (format!("{:08x}", f.name_hash), f.offset, f.bit_width))
                .collect::<Vec<_>>()
        );
        for (id, r) in c.records().unwrap_or_default() {
            let n = names.get(&id).copied().unwrap_or("?");
            if n.contains("Airstike Bomb")
                || n.contains("Smart Bomb")
                || comp == "RedEffectTweak" && n.to_lowercase().contains("bomb")
            {
                let words: Vec<String> = r
                    .chunks(4)
                    .map(|w| {
                        let v = u32::from_le_bytes([
                            w[0],
                            w.get(1).copied().unwrap_or(0),
                            w.get(2).copied().unwrap_or(0),
                            w.get(3).copied().unwrap_or(0),
                        ]);
                        format!("{v:08x}({:.3})", f32::from_bits(v))
                    })
                    .collect();
                println!("  {n}: {}", words.join(" "));
            }
        }
    }
    Ok(())
}
