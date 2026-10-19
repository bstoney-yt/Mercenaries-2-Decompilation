//! Measures the wardrobe's outfits (`WifPmcInterior._tOutfits`: each
//! hero's models, named in your own copy's scripts): whether each model
//! is in the archive and parses, its node count, and how many clips fit
//! its skeleton compared with the hero's own model, which the hero's
//! clips are bound to when the outfit is worn.
//!
//! ```text
//! cargo run --release -p m2-engine --example outfits -- "<install dir>" <base model> <outfit model>...
//! ```
//!
//! Prints names, hashes and numbers only.

use std::path::Path;

use m2_assets::name_hash;
use m2_engine::Install;
use m2_engine::anim::{Skeleton, load_clips};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [root, base, outfits @ ..] = &args[..] else {
        eprintln!("usage: outfits <install dir> <base model> <outfit model>...");
        std::process::exit(2);
    };
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let mut fits = |name: &str| -> Result<(usize, usize), Box<dyn std::error::Error>> {
        let data = source.model(name_hash(name))?;
        let nodes = data.parse()?.nodes;
        let skeleton = Skeleton::from_nodes(&nodes);
        let clips = load_clips(&mut source, &skeleton, &[""])?;
        Ok((nodes.len(), clips.len()))
    };
    let (nodes, own) = fits(base)?;
    println!("{base}\t{nodes} nodes\t{own} clips fit (the hero's own model)");
    let (mut ok, mut full) = (0, 0);
    for name in outfits {
        match fits(name) {
            Ok((nodes, n)) => {
                ok += 1;
                if n >= own {
                    full += 1;
                }
                println!("{name}\t{nodes} nodes\t{n} clips fit");
            }
            Err(e) => println!("{name}\tfails: {e}"),
        }
    }
    println!(
        "{ok} of {} outfits load; {full} fit at least as many clips as {base}",
        outfits.len()
    );
    Ok(())
}
