//! Lists or extracts blocks from a `.wad` archive in your own installation.
//!
//! ```text
//! cargo run -p m2-assets --example wad -- list    <archive.wad>
//! cargo run -p m2-assets --example wad -- assets  <archive.wad>
//! cargo run -p m2-assets --example wad -- names   <archive.wad>
//! cargo run -p m2-assets --example wad -- extract <archive.wad> <block path> <out file>
//! cargo run -p m2-assets --example wad -- textures <archive.wad>
//! cargo run -p m2-assets --example wad -- dds     <archive.wad> <texture name> <out.dds>
//! ```
//!
//! `textures` parses every texture header, finds its streamed mips and checks
//! their lengths; `dds` writes one texture's whole chain as a `.dds` file.
//!
//! Extracted files are for local inspection; keep them out of the repository
//! (`dumps/` and `local/` are ignored).

use std::fs::File;
use std::io::BufReader;
use std::process::ExitCode;

use std::collections::HashMap;
use std::io::{Read, Seek};

use m2_assets::texture::{BODY, INFO};
use m2_assets::{BlockContents, PixelFormat, Texture, TextureInfo, TextureKind, Ucfx, WadArchive};

/// Type hash of texture assets.
const TEXTURE_TYPE: u32 = 0xf011_157a;

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
                "usage: wad <list|assets|names|extract|textures|dds> <archive.wad> [args]".into(),
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
        ("names", []) => {
            // Decodes every block; use --release for the large archives.
            let types = index.type_hashes().to_vec();
            for block in 0..index.blocks().len() {
                let decoded = match wad.read_block(block) {
                    Ok(d) => d,
                    Err(e) => {
                        eprintln!("block {block}: {e}");
                        continue;
                    }
                };
                let contents = match BlockContents::parse(&decoded) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("block {block}: {e}");
                        continue;
                    }
                };
                for (entry, data) in contents.iter() {
                    let ty = types.iter().position(|&t| t == entry.type_hash);
                    let name = match Ucfx::parse(data) {
                        Ok(u) => u.name().unwrap_or("-").to_owned(),
                        Err(e) => format!("<{e}>"),
                    };
                    println!(
                        "{block:5} {:08x} type {:>2} {:#9x} {name}",
                        entry.name_hash,
                        ty.map_or("?".to_owned(), |t| t.to_string()),
                        entry.len,
                    );
                }
            }
        }
        ("extract", [block, out]) => {
            let data = wad
                .read_block_by_path(block)?
                .ok_or_else(|| format!("no block named {block}"))?;
            std::fs::write(out, &data)?;
            println!("wrote {} bytes to {out}", data.len());
        }
        ("textures", []) => textures(&mut wad)?,
        ("dds", [name, out]) => {
            let dds = texture_dds(&mut wad, name)?;
            std::fs::write(out, &dds)?;
            println!("wrote {} bytes to {out}", dds.len());
        }
        _ => return Err(format!("unknown command or wrong arguments: {args:?}").into()),
    }
    Ok(())
}

type AnyResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Calls `f` with every texture-typed asset in `block`.
fn each_texture<R: Read + Seek>(
    wad: &mut WadArchive<R>,
    block: usize,
    mut f: impl FnMut(u32, &[u8]),
) -> AnyResult<()> {
    let decoded = wad.read_block(block)?;
    let contents = BlockContents::parse(&decoded)?;
    for (entry, data) in contents.iter() {
        if entry.type_hash == TEXTURE_TYPE {
            f(entry.name_hash, data);
        }
    }
    Ok(())
}

/// The quality tier a block holds, from the `_Qn` at the end of its path.
fn block_tier(path: &str) -> Option<usize> {
    let at = path.rfind("_Q")?;
    let digit = path[at + 2..].chars().next()?.to_digit(10)?;
    Some(digit as usize)
}

/// The `BODY` of a streamed mip chunk (a texture asset without `INFO`).
fn chunk_body(data: &[u8]) -> Option<&[u8]> {
    let u = Ucfx::parse(data).ok()?;
    match u.top_level(INFO) {
        Some(_) => None,
        None => u.top_level(BODY),
    }
}

fn textures<R: Read + Seek>(wad: &mut WadArchive<R>) -> AnyResult<()> {
    // name hash -> header, and (name hash, tier) -> chunk length.
    let mut headers: Vec<(u32, String, TextureInfo)> = Vec::new();
    let mut chunks: HashMap<(u32, usize), usize> = HashMap::new();
    let mut errors = 0;
    let paths: Vec<String> = wad
        .index()
        .blocks()
        .iter()
        .map(|b| b.path.clone())
        .collect();
    for (block, path) in paths.iter().enumerate() {
        let tier = block_tier(path);
        let r = each_texture(wad, block, |hash, data| {
            if let Some(body) = chunk_body(data) {
                match tier {
                    Some(t) => {
                        chunks.insert((hash, t), body.len());
                    }
                    None => {
                        eprintln!("{hash:08x}: mip chunk in untiered block {path}");
                        errors += 1;
                    }
                }
                return;
            }
            match Texture::parse(data) {
                Ok(t) => headers.push((hash, t.name.unwrap_or("-").to_owned(), t.info)),
                Err(e) => {
                    eprintln!("{hash:08x} in {path}: {e}");
                    errors += 1;
                }
            }
        });
        if let Err(e) = r {
            eprintln!("block {block}: {e}");
            errors += 1;
        }
    }
    for (hash, name, info) in &headers {
        for tier in 0..3 {
            let got = chunks.get(&(*hash, tier)).copied();
            let ok = match (info.has_tier(tier), got) {
                (true, Some(len)) => len as u64 == info.tier_len(tier),
                (has, got) => !has && got.is_none(),
            };
            if !ok {
                eprintln!("{name}: tier Q{tier} chunk {got:?} does not match the header");
                errors += 1;
            }
        }
        println!(
            "{hash:08x} {:4}x{:<4} {:?} {:?} mips {:2} flags {:#04x} tiers {:04x?} {name}",
            info.width,
            info.height,
            info.format,
            info.kind,
            info.mip_count,
            info.flags,
            info.tier_masks
        );
    }
    eprintln!(
        "{} textures, {} streamed chunks, {errors} errors",
        headers.len(),
        chunks.len()
    );
    Ok(())
}

fn texture_dds<R: Read + Seek>(wad: &mut WadArchive<R>, name: &str) -> AnyResult<Vec<u8>> {
    let blocks = wad.index().blocks().len();
    let mut found = None;
    for block in 0..blocks {
        each_texture(wad, block, |hash, data| {
            if found.is_none() && Ucfx::parse(data).is_ok_and(|u| u.name() == Some(name)) {
                found = Some((hash, data.to_vec()));
            }
        })?;
        if found.is_some() {
            break;
        }
    }
    let (hash, asset) = found.ok_or_else(|| format!("no texture named {name}"))?;
    let texture = Texture::parse(&asset)?;

    // Streamed tiers live in blocks the asset table lists for this hash.
    let mut candidates: Vec<usize> = Vec::new();
    for a in wad.index().assets_by_hash(hash) {
        candidates.extend(
            std::iter::once(a.block)
                .chain(a.alt_blocks())
                .map(usize::from),
        );
    }
    candidates.sort_unstable();
    candidates.dedup();
    let mut tiers: [Option<Vec<u8>>; 3] = Default::default();
    for block in candidates {
        let Some(tier) = block_tier(&wad.index().blocks()[block].path).filter(|&t| t < 3) else {
            continue;
        };
        each_texture(wad, block, |h, data| {
            if h == hash
                && let Some(body) = chunk_body(data)
            {
                tiers[tier] = Some(body.to_vec());
            }
        })?;
    }
    let chain = texture.assemble([
        tiers[0].as_deref(),
        tiers[1].as_deref(),
        tiers[2].as_deref(),
    ])?;
    Ok(dds(&texture.info, &chain))
}

/// A `.dds` file holding `chain`.
fn dds(info: &TextureInfo, chain: &[u8]) -> Vec<u8> {
    let mut out = b"DDS ".to_vec();
    let mut put = |v: u32| out.extend_from_slice(&v.to_le_bytes());
    // Header: size; flags CAPS|HEIGHT|WIDTH|PIXELFORMAT|MIPMAPCOUNT|LINEARSIZE.
    put(124);
    put(0x1 | 0x2 | 0x4 | 0x1000 | 0x2_0000 | 0x8_0000);
    put(info.height.into());
    put(info.width.into());
    put(info.format.image_len(info.width.into(), info.height.into()) as u32);
    put(0);
    put(info.mip_count.into());
    (0..11).for_each(|_| put(0));
    // Pixel format.
    put(32);
    match info.format {
        PixelFormat::A8R8G8B8 => {
            put(0x41);
            put(0);
            put(32);
            put(0x00ff_0000);
            put(0x0000_ff00);
            put(0x0000_00ff);
            put(0xff00_0000);
        }
        f => {
            put(0x4);
            put(f.to_d3d());
            (0..5).for_each(|_| put(0));
        }
    }
    // Caps: TEXTURE|MIPMAP|COMPLEX; caps2: cube map with all faces.
    put(0x1000 | 0x40_0000 | 0x8);
    put(if info.kind == TextureKind::Cube {
        0x200 | 0xFC00
    } else {
        0
    });
    (0..3).for_each(|_| put(0));
    out.extend_from_slice(chain);
    out
}
