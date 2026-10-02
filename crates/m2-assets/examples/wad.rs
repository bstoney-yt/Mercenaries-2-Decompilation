//! Lists or extracts blocks from a `.wad` archive in your own installation.
//!
//! ```text
//! cargo run -p m2-assets --example wad -- list    <archive.wad>
//! cargo run -p m2-assets --example wad -- assets  <archive.wad>
//! cargo run -p m2-assets --example wad -- extract <archive.wad> <block path> <out file>
//! ```
//!
//! Extracted files are for local inspection; keep them out of the repository
//! (`dumps/` and `local/` are ignored).

use std::fs::File;
use std::io::BufReader;
use std::process::ExitCode;

use m2_assets::WadArchive;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let (cmd, path) = match args {
        [cmd, path, ..] => (cmd.as_str(), path),
        _ => {
            return Err(
                "usage: wad <list|assets|extract> <archive.wad> [block path] [out file]".into(),
            );
        }
    };
    let mut wad = WadArchive::open(BufReader::new(File::open(path)?))?;
    let index = wad.index();
    match (cmd, &args[2..]) {
        ("list", []) => {
            let h = index.header();
            println!(
                "{} blocks, {} assets, {} types, data at {:#x}",
                h.block_count, h.asset_count, h.type_count, h.data_offset
            );
            for (i, b) in index.blocks().iter().enumerate() {
                println!(
                    "{i:5} {:#012x} stored {:#7x} sectors {:#6x} flags {:#06x} assets {:4} {}",
                    b.offset(),
                    b.stored_sectors,
                    b.sectors,
                    b.flags,
                    index.assets_in_block(i).count(),
                    b.path
                );
            }
        }
        ("assets", []) => {
            for a in index.assets() {
                let alt: Vec<String> = a.alt_blocks().map(|b| b.to_string()).collect();
                println!(
                    "{:08x} type {:2} ({:08x}) block {:5} alt [{}]",
                    a.name_hash,
                    a.type_index,
                    index.type_hash(a),
                    a.block,
                    alt.join(",")
                );
            }
        }
        ("extract", [block, out]) => {
            let data = wad
                .read_block_by_path(block)?
                .ok_or_else(|| format!("no block named {block}"))?;
            std::fs::write(out, &data)?;
            println!("wrote {} bytes to {out}", data.len());
        }
        _ => return Err(format!("unknown command or wrong arguments: {args:?}").into()),
    }
    Ok(())
}
