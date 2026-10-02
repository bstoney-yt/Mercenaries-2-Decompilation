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
//! ```
//!
//! `textures` parses every texture header, finds its streamed mips and checks
//! their lengths; `dds` writes one texture's whole chain as a `.dds` file.
//! `models` parses every model with its streamed chunks and checks indices,
//! bounds and bone palettes; `obj` writes one level of detail of one model
//! (by its name hash in hex) as a Wavefront `.obj` file, in model space.
//!
//! Extracted files are for local inspection; keep them out of the repository
//! (`dumps/` and `local/` are ignored).

use std::fs::File;
use std::io::BufReader;
use std::process::ExitCode;

use std::collections::HashMap;
use std::io::{Read, Seek};

use m2_assets::model::{GroupExtra, MODEL_TYPE};
use m2_assets::texture::{BODY, INFO};
use m2_assets::vertex::DeclUsage;
use m2_assets::{
    BlockContents, Model, PixelFormat, PrimitiveGroup, Texture, TextureInfo, TextureKind, Ucfx,
    WadArchive,
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
                "usage: wad <list|assets|names|extract|textures|dds|models|obj> <archive.wad> [args]".into(),
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
        ("models", []) => models(&mut wad)?,
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
                let outside = p
                    .iter()
                    .filter(|v| !aabb.contains([v[0], v[1], v[2]]))
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
