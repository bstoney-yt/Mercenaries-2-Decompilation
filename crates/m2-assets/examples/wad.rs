//! Lists or extracts blocks from a `.wad` archive in your own installation.
//!
//! ```text
//! cargo run -p m2-assets --example wad -- list    <archive.wad>
//! cargo run -p m2-assets --example wad -- assets  <archive.wad>
//! cargo run -p m2-assets --example wad -- names   <archive.wad>
//! cargo run -p m2-assets --example wad -- extract <archive.wad> <block path> <out file>
//! cargo run -p m2-assets --example wad -- textures <archive.wad>
//! cargo run -p m2-assets --example wad -- dds     <archive.wad> <texture name> <out.dds>
//! cargo run -p m2-assets --example wad -- models  <archive.wad>
//! cargo run -p m2-assets --example wad -- obj     <archive.wad> <model hash> <out.obj> [lod]
//! cargo run -p m2-assets --example wad -- material <archive.wad> <material hash>
//! cargo run -p m2-assets --example wad -- anims   <archive.wad>
//! cargo run -p m2-assets --example wad -- world   <archive.wad>
//! cargo run -p m2-assets --example wad -- entities <archive.wad>
//! cargo run -p m2-assets --example wad -- hash    <archive.wad> <name>...
//! cargo run -p m2-assets --example wad -- grep    <archive.wad> <text> [max hits shown]
//! cargo run -p m2-assets --example wad -- types   <archive.wad>
//! cargo run -p m2-assets --example wad -- scripts <archive.wad>
//! cargo run -p m2-assets --example wad -- script  <archive.wad> <script hash>
//! cargo run -p m2-assets --example wad -- calls   <archive.wad> <callee> [first arg] [count]
//! cargo run -p m2-assets --example wad -- strings <archive.wad> [text]
//! cargo run -p m2-assets --example wad -- movies  <archive.wad>
//! cargo run -p m2-assets --example wad -- actions <archive.wad> <movie name>
//! cargo run -p m2-assets --example wad -- fonts   <archive.wad>
//! ```
//!
//! `textures` parses every texture header, finds its streamed mips and checks
//! their lengths; `dds` writes one texture's whole chain as a `.dds` file.
//! `models` parses every model with its streamed chunks and checks indices,
//! bounds and bone palettes; `obj` writes one level of detail of one model
//! (by its name hash in hex) as a Wavefront `.obj` file, in model space.
//! `material` finds every model drawing with a material (by its name hash
//! in hex) and describes it: flags, levels of detail, segments, and for
//! each primitive group using the material its geometry kind, vertex
//! elements and the range of every element's values (no vertex data).
//! `anims` decodes every animation clip, finds a model whose nodes cover its
//! tracks, and checks quaternion lengths, frame rates, the first frame
//! against the model's bind pose, and that sampled poses are finite; it
//! also parses every material animation.
//! `world` parses every terrain cell, low-resolution tile and entity layer,
//! checks material references and that the layout layer places each cell
//! on the 400 m grid, and summarises the components.
//! `entities` lists every layer entity template (`civ_car 0x0012abcd` is
//! the template `civ_car`), how many entities it has, which components they
//! carry and which asset types its name's hash finds.
//! `hash` prints each name's hash and the assets of the archive it keys.
//! `grep` finds which assets hold a piece of text and shows the readable
//! bytes around each hit, to find where a name is defined.
//! `types` summarises every asset type: how many, their sizes, a few names
//! and block paths, the section layout of one, any `Lua` bytecode or
//! script-like text, and the longest readable strings in one sample, to
//! find which types hold what (missions, scripts) without dumping data.
//! `scripts` reads every compiled Lua script and lists, per script, its
//! name, block and the globals it defines, then counts across all scripts
//! the globals they read, the `Global.key` members they use and the method
//! names they call: the engine functions the scripts expect to exist. It
//! also counts module loads (`import`, `inherit`, ...) by name and guesses
//! which script each module is, from the members other scripts use on it
//! and the globals each script defines. `script` prints one script's
//! instruction listing. `calls` prints a few places scripts call
//! `callee` (`Event.Create`), optionally only those whose first argument
//! is the given global or string (`Event.TimerRelative`), with the
//! instructions building the call and the start of any callback function
//! named in it: how the engine function is used.
//! `strings` lists every string table (block, count, whether its keys are
//! sorted) and, for the text with id `text` (`VzaCon001.Objectives.001`)
//! else the strings that are exactly `text` (else contain it),
//! their key and the few strings stored either side, cut to 60
//! characters: pairs of key and text to find the key hash with.
//! `movies` parses every Scaleform movie (each copy, block by block) and
//! lists its name (the exporter's swf name), size, stage, frame rate and
//! count, tag and character counts and any tags that did not parse, then
//! checks every external image names a texture in the archive.
//! `fonts` parses every bitmap font: line height, glyph and material counts,
//! the glyphs' code range and largest size, and each material's texture
//! (by asset hash) with its size, checking every glyph rectangle lies on
//! its page and agrees with the glyph's pixel size.
//!
//! Extracted files are for local inspection; keep them out of the repository
//! (`dumps/` and `local/` are ignored).

use std::fs::File;
use std::io::BufReader;
use std::process::ExitCode;

use std::collections::HashMap;
use std::io::{Read, Seek};

use m2_assets::animation::{ANIMATION_TYPE, AnimationError};
use m2_assets::material_animation::MaterialAnimation;
use m2_assets::model::{GroupExtra, MODEL_TYPE, Node};
use m2_assets::texture::{BODY, INFO};
use m2_assets::vertex::DeclUsage;
use m2_assets::{
    BlockContents, Clip, Codec, Model, PixelFormat, PrimitiveGroup, Texture, TextureInfo,
    TextureKind, Ucfx, WadArchive,
};
use m2_assets::{StringTable, strings::STRINGS_TYPE};
use m2_assets::{
    font::{FONT_TYPE, Font},
    gfx::{Character, MOVIE_TYPE, movie_from_asset},
    name_hash,
};

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
                "usage: wad <list|assets|names|extract|textures|dds|models|obj|anims|world|types> <archive.wad> [args]".into(),
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
        ("types", []) => types(&mut wad)?,
        ("strings", rest) => string_tables(&mut wad, rest.first().map(String::as_str))?,
        ("scripts", []) => scripts(&mut wad)?,
        ("script", [hash]) => script_listing(&mut wad, u32::from_str_radix(hash, 16)?)?,
        ("calls", [callee, rest @ ..]) => {
            // `[first arg] [count]`; a lone number is the count, and "-"
            // is no filter.
            let (shape, count) = match rest {
                [n] if n.parse::<usize>().is_ok() => (None, n.parse()?),
                [f, n, ..] => (Some(f.as_str()).filter(|f| *f != "-"), n.parse()?),
                [f] => (Some(f.as_str()).filter(|f| *f != "-"), 8),
                [] => (None, 8),
            };
            call_sites(&mut wad, callee, shape, count)?
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
        ("movies", []) => movies(&mut wad)?,
        ("actions", [name]) => actions(&mut wad, name)?,
        ("fonts", []) => fonts(&mut wad)?,
        ("dds", [name, out]) => {
            let dds = texture_dds(&mut wad, name)?;
            std::fs::write(out, &dds)?;
            println!("wrote {} bytes to {out}", dds.len());
        }
        ("models", []) => models(&mut wad)?,
        ("material", [hash]) => material_users(&mut wad, u32::from_str_radix(hash, 16)?)?,
        ("anims", []) => anims(&mut wad)?,
        ("world", []) => world(&mut wad)?,
        ("entities", []) => entities(&mut wad)?,
        ("grep", [text, rest @ ..]) => {
            // Which assets hold `text`, and the readable text around it.
            let limit: usize = rest.first().map_or(Ok(40), |n| n.parse())?;
            let types = index.type_hashes().to_vec();
            let paths: Vec<String> = index.blocks().iter().map(|b| b.path.clone()).collect();
            let needle = text.as_bytes();
            let mut shown = 0;
            let mut hits = 0;
            for (block, path) in paths.iter().enumerate() {
                let decoded = match wad.read_block(block) {
                    Ok(d) => d,
                    Err(e) => {
                        eprintln!("block {block}: {e}");
                        continue;
                    }
                };
                let Ok(contents) = BlockContents::parse(&decoded) else {
                    continue;
                };
                for (entry, data) in contents.iter() {
                    let ty = types.iter().position(|&t| t == entry.type_hash);
                    for at in memchr_all(needle, data) {
                        hits += 1;
                        if shown >= limit {
                            continue;
                        }
                        shown += 1;
                        let from = at.saturating_sub(48);
                        let to = (at + needle.len() + 48).min(data.len());
                        let around: String = data[from..to]
                            .iter()
                            .map(|&b| {
                                if (0x20..0x7f).contains(&b) {
                                    b as char
                                } else {
                                    '.'
                                }
                            })
                            .collect();
                        println!(
                            "{path} {:08x} type {} {:#x}/{:#x}: {around}",
                            entry.name_hash,
                            ty.map_or("?".to_owned(), |t| t.to_string()),
                            at,
                            data.len()
                        );
                    }
                }
            }
            println!("{hits} hits, {shown} shown");
        }
        ("hash", names) => {
            // The index alone: which assets, of which types, each name finds.
            for name in names {
                let hash = m2_assets::name_hash(name);
                let types: Vec<String> = index
                    .assets()
                    .iter()
                    .filter(|a| a.name_hash == hash)
                    .map(|a| {
                        format!(
                            "type {} ({:08x}) block {}",
                            a.type_index,
                            index.type_hash(a),
                            a.block
                        )
                    })
                    .collect();
                println!(
                    "{hash:08x} {name}: {}",
                    if types.is_empty() {
                        "-".to_owned()
                    } else {
                        types.join("; ")
                    }
                );
            }
        }
        ("obj", [hash, out, rest @ ..]) => {
            let hash = u32::from_str_radix(hash.trim_start_matches("0x"), 16)?;
            let lod = match rest {
                [] => 0,
                [lod] => lod.parse()?,
                _ => return Err("usage: obj <archive.wad> <model hash> <out.obj> [lod]".into()),
            };
            let obj = model_obj(&mut wad, hash, lod)?;
            std::fs::write(out, obj)?;
            println!("wrote {out}");
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
    // Header: size; flags CAPS|HEIGHT|WIDTH|PIXELFORMAT|MIPMAPCOUNT plus
    // LINEARSIZE (compressed) or PITCH (uncompressed).
    let (size_flag, pitch) = match info.format {
        PixelFormat::A8R8G8B8 => (0x8, u32::from(info.width) * 4),
        f => (
            0x8_0000,
            f.image_len(info.width.into(), info.height.into()) as u32,
        ),
    };
    put(124);
    put(0x1 | 0x2 | 0x4 | 0x1000 | 0x2_0000 | size_flag);
    put(info.height.into());
    put(info.width.into());
    put(pitch);
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

/// A model's resident asset and its streamed chunks, ordered by their offset
/// in the reassembled asset (the block directory's field at 0x08).
#[derive(Default)]
struct ModelBytes {
    path: String,
    base: Option<Vec<u8>>,
    chunks: Vec<(u32, Vec<u8>)>,
}

impl ModelBytes {
    fn assemble(&mut self) -> AnyResult<Model<'_>> {
        self.chunks.sort_by_key(|c| c.0);
        let base = self.base.as_deref().ok_or("no resident asset")?;
        let mut model = Model::parse(base)?;
        for (_, chunk) in &self.chunks {
            model.add_streamed(chunk)?;
        }
        Ok(model)
    }
}

/// Adds the model assets of `block` to `out`.
fn collect_models<R: Read + Seek>(
    wad: &mut WadArchive<R>,
    block: usize,
    only: Option<u32>,
    out: &mut HashMap<u32, ModelBytes>,
) -> AnyResult<()> {
    let path = wad.index().blocks()[block].path.clone();
    let decoded = wad.read_block(block)?;
    let contents = BlockContents::parse(&decoded)?;
    for (entry, data) in contents.iter() {
        if entry.type_hash != MODEL_TYPE || only.is_some_and(|h| h != entry.name_hash) {
            continue;
        }
        let m = out.entry(entry.name_hash).or_default();
        if entry.unk_0x08 == 0 {
            m.path.clone_from(&path);
            m.base = Some(data.to_vec());
        } else {
            m.chunks.push((entry.unk_0x08, data.to_vec()));
        }
    }
    Ok(())
}

/// Problems in a group that parsing does not reject, as found in the data.
fn check_group(model: &Model<'_>, g: &PrimitiveGroup<'_>) -> Vec<String> {
    let mut problems = Vec::new();
    match &g.info.extra {
        GroupExtra::Bounds { aabb, .. } => {
            if let Some(pos) = g.decl.find(DeclUsage::Position, 0)
                && let Some(p) = g.vertices.read(pos)
            {
                // The bounds were computed before positions were rounded to
                // half floats, so allow for the rounding.
                let slack = |b: f32| b.abs() * 2e-3 + 1e-3;
                let outside = p
                    .iter()
                    .filter(|v| {
                        (0..3).any(|i| {
                            v[i] < aabb.min[i] - slack(aabb.min[i])
                                || v[i] > aabb.max[i] + slack(aabb.max[i])
                        })
                    })
                    .count();
                if outside > 0 {
                    problems.push(format!("{outside} vertices outside the group bounds"));
                }
            }
        }
        GroupExtra::Palette(_) => {
            let bones = g.info.bone_nodes();
            if bones.iter().any(|&b| usize::from(b) >= model.nodes.len()) {
                problems.push("bone palette names a node past HIER".to_owned());
            }
            if let Some(idx) = g.decl.find(DeclUsage::BlendIndices, 0)
                && let Some(v) = g.vertices.read(idx)
                && v.iter().flatten().any(|&b| b as usize >= bones.len())
            {
                problems.push("blend index past the bone palette".to_owned());
            }
        }
    }
    if let Err(e) = g.layout() {
        problems.push(format!("no renderer layout: {e}"));
    }
    problems
}

fn models<R: Read + Seek>(wad: &mut WadArchive<R>) -> AnyResult<()> {
    let mut all: HashMap<u32, ModelBytes> = HashMap::new();
    let mut errors = 0;
    for block in 0..wad.index().blocks().len() {
        if let Err(e) = collect_models(wad, block, None, &mut all) {
            eprintln!("block {block}: {e}");
            errors += 1;
        }
    }
    let mut hashes: Vec<u32> = all.keys().copied().collect();
    hashes.sort_unstable();
    let (mut groups, mut warnings, mut chunks) = (0, 0, 0);
    for hash in hashes {
        let Some(bytes) = all.get_mut(&hash) else {
            continue;
        };
        chunks += bytes.chunks.len();
        let path = bytes.path.clone();
        let model = match bytes.assemble() {
            Ok(m) => m,
            Err(e) => {
                eprintln!("{hash:08x} {path}: {e}");
                errors += 1;
                continue;
            }
        };
        if !model.is_complete() {
            eprintln!("{hash:08x}: slots {:?} missing", model.missing_slots());
            errors += 1;
        }
        let mut kinds = Vec::new();
        for slot in 0..model.info.slot_count as u8 {
            let Some(g) = model.geometry(slot) else {
                continue;
            };
            kinds.push(format!("{:?}", g.kind));
            for group in &g.groups {
                groups += 1;
                for p in check_group(&model, group) {
                    eprintln!("{hash:08x} slot {slot}: {p}");
                    warnings += 1;
                }
            }
        }
        kinds.dedup();
        println!(
            "{hash:08x} nodes {:3} materials {:3} slots {:3} lods {} chunks {} {} {path}",
            model.nodes.len(),
            model.materials.len(),
            model.info.slot_count,
            model.info.lod_count,
            bytes.chunks.len(),
            kinds.join(","),
        );
    }
    eprintln!(
        "{} models, {chunks} streamed chunks, {groups} primitive groups, {errors} errors, {warnings} warnings",
        all.len()
    );
    Ok(())
}

/// Describes every model drawing with the material named `material`.
fn material_users<R: Read + Seek>(wad: &mut WadArchive<R>, material: u32) -> AnyResult<()> {
    let mut all: HashMap<u32, ModelBytes> = HashMap::new();
    for block in 0..wad.index().blocks().len() {
        let _ = collect_models(wad, block, None, &mut all);
    }
    let mut hashes: Vec<u32> = all.keys().copied().collect();
    hashes.sort_unstable();
    let mut shown = 0;
    for hash in hashes {
        let Some(bytes) = all.get_mut(&hash) else {
            continue;
        };
        let path = bytes.path.clone();
        let Ok(model) = bytes.assemble() else {
            continue;
        };
        let Some(index) = model.materials.iter().position(|m| m.name_hash == material) else {
            continue;
        };
        shown += 1;
        if shown > 6 {
            println!("{hash:08x} {path} (more not described)");
            continue;
        }
        let m = &model.materials[index];
        println!(
            "\n== model {hash:08x} {path}: flags {:#x}, lods {}, lod distance {}, nodes {}, materials {}, tiny members {}",
            model.info.flags,
            model.info.lod_count,
            model.info.lod_distance,
            model.nodes.len(),
            model.materials.len(),
            model.tiny_members().len(),
        );
        println!(
            "material {index}: technique {:08x}, flags {:04x}, textures {:08x?}, constants {:?}",
            m.technique, m.flags, m.textures, m.constants
        );
        for s in &model.segments {
            let kind = model.geometry(s.slot).map(|g| format!("{:?}", g.kind));
            println!(
                "segment node {} slot {} lod mask {:#04b} {}",
                s.node,
                s.slot,
                s.lod_mask,
                kind.unwrap_or_else(|| "(not loaded)".into())
            );
        }
        for slot in 0..model.info.slot_count as u8 {
            let Some(g) = model.geometry(slot) else {
                continue;
            };
            for (gi, group) in g.groups.iter().enumerate() {
                let uses: Vec<String> = group
                    .material_ranges
                    .iter()
                    .map(|r| format!("{}:{}", r.material, r.index_count()))
                    .collect();
                if !group
                    .material_ranges
                    .iter()
                    .any(|r| r.material as usize == index)
                {
                    println!(
                        "slot {slot} group {gi}: {:?}, ranges (material:indices) {}",
                        g.kind,
                        uses.join(" ")
                    );
                    continue;
                }
                println!(
                    "slot {slot} group {gi}: {:?}, {} vertices, stride {}, ranges (material:indices) {}",
                    g.kind,
                    group.vertices.count,
                    group.vertices.stride,
                    uses.join(" ")
                );
                for e in group.decl.elements() {
                    let Some(values) = group.vertices.read(e) else {
                        println!(
                            "  {:?}{} {:?} at {}: unreadable",
                            e.usage, e.usage_index, e.ty, e.offset
                        );
                        continue;
                    };
                    let mut lo = [f32::MAX; 4];
                    let mut hi = [f32::MIN; 4];
                    let mut distinct_w = std::collections::BTreeSet::new();
                    for v in &values {
                        for c in 0..4 {
                            lo[c] = lo[c].min(v[c]);
                            hi[c] = hi[c].max(v[c]);
                        }
                        distinct_w.insert(v[3].to_bits());
                    }
                    let outside = if e.usage == DeclUsage::TexCoord {
                        let n = values
                            .iter()
                            .filter(|v| {
                                !(0.0..=1.0).contains(&v[0]) || !(0.0..=1.0).contains(&v[1])
                            })
                            .count();
                        format!(", {n} outside 0..1")
                    } else {
                        String::new()
                    };
                    println!(
                        "  {:?}{} {:?} at {}: min {:.3?} max {:.3?}, {} distinct w{outside}",
                        e.usage,
                        e.usage_index,
                        e.ty,
                        e.offset,
                        lo,
                        hi,
                        distinct_w.len()
                    );
                }
            }
        }
    }
    println!("\n{shown} models use material {material:08x}");
    Ok(())
}

fn model_obj<R: Read + Seek>(wad: &mut WadArchive<R>, hash: u32, lod: u32) -> AnyResult<String> {
    let mut blocks: Vec<usize> = Vec::new();
    for a in wad.index().assets_by_hash(hash) {
        blocks.extend(
            std::iter::once(a.block)
                .chain(a.alt_blocks())
                .map(usize::from),
        );
    }
    blocks.sort_unstable();
    blocks.dedup();
    let mut found = HashMap::new();
    for block in blocks {
        collect_models(wad, block, Some(hash), &mut found)?;
    }
    let bytes = found
        .get_mut(&hash)
        .ok_or_else(|| format!("no model {hash:08x}"))?;
    let model = bytes.assemble()?;
    if !model.has_lod(lod) {
        return Err(format!("LOD {lod} is not complete").into());
    }

    use std::fmt::Write;
    let mut obj = format!("# model {hash:08x}, LOD {lod}\n");
    let mut base = 1;
    for (n, (segment, geometry)) in model.lod(lod).enumerate() {
        let to_model = model
            .node_to_model(segment.node)
            .ok_or("segment node out of range")?;
        for (gi, g) in geometry.groups.iter().enumerate() {
            let read = |usage| {
                g.decl
                    .find(usage, 0)
                    .and_then(|e| g.vertices.read(e))
                    .unwrap_or_default()
            };
            let pos = read(DeclUsage::Position);
            let uv = read(DeclUsage::TexCoord);
            let nrm = read(DeclUsage::Normal);
            writeln!(obj, "o seg{n}_node{}_group{gi}", segment.node)?;
            for p in &pos {
                let v = to_model.transform_point3(m2_math::glam::Vec3::new(p[0], p[1], p[2]));
                writeln!(obj, "v {} {} {}", v.x, v.y, v.z)?;
            }
            for t in &uv {
                writeln!(obj, "vt {} {}", t[0], 1.0 - t[1])?;
            }
            for v in &nrm {
                let v = to_model
                    .transform_vector3(m2_math::glam::Vec3::new(v[0], v[1], v[2]))
                    .normalize_or_zero();
                writeln!(obj, "vn {} {} {}", v.x, v.y, v.z)?;
            }
            let (has_uv, has_nrm) = (!uv.is_empty(), !nrm.is_empty());
            for range in &g.material_ranges {
                let material = &model.materials[range.material as usize];
                writeln!(
                    obj,
                    "usemtl m{}_{:08x} # textures {:08x?}",
                    range.material, material.name_hash, material.textures
                )?;
                for tri in g.triangles(range) {
                    let corner = |i: u16| {
                        let i = base + usize::from(i);
                        match (has_uv, has_nrm) {
                            (true, true) => format!("{i}/{i}/{i}"),
                            (true, false) => format!("{i}/{i}"),
                            (false, true) => format!("{i}//{i}"),
                            (false, false) => i.to_string(),
                        }
                    };
                    // The engine is left-handed; reverse the winding for
                    // right-handed .obj viewers.
                    writeln!(
                        obj,
                        "f {} {} {}",
                        corner(tri[0]),
                        corner(tri[2]),
                        corner(tri[1])
                    )?;
                }
            }
            base += pos.len();
        }
    }
    Ok(obj)
}

fn anims<R: Read + Seek>(wad: &mut WadArchive<R>) -> AnyResult<()> {
    // Model node lists and raw clip assets, in one pass over the blocks.
    let mut skeletons: Vec<(u32, Vec<Node>)> = Vec::new();
    let mut clips: Vec<(u32, Vec<u8>)> = Vec::new();
    for block in 0..wad.index().blocks().len() {
        let decoded = wad.read_block(block)?;
        let contents = BlockContents::parse(&decoded)?;
        for (entry, data) in contents.iter() {
            match entry.type_hash {
                MODEL_TYPE if entry.unk_0x08 == 0 => match Model::parse(data) {
                    Ok(m) => skeletons.push((entry.name_hash, m.nodes)),
                    Err(e) => eprintln!("model {:08x}: {e}", entry.name_hash),
                },
                ANIMATION_TYPE => clips.push((entry.name_hash, data.to_vec())),
                _ => {}
            }
        }
    }
    let mut by_node: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, (_, nodes)) in skeletons.iter().enumerate() {
        for n in nodes {
            by_node.entry(n.name_hash).or_default().push(i);
        }
    }

    let mut codecs: HashMap<&str, usize> = HashMap::new();
    let mut rates: HashMap<String, usize> = HashMap::new();
    let (mut material, mut errors, mut covered, mut events) = (0, 0, 0, 0);
    let mut material_tracks = 0;
    let (mut bind_checked, mut bind_matched, mut non_finite) = (0usize, 0usize, 0);
    let mut worst_quat = 0.0f32;
    for (hash, data) in &clips {
        let clip = match Clip::parse(data) {
            Ok(c) => c,
            Err(AnimationError::MaterialAnimation) => {
                match MaterialAnimation::parse(data) {
                    Ok(m) => {
                        material += 1;
                        material_tracks +=
                            m.materials.iter().map(|m| m.tracks.len()).sum::<usize>();
                    }
                    Err(e) => {
                        errors += 1;
                        eprintln!("material animation {hash:08x}: {e}");
                    }
                }
                continue;
            }
            Err(e) => {
                errors += 1;
                if errors <= 20 {
                    eprintln!("clip {hash:08x}: {e}");
                }
                continue;
            }
        };
        let name = match clip.header.codec {
            Codec::Interleaved => "interleaved",
            Codec::Delta => "delta",
            Codec::Wavelet => "wavelet",
        };
        *codecs.entry(name).or_default() += 1;
        let rate = clip
            .frame_rate()
            .map_or("-".to_owned(), |r| format!("{r:.2}"));
        *rates.entry(rate).or_default() += 1;
        events += clip.events.len();
        for f in 0..clip.frame_count {
            for t in clip.frame(f).unwrap_or_default() {
                worst_quat = worst_quat.max((t.rotation.length() - 1.0).abs());
            }
        }

        // The first model whose nodes include every track.
        let model = clip.track_nodes.first().and_then(|first| {
            by_node.get(first)?.iter().copied().find(|&m| {
                let nodes = &skeletons[m].1;
                clip.track_nodes
                    .iter()
                    .all(|h| nodes.iter().any(|n| n.name_hash == *h))
            })
        });
        let Some(model) = model else { continue };
        covered += 1;
        let nodes = &skeletons[model].1;
        let binding = clip.bind(nodes);
        if let Some(first) = clip.frame(0) {
            for (node, track) in nodes.iter().zip(binding.node_tracks()) {
                let Some(t) = track.and_then(|t| first.get(t)) else {
                    continue;
                };
                bind_checked += 1;
                let row = node.local.m[3];
                let p = t.translation;
                if (row[0] - p.x).abs() < 1e-3
                    && (row[1] - p.y).abs() < 1e-3
                    && (row[2] - p.z).abs() < 1e-3
                {
                    bind_matched += 1;
                }
            }
        }
        let d = clip.header.duration;
        for time in [0.0, d * 0.37, d] {
            let world = binding.model_matrices(&clip.sample(time));
            if world
                .iter()
                .flat_map(|m| m.m.iter().flatten())
                .any(|v| !v.is_finite())
            {
                non_finite += 1;
            }
        }
    }
    let mut codecs: Vec<_> = codecs.into_iter().collect();
    codecs.sort();
    let mut rates: Vec<_> = rates.into_iter().collect();
    rates.sort();
    println!(
        "{} animation assets: {} clips decoded {codecs:?}, {material} material animations ({material_tracks} parameter tracks), {errors} errors",
        clips.len(),
        clips.len() - material - errors
    );
    println!("frame rates {rates:?}");
    println!("{events} events; largest |quaternion length - 1| {worst_quat:.5}");
    println!(
        "{covered} clips covered by one of {} models",
        skeletons.len()
    );
    println!(
        "frame 0 translation matches the HIER local on {bind_matched} of {bind_checked} bound tracks"
    );
    println!("{non_finite} sampled poses with non-finite matrices");
    Ok(())
}

/// Counts from parsing every model's `PHY2` section.
#[derive(Default)]
struct PhysicsTally {
    models: usize,
    with: usize,
    kinds: std::collections::BTreeMap<&'static str, usize>,
    triangles: usize,
    bodies: usize,
    fixed: usize,
    bad_node: usize,
    errors: usize,
}

impl PhysicsTally {
    fn add(&mut self, hash: u32, data: &[u8]) {
        use m2_assets::physics::Geometry;
        self.models += 1;
        let Some(phy2) = Ucfx::parse(data).ok().and_then(|u| u.top_level(*b"PHY2")) else {
            return;
        };
        self.with += 1;
        let parsed = Model::parse(data).map_err(|e| e.to_string()).and_then(|m| {
            Ok((
                m,
                m2_assets::Physics::parse(phy2).map_err(|e| e.to_string())?,
            ))
        });
        let (model, p) = match parsed {
            Ok(x) => x,
            Err(e) => {
                self.errors += 1;
                eprintln!("collision {hash:08x}: {e}");
                return;
            }
        };
        self.bodies += p.bodies.len();
        self.fixed += p.bodies.iter().filter(|b| b.is_fixed()).count();
        for (k, s) in p.shapes.iter().enumerate() {
            let kind = match s.geometry {
                Geometry::Box { .. } => "box",
                Geometry::Sphere => "sphere",
                Geometry::Capsule { .. } => "capsule",
                Geometry::Convex { .. } => "convex",
                Geometry::Mesh { .. } => "mesh",
            };
            *self.kinds.entry(kind).or_default() += 1;
            self.triangles += s.geometry.triangles(s.radius).1.len();
            if p.shape_node(k)
                .is_none_or(|n| n as usize >= model.nodes.len())
            {
                self.bad_node += 1;
            }
        }
    }
}

fn world<R: Read + Seek>(wad: &mut WadArchive<R>) -> AnyResult<()> {
    use m2_assets::layer::LAYER_TYPE;
    use m2_assets::terrain::{CELL_SIZE, LOW_RES_TYPE, TERRAIN_TYPE};
    use m2_assets::{Layer, LowResTile, TerrainCell};
    let (mut cells, mut tiles, mut layers) = (HashMap::new(), HashMap::new(), Vec::new());
    let mut scatter: Vec<Vec<u8>> = Vec::new();
    let mut phys = PhysicsTally::default();
    // Sound banks: (parsed, sounds, seconds, other layout).
    let mut banks = (0usize, 0usize, 0f64, 0usize);
    for block in 0..wad.index().blocks().len() {
        let decoded = wad.read_block(block)?;
        for (e, data) in BlockContents::parse(&decoded)?.iter() {
            match e.type_hash {
                MODEL_TYPE if e.unk_0x08 == 0 => phys.add(e.name_hash, data),
                m2_assets::sound::SOUND_BANK_TYPE => match m2_assets::SoundBank::parse(data) {
                    Ok(b) => {
                        banks.0 += 1;
                        banks.1 += b.sounds.len();
                        banks.2 += b.sounds.iter().map(|s| f64::from(s.seconds())).sum::<f64>();
                    }
                    // The music bank and one other use a streaming layout.
                    Err(_) => banks.3 += 1,
                },
                TERRAIN_TYPE => {
                    cells.insert(e.name_hash, data.to_vec());
                }
                LOW_RES_TYPE => {
                    tiles.insert(e.name_hash, data.to_vec());
                }
                LAYER_TYPE => layers.push((e.name_hash, data.to_vec())),
                m2_assets::scatter::SCATTER_TYPE => scatter.push(data.to_vec()),
                _ => {}
            }
        }
    }
    let mut errors = phys.errors;
    println!(
        "{} sound banks: {} sounds, {:.0} minutes of 16-bit PCM; {} in another layout (streamed)",
        banks.0,
        banks.1,
        banks.2 / 60.0,
        banks.3
    );
    println!(
        "{} models, {} with collision: {} shapes ({:?}), {} triangles; {} bodies ({} fixed); {} shapes on a node the model lacks",
        phys.models,
        phys.with,
        phys.kinds.values().sum::<usize>(),
        phys.kinds,
        phys.triangles,
        phys.bodies,
        phys.fixed,
        phys.bad_node
    );
    let (mut groups, mut ranges, mut triangles, mut materials, mut bad_index) = (0, 0, 0, 0, 0);
    for (hash, data) in &cells {
        match TerrainCell::parse(data) {
            Ok(c) => {
                materials += c.materials.len();
                for g in c.patches.iter().flat_map(|p| &p.groups) {
                    groups += 1;
                    for r in &g.material_ranges {
                        ranges += 1;
                        triangles += g.triangles(r).len();
                        bad_index += usize::from(r.material as usize >= c.materials.len());
                    }
                }
            }
            Err(e) => {
                errors += 1;
                eprintln!("cell {hash:08x}: {e}");
            }
        }
    }
    for (hash, data) in &tiles {
        if let Err(e) = LowResTile::parse(data) {
            errors += 1;
            eprintln!("tile {hash:08x}: {e}");
        }
    }
    let (mut meshes, mut instances) = (0, 0);
    for data in &scatter {
        match m2_assets::Scatter::parse(data) {
            Ok(s) => {
                meshes += s.meshes.len();
                instances += s.placements().count();
            }
            Err(e) => {
                errors += 1;
                eprintln!("scatter: {e}");
            }
        }
    }
    println!(
        "{} scatter assets: {meshes} clump meshes, {instances} instances",
        scatter.len()
    );
    println!(
        "{} terrain cells ({groups} groups, {ranges} material ranges, {triangles} triangles, {materials} materials, {bad_index} ranges past their materials), {} low-res tiles",
        cells.len(),
        tiles.len()
    );

    let mut comps: HashMap<String, (usize, usize, bool)> = HashMap::new();
    let (mut entities, mut placed, mut placed_cells, mut on_grid, mut placed_tiles, mut models) =
        (0, 0, 0, 0, 0, 0);
    for (hash, data) in &layers {
        let layer = match Layer::parse(data) {
            Ok(l) => l,
            Err(e) => {
                errors += 1;
                eprintln!("layer {hash:08x}: {e}");
                continue;
            }
        };
        for c in &layer.components {
            let fixed = c.records().is_some() || c.name == "Name" || c.name == "PointLocation";
            let e = comps.entry(c.name.to_owned()).or_insert((0, 0, true));
            e.0 += 1;
            e.1 += c.count as usize;
            e.2 &= fixed;
        }
        let placements = layer.placements();
        entities += layer.names().len();
        placed += placements.len();
        models += layer.hashes("ModelName").len();
        for (entity, cell) in layer.hashes("TerrainObject") {
            placed_cells += usize::from(cells.contains_key(&cell));
            if let Some(p) = placements.get(&entity) {
                let grid = |v: f32| ((v + CELL_SIZE / 2.0) / CELL_SIZE).fract().abs() < 1e-4;
                on_grid += usize::from(grid(p.position[0]) && grid(p.position[2]));
            }
        }
        for (_, tile) in layer.hashes("LowResTerrainObject") {
            placed_tiles += usize::from(tiles.contains_key(&tile));
        }
    }
    println!(
        "{} layers: {entities} named entities, {placed} placements, {models} with a model; \
         layout places {placed_cells} cells ({on_grid} centred on the 400 m grid) and {placed_tiles} low-res tiles",
        layers.len()
    );
    let mut comps: Vec<_> = comps.into_iter().collect();
    comps.sort_by_key(|c| std::cmp::Reverse(c.1.1));
    let irregular: Vec<&str> = comps
        .iter()
        .filter(|c| !c.1.2)
        .map(|c| c.0.as_str())
        .collect();
    println!(
        "{} component types; irregular record sizes: {irregular:?}",
        comps.len()
    );
    for (name, (layers, count, _)) in comps.iter() {
        println!("  {name:24} {count:7} records in {layers} layers");
    }
    eprintln!("{errors} errors");
    Ok(())
}

/// Per type: count, bytes, sample names and paths, layout and strings.
fn string_tables<R: Read + Seek>(
    wad: &mut WadArchive<R>,
    text: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let short = |s: &str| -> String {
        let s: String = s.chars().take(60).collect();
        format!("{s:?}")
    };
    for block in 0..wad.index().blocks().len() {
        let path = wad.index().blocks()[block].path.clone();
        let Ok(decoded) = wad.read_block(block) else {
            continue;
        };
        let Ok(contents) = BlockContents::parse(&decoded) else {
            continue;
        };
        for (entry, data) in contents.iter() {
            if entry.type_hash != STRINGS_TYPE {
                continue;
            }
            let table = StringTable::parse(data)?;
            // The keys in stored order, by where their text is.
            let ucfx = Ucfx::parse(data)?;
            let keys = ucfx.top_level(*b"KEYS").unwrap_or_default();
            let mut order: Vec<(u32, u32)> = keys
                .get(4..)
                .unwrap_or_default()
                .chunks_exact(8)
                .map(|c| {
                    let k = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                    let o = u32::from_le_bytes([c[4], c[5], c[6], c[7]]);
                    (k, o)
                })
                .collect();
            let sorted = order.is_sorted_by_key(|e| e.0);
            let by_text = order.is_sorted_by_key(|e| e.1);
            println!(
                "{path} asset {:08x}: {} strings, keys sorted {sorted}, texts in key order {by_text}",
                entry.name_hash,
                table.strings.len()
            );
            let Some(text) = text else { continue };
            order.sort_by_key(|e| e.1);
            let id = m2_assets::name_hash(text);
            let exact: Vec<usize> = (0..order.len())
                .filter(|&i| order[i].0 == id || table.get(order[i].0) == Some(text))
                .collect();
            let hits = if exact.is_empty() {
                (0..order.len())
                    .filter(|&i| table.get(order[i].0).is_some_and(|s| s.contains(text)))
                    .take(10)
                    .collect()
            } else {
                exact
            };
            for i in hits {
                println!("  match:");
                let from = i.saturating_sub(4);
                for (j, &(k, o)) in order.iter().enumerate().skip(from).take(i + 5 - from) {
                    let mark = if j == i { '>' } else { ' ' };
                    println!(
                        "  {mark} {k:08x} at {o:#8x} {}",
                        short(table.get(k).unwrap_or_default())
                    );
                }
            }
        }
    }
    Ok(())
}

fn types<R: std::io::Read + std::io::Seek>(
    wad: &mut WadArchive<R>,
) -> Result<(), Box<dyn std::error::Error>> {
    #[derive(Default)]
    struct Seen {
        count: usize,
        bytes: usize,
        largest: usize,
        names: Vec<String>,
        paths: Vec<String>,
        lua: usize,
        layout: String,
        strings: Vec<String>,
    }
    let paths: Vec<String> = wad
        .index()
        .blocks()
        .iter()
        .map(|b| b.path.clone())
        .collect();
    let mut seen: std::collections::BTreeMap<u32, Seen> = Default::default();
    for (block, path) in paths.iter().enumerate() {
        let decoded = match wad.read_block(block) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("block {block}: {e}");
                continue;
            }
        };
        let Ok(contents) = BlockContents::parse(&decoded) else {
            continue;
        };
        for (entry, data) in contents.iter() {
            let s = seen.entry(entry.type_hash).or_default();
            s.count += 1;
            s.bytes += data.len();
            if data.windows(4).any(|w| w == b"\x1bLua") {
                s.lua += 1;
            }
            let ucfx = Ucfx::parse(data).ok();
            if let Some(n) = ucfx.as_ref().and_then(|u| u.name())
                && s.names.len() < 6
            {
                s.names.push(n.to_owned());
            }
            if s.paths.len() < 3 && !s.paths.contains(path) {
                s.paths.push(path.clone());
            }
            // The largest instance stands for the type.
            if data.len() > s.largest {
                s.largest = data.len();
                s.layout = match &ucfx {
                    Some(u) => u
                        .sections()
                        .iter()
                        .take(24)
                        .map(|x| {
                            format!(
                                "{}{}{}",
                                "  ".repeat(x.depth),
                                String::from_utf8_lossy(&x.fourcc),
                                x.data.map_or(String::new(), |d| format!(" {}", d.len()))
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(" |"),
                    None => format!("not UCFX: {:02x?}", &data[..data.len().min(16)]),
                };
                s.strings = readable(data, 6);
            }
        }
    }
    let known = wad.index().type_hashes().to_vec();
    for (ty, s) in &seen {
        println!(
            "type {ty:08x} (#{}): {} assets, {} bytes (largest {}), lua {}",
            known
                .iter()
                .position(|t| t == ty)
                .map_or("?".into(), |i| i.to_string()),
            s.count,
            s.bytes,
            s.largest,
            s.lua
        );
        println!("  names: {}", s.names.join(", "));
        println!("  blocks: {}", s.paths.join(", "));
        println!("  layout: {}", s.layout);
        println!("  strings: {}", s.strings.join(" | "));
    }
    Ok(())
}

/// Up to 12 of the longest runs of printable ASCII at least `min` long,
/// each cut to 60 characters.
fn readable(data: &[u8], min: usize) -> Vec<String> {
    let mut runs: Vec<&[u8]> = data
        .split(|b| !(0x20..0x7f).contains(b))
        .filter(|r| r.len() >= min)
        .collect();
    runs.sort_by_key(|r| std::cmp::Reverse(r.len()));
    runs.dedup();
    runs.iter()
        .take(12)
        .map(|r| String::from_utf8_lossy(&r[..r.len().min(60)]).into_owned())
        .collect()
}

fn scripts<R: std::io::Read + std::io::Seek>(
    wad: &mut WadArchive<R>,
) -> Result<(), Box<dyn std::error::Error>> {
    use m2_assets::lua::{Chunk, NameUse, SCRIPT_TYPE};
    use std::collections::{BTreeMap, BTreeSet};

    let paths: Vec<String> = wad
        .index()
        .blocks()
        .iter()
        .map(|b| b.path.clone())
        .collect();
    // Per use, per name: (uses, scripts using it).
    let mut totals: BTreeMap<NameUse, BTreeMap<String, (usize, usize)>> = BTreeMap::new();
    let mut versions: BTreeMap<String, usize> = BTreeMap::new();
    let (mut parsed, mut failed) = (0, 0);
    // Module loads: (callee, name) -> (calls, scripts).
    let mut loads: BTreeMap<(String, String), (usize, usize)> = BTreeMap::new();
    // Name-lookup calls with constant names: callee -> name -> calls.
    let mut lookups: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    // Every script's name and the globals it defines.
    let mut defined: Vec<(String, BTreeSet<String>)> = Vec::new();
    for (block, path) in paths.iter().enumerate() {
        let decoded = match wad.read_block(block) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("block {block}: {e}");
                continue;
            }
        };
        let Ok(contents) = BlockContents::parse(&decoded) else {
            continue;
        };
        for (entry, data) in contents.iter() {
            if entry.type_hash != SCRIPT_TYPE {
                continue;
            }
            let name = Ucfx::parse(data)
                .ok()
                .and_then(|u| u.name().map(str::to_owned))
                .unwrap_or_else(|| format!("{:08x}", entry.name_hash));
            let chunk = match Chunk::from_asset(data) {
                Ok(c) => c,
                Err(e) => {
                    failed += 1;
                    let at = data.windows(4).position(|w| w == b"\x1bLua");
                    let head = at.map(|a| &data[a..data.len().min(a + 24)]);
                    println!(
                        "FAILED {name} [{path}] {} bytes: {e}; header {head:02x?}",
                        data.len()
                    );
                    continue;
                }
            };
            parsed += 1;
            *versions
                .entry(format!(
                    "{:?} {}",
                    chunk.version,
                    if chunk.big_endian {
                        "big-endian"
                    } else {
                        "little-endian"
                    }
                ))
                .or_default() += 1;
            let names = chunk.main.names(chunk.version);
            let mut seen: BTreeSet<(NameUse, &str)> = BTreeSet::new();
            for (u, n) in &names {
                let t = totals.entry(*u).or_default().entry(n.clone()).or_default();
                t.0 += 1;
                if seen.insert((*u, n)) {
                    t.1 += 1;
                }
            }
            let mut loaded: BTreeSet<(String, String)> = BTreeSet::new();
            for call in chunk.main.calls() {
                let Some(arg) = call.first else { continue };
                if matches!(
                    call.callee.as_str(),
                    "import" | "inherit" | "dynamic_import" | "dynamic_remove"
                ) {
                    let key = (call.callee, arg);
                    let t = loads.entry(key.clone()).or_default();
                    t.0 += 1;
                    if loaded.insert(key) {
                        t.1 += 1;
                    }
                } else if call.callee.contains('.') {
                    *lookups
                        .entry(call.callee)
                        .or_default()
                        .entry(arg)
                        .or_default() += 1;
                }
            }
            defined.push((
                name.clone(),
                seen.iter()
                    .filter(|(u, _)| *u == NameUse::SetGlobal)
                    .map(|(_, n)| (*n).to_owned())
                    .collect(),
            ));
            let defines: Vec<&str> = seen
                .iter()
                .filter(|(u, _)| *u == NameUse::SetGlobal)
                .map(|(_, n)| *n)
                .take(12)
                .collect();
            println!(
                "script {name} [{path}] {} bytes, {} functions, {} instructions; defines {}",
                data.len(),
                chunk.main.all().len(),
                chunk.main.all().iter().map(|f| f.code.len()).sum::<usize>(),
                defines.join(" "),
            );
        }
    }
    println!();
    println!("{parsed} scripts read, {failed} failed");
    for (v, n) in &versions {
        println!("  {v}: {n}");
    }
    for (u, names) in &totals {
        let mut sorted: Vec<_> = names.iter().collect();
        sorted.sort_by(|a, b| b.1.1.cmp(&a.1.1).then(b.1.0.cmp(&a.1.0)).then(a.0.cmp(b.0)));
        println!();
        println!("{u:?}: {} names (uses, scripts)", sorted.len());
        for (n, (uses, scripts)) in sorted {
            println!("  {n} {uses} {scripts}");
        }
    }

    // Which script is each module? The one defining most of the members
    // other scripts use on it.
    let members = totals.get(&NameUse::Member).cloned().unwrap_or_default();
    println!();
    println!(
        "Module loads: callee name calls scripts -> best script (members it defines / members used)"
    );
    for ((callee, module), (calls, scripts)) in &loads {
        let short = module.rsplit(['/', '\\', '.']).next().unwrap_or(module);
        let used: Vec<&str> = members
            .keys()
            .filter_map(|m| m.strip_prefix(short)?.strip_prefix('.'))
            .collect();
        let best = defined
            .iter()
            .map(|(n, d)| (used.iter().filter(|u| d.contains(**u)).count(), n))
            .max();
        let guess = match best {
            Some((hits, n)) if hits > 0 => format!("{n} ({hits}/{})", used.len()),
            _ => format!("? (0/{})", used.len()),
        };
        println!("  {callee} {module:?} {calls} {scripts} -> {guess}");
    }

    println!();
    println!("Calls with a constant first argument (callee: argument count)");
    for (callee, args) in &lookups {
        let total: usize = args.values().sum();
        if total < 5 {
            continue;
        }
        let mut sorted: Vec<_> = args.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let shown: Vec<String> = sorted
            .iter()
            .take(12)
            .map(|(a, n)| format!("{a:?} {n}"))
            .collect();
        println!(
            "  {callee} ({} names, {total} calls): {}",
            args.len(),
            shown.join(", ")
        );
    }
    Ok(())
}

fn script_listing<R: std::io::Read + std::io::Seek>(
    wad: &mut WadArchive<R>,
    hash: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    use m2_assets::lua::{Chunk, SCRIPT_TYPE};
    for block in 0..wad.index().blocks().len() {
        let decoded = wad.read_block(block)?;
        let Ok(contents) = BlockContents::parse(&decoded) else {
            continue;
        };
        for (entry, data) in contents.iter() {
            if entry.type_hash == SCRIPT_TYPE && entry.name_hash == hash {
                let chunk = Chunk::from_asset(data)?;
                println!("{:?}, {} bytes", chunk.version, data.len());
                print!("{}", chunk.main.listing());
                return Ok(());
            }
        }
    }
    Err(format!("no script {hash:08x}").into())
}

fn call_sites<R: std::io::Read + std::io::Seek>(
    wad: &mut WadArchive<R>,
    callee: &str,
    first: Option<&str>,
    count: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    use m2_assets::lua::{Arg, Chunk, Function, SCRIPT_TYPE};

    /// The function a script binds to global `name` at its top level
    /// (`CLOSURE Rn Fk` then `SETGLOBAL Rn "name"`).
    fn bound<'f>(main: &'f Function, name: &str) -> Option<&'f Function> {
        let mut closure = None;
        for &word in &main.code {
            let op = word & 0x3f;
            if op == 36 {
                closure = Some((word >> 14) as usize);
            } else if op == 7 {
                let k = (word >> 14) as usize;
                if matches!(main.constants.get(k), Some(m2_assets::lua::Constant::String(s)) if s == name)
                {
                    return closure.and_then(|c| main.functions.get(c));
                }
                closure = None;
            }
        }
        None
    }

    let mut shown = 0;
    let mut scripts = std::collections::BTreeSet::new();
    'blocks: for block in 0..wad.index().blocks().len() {
        let decoded = wad.read_block(block)?;
        let Ok(contents) = BlockContents::parse(&decoded) else {
            continue;
        };
        for (entry, data) in contents.iter() {
            if entry.type_hash != SCRIPT_TYPE || scripts.contains(&entry.name_hash) {
                continue;
            }
            let Ok(chunk) = Chunk::from_asset(data) else {
                continue;
            };
            let all = chunk.main.all();
            for call in chunk.main.calls() {
                if call.callee != callee {
                    continue;
                }
                let matches_first = match (first, call.args.as_ref().and_then(|a| a.first())) {
                    (None, _) => true,
                    (Some(want), Some(Arg::Global(g))) => g == want,
                    (Some(want), Some(Arg::Str(s))) => s == want,
                    _ => false,
                };
                if !matches_first {
                    continue;
                }
                let (index, start, end) = call.site;
                let f = all[index];
                println!(
                    "== script {:08x}, function {index} (line {}), instructions {start}..={end}",
                    entry.name_hash, f.line_defined
                );
                // A few instructions before the callee load show where
                // the arguments came from.
                for pc in start.saturating_sub(4)..=end {
                    println!("  {}", f.instruction_text(pc));
                }
                for a in call.args.iter().flatten() {
                    if let Arg::Global(name) = a
                        && let Some(cb) = bound(&chunk.main, name)
                    {
                        println!(
                            "  -- callback {name}: {} params{}",
                            cb.param_count,
                            if cb.is_vararg != 0 { ", vararg" } else { "" }
                        );
                        for pc in 0..cb.code.len().min(16) {
                            println!("    {}", cb.instruction_text(pc));
                        }
                    }
                }
                scripts.insert(entry.name_hash);
                shown += 1;
                if shown >= count {
                    break 'blocks;
                }
                // One site per script, for variety.
                break;
            }
        }
    }
    println!("{shown} call sites shown");
    Ok(())
}

/// Every layer entity's template, which assets its name finds, and which
/// components its entities carry: `entity name 0x001384eb` or
/// `_entity name 26` stems to `entity name`, whose `name_hash` is looked
/// up in every type of the archive.
fn entities<R: std::io::Read + std::io::Seek>(
    wad: &mut WadArchive<R>,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::collections::{BTreeMap, BTreeSet};

    use m2_assets::Layer;
    use m2_assets::layer::{LAYER_TYPE, TEMPLATE_DB_TYPE};

    #[derive(Default)]
    struct Template {
        entities: usize,
        layers: BTreeSet<String>,
        components: BTreeSet<String>,
        /// Type indices of the assets the stem's hash finds.
        types: BTreeSet<usize>,
        /// The template database's model for the name, and the
        /// database's components of it.
        db_model: Option<u32>,
        db_components: Option<BTreeSet<String>>,
    }

    let types = wad.index().type_hashes().to_vec();
    let paths: Vec<String> = wad
        .index()
        .blocks()
        .iter()
        .map(|b| b.path.clone())
        .collect();
    let mut assets: HashMap<u32, BTreeSet<usize>> = HashMap::new();
    let mut layers: Vec<(String, Vec<u8>)> = Vec::new();
    let mut template_db: Option<Vec<u8>> = None;
    for (block, path) in paths.iter().enumerate() {
        let decoded = wad.read_block(block)?;
        for (e, data) in BlockContents::parse(&decoded)?.iter() {
            if let Some(t) = types.iter().position(|&t| t == e.type_hash) {
                assets.entry(e.name_hash).or_default().insert(t);
            }
            if e.type_hash == LAYER_TYPE {
                layers.push((path.clone(), data.to_vec()));
            }
            if e.type_hash == TEMPLATE_DB_TYPE {
                template_db = Some(data.to_vec());
            }
        }
    }

    // The template database: template name to its id, model and
    // components.
    let mut db_models: HashMap<String, u32> = HashMap::new();
    let mut db_components: HashMap<String, BTreeSet<String>> = HashMap::new();
    if let Some(data) = &template_db {
        match Layer::parse(data) {
            Ok(db) => {
                println!(
                    "template database: {} components, enum {} bytes, UNIQ {} bytes, {} names",
                    db.components.len(),
                    db.enums.len(),
                    db.uniq.len(),
                    db.names().len()
                );
                let uniq: Vec<String> = db
                    .uniq
                    .iter()
                    .take(64)
                    .map(|b| format!("{b:02x}"))
                    .collect();
                println!("  UNIQ starts {}", uniq.join(" "));
                let mut by_id: HashMap<u32, BTreeSet<String>> = HashMap::new();
                for c in &db.components {
                    let records = c.records();
                    println!(
                        "  {:28} {:6} records, {} fields, record {} bytes, data {} bytes{}",
                        c.name,
                        c.count,
                        c.fields.len(),
                        c.record_size,
                        c.data.len(),
                        if records.is_some() {
                            ""
                        } else {
                            " (irregular)"
                        }
                    );
                    for (id, _) in records.unwrap_or_default() {
                        by_id.entry(id).or_default().insert(c.name.to_owned());
                    }
                }
                let models = db.hashes("ModelName");
                for (id, name) in db.names() {
                    if let Some(&m) = models.get(&id) {
                        db_models.insert(name.to_owned(), m);
                    }
                    db_components.insert(name.to_owned(), by_id.remove(&id).unwrap_or_default());
                }
                println!(
                    "  {} templates have a ModelName, {} of them a model the archive holds",
                    db_models.len(),
                    db_models
                        .values()
                        .filter(|m| assets.get(m).is_some_and(|t| t.contains(&19)))
                        .count()
                );
            }
            Err(e) => eprintln!("template database: {e}"),
        }
    }

    let stem = |name: &str| -> String {
        let name = name.trim_start_matches('_');
        name.rsplit_once(' ').map_or(name, |(s, _)| s).to_owned()
    };
    let mut templates: BTreeMap<String, Template> = BTreeMap::new();
    // Per component: entities, and how many of them stem to an asset.
    let mut by_component: BTreeMap<String, (usize, usize, BTreeSet<usize>)> = BTreeMap::new();
    let mut errors = 0;
    for (path, data) in &layers {
        let layer = match Layer::parse(data) {
            Ok(l) => l,
            Err(e) => {
                errors += 1;
                eprintln!("layer in {path}: {e}");
                continue;
            }
        };
        let mut components: HashMap<u32, BTreeSet<String>> = HashMap::new();
        for c in &layer.components {
            for (entity, _) in c.records().unwrap_or_default() {
                components
                    .entry(entity)
                    .or_default()
                    .insert(c.name.to_owned());
            }
        }
        for (entity, name) in layer.names() {
            let stem = stem(name);
            let found = assets.get(&m2_assets::name_hash(&stem));
            let comps = components.remove(&entity).unwrap_or_default();
            for c in &comps {
                let e = by_component.entry(c.clone()).or_default();
                e.0 += 1;
                if let Some(f) = found {
                    e.1 += 1;
                    e.2.extend(f.iter().copied());
                }
            }
            let db_model = db_models.get(&stem).copied();
            let db_comps = db_components.get(&stem).cloned();
            let t = templates.entry(stem).or_default();
            t.entities += 1;
            t.layers.insert(path.clone());
            t.components.extend(comps);
            if let Some(f) = found {
                t.types.extend(f.iter().copied());
            }
            t.db_model = db_model;
            t.db_components = db_comps;
        }
    }
    println!("components: entities, of which the name finds an asset (of these types)");
    for (name, (entities, found, types)) in &by_component {
        println!("  {name:28} {entities:7} {found:7} {types:?}");
    }
    let mut list: Vec<(&String, &Template)> = templates.iter().collect();
    list.sort_by_key(|(_, t)| std::cmp::Reverse(t.entities));
    println!(
        "{} templates over {} entities ({} in the template database, {} with a model there, {} with a model by name): count, asset types found, db model, name, components, db components, layers",
        list.len(),
        list.iter().map(|(_, t)| t.entities).sum::<usize>(),
        list.iter()
            .filter(|(_, t)| t.db_components.is_some())
            .count(),
        list.iter().filter(|(_, t)| t.db_model.is_some()).count(),
        list.iter().filter(|(_, t)| t.types.contains(&19)).count(),
    );
    for (name, t) in list {
        let types: Vec<String> = t.types.iter().map(|t| t.to_string()).collect();
        let comps: Vec<&str> = t.components.iter().map(String::as_str).collect();
        let layers: Vec<&str> = t.layers.iter().map(String::as_str).collect();
        let db_comps: Vec<&str> = t
            .db_components
            .iter()
            .flatten()
            .map(String::as_str)
            .collect();
        println!(
            "{:6} [{}] {} {name} {comps:?} db{db_comps:?} {}",
            t.entities,
            types.join(","),
            t.db_model.map_or("-".to_owned(), |m| format!("{m:08x}")),
            layers.join(",")
        );
    }
    eprintln!("{errors} errors");
    Ok(())
}

/// Every offset at which `needle` occurs in `hay`.
fn memchr_all<'a>(needle: &'a [u8], hay: &'a [u8]) -> impl Iterator<Item = usize> + 'a {
    hay.windows(needle.len().max(1))
        .enumerate()
        .filter(move |(_, w)| !needle.is_empty() && *w == needle)
        .map(|(i, _)| i)
}

/// The decoded blocks holding assets of `ty`, with those assets' entries.
fn blocks_with_type<R: Read + Seek>(wad: &mut WadArchive<R>, ty: u32) -> Vec<usize> {
    let index = wad.index();
    let mut blocks: Vec<usize> = index
        .assets()
        .iter()
        .filter(|a| index.type_hash(a) == ty)
        .map(|a| usize::from(a.block))
        .collect();
    blocks.sort_unstable();
    blocks.dedup();
    blocks
}

/// A texture's header, found by its asset hash.
fn texture_info<R: Read + Seek>(wad: &mut WadArchive<R>, hash: u32) -> Option<TextureInfo> {
    let blocks: Vec<usize> = wad
        .index()
        .assets_by_hash(hash)
        .map(|a| usize::from(a.block))
        .collect();
    for b in blocks {
        let decoded = wad.read_block(b).ok()?;
        let contents = BlockContents::parse(&decoded).ok()?;
        if let Some(e) = contents.find(hash, TEXTURE_TYPE)
            && let Ok(t) = Texture::parse(contents.asset_data(e))
        {
            return Some(t.info);
        }
    }
    None
}

/// Lists a movie's timelines (the root's and each sprite's): frame
/// labels, exported names, placed clips' names and every action, one per
/// line, with pushed values (constants resolved). For reading what a
/// movie's scripts wait for; the output is the game's own: keep it local.
fn actions<R: Read + Seek>(wad: &mut WadArchive<R>, name: &str) -> AnyResult<()> {
    let hash = m2_assets::name_hash(name);
    let mut found = None;
    for block in blocks_with_type(wad, MOVIE_TYPE) {
        let decoded = wad.read_block(block)?;
        let contents = BlockContents::parse(&decoded)?;
        for (e, data) in contents.iter() {
            if e.type_hash == MOVIE_TYPE && e.name_hash == hash {
                found = Some(movie_from_asset(data)?);
            }
        }
    }
    let movie = found.ok_or_else(|| format!("no movie {name} ({hash:08x})"))?;
    let root: Vec<(u16, &[u8])> = movie
        .tags
        .iter()
        .map(|t| (t.code, &movie.body[t.offset..t.offset + t.len]))
        .collect();
    timeline_actions("root", &root);
    // Blend modes, where set.
    let lib = m2_assets::flash::Library::new(&movie);
    let names: HashMap<u16, String> = lib.exports.iter().map(|(n, &id)| (id, n.clone())).collect();
    let mut timelines: Vec<(String, &m2_assets::flash::player::Timeline)> =
        vec![("root".into(), &lib.root)];
    for (id, t) in &lib.sprites {
        timelines.push((format!("sprite {id}"), t));
    }
    for (who, t) in timelines {
        for (f, controls) in t.frames.iter().enumerate() {
            for c in controls {
                if let m2_assets::flash::player::Control::Place(p) = c
                    && let Some(blend) = p.blend.filter(|&b| b > 1)
                {
                    let ch = p.character;
                    println!(
                        "{who} frame {}: blend {blend} on depth {} character {ch:?} {:?} as {:?}",
                        f + 1,
                        p.depth,
                        ch.and_then(|c| names.get(&c)),
                        p.name
                    );
                }
            }
        }
    }
    Ok(())
}

fn timeline_actions(who: &str, tags: &[(u16, &[u8])]) {
    let mut frame = 1;
    for &(code, data) in tags {
        match code {
            1 => frame += 1,
            43 => println!("{who} frame {frame}: label {:?}", cstr(data)),
            56 => {
                let n = u16::from_le_bytes([data[0], data[1]]);
                let mut p = 2;
                for _ in 0..n {
                    let id = u16::from_le_bytes([data[p], data[p + 1]]);
                    let s = cstr(&data[p + 2..]);
                    p += 3 + s.len();
                    println!("{who}: export {id} as {s:?}");
                }
            }
            39 => {
                let id = u16::from_le_bytes([data[0], data[1]]);
                let inner = m2_assets::flash::tags(&data[4..]);
                timeline_actions(&format!("sprite {id}"), &inner);
            }
            26 | 70 => {
                // PlaceObject2/3: the clip's name, if any.
                let flags = data[0];
                let mut p = if code == 70 { 2 } else { 1 } + 2;
                let flags3 = if code == 70 { data[1] } else { 0 };
                if code == 70 && (flags3 & 0x08 != 0 || (flags3 & 0x10 != 0 && flags & 0x02 != 0)) {
                    p += cstr(&data[p..]).len() + 1;
                }
                let mut id = None;
                if flags & 0x02 != 0 {
                    id = Some(u16::from_le_bytes([data[p], data[p + 1]]));
                    p += 2;
                }
                // The matrix and colour transform are bit-packed: the name
                // is found only when neither is there.
                if flags & 0x20 != 0 && flags & 0x0c == 0 {
                    if flags & 0x10 != 0 {
                        p += 2;
                    }
                    println!(
                        "{who} frame {frame}: places {id:?} as {:?}",
                        cstr(&data[p..])
                    );
                } else if flags & 0x20 != 0 {
                    println!("{who} frame {frame}: places {id:?} (named)");
                }
                if flags & 0x80 != 0 {
                    println!("{who} frame {frame}: clip events on {id:?}");
                }
            }
            12 => {
                println!("{who} frame {frame}: actions");
                disassemble(data, "  ");
            }
            59 => {
                let id = u16::from_le_bytes([data[0], data[1]]);
                println!("{who}: init actions for {id}");
                disassemble(&data[2..], "  ");
            }
            _ => {}
        }
    }
}

fn cstr(data: &[u8]) -> String {
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    String::from_utf8_lossy(&data[..end]).into_owned()
}

fn disassemble(code: &[u8], indent: &str) {
    let mut pool: Vec<String> = Vec::new();
    let mut p = 0;
    while p < code.len() {
        let at = p;
        let op = code[p];
        p += 1;
        if op == 0 {
            break;
        }
        let mut body: &[u8] = &[];
        if op >= 0x80 {
            if p + 2 > code.len() {
                break;
            }
            let len = usize::from(u16::from_le_bytes([code[p], code[p + 1]]));
            p += 2;
            body = &code[p..(p + len).min(code.len())];
            p += len;
        }
        let name = match op {
            0x04 => "nextFrame",
            0x05 => "prevFrame",
            0x06 => "play",
            0x07 => "stop",
            0x0A => "add",
            0x0B => "subtract",
            0x0C => "multiply",
            0x0D => "divide",
            0x0E => "equals",
            0x0F => "less",
            0x10 => "and",
            0x11 => "or",
            0x12 => "not",
            0x17 => "pop",
            0x1C => "getVariable",
            0x1D => "setVariable",
            0x20 => "setTarget2",
            0x21 => "stringAdd",
            0x22 => "getProperty",
            0x23 => "setProperty",
            0x24 => "cloneSprite",
            0x26 => "trace",
            0x34 => "getTime",
            0x3A => "delete",
            0x3B => "delete2",
            0x3C => "defineLocal",
            0x3D => "callFunction",
            0x3E => "return",
            0x40 => "new",
            0x41 => "defineLocal2",
            0x42 => "initArray",
            0x43 => "initObject",
            0x44 => "typeof",
            0x47 => "add2",
            0x48 => "less2",
            0x49 => "equals2",
            0x4C => "pushDuplicate",
            0x4E => "getMember",
            0x4F => "setMember",
            0x50 => "increment",
            0x51 => "decrement",
            0x52 => "callMethod",
            0x53 => "newMethod",
            0x54 => "instanceOf",
            0x66 => "strictEquals",
            0x67 => "greater",
            0x87 => "storeRegister",
            0x99 => "jump",
            0x9D => "if",
            0x9F => "gotoFrame2",
            0x8B => "setTarget",
            0x94 => "with",
            _ => "",
        };
        match op {
            0x88 => {
                pool.clear();
                let mut q = 2;
                while q < body.len() {
                    let s = cstr(&body[q..]);
                    q += s.len() + 1;
                    pool.push(s);
                }
            }
            0x96 => {
                let mut vals = Vec::new();
                let mut q = 0;
                while q < body.len() {
                    let t = body[q];
                    q += 1;
                    let v = match t {
                        0 => {
                            let s = cstr(&body[q..]);
                            q += s.len() + 1;
                            format!("{s:?}")
                        }
                        1 => {
                            q += 4;
                            format!(
                                "{}",
                                f32::from_le_bytes(body[q - 4..q].try_into().unwrap_or([0; 4]))
                            )
                        }
                        2 => "null".into(),
                        3 => "undefined".into(),
                        4 => {
                            q += 1;
                            format!("r{}", body[q - 1])
                        }
                        5 => {
                            q += 1;
                            format!("{}", body[q - 1] != 0)
                        }
                        6 => {
                            q += 8;
                            let b = &body[q - 8..q];
                            let mut swapped = [0u8; 8];
                            swapped[..4].copy_from_slice(&b[4..]);
                            swapped[4..].copy_from_slice(&b[..4]);
                            format!("{}", f64::from_le_bytes(swapped))
                        }
                        7 => {
                            q += 4;
                            format!(
                                "{}",
                                i32::from_le_bytes(body[q - 4..q].try_into().unwrap_or([0; 4]))
                            )
                        }
                        8 => {
                            q += 1;
                            format!(
                                "{:?}",
                                pool.get(usize::from(body[q - 1]))
                                    .cloned()
                                    .unwrap_or_default()
                            )
                        }
                        9 => {
                            q += 2;
                            let i = usize::from(u16::from_le_bytes([body[q - 2], body[q - 1]]));
                            format!("{:?}", pool.get(i).cloned().unwrap_or_default())
                        }
                        _ => break,
                    };
                    vals.push(v);
                }
                println!("{indent}{at:5} push {}", vals.join(", "));
            }
            0x9B => {
                let fname = cstr(body);
                let mut q = fname.len() + 1;
                let n = u16::from_le_bytes([body[q], body[q + 1]]);
                q += 2;
                for _ in 0..n {
                    q += cstr(&body[q..]).len() + 1;
                }
                let size = usize::from(u16::from_le_bytes([body[q], body[q + 1]]));
                println!("{indent}{at:5} function {fname:?} to {}", p + size);
            }
            0x8E => {
                let fname = cstr(body);
                let mut q = fname.len() + 1;
                let n = u16::from_le_bytes([body[q], body[q + 1]]);
                let regs = body[q + 2];
                let flags = u16::from_le_bytes([body[q + 3], body[q + 4]]);
                q += 5;
                let mut params = Vec::new();
                for _ in 0..n {
                    let reg = body[q];
                    let pname = cstr(&body[q + 1..]);
                    q += 1 + pname.len() + 1;
                    params.push(format!("r{reg}={pname}"));
                }
                let size = usize::from(u16::from_le_bytes([body[q], body[q + 1]]));
                println!(
                    "{indent}{at:5} function2 {fname:?}({}) regs {regs} flags {flags:#06x} to {}",
                    params.join(", "),
                    p + size
                );
            }
            0x8C => println!("{indent}{at:5} gotoLabel {:?}", cstr(body)),
            0x81 => println!(
                "{indent}gotoFrame {}",
                u16::from_le_bytes([body[0], body[1]]) + 1
            ),
            0x83 => println!("{indent}{at:5} getURL {:?}", cstr(body)),
            0x9A => println!("{indent}{at:5} getURL2"),
            0x99 | 0x9D => {
                let off = i16::from_le_bytes([body[0], body[1]]) as i64;
                println!(
                    "{indent}{at:5} {} -> {}",
                    if op == 0x99 { "jump" } else { "if" },
                    p as i64 + off
                );
            }
            _ if !name.is_empty() => println!("{indent}{at:5} {name}"),
            _ => println!("{indent}{at:5} op {op:02x}"),
        }
    }
}

fn movies<R: Read + Seek>(wad: &mut WadArchive<R>) -> AnyResult<()> {
    let (mut count, mut failed, mut tag_errors, mut images, mut missing) = (0, 0, 0, 0, 0);
    for block in blocks_with_type(wad, MOVIE_TYPE) {
        let path = wad.index().blocks()[block].path.clone();
        let decoded = wad.read_block(block)?;
        let contents = BlockContents::parse(&decoded)?;
        for (e, data) in contents.iter() {
            if e.type_hash != MOVIE_TYPE {
                continue;
            }
            count += 1;
            let movie = match movie_from_asset(data) {
                Ok(m) => m,
                Err(err) => {
                    println!("{:08x} {path}: {err}", e.name_hash);
                    failed += 1;
                    continue;
                }
            };
            let s = movie.summary();
            let name = s.exporter.as_ref().map_or("-", |x| x.4.as_str());
            let (w, h) = movie.frame_size.size_px();
            let tags: u32 = s.tag_counts.values().sum();
            println!(
                "{:08x} {name:<24} {:>8} bytes {w:>5}x{h:<4} {:>4} fps {:>5} frames {tags:>6} tags {:>4} chars {} errors  {}",
                e.name_hash,
                movie.body.len(),
                movie.frame_rate,
                movie.frame_count,
                s.characters.len(),
                s.errors.len(),
                path.rsplit('\\').next().unwrap_or(&path),
            );
            tag_errors += s.errors.len();
            for (timeline, code, offset) in &s.errors {
                println!(
                    "    tag {code} ({}) at {offset:#x} in timeline {timeline} did not parse",
                    m2_assets::gfx::tag::name(*code)
                );
            }
            for c in s.characters.values() {
                let Character::ExternalImage(img) = c else {
                    continue;
                };
                images += 1;
                let key = if img.export_name.is_empty() {
                    let f = img.file_name.rsplit(['\\', '/']).next().unwrap_or("");
                    f.split('.').next().unwrap_or(f)
                } else {
                    img.export_name.as_str()
                };
                let hash = name_hash(key);
                let index = wad.index();
                if !index
                    .assets_by_hash(hash)
                    .any(|a| index.type_hash(a) == TEXTURE_TYPE)
                {
                    println!("    image {key:?} ({hash:08x}): no texture");
                    missing += 1;
                }
            }
        }
    }
    println!(
        "{count} movies, {failed} failed to parse, {tag_errors} tag errors, \
         {images} external images, {missing} without a texture"
    );
    Ok(())
}

fn fonts<R: Read + Seek>(wad: &mut WadArchive<R>) -> AnyResult<()> {
    let (mut count, mut bad) = (0, 0);
    let mut sizes: HashMap<u32, Option<(u16, u16)>> = HashMap::new();
    for block in blocks_with_type(wad, FONT_TYPE) {
        let path = wad.index().blocks()[block].path.clone();
        let decoded = wad.read_block(block)?;
        let contents = BlockContents::parse(&decoded)?;
        let fonts: Vec<(u32, Vec<u8>)> = contents
            .iter()
            .filter(|(e, _)| e.type_hash == FONT_TYPE)
            .map(|(e, d)| (e.name_hash, d.to_vec()))
            .collect();
        for (hash, data) in fonts {
            count += 1;
            let font = match Font::parse(&data) {
                Ok(f) => f,
                Err(err) => {
                    println!("{hash:08x} {path}: {err}");
                    bad += 1;
                    continue;
                }
            };
            let codes = font.glyphs.iter().map(|g| g.code);
            let (lo, hi) = (codes.clone().min().unwrap_or(0), codes.max().unwrap_or(0));
            let tallest = font.glyphs.iter().map(|g| g.height).max().unwrap_or(0);
            let off_page = font.glyphs.iter().filter(|g| g.material != 0).count();
            println!(
                "{hash:08x} line {} tallest {} ({tallest} by glyph) +0x04 {} \
                 {} glyphs ({off_page} off page 0) U+{lo:04X}..U+{hi:04X}  {}",
                font.line_height,
                font.tallest,
                font.unk_0x04,
                font.glyphs.len(),
                path.rsplit('\\').next().unwrap_or(&path),
            );
            let sample: Vec<String> = "0 9 A g ."
                .split(' ')
                .filter_map(|c| {
                    let g = font.glyph(u16::from(c.as_bytes()[0]))?;
                    Some(format!("{c} {}x{} y+{}", g.width, g.height, g.y_offset))
                })
                .collect();
            println!("    glyphs: {}", sample.join(", "));
            for (i, m) in font.materials.iter().enumerate() {
                let size = *sizes
                    .entry(m.texture)
                    .or_insert_with(|| texture_info(wad, m.texture).map(|t| (t.width, t.height)));
                let glyphs: Vec<_> = font
                    .glyphs
                    .iter()
                    .filter(|g| usize::from(g.material) == i)
                    .collect();
                let mut mismatched = 0;
                if let Some((w, h)) = size {
                    for g in &glyphs {
                        let pw = (g.uv[2] - g.uv[0]) * f32::from(w);
                        let ph = (g.uv[3] - g.uv[1]) * f32::from(h);
                        let on_page = g.uv.iter().all(|x| (0.0..=1.0).contains(x));
                        if !on_page
                            || (pw - f32::from(g.width)).abs() > 1.5
                            || (ph - f32::from(g.height)).abs() > 1.5
                        {
                            mismatched += 1;
                        }
                    }
                }
                println!(
                    "    material {i}: texture {:08x} {} {} glyphs, {mismatched} off their rectangle",
                    m.texture,
                    size.map_or("not found".to_owned(), |(w, h)| format!("{w}x{h}")),
                    glyphs.len(),
                );
                if size.is_none() || mismatched > 0 {
                    bad += 1;
                }
            }
        }
    }
    println!("{count} fonts, {bad} problems");
    Ok(())
}
