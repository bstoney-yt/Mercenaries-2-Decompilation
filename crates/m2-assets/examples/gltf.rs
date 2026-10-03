//! Exports a model, its skeleton, textures and animations from a `.wad` in
//! your own installation as a binary glTF (`.glb`) file, to view in Blender,
//! Windows 3D Viewer or any other glTF viewer.
//!
//! ```text
//! cargo run --release -p m2-assets --example gltf -- rigs   <vz.wad>
//! cargo run --release -p m2-assets --example gltf -- export <vz.wad> <model hash> <out.glb> [options]
//!     --clips N       export at most N animation clips (default 40)
//!     --filter TEXT   only clips from blocks whose path contains TEXT
//!     --lod N         level of detail to export (default 0, the finest)
//! cargo run --release -p m2-assets --example gltf -- view   <file.glb> [port]
//! ```
//!
//! `rigs` lists the models that animation clips fit (every track names one
//! of the model's nodes), most clips first, with the blocks they come from.
//! `export` writes the model at one level of detail with its node hierarchy
//! as the glTF node tree, skinned meshes bound to it, base-colour textures,
//! and each fitting clip as a glTF animation named after its hash and block.
//! `view` serves a small three.js viewer for a `.glb` on 127.0.0.1 (port
//! 8765 by default) with an animation picker; open the printed address in a
//! browser. The page loads three.js from cdn.jsdelivr.net.
//!
//! The viewer also has a play mode for characters exported with their
//! locomotion clips (for a human rig, `--filter resident`): WASD moves
//! relative to the camera, Shift sprints, Q switches walk and run. Clips
//! carry their motion on the root node and are otherwise in place, so the
//! page measures each clip's speed and direction from the root track, strips
//! the root's horizontal motion, and moves the character at that speed.
//!
//! The engine is left-handed; the export mirrors Z to suit glTF. Exported
//! files hold game data: keep them out of the repository (`dumps/` and
//! `local/` are ignored) and do not share them.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::process::ExitCode;

use m2_assets::animation::ANIMATION_TYPE;
use m2_assets::model::{GeometryKind, MODEL_TYPE};
use m2_assets::texture::{BODY, INFO, PixelFormat};
use m2_assets::vertex::DeclUsage;
use m2_assets::{BlockContents, Clip, Model, Texture, Ucfx, WadArchive};
use m2_math::glam::{Mat4, Quat, Vec3};

/// Type hash of texture assets.
const TEXTURE_TYPE: u32 = 0xf011_157a;

type AnyResult<T> = Result<T, Box<dyn std::error::Error>>;

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

struct Options {
    clips: usize,
    filter: Option<String>,
    lod: u32,
}

fn run(args: &[String]) -> AnyResult<()> {
    match args {
        [cmd, wad] if cmd == "rigs" => rigs(&mut open(wad)?),
        [cmd, wad, hash, out, rest @ ..] if cmd == "export" => {
            let mut o = Options {
                clips: 40,
                filter: None,
                lod: 0,
            };
            let mut it = rest.iter();
            while let Some(flag) = it.next() {
                let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
                match flag.as_str() {
                    "--clips" => o.clips = value.parse()?,
                    "--filter" => o.filter = Some(value.to_lowercase()),
                    "--lod" => o.lod = value.parse()?,
                    _ => return Err(format!("unknown option {flag}").into()),
                }
            }
            let hash = u32::from_str_radix(hash.trim_start_matches("0x"), 16)?;
            let glb = export(&mut open(wad)?, hash, &o)?;
            std::fs::write(out, glb)?;
            println!("wrote {out}");
            Ok(())
        }
        [cmd, glb, rest @ ..] if cmd == "view" => {
            let port = match rest {
                [] => 8765,
                [p] => p.parse()?,
                _ => return Err("usage: gltf view <file.glb> [port]".into()),
            };
            view(glb, port)
        }
        _ => Err(
            "usage: gltf rigs <vz.wad> | gltf export <vz.wad> <model hash> <out.glb> \
                  [--clips N] [--filter TEXT] [--lod N] | gltf view <file.glb> [port]"
                .into(),
        ),
    }
}

fn open(path: &str) -> AnyResult<WadArchive<BufReader<File>>> {
    Ok(WadArchive::open(BufReader::new(File::open(path)?))?)
}

/// A block's short name: its path without the folder or the `_P000_Q3` tail.
fn block_name(path: &str) -> &str {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    name.split("_P0").next().unwrap_or(name)
}

/// The quality tier a block holds, from the `_Qn` at the end of its path.
fn block_tier(path: &str) -> Option<usize> {
    let at = path.rfind("_Q")?;
    Some(path[at + 2..].chars().next()?.to_digit(10)? as usize)
}

/// The track node hashes in an animation asset's `trnm`, without decoding
/// the clip. `None` for material animations.
fn track_hashes(asset: &[u8]) -> Option<Vec<u32>> {
    let t = Ucfx::parse(asset).ok()?.top_level(*b"trnm")?;
    let n = usize::from(u16::from_le_bytes([*t.first()?, *t.get(1)?]));
    (0..n)
        .map(|i| {
            let at = 8 + 4 * i;
            Some(u32::from_le_bytes(t.get(at..at + 4)?.try_into().ok()?))
        })
        .collect()
}

/// Every animation asset: (name hash, block path, bytes).
fn all_clips<R: Read + Seek>(wad: &mut WadArchive<R>) -> AnyResult<Vec<(u32, String, Vec<u8>)>> {
    let mut out = Vec::new();
    for block in 0..wad.index().blocks().len() {
        let path = wad.index().blocks()[block].path.clone();
        let decoded = wad.read_block(block)?;
        for (e, data) in BlockContents::parse(&decoded)?.iter() {
            if e.type_hash == ANIMATION_TYPE {
                out.push((e.name_hash, path.clone(), data.to_vec()));
            }
        }
    }
    Ok(out)
}

fn rigs<R: Read + Seek>(wad: &mut WadArchive<R>) -> AnyResult<()> {
    let mut models: Vec<(u32, String, HashSet<u32>)> = Vec::new();
    let mut clips = Vec::new();
    for block in 0..wad.index().blocks().len() {
        let path = wad.index().blocks()[block].path.clone();
        let decoded = wad.read_block(block)?;
        for (e, data) in BlockContents::parse(&decoded)?.iter() {
            if e.type_hash == MODEL_TYPE && e.unk_0x08 == 0 {
                if let Ok(m) = Model::parse(data) {
                    let nodes = m.nodes.iter().map(|n| n.name_hash).collect();
                    models.push((e.name_hash, path.clone(), nodes));
                }
            } else if e.type_hash == ANIMATION_TYPE
                && let Some(t) = track_hashes(data)
            {
                clips.push((path.clone(), t));
            }
        }
    }
    let mut rows: Vec<(usize, u32, &str, usize, String)> = Vec::new();
    for (hash, path, nodes) in &models {
        let mut groups: BTreeMap<&str, usize> = BTreeMap::new();
        for (cpath, tracks) in &clips {
            if tracks.iter().all(|t| nodes.contains(t)) {
                *groups.entry(block_name(cpath)).or_default() += 1;
            }
        }
        let total: usize = groups.values().sum();
        if total > 0 {
            let mut g: Vec<_> = groups.into_iter().collect();
            g.sort_by_key(|x| std::cmp::Reverse(x.1));
            let top: Vec<String> = g.iter().take(4).map(|(n, c)| format!("{n} {c}")).collect();
            rows.push((total, *hash, block_name(path), nodes.len(), top.join(", ")));
        }
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    println!("clips  model     nodes  model block / clip blocks");
    for (total, hash, block, nodes, top) in &rows {
        println!("{total:5}  {hash:08x}  {nodes:5}  {block}  [{top}]");
    }
    println!("{} models have clips that fit them", rows.len());
    Ok(())
}

/// Assembles a model with every streamed chunk.
fn load_model<R: Read + Seek>(
    wad: &mut WadArchive<R>,
    hash: u32,
) -> AnyResult<(Vec<u8>, Vec<Vec<u8>>)> {
    let mut blocks: Vec<usize> = wad
        .index()
        .assets_by_hash(hash)
        .flat_map(|a| std::iter::once(a.block).chain(a.alt_blocks()))
        .map(usize::from)
        .collect();
    blocks.sort_unstable();
    blocks.dedup();
    let (mut base, mut chunks) = (None, Vec::new());
    for b in blocks {
        let decoded = wad.read_block(b)?;
        for (e, data) in BlockContents::parse(&decoded)?.iter() {
            if e.type_hash == MODEL_TYPE && e.name_hash == hash {
                if e.unk_0x08 == 0 {
                    base = Some(data.to_vec());
                } else {
                    chunks.push((e.unk_0x08, data.to_vec()));
                }
            }
        }
    }
    chunks.sort_by_key(|c| c.0);
    let base = base.ok_or_else(|| format!("no model {hash:08x}"))?;
    Ok((base, chunks.into_iter().map(|c| c.1).collect()))
}

/// Decodes a texture's largest mip (with its streamed tiers if present) to
/// RGBA8. Returns (width, height, pixels, has alpha).
fn load_texture<R: Read + Seek>(
    wad: &mut WadArchive<R>,
    hash: u32,
) -> Option<(u32, u32, Vec<u8>, bool)> {
    let mut blocks: Vec<usize> = wad
        .index()
        .assets_by_hash(hash)
        .flat_map(|a| std::iter::once(a.block).chain(a.alt_blocks()))
        .map(usize::from)
        .collect();
    blocks.sort_unstable();
    blocks.dedup();
    let mut base = None;
    let mut tiers: [Option<Vec<u8>>; 3] = Default::default();
    for b in blocks {
        let tier = block_tier(&wad.index().blocks()[b].path);
        let decoded = wad.read_block(b).ok()?;
        for (e, data) in BlockContents::parse(&decoded).ok()?.iter() {
            if e.type_hash != TEXTURE_TYPE || e.name_hash != hash {
                continue;
            }
            let u = Ucfx::parse(data).ok()?;
            if u.top_level(INFO).is_some() {
                base = Some(data.to_vec());
            } else if let (Some(t), Some(body)) = (tier.filter(|&t| t < 3), u.top_level(BODY)) {
                tiers[t] = Some(body.to_vec());
            }
        }
    }
    let base = base?;
    let tex = Texture::parse(&base).ok()?;
    let alpha = matches!(
        tex.info.format,
        PixelFormat::Dxt3 | PixelFormat::Dxt5 | PixelFormat::A8R8G8B8
    );
    let chain = tex.assemble([
        tiers[0].as_deref(),
        tiers[1].as_deref(),
        tiers[2].as_deref(),
    ]);
    let mip = match &chain {
        Ok(c) => tex.chain_mips(c).ok()?.into_iter().next()?,
        Err(_) => tex.resident_mips().into_iter().next()?,
    };
    let rgba = mip.to_rgba8(tex.info.format).ok()?;
    Some((mip.width, mip.height, rgba, alpha))
}

/// A PNG of RGBA8 pixels (filter 0, zlib via miniz_oxide).
fn png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut c = !0u32;
        for &b in data {
            c ^= u32::from(b);
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
        }
        !c
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |tag: &[u8; 4], data: &[u8]| {
        out.extend((data.len() as u32).to_be_bytes());
        let mut body = tag.to_vec();
        body.extend_from_slice(data);
        out.extend(&body);
        out.extend(crc32(&body).to_be_bytes());
    };
    let mut ihdr = width.to_be_bytes().to_vec();
    ihdr.extend(height.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    let mut raw = Vec::with_capacity(rgba.len() + height as usize);
    for row in rgba.chunks_exact(width as usize * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    chunk(
        b"IDAT",
        &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6),
    );
    chunk(b"IEND", &[]);
    out
}

/// Mirrors Z: the engine is left-handed, glTF right-handed.
fn flip_m(m: Mat4) -> Mat4 {
    let s = Mat4::from_scale(Vec3::new(1.0, 1.0, -1.0));
    s * m * s
}

fn flip_v([x, y, z]: [f32; 3]) -> [f32; 3] {
    [x, y, -z]
}

fn flip_q([x, y, z, w]: [f32; 4]) -> [f32; 4] {
    [-x, -y, z, w]
}

/// The glTF binary buffer and JSON arrays under construction.
#[derive(Default)]
struct Gltf {
    bin: Vec<u8>,
    views: Vec<String>,
    accessors: Vec<String>,
}

impl Gltf {
    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        self.bin.resize(self.bin.len().next_multiple_of(4), 0);
        let target = target.map_or(String::new(), |t| format!(r#","target":{t}"#));
        self.views.push(format!(
            r#"{{"buffer":0,"byteOffset":{},"byteLength":{}{target}}}"#,
            self.bin.len(),
            bytes.len()
        ));
        self.bin.extend_from_slice(bytes);
        self.views.len() - 1
    }

    /// An accessor over f32 values, `width` per element.
    fn floats(&mut self, values: &[f32], width: usize, bounds: bool, target: Option<u32>) -> usize {
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        let view = self.view(&bytes, target);
        let ty = [
            "", "SCALAR", "VEC2", "VEC3", "VEC4", "", "", "", "", "", "", "", "", "", "", "",
            "MAT4",
        ][width];
        let mut minmax = String::new();
        if bounds {
            let (mut lo, mut hi) = (vec![f32::MAX; width], vec![f32::MIN; width]);
            for e in values.chunks_exact(width) {
                for (i, v) in e.iter().enumerate() {
                    lo[i] = lo[i].min(*v);
                    hi[i] = hi[i].max(*v);
                }
            }
            let _ = write!(minmax, r#","min":{lo:?},"max":{hi:?}"#);
        }
        self.accessors.push(format!(
            r#"{{"bufferView":{view},"componentType":5126,"count":{},"type":"{ty}"{minmax}}}"#,
            values.len() / width
        ));
        self.accessors.len() - 1
    }

    fn u16s(&mut self, values: &[u16], width: usize, target: Option<u32>) -> usize {
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        let view = self.view(&bytes, target);
        let ty = if width == 1 { "SCALAR" } else { "VEC4" };
        self.accessors.push(format!(
            r#"{{"bufferView":{view},"componentType":5123,"count":{},"type":"{ty}"}}"#,
            values.len() / width
        ));
        self.accessors.len() - 1
    }

    fn glb(self, json: String) -> Vec<u8> {
        let mut json = json.into_bytes();
        json.resize(json.len().next_multiple_of(4), b' ');
        let mut bin = self.bin;
        bin.resize(bin.len().next_multiple_of(4), 0);
        let mut out = b"glTF".to_vec();
        out.extend(2u32.to_le_bytes());
        out.extend(((12 + 8 + json.len() + 8 + bin.len()) as u32).to_le_bytes());
        out.extend((json.len() as u32).to_le_bytes());
        out.extend(b"JSON");
        out.extend(json);
        out.extend((bin.len() as u32).to_le_bytes());
        out.extend(b"BIN\0");
        out.extend(bin);
        out
    }
}

fn export<R: Read + Seek>(wad: &mut WadArchive<R>, hash: u32, o: &Options) -> AnyResult<Vec<u8>> {
    let (base, chunks) = load_model(wad, hash)?;
    let mut model = Model::parse(&base)?;
    for c in &chunks {
        model.add_streamed(c)?;
    }
    if !model.has_lod(o.lod) {
        return Err(format!("LOD {} is not complete", o.lod).into());
    }
    let mut g = Gltf::default();
    let node_count = model.nodes.len();

    // Nodes 0..n are the HIER nodes; mesh nodes follow.
    let mut nodes: Vec<String> = Vec::new();
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); node_count];
    let mut roots = Vec::new();
    for (i, n) in model.nodes.iter().enumerate() {
        match n.parent.map(usize::from).filter(|&p| p < node_count) {
            Some(p) => children[p].push(i),
            None => roots.push(i),
        }
    }

    // Materials and textures.
    let mut images = Vec::new();
    let mut textures: HashMap<u32, Option<(usize, bool)>> = HashMap::new();
    let mut materials = Vec::new();
    for m in &model.materials {
        let tex = match m.textures.first().copied().filter(|&t| t != 0) {
            Some(t) => *textures.entry(t).or_insert_with(|| {
                let (w, h, rgba, alpha) = load_texture(wad, t)?;
                let view = g.view(&png(w, h, &rgba), None);
                images.push(format!(
                    r#"{{"bufferView":{view},"mimeType":"image/png","name":"{t:08x}"}}"#
                ));
                Some((images.len() - 1, alpha))
            }),
            None => None,
        };
        materials.push(match tex {
            Some((img, alpha)) => format!(
                r#"{{"name":"{:08x}","pbrMetallicRoughness":{{"baseColorTexture":{{"index":{img}}},"metallicFactor":0,"roughnessFactor":0.8}}{}}}"#,
                m.name_hash,
                if alpha { r#","alphaMode":"MASK","alphaCutoff":0.5"# } else { "" }
            ),
            None => format!(
                r#"{{"name":"{:08x}","pbrMetallicRoughness":{{"metallicFactor":0,"roughnessFactor":0.8}}}}"#,
                m.name_hash
            ),
        });
    }

    // Meshes: one per geometry slot, reused by every segment placing it.
    let mut meshes = Vec::new();
    let mut slot_mesh: HashMap<u8, usize> = HashMap::new();
    let mut mesh_nodes: Vec<String> = Vec::new();
    let mut skinned = false;
    let segments: Vec<_> = model.lod(o.lod).map(|(s, geom)| (*s, geom)).collect();
    for (segment, geom) in &segments {
        let skin = geom.kind == GeometryKind::Skin;
        let mesh = match slot_mesh.get(&segment.slot) {
            Some(&m) => m,
            None => {
                let mut prims = Vec::new();
                for grp in &geom.groups {
                    let read = |u| grp.decl.find(u, 0).and_then(|e| grp.vertices.read(e));
                    let Some(pos) = read(DeclUsage::Position) else {
                        continue;
                    };
                    let p: Vec<f32> = pos
                        .iter()
                        .flat_map(|v| flip_v([v[0], v[1], v[2]]))
                        .collect();
                    let mut attrs = format!(r#""POSITION":{}"#, g.floats(&p, 3, true, Some(34962)));
                    if let Some(nrm) = read(DeclUsage::Normal) {
                        let n: Vec<f32> = nrm
                            .iter()
                            .flat_map(|v| {
                                let n = Vec3::new(v[0], v[1], v[2])
                                    .try_normalize()
                                    .unwrap_or(Vec3::Y);
                                flip_v(n.to_array())
                            })
                            .collect();
                        let _ = write!(
                            attrs,
                            r#","NORMAL":{}"#,
                            g.floats(&n, 3, false, Some(34962))
                        );
                    }
                    if let Some(uv) = read(DeclUsage::TexCoord) {
                        let t: Vec<f32> = uv.iter().flat_map(|v| [v[0], v[1]]).collect();
                        let _ = write!(
                            attrs,
                            r#","TEXCOORD_0":{}"#,
                            g.floats(&t, 2, false, Some(34962))
                        );
                    }
                    if skin
                        && let (Some(idx), Some(wts)) =
                            (read(DeclUsage::BlendIndices), read(DeclUsage::BlendWeight))
                    {
                        let palette = grp.info.bone_nodes();
                        let mut joints = Vec::with_capacity(idx.len() * 4);
                        let mut weights = Vec::with_capacity(idx.len() * 4);
                        for (i, w) in idx.iter().zip(&wts) {
                            let sum: f32 = w.iter().sum();
                            for k in 0..4 {
                                let node = palette
                                    .get(i[k] as usize)
                                    .copied()
                                    .filter(|&n| usize::from(n) < node_count);
                                joints.push(node.unwrap_or(0));
                                let wk = if node.is_some() && sum > 0.0 {
                                    w[k] / sum
                                } else {
                                    0.0
                                };
                                weights.push(wk);
                            }
                            if sum <= 0.0 {
                                let at = weights.len() - 4;
                                weights[at] = 1.0;
                            }
                        }
                        let j = g.u16s(&joints, 4, Some(34962));
                        let w = g.floats(&weights, 4, false, Some(34962));
                        let _ = write!(attrs, r#","JOINTS_0":{j},"WEIGHTS_0":{w}"#);
                        skinned = true;
                    }
                    for range in &grp.material_ranges {
                        // Direct3D's front faces are clockwise, glTF's are
                        // counter-clockwise; the Z mirror alone does not
                        // reconcile them, so reverse each triangle.
                        let tris: Vec<u16> = grp
                            .triangles(range)
                            .into_iter()
                            .flat_map(|[a, b, c]| [a, c, b])
                            .collect();
                        if tris.is_empty() {
                            continue;
                        }
                        let ind = g.u16s(&tris, 1, Some(34963));
                        let material =
                            (range.material as usize).min(materials.len().saturating_sub(1));
                        prims.push(format!(
                            r#"{{"attributes":{{{attrs}}},"indices":{ind}{}}}"#,
                            if materials.is_empty() {
                                String::new()
                            } else {
                                format!(r#","material":{material}"#)
                            }
                        ));
                    }
                }
                meshes.push(format!(
                    r#"{{"name":"slot{}","primitives":[{}]}}"#,
                    segment.slot,
                    prims.join(",")
                ));
                slot_mesh.insert(segment.slot, meshes.len() - 1);
                meshes.len() - 1
            }
        };
        let index = node_count + mesh_nodes.len();
        if skin {
            // A skinned mesh's own transform is ignored; it sits at the root.
            mesh_nodes.push(format!(
                r#"{{"name":"skin slot{}","mesh":{mesh},"skin":0}}"#,
                segment.slot
            ));
            roots.push(index);
        } else {
            mesh_nodes.push(format!(
                r#"{{"name":"slot{}","mesh":{mesh}}}"#,
                segment.slot
            ));
            match children.get_mut(usize::from(segment.node)) {
                Some(c) => c.push(index),
                None => roots.push(index),
            }
        }
    }

    for (i, n) in model.nodes.iter().enumerate() {
        let (s, r, t) = flip_m(Mat4::from(n.local)).to_scale_rotation_translation();
        let kids = if children[i].is_empty() {
            String::new()
        } else {
            format!(r#","children":{:?}"#, children[i])
        };
        nodes.push(format!(
            r#"{{"name":"{:08x}","translation":{:?},"rotation":{:?},"scale":{:?}{kids}}}"#,
            n.name_hash,
            t.to_array(),
            r.normalize().to_array(),
            s.to_array()
        ));
    }
    nodes.extend(mesh_nodes);

    let mut skins = String::new();
    if skinned {
        let ibm: Vec<f32> = model
            .nodes
            .iter()
            .flat_map(|n| flip_m(Mat4::from(n.inverse_bind)).to_cols_array())
            .collect();
        let acc = g.floats(&ibm, 16, false, None);
        let joints: Vec<usize> = (0..node_count).collect();
        skins = format!(r#","skins":[{{"inverseBindMatrices":{acc},"joints":{joints:?}}}]"#);
    }

    // Animations that fit the model.
    let node_of: HashMap<u32, usize> = model
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.name_hash, i))
        .collect();
    let mut animations = Vec::new();
    let mut fitting = 0;
    for (chash, path, data) in all_clips(wad)? {
        if animations.len() >= o.clips {
            break;
        }
        let Some(tracks) = track_hashes(&data) else {
            continue;
        };
        if !tracks.iter().all(|t| node_of.contains_key(t)) {
            continue;
        }
        let name = block_name(&path).to_owned();
        if o.filter
            .as_ref()
            .is_some_and(|f| !name.to_lowercase().contains(f))
        {
            continue;
        }
        fitting += 1;
        let clip = match Clip::parse(&data) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("clip {chash:08x}: {e}");
                continue;
            }
        };
        let frames = clip.frame_count as usize;
        let fps = clip.frame_rate().unwrap_or(30.0);
        let times: Vec<f32> = (0..frames.max(1)).map(|f| f as f32 / fps).collect();
        let input = g.floats(&times, 1, true, None);
        let (mut samplers, mut channels) = (Vec::new(), Vec::new());
        for (t, th) in tracks.iter().enumerate() {
            let node = node_of[th];
            let (mut tr, mut rot, mut sc) = (Vec::new(), Vec::new(), Vec::new());
            let mut prev = Quat::IDENTITY.to_array();
            for f in 0..frames.max(1) {
                let Some(x) = clip.frame(f as u32).and_then(|fr| fr.get(t)) else {
                    break;
                };
                tr.extend(flip_v([x.translation.x, x.translation.y, x.translation.z]));
                let mut q = flip_q([x.rotation.x, x.rotation.y, x.rotation.z, x.rotation.w]);
                let len = q.iter().map(|c| c * c).sum::<f32>().sqrt();
                if len > 0.0 {
                    q.iter_mut().for_each(|c| *c /= len);
                }
                // Keep neighbouring keys on the same hemisphere.
                if q.iter().zip(prev).map(|(a, b)| a * b).sum::<f32>() < 0.0 {
                    q.iter_mut().for_each(|c| *c = -*c);
                }
                prev = q;
                rot.extend(q);
                sc.extend([x.scale.x, x.scale.y, x.scale.z]);
            }
            if tr.len() != 3 * times.len() {
                continue;
            }
            let mut add = |path: &str, values: &[f32], width: usize, g: &mut Gltf| {
                let out = g.floats(values, width, false, None);
                samplers.push(format!(
                    r#"{{"input":{input},"output":{out},"interpolation":"LINEAR"}}"#
                ));
                channels.push(format!(
                    r#"{{"sampler":{},"target":{{"node":{node},"path":"{path}"}}}}"#,
                    samplers.len() - 1
                ));
            };
            add("translation", &tr, 3, &mut g);
            add("rotation", &rot, 4, &mut g);
            if sc.iter().any(|&v| (v - 1.0).abs() > 1e-4) {
                add("scale", &sc, 3, &mut g);
            }
        }
        animations.push(format!(
            r#"{{"name":"{name} {chash:08x}","samplers":[{}],"channels":[{}]}}"#,
            samplers.join(","),
            channels.join(",")
        ));
    }
    eprintln!(
        "{} nodes, {} meshes, {} materials, {} textures, {} animations{}",
        node_count,
        meshes.len(),
        materials.len(),
        images.len(),
        animations.len(),
        if fitting > animations.len() {
            " (some clips failed)"
        } else {
            ""
        }
    );

    let textures_json: Vec<String> = (0..images.len())
        .map(|i| format!(r#"{{"source":{i},"sampler":0}}"#))
        .collect();
    let mut json = format!(
        r#"{{"asset":{{"version":"2.0","generator":"m2-assets gltf example"}},"scene":0,"scenes":[{{"name":"{hash:08x}","nodes":{roots:?}}}],"nodes":[{}],"meshes":[{}]"#,
        nodes.join(","),
        meshes.join(",")
    );
    if !materials.is_empty() {
        let _ = write!(json, r#","materials":[{}]"#, materials.join(","));
    }
    if !images.is_empty() {
        let _ = write!(
            json,
            r#","images":[{}],"textures":[{}],"samplers":[{{"magFilter":9729,"minFilter":9987,"wrapS":10497,"wrapT":10497}}]"#,
            images.join(","),
            textures_json.join(",")
        );
    }
    json += &skins;
    if !animations.is_empty() {
        let _ = write!(json, r#","animations":[{}]"#, animations.join(","));
    }
    let _ = write!(
        json,
        r#","accessors":[{}],"bufferViews":[{}],"buffers":[{{"byteLength":{}}}]}}"#,
        g.accessors.join(","),
        g.views.join(","),
        g.bin.len().next_multiple_of(4)
    );
    Ok(g.glb(json))
}

/// Serves the viewer page and `glb` on 127.0.0.1 until stopped.
fn view(glb: &str, port: u16) -> AnyResult<()> {
    use std::io::{BufRead, Write};
    let page = include_str!("gltf_viewer.html");
    let model = std::fs::read(glb)?;
    let name = std::path::Path::new(glb)
        .file_name()
        .map_or("model.glb".into(), |n| n.to_string_lossy().into_owned());
    let listener = std::net::TcpListener::bind(("127.0.0.1", port))?;
    println!("viewing {name}: open http://127.0.0.1:{port}/?name={name} (Ctrl+C to stop)");
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let mut line = String::new();
        if BufReader::new(&stream).read_line(&mut line).is_err() {
            continue;
        }
        let path = line.split_whitespace().nth(1).unwrap_or("/");
        let path = path.split('?').next().unwrap_or("/");
        let (status, ty, body): (&str, &str, &[u8]) = match path {
            "/" | "/index.html" => ("200 OK", "text/html; charset=utf-8", page.as_bytes()),
            "/model.glb" => ("200 OK", "model/gltf-binary", &model),
            _ => ("404 Not Found", "text/plain", b"not found"),
        };
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {ty}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream
            .write_all(head.as_bytes())
            .and_then(|()| stream.write_all(body));
    }
    Ok(())
}
