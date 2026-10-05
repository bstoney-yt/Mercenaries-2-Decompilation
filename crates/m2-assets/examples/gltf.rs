//! Exports a model, its skeleton, textures and animations from a `.wad` in
//! your own installation as a binary glTF (`.glb`) file, to view in Blender,
//! Windows 3D Viewer or any other glTF viewer.
//!
//! ```text
//! cargo run --release -p m2-assets --example gltf -- rigs   <vz.wad>
//! cargo run --release -p m2-assets --example gltf -- export <vz.wad> <model hash> <out.glb> [options]
//!     --clips N       export at most N animation clips (default 40)
//!     --filter TEXT   only clips from blocks whose name contains TEXT (or any of TEXT,TEXT,...)
//!     --lod N         level of detail to export (default 0, the finest)
//! cargo run --release -p m2-assets --example gltf -- world  <vz.wad> <out.glb> [options]
//!     --detail X,Z    full-detail terrain around this point (game x, z)
//!     --radius M      ... out to this distance (default 600)
//!     --objects WHICH static (default: layers_static), all (mission states too) or none
//!     --sea Y         sea surface height (default -35, an estimate)
//!     --vehicles N    drivable vehicles parked near --detail (default 8, 0 for none)
//! cargo run --release -p m2-assets --example gltf -- npcs   <vz.wad> <out dir> [options]
//!     --detail X,Z    take spawn points around this point (game x, z)
//!     --radius M      ... out to this distance (default 400)
//!     --models N      character models to export (default 8)
//! cargo run --release -p m2-assets --example gltf -- sounds <vz.wad> <out dir>
//! cargo run --release -p m2-assets --example gltf -- view   <file.glb> [--world <world.glb>] [--npcs <dir>] [--sounds <dir>]
//!     [--streams <install data dir>] [--port N]
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
//! `world` exports the island's terrain placed by the layout layer: the
//! low-resolution tiles everywhere, full-detail cells near `--detail`, a sea
//! plane; full-detail cells use their own 512 px colour maps and the tiles
//! the island-wide one (`vz_lrterrain`). It also places the buildings and
//! props named by `ModelName` entities, and in full-detail cells the
//! scatter (grass, bushes, rubble, litter) as GPU-instanced meshes
//! (`EXT_mesh_gpu_instancing`). With `view --world`, play mode starts on the island
//! (`?spawn=X,Z` in the page address picks the point) and the character
//! follows the ground; objects are drawn within `?view=` metres (default
//! 1500) and scatter within `?veg=` metres (default 150).
//!
//! Collision: a placed model's `PHY2` shapes (see `m2_assets::physics`) are
//! exported as a child node `col ...` of each placement. The viewer stops
//! the character at them, lets it step up 0.5 m onto them, and slides it
//! along walls. Placements without `PHY2`, which are mostly the merged
//! building batches (`TINY` models), collide through their render triangles
//! instead: a fallback the page labels as such. The character also cannot
//! climb terrain steeper than 50 degrees, slows down uphill, and falls off
//! edges.
//!
//! Vehicles: `--vehicles N` parks N drivable ground vehicles in a row 12 m
//! north of `--detail`, one of each kind first (car, truck, civilian model,
//! APC, tank, then the rest). Each is a node `veh ...` holding the body and
//! one `wheel N` node per wheel, centred on the wheel so the viewer can
//! spin and steer it, with the kind, wheels, bounds and PHY2 mass in the
//! node's extras. Kinds come from texture names and wheels from geometry
//! (see `m2_assets::vehicle`). In play mode, E gets in and out; W/S drive,
//! A/D steer and Space brakes. The handling is a simple arcade model tuned
//! by hand, not the game's. Helicopters (a UH-1 first) come with their main
//! and tail rotors as `rotor N` nodes, found by their marker names, and fly
//! with an arcade model also tuned by hand: Space climbs, Shift descends,
//! W/S fly forward and back, A/D turn, and E gets out once landed.
//! Vehicles collide with each other through their footprint boxes. The
//! extras also carry the driver's seat and
//! door markers. With the character exported with its vehicle clips
//! (`--filter resident,vehicleclassanimgroup`), it climbs in with the
//! game's enter clip for the vehicle's class, sits in the seated loop while
//! driving, and climbs out with the enter clip played backwards; the clips
//! are told apart by their root motion, relative to the driver's seat.
//!
//! `npcs` writes character models for the world (`npc_<hash>.glb`, skinned
//! and textured, without clips) and `npcs.json` with spawn points from
//! layer entities that place soldiers (`Ai`, `AiPatrol`,
//! `PopulationSimpleSpawner`). Every skinned human model shares the
//! playable character's skeleton (root node `cbc1eb51`), so the viewer
//! drives them with its clips. Soldier spawns get the army model their
//! entity name asks for ("VZ Soldier", "VZ Heavy", "VZ Deathsquad");
//! civilians (`?npcs=N` in the page address) are scattered around the start.
//! A soldier whose `AiPatrol` names a `Path` entity gets that path as a
//! `route` (world points, glTF axes) and walks it: round again if it closes
//! on itself, back the way it came otherwise. The rest of their behaviour
//! (idling, wandering near home, stepping aside, running from fast vehicles,
//! the pauses at a route's ends) is invented, not the game's AI. `npcs` also writes a
//! rifle (`wpn_<hash>.glb`, the AK-47, with its muzzle marker). The viewer
//! puts it in the right hand of the player and of every soldier, held the
//! way the game's two-handed rifle clip holds it; click or hold F to fire,
//! hold the right button to aim. Soldiers that hear gunfire shoot back,
//! civilians run, and the hit and fall reactions use the game's clips. The
//! combat rules (damage, accuracy, fire rates, health) are invented. 1 and 2
//! switch between the rifle and the RPG (the game's launcher and rocket
//! models), G throws a grenade; blasts throw people with the game's
//! knock-back fall clip and wreck vehicles. The blast's look and numbers
//! are invented.
//!
//! `sounds` writes the game's sound banks the viewer uses (the rifle,
//! vehicle engines, town and jungle ambience) as WAV files, with
//! `sounds.json` giving each sound's length, loudness and whether it loops.
//! The banks are 16-bit PCM (see `m2_assets::sound`). A sound's role in its
//! bank is named only by hash, so the viewer picks by those measurements:
//! the loudest short mono sound for the shot, the loudest long loop for an
//! engine.
//!
//! `view --streams <install data dir>` also plays the game's music and
//! recorded ambience: the server reads their indexes from `vz.wad` (see
//! `m2_assets::sound::StreamIndex`) and decodes a track from
//! `Audios/music.pws` or `ambience.pws` when the page asks for it; nothing
//! is written to disk. With `English.wad` beside `vz.wad` it also lists the
//! 27 resident voice sets (`m2_assets::voice`) and decodes their lines from
//! `vo_stream.english.pws`, so soldiers and civilians shout in Play mode.
//! Which set is the VZ army is named only by hash, so the page picks the
//! soldiers' and civilians' sets by ear (Voices panel); the defaults were
//! checked by ear, and the moments they shout are invented.
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
        [cmd, wad, out, rest @ ..] if cmd == "world" => {
            let mut w = WorldOptions {
                detail: None,
                radius: 600.0,
                objects: Objects::Static,
                sea: -35.0,
                vehicles: 8,
            };
            let mut it = rest.iter();
            while let Some(flag) = it.next() {
                let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
                match flag.as_str() {
                    "--detail" => {
                        let (x, z) = value.split_once(',').ok_or("--detail takes X,Z")?;
                        w.detail = Some([x.trim().parse()?, z.trim().parse()?]);
                    }
                    "--radius" => w.radius = value.parse()?,
                    "--sea" => w.sea = value.parse()?,
                    "--vehicles" => w.vehicles = value.parse()?,
                    "--objects" => {
                        w.objects = match value.as_str() {
                            "static" => Objects::Static,
                            "all" => Objects::All,
                            "none" => Objects::None,
                            _ => return Err("--objects takes static, all or none".into()),
                        }
                    }
                    _ => return Err(format!("unknown option {flag}").into()),
                }
            }
            let glb = export_world(&mut open(wad)?, &w)?;
            std::fs::write(out, glb)?;
            println!("wrote {out}");
            Ok(())
        }
        [cmd, wad, dir] if cmd == "sounds" => export_sounds(&mut open(wad)?, dir),
        [cmd, wad, dir, rest @ ..] if cmd == "npcs" => {
            let mut o = NpcOptions {
                detail: [0.0, 0.0],
                radius: 400.0,
                models: 8,
            };
            let mut it = rest.iter();
            while let Some(flag) = it.next() {
                let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
                match flag.as_str() {
                    "--detail" => {
                        let (x, z) = value.split_once(',').ok_or("--detail takes X,Z")?;
                        o.detail = [x.trim().parse()?, z.trim().parse()?];
                    }
                    "--radius" => o.radius = value.parse()?,
                    "--models" => o.models = value.parse()?,
                    _ => return Err(format!("unknown option {flag}").into()),
                }
            }
            export_npcs(&mut open(wad)?, dir, &o)
        }
        [cmd, glb, rest @ ..] if cmd == "view" => {
            let (mut port, mut world, mut npcs, mut sounds, mut streams) =
                (8765, None, None, None, None);
            let mut it = rest.iter();
            while let Some(arg) = it.next() {
                match arg.as_str() {
                    "--world" => world = Some(it.next().ok_or("--world needs a file")?.as_str()),
                    "--npcs" => npcs = Some(it.next().ok_or("--npcs needs a folder")?.as_str()),
                    "--sounds" => {
                        sounds = Some(it.next().ok_or("--sounds needs a folder")?.as_str())
                    }
                    "--streams" => {
                        streams = Some(it.next().ok_or("--streams needs a data folder")?.as_str())
                    }
                    "--port" => port = it.next().ok_or("--port needs a value")?.parse()?,
                    p => port = p.parse()?,
                }
            }
            view(glb, world, npcs, sounds, streams, port)
        }
        _ => Err(
            "usage: gltf rigs <vz.wad> | gltf export <vz.wad> <model hash> <out.glb> \
                  [--clips N] [--filter TEXT] [--lod N] | gltf world <vz.wad> <out.glb> \
                  [--detail X,Z] [--radius M] | gltf view <file.glb> [--world world.glb] [--port N]"
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

/// Decodes a texture's largest mip no wider than `max` (with its streamed
/// tiers if present) to RGBA8. Returns (width, height, pixels, has alpha).
fn load_texture<R: Read + Seek>(
    wad: &mut WadArchive<R>,
    hash: u32,
    max: u32,
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
        Ok(c) => tex
            .chain_mips(c)
            .ok()?
            .into_iter()
            .find(|m| m.width <= max)?,
        Err(_) => tex.resident_mips().into_iter().find(|m| m.width <= max)?,
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

    fn u32s(&mut self, values: &[u32], target: Option<u32>) -> usize {
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        let view = self.view(&bytes, target);
        self.accessors.push(format!(
            r#"{{"bufferView":{view},"componentType":5125,"count":{},"type":"SCALAR"}}"#,
            values.len()
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
                let (w, h, rgba, alpha) = load_texture(wad, t, u32::MAX)?;
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
    // No clips wanted: skip reading every clip in the archive.
    let clips = if o.clips == 0 {
        Vec::new()
    } else {
        all_clips(wad)?
    };
    for (chash, path, data) in clips {
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
            .is_some_and(|f| !f.split(',').any(|f| name.to_lowercase().contains(f.trim())))
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

/// The streamed music, ambience and voices of an installation: the index
/// banks (see `m2_assets::sound::StreamIndex`) and where the `.pws` files are.
struct Streams {
    dir: std::path::PathBuf,
    music: m2_assets::StreamIndex,
    ambience: m2_assets::StreamIndex,
    voice: Option<Voices>,
}

/// The voice stream (from `English.wad`) and the resident voice sets' cue
/// tables (see `m2_assets::voice`).
struct Voices {
    index: m2_assets::StreamIndex,
    /// On disk the index's `vo_stream.pws` carries the language:
    /// `vo_stream.english.pws`.
    file: String,
    sets: Vec<m2_assets::CueTable>,
}

/// Reads the voice stream index and the `vo_resident` cue tables.
fn load_voices(wad_path: &std::path::Path) -> AnyResult<Voices> {
    use m2_assets::sound::SOUND_BANK_TYPE;
    use m2_assets::voice::CUE_TABLE_TYPE;
    let mut wad = open(&wad_path.to_string_lossy())?;
    let (mut index, mut sets) = (None, Vec::new());
    for block in 0..wad.index().blocks().len() {
        let path = wad.index().blocks()[block].path.clone();
        let resident = path.contains("vo_resident");
        if !resident && !path.contains("\\resident_P") {
            continue;
        }
        let decoded = wad.read_block(block)?;
        for (e, data) in BlockContents::parse(&decoded)?.iter() {
            match e.type_hash {
                SOUND_BANK_TYPE => {
                    if let Ok(i) = m2_assets::StreamIndex::parse(data)
                        && i.file.starts_with("vo_stream")
                    {
                        index = Some(i);
                    }
                }
                CUE_TABLE_TYPE if resident => sets.push(m2_assets::CueTable::parse(data)?),
                _ => {}
            }
        }
    }
    let index = index.ok_or("no voice stream index")?;
    let language = wad_path
        .file_stem()
        .map_or("english".into(), |s| s.to_string_lossy().to_lowercase());
    let file = index.file.replace(".pws", &format!(".{language}.pws"));
    sets.sort_by_key(|t| t.hash);
    Ok(Voices { index, file, sets })
}

/// Reads the music and ambience indexes from the install's `vz.wad`.
fn load_streams(data_dir: &str) -> AnyResult<Streams> {
    use m2_assets::sound::SOUND_BANK_TYPE;
    let dir = std::path::PathBuf::from(data_dir);
    let mut wad = open(&dir.join("vz.wad").to_string_lossy())?;
    let (mut music, mut ambience) = (None, None);
    for block in 0..wad.index().blocks().len() {
        let path = wad.index().blocks()[block].path.clone();
        if !path.contains("music") && !path.contains("sound_resident") {
            continue;
        }
        let decoded = wad.read_block(block)?;
        for (e, data) in BlockContents::parse(&decoded)?.iter() {
            if e.type_hash != SOUND_BANK_TYPE {
                continue;
            }
            let Ok(idx) = m2_assets::StreamIndex::parse(data) else {
                continue;
            };
            match idx.file.as_str() {
                "music.pws" => music = Some(idx),
                "ambience.pws" => ambience = Some(idx),
                _ => {}
            }
        }
    }
    let english = dir.join("English.wad");
    let voice = english
        .exists()
        .then(|| load_voices(&english))
        .transpose()?;
    Ok(Streams {
        dir: dir.join("Audios"),
        music: music.ok_or("no music index in vz.wad")?,
        ambience: ambience.ok_or("no ambience index in vz.wad")?,
        voice,
    })
}

/// Decodes stream `i` of `music`, `ambience` or `voice` to a WAV file.
fn stream_wav(s: &Streams, which: &str, i: usize) -> Option<Vec<u8>> {
    use std::io::SeekFrom;
    let (idx, file) = match which {
        "music" => (&s.music, &s.music.file),
        "ambience" => (&s.ambience, &s.ambience.file),
        "voice" => s.voice.as_ref().map(|v| (&v.index, &v.file))?,
        _ => return None,
    };
    let t = idx.streams.get(i)?;
    let mut f = File::open(s.dir.join(file)).ok()?;
    f.seek(SeekFrom::Start(u64::from(t.offset))).ok()?;
    let mut buf = vec![0u8; t.encoded_len as usize];
    f.read_exact(&mut buf).ok()?;
    let pcm = m2_assets::sound::decode_xbox_adpcm(&buf, t.channels);
    Some(m2_assets::sound::wav_from_samples(&pcm, t.channels, t.rate))
}

/// Serves the viewer page and `glb` on 127.0.0.1 until stopped.
fn view(
    glb: &str,
    world: Option<&str>,
    npcs: Option<&str>,
    sounds: Option<&str>,
    streams: Option<&str>,
    port: u16,
) -> AnyResult<()> {
    use std::io::{BufRead, Write};
    let page = include_str!("gltf_viewer.html");
    let streams = streams.map(load_streams).transpose()?;
    if let Some(s) = &streams {
        println!(
            "streaming {} music tracks, {} ambience recordings and {} voice lines in {} voice sets from {}",
            s.music.streams.len(),
            s.ambience.streams.len(),
            s.voice.as_ref().map_or(0, |v| v.index.streams.len()),
            s.voice.as_ref().map_or(0, |v| v.sets.len()),
            s.dir.display()
        );
    }
    let model = std::fs::read(glb)?;
    let world = world.map(std::fs::read).transpose()?;
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
        // NPCs from `gltf npcs`: the list and the model files, read per
        // request; only names the export writes are served.
        let npc_file = npcs.and_then(|dir| {
            let name = match path {
                "/npcs.json" => "npcs.json",
                p => p.strip_prefix("/npc/").filter(|n| {
                    n.len() == 16
                        && (n.starts_with("npc_") || n.starts_with("wpn_"))
                        && n.ends_with(".glb")
                        && n[4..12].chars().all(|c| c.is_ascii_hexdigit())
                })?,
            };
            std::fs::read(std::path::Path::new(dir).join(name)).ok()
        });
        // Sounds from `gltf sounds`: the list and WAV files named as written.
        let sound_file = sounds.and_then(|dir| {
            let name = match path {
                "/sounds.json" => "sounds.json",
                p => p.strip_prefix("/snd/").filter(|n| {
                    n.starts_with("snd_")
                        && n.ends_with(".wav")
                        && n.len() < 80
                        && n.chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
                })?,
            };
            std::fs::read(std::path::Path::new(dir).join(name)).ok()
        });
        // Music and ambience, decoded from the install when asked for.
        let stream_file = streams.as_ref().and_then(|s| {
            if path == "/streams.json" {
                let secs = |i: &m2_assets::StreamIndex| {
                    i.streams
                        .iter()
                        .map(|t| format!("{:.1}", t.seconds()))
                        .collect::<Vec<_>>()
                        .join(",")
                };
                // Each voice set's positional lines: [stream, seconds,
                // fade-from and silent-beyond distances].
                let voices = s.voice.as_ref().map_or(String::new(), |v| {
                    v.sets
                        .iter()
                        .map(|t| {
                            let lines: Vec<String> = t
                                .lines
                                .iter()
                                .filter(|l| l.is_positional())
                                .filter_map(|l| {
                                    let st = v.index.streams.get(l.stream as usize)?;
                                    Some(format!(
                                        "[{},{:.1},{},{}]",
                                        l.stream,
                                        st.seconds(),
                                        l.min_distance,
                                        l.max_distance
                                    ))
                                })
                                .collect();
                            format!(
                                r#"{{"set":"{:08x}","lines":[{}]}}"#,
                                t.hash,
                                lines.join(",")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(",")
                });
                return Some(
                    format!(
                        r#"{{"music":[{}],"ambience":[{}],"voices":[{voices}]}}"#,
                        secs(&s.music),
                        secs(&s.ambience)
                    )
                    .into_bytes(),
                );
            }
            let rest = path.strip_prefix("/stream/")?.strip_suffix(".wav")?;
            let (which, i) = rest.split_once('/')?;
            stream_wav(s, which, i.parse().ok()?)
        });
        if let Some(f) = &stream_file {
            let ty = if path.ends_with(".json") {
                "application/json"
            } else {
                "audio/wav"
            };
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {ty}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
                f.len()
            );
            let _ = stream
                .write_all(head.as_bytes())
                .and_then(|()| stream.write_all(f));
            continue;
        }
        let (status, ty, body): (&str, &str, &[u8]) = match (path, &npc_file, &sound_file) {
            ("/npcs.json", Some(f), _) => ("200 OK", "application/json", f),
            (_, Some(f), _) => ("200 OK", "model/gltf-binary", f),
            ("/sounds.json", _, Some(f)) => ("200 OK", "application/json", f),
            (_, _, Some(f)) => ("200 OK", "audio/wav", f),
            _ => match path {
                "/" | "/index.html" => ("200 OK", "text/html; charset=utf-8", page.as_bytes()),
                "/model.glb" => ("200 OK", "model/gltf-binary", &model),
                "/world.glb" if world.is_some() => (
                    "200 OK",
                    "model/gltf-binary",
                    world.as_deref().unwrap_or_default(),
                ),
                _ => ("404 Not Found", "text/plain", b"not found"),
            },
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

struct NpcOptions {
    /// Centre (game x, z) of the area to take spawn points from.
    detail: [f32; 2],
    radius: f32,
    /// How many character models to export.
    models: usize,
}

/// Root node name of the shared human skeleton (Jennifer's and every
/// skinned human model's), so the character's clips drive them all.
const HUMAN_ROOT: u32 = 0xcbc1_eb51;

/// Writes `npc_<hash>.glb` character models (skinned, textured, without
/// clips: the viewer drives them with the playable character's clips) and
/// `npcs.json`: the models, and spawn points from layer entities that
/// place people (`PopulationSimpleSpawner`, `Ai`, `AiPatrol`) near
/// `detail`, with the patrol route of each that has one. Models are chosen by
/// texture name (`<faction>_hum_<role>_..`), civilians first; which model a
/// spawner really spawns is not decoded.
fn export_npcs<R: Read + Seek>(
    wad: &mut WadArchive<R>,
    dir: &str,
    o: &NpcOptions,
) -> AnyResult<()> {
    use m2_assets::layer::LAYER_TYPE;
    let mut texture_names: HashMap<u32, String> = HashMap::new();
    let mut humans: Vec<(u32, Vec<u32>)> = Vec::new();
    // Small static models: (hash, textures, nodes), for hand weapons.
    let mut weapons: Vec<(u32, Vec<u32>, usize)> = Vec::new();
    // (glTF position, entity name, component, entity)
    let mut spawns: Vec<([f32; 3], String, &'static str, u32)> = Vec::new();
    // Patrol routes: soldier -> path entity (AiPatrol's third field), path
    // entity -> path asset (Path's field), path assets, and every entity's
    // placement.
    let mut patrol_of: HashMap<u32, u32> = HashMap::new();
    let mut path_of: HashMap<u32, u32> = HashMap::new();
    let mut paths: HashMap<u32, Vec<[f32; 3]>> = HashMap::new();
    let mut placed_at: HashMap<u32, m2_assets::Placement> = HashMap::new();
    let mut roads: Vec<(u32, u32, u32, u32)> = Vec::new();
    let mut intersections: HashMap<u32, Vec<u32>> = HashMap::new();
    for block in 0..wad.index().blocks().len() {
        let decoded = wad.read_block(block)?;
        for (e, data) in BlockContents::parse(&decoded)?.iter() {
            match e.type_hash {
                TEXTURE_TYPE => {
                    if let Some(n) = Ucfx::parse(data).ok().and_then(|u| u.name()) {
                        texture_names.insert(e.name_hash, n.to_ascii_lowercase());
                    }
                }
                MODEL_TYPE if e.unk_0x08 == 0 => {
                    let Ok(m) = Model::parse(data) else { continue };
                    let b = &m.info.bounds;
                    let size = (0..3).map(|a| b.max[a] - b.min[a]).fold(0.0f32, f32::max);
                    if m.info.flags & 0x800 == 0 && (0.05..1.6).contains(&size) {
                        weapons.push((
                            e.name_hash,
                            m.materials
                                .iter()
                                .flat_map(|mt| mt.textures.iter().copied())
                                .collect(),
                            m.nodes.len(),
                        ));
                    }
                    if m.info.flags & 0x800 != 0
                        && m.nodes.first().map(|n| n.name_hash) == Some(HUMAN_ROOT)
                    {
                        humans.push((
                            e.name_hash,
                            m.materials
                                .iter()
                                .flat_map(|mt| mt.textures.iter().copied())
                                .collect(),
                        ));
                    }
                }
                LAYER_TYPE => {
                    let Ok(layer) = m2_assets::Layer::parse(data) else {
                        continue;
                    };
                    let names = layer.names();
                    let places = layer.placements();
                    let word = |rec: &[u8], at: usize| {
                        rec.get(at..at + 4)
                            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    };
                    if let Some(c) = layer.component("AiPatrol") {
                        for (id, rec) in c.records().unwrap_or_default() {
                            if let Some(t) = word(rec, 8) {
                                patrol_of.insert(id, t);
                            }
                        }
                    }
                    if let Some(c) = layer.component("Path") {
                        for (id, rec) in c.records().unwrap_or_default() {
                            if let Some(h) = word(rec, 0) {
                                path_of.insert(id, h);
                            }
                        }
                    }
                    // Roads: centreline path asset, then the intersections
                    // at its ends (0 where it joins another road directly).
                    if let Some(c) = layer.component("Road") {
                        for (id, rec) in c.records().unwrap_or_default() {
                            if let (Some(p), Some(a), Some(b)) =
                                (word(rec, 0), word(rec, 8), word(rec, 12))
                            {
                                roads.push((id, p, a, b));
                            }
                        }
                    }
                    if let Some(c) = layer.component("RoadIntersection") {
                        for (id, rec) in c.records().unwrap_or_default() {
                            let joined: Vec<u32> = (0..4)
                                .filter_map(|i| word(rec, i * 4))
                                .filter(|&r| r != 0)
                                .collect();
                            intersections.insert(id, joined);
                        }
                    }
                    placed_at.extend(places.iter().map(|(&id, p)| (id, *p)));
                    for component in ["PopulationSimpleSpawner", "Ai", "AiPatrol"] {
                        let Some(c) = layer.component(component) else {
                            continue;
                        };
                        for (id, _) in c.records().unwrap_or_default() {
                            let Some(p) = places.get(&id) else { continue };
                            let [x, y, z] = p.position;
                            if (x - o.detail[0]).hypot(z - o.detail[1]) > o.radius
                                || spawns
                                    .iter()
                                    .any(|s| s.1 == names.get(&id).copied().unwrap_or(""))
                            {
                                continue;
                            }
                            let name = names.get(&id).copied().unwrap_or("-").to_owned();
                            spawns.push((flip_v([x, y, z]), name, component, id));
                        }
                    }
                }
                m2_assets::path::PATH_TYPE => {
                    if let Ok(p) = m2_assets::Path::parse(data) {
                        paths.insert(e.name_hash, p.points);
                    }
                }
                _ => {}
            }
        }
    }
    // Models: one per role, civilians first, then soldiers; not the
    // player characters.
    let mut roles: Vec<(bool, String, u32)> = Vec::new();
    for (hash, textures) in &humans {
        let Some(name) = textures
            .iter()
            .filter_map(|t| texture_names.get(t))
            .find(|n| n.contains("_hum_"))
        else {
            continue;
        };
        let words: Vec<&str> = name.split('_').collect();
        if words.len() < 3 || words[0] == "pmc" || name.contains("chicken") {
            continue;
        }
        let role = words[..3].join("_");
        if roles.iter().any(|r| r.1 == role) {
            continue;
        }
        roles.push((words[0] != "civ", role, *hash));
    }
    roles.sort();
    let civilians = roles.iter().filter(|r| !r.0).count();
    // The army roles the spawn names ask for ("VZ Soldier", "VZ Heavy ...",
    // "VZ Deathsquad ..."), then civilians for the rest.
    let army = ["vz_hum_soldier", "vz_hum_soldierelite", "vz_hum_deathsquad"];
    let chosen: Vec<&(bool, String, u32)> = roles
        .iter()
        .filter(|r| army.contains(&r.1.as_str()))
        .chain(roles.iter().filter(|r| !r.0))
        .take(o.models.max(army.len() + 1))
        .collect();
    // A spawn's model, by its entity name; spawns of vehicles, targets and
    // buildings, and of other armies, are left out.
    let model_for = |name: &str| -> Option<usize> {
        let n = name.to_ascii_lowercase();
        let other = [
            "amx30", "scorpion", "hind", "m151", "boat", "target", "_bld_", "heli", "tank", "truck",
        ];
        if other.iter().any(|w| n.contains(w)) || !n.starts_with("vz ") {
            return None;
        }
        let role = if n.contains("deathsquad") || n.contains("mook") {
            "vz_hum_deathsquad"
        } else if n.contains("heavy") || n.contains("elite") {
            "vz_hum_soldierelite"
        } else {
            "vz_hum_soldier"
        };
        chosen.iter().position(|r| r.1 == role)
    };
    std::fs::create_dir_all(dir)?;
    let mut models_json = Vec::new();
    for (soldier, role, hash) in &chosen {
        let glb = export(
            wad,
            *hash,
            &Options {
                clips: 0,
                filter: None,
                lod: 0,
            },
        )?;
        let file = format!("npc_{hash:08x}.glb");
        std::fs::write(std::path::Path::new(dir).join(&file), glb)?;
        models_json.push(format!(
            r#"{{"file":"{file}","name":"{role}","soldier":{soldier}}}"#
        ));
    }
    // Weapons, by texture name: the AK-47 everyone carries, the RPG
    // launcher, its rocket and a frag grenade; with the muzzle marker (node
    // b1486ef3; the grip is the model's origin) where there is one.
    let mut weapons_json = Vec::new();
    let mut weapons_used = Vec::new();
    for (name, texture) in [
        ("ak47", "global_weapon_ak47"),
        ("rpg", "global_weapon_rpg"),
        ("rocket", "global_weapon_rpgrocket"),
        ("grenade", "global_weapon_fraggrenade"),
    ] {
        // The launcher carries the rocket's texture too: one model per name.
        let found = weapons
            .iter()
            .filter(|w| !weapons_used.contains(&w.0))
            .filter(|w| {
                w.1.iter()
                    .filter_map(|t| texture_names.get(t))
                    .any(|n| n == texture || n.starts_with(&format!("{texture}_")))
            })
            .max_by_key(|w| w.2);
        let Some(&(hash, _, _)) = found else { continue };
        weapons_used.push(hash);
        let (base, chunks) = load_model(wad, hash)?;
        let mut model = Model::parse(&base)?;
        for c in &chunks {
            model.add_streamed(c)?;
        }
        let muzzle = m2_assets::vehicle::marker(&model, 0xb148_6ef3).unwrap_or([
            0.0,
            0.0,
            model.info.bounds.max[2],
        ]);
        let glb = export(
            wad,
            hash,
            &Options {
                clips: 0,
                filter: None,
                lod: 0,
            },
        )?;
        let file = format!("wpn_{hash:08x}.glb");
        std::fs::write(std::path::Path::new(dir).join(&file), glb)?;
        weapons_json.push(format!(
            r#"{{"file":"{file}","name":"{name}","muzzle":{:?}}}"#,
            flip_v(muzzle)
        ));
    }
    let clean = |s: &str| {
        s.chars()
            .filter(|c| *c != '"' && *c != '\\' && !c.is_control())
            .collect::<String>()
    };
    // A path's points placed by the entity that names it (game axes).
    let placed = |place: &m2_assets::Placement, points: &[[f32; 3]]| -> Vec<[f32; 3]> {
        let [qx, qy, qz, qw] = place.rotation;
        let q = Quat::from_xyzw(qx, qy, qz, qw).normalize();
        let at = Vec3::from(place.position);
        points
            .iter()
            .map(|p| (at + q * Vec3::from(*p)).to_array())
            .collect()
    };
    let points_json = |pts: &[[f32; 3]]| {
        pts.iter()
            .map(|p| format!("{:?}", flip_v(*p)))
            .collect::<Vec<_>>()
            .join(",")
    };
    let spawns_json: Vec<String> = spawns
        .iter()
        .filter_map(|(at, name, from, id)| {
            let model = model_for(name)?;
            // The patrol route, placed by its path entity, in glTF axes.
            let route = patrol_of
                .get(id)
                .and_then(|target| Some((placed_at.get(target)?, paths.get(path_of.get(target)?)?)))
                .map(|(place, points)| {
                    format!(r#","route":[{}]"#, points_json(&placed(place, points)))
                })
                .unwrap_or_default();
            Some(format!(
                r#"{{"at":{at:?},"name":"{}","from":"{from}","model":{model}{route}}}"#,
                clean(name)
            ))
        })
        .collect();
    let routes = spawns_json
        .iter()
        .filter(|s| s.contains("\"route\""))
        .count();
    // Roads near `detail`: centrelines in glTF axes, and the intersections
    // (by entity id) at their two ends, 0 where none is named.
    let near = |p: &[f32; 3]| (p[0] - o.detail[0]).hypot(p[2] - o.detail[1]) <= o.radius;
    let mut road_lines: Vec<(u32, u32, u32, Vec<[f32; 3]>)> = Vec::new();
    for &(id, path, a, b) in &roads {
        let (Some(place), Some(points)) = (placed_at.get(&id), paths.get(&path)) else {
            continue;
        };
        let pts = placed(place, points);
        if pts.len() >= 2 && pts.iter().any(near) {
            road_lines.push((id, a, b, pts));
        }
    }
    // How the data fits: does the first point end at intersection `a`, and
    // do the other ends meet another road's end?
    let (mut a_first, mut a_named) = (0, 0);
    for (_, a, _, pts) in &road_lines {
        if let Some(p) = placed_at.get(a) {
            a_named += 1;
            let at = Vec3::from(p.position);
            let (first, last) = (Vec3::from(pts[0]), Vec3::from(pts[pts.len() - 1]));
            if first.distance(at) < last.distance(at) {
                a_first += 1;
            }
        }
    }
    let ends: Vec<Vec3> = road_lines
        .iter()
        .flat_map(|r| [Vec3::from(r.3[0]), Vec3::from(r.3[r.3.len() - 1])])
        .collect();
    let met = ends
        .iter()
        .enumerate()
        .filter(|&(i, e)| {
            ends.iter()
                .enumerate()
                .any(|(j, f)| j / 2 != i / 2 && e.distance(*f) < 1.5)
        })
        .count();
    let roads_json: Vec<String> = road_lines
        .iter()
        .map(|(id, a, b, pts)| {
            format!(
                r#"{{"id":{id},"a":{a},"b":{b},"points":[{}]}}"#,
                points_json(pts)
            )
        })
        .collect();
    let junctions_json: Vec<String> = intersections
        .iter()
        .filter(|(_, joined)| joined.iter().any(|r| road_lines.iter().any(|l| l.0 == *r)))
        .filter_map(|(id, joined)| {
            let at = flip_v(placed_at.get(id)?.position);
            Some(format!(r#"{{"id":{id},"at":{at:?},"roads":{joined:?}}}"#))
        })
        .collect();
    std::fs::write(
        std::path::Path::new(dir).join("npcs.json"),
        format!(
            "{{\"models\":[{}],\n\"weapons\":[{}],\n\"spawns\":[{}],\n\"roads\":[{}],\n\"junctions\":[{}]}}\n",
            models_json.join(","),
            weapons_json.join(","),
            spawns_json.join(",\n"),
            roads_json.join(",\n"),
            junctions_json.join(",\n")
        ),
    )?;
    println!(
        "{} roads within {} m ({} in all): first point at intersection a on {a_first} of {a_named}; {met} of {} road ends meet another road's end; {} intersections",
        road_lines.len(),
        o.radius,
        roads.len(),
        ends.len(),
        junctions_json.len()
    );
    println!(
        "{} human models ({} roles, {civilians} civilian); exported {}: {}",
        humans.len(),
        roles.len(),
        chosen.len(),
        chosen
            .iter()
            .map(|r| r.1.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mut from: BTreeMap<&str, usize> = BTreeMap::new();
    for s in &spawns {
        *from.entry(s.2).or_default() += 1;
    }
    println!(
        "{} spawn points within {} m: {from:?}; {} placed with a soldier model, {routes} with a patrol route ({} paths in all)",
        spawns.len(),
        o.radius,
        spawns_json.len(),
        paths.len()
    );
    Ok(())
}

/// Sound banks the viewer uses, by block name: the rifle, vehicle engines,
/// town ambience, the RPG and explosions.
const SOUND_BANKS: [&str; 13] = [
    "wpn_assaultrifle",
    "veh_4cylindernew",
    "veh_jeep",
    "veh_8cylinderold",
    "veh_tankbig",
    "veh_tankmed",
    "veh_semi",
    "veh_helicoptermedium",
    "amb_citymed",
    "amb_jungle",
    "wpn_rpg",
    "bulding_destruct",
    "support_artillery",
];

/// Writes the sounds of `SOUND_BANKS` as WAV files and `sounds.json`
/// describing them. A sound's role within its bank is named only by hash,
/// so each is described by measurements the viewer picks from: length,
/// loudness, and whether it loops (as loud at its end as at its start).
fn export_sounds<R: Read + Seek>(wad: &mut WadArchive<R>, dir: &str) -> AnyResult<()> {
    use m2_assets::sound::{SOUND_BANK_TYPE, SoundBank};
    std::fs::create_dir_all(dir)?;
    let mut banks_json = Vec::new();
    let (mut files, mut bytes) = (0, 0);
    for block in 0..wad.index().blocks().len() {
        let name = block_name(&wad.index().blocks()[block].path).to_owned();
        if !SOUND_BANKS.contains(&name.as_str()) {
            continue;
        }
        let decoded = wad.read_block(block)?;
        for (e, data) in BlockContents::parse(&decoded)?.iter() {
            if e.type_hash != SOUND_BANK_TYPE {
                continue;
            }
            let bank = SoundBank::parse(data)?;
            let mut sounds = Vec::new();
            for s in &bank.sounds {
                let v: Vec<f32> = s
                    .pcm
                    .chunks_exact(2)
                    .map(|c| f32::from(i16::from_le_bytes([c[0], c[1]])) / 32768.0)
                    .collect();
                let rms = |x: &[f32]| {
                    (x.iter().map(|a| a * a).sum::<f32>() / x.len().max(1) as f32).sqrt()
                };
                let edge = (s.rate as usize / 10 * usize::from(s.channels)).min(v.len());
                let (all, head, tail) = (rms(&v), rms(&v[..edge]), rms(&v[v.len() - edge..]));
                let loops =
                    head > 0.05 && tail > 0.05 && (head - tail).abs() < 0.4 * head.max(tail);
                let file = format!("snd_{name}_{:08x}.wav", s.hash);
                let wav = s.wav();
                bytes += wav.len();
                files += 1;
                std::fs::write(std::path::Path::new(dir).join(&file), wav)?;
                sounds.push(format!(
                    r#"{{"file":"{file}","seconds":{:.3},"channels":{},"rms":{all:.3},"loops":{loops}}}"#,
                    s.seconds(),
                    s.channels
                ));
            }
            banks_json.push(format!(r#""{name}":[{}]"#, sounds.join(",")));
        }
    }
    std::fs::write(
        std::path::Path::new(dir).join("sounds.json"),
        format!("{{{}}}\n", banks_json.join(",\n")),
    )?;
    println!(
        "{} banks, {files} sounds, {} MB of WAV in {dir}",
        banks_json.len(),
        bytes / (1 << 20)
    );
    Ok(())
}

struct WorldOptions {
    /// Centre of the full-detail area in game coordinates (x, z).
    detail: Option<[f32; 2]>,
    radius: f32,
    objects: Objects,
    /// Sea surface height. An estimate: towns and coastal flats sit at about
    /// -33 m, so the water is just below; where the game stores it is not
    /// known yet.
    sea: f32,
    /// How many drivable vehicles to park near the detail point.
    vehicles: usize,
}

/// Which placed models to export.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Objects {
    /// Only `layers_static`, the world as it always is.
    Static,
    /// Every layer, mission states included (they add alternate versions).
    All,
    None,
}

/// Side of the island in metres (20 cells of 400 m).
const ISLAND: f32 = 8000.0;

/// A world-space mesh piece: positions and normals already mirrored for
/// glTF, triangles in glTF winding, texture coordinates for its colour map.
struct Piece {
    name: String,
    /// Texture hash of the colour map to draw it with.
    map: u32,
    positions: Vec<f32>,
    normals: Vec<f32>,
    uvs: Vec<f32>,
    indices: Vec<u16>,
}

/// `origin` places the group in the world; `uv` maps a vertex's world
/// position (game axes) to its colour-map coordinates.
fn piece(
    name: String,
    map: u32,
    origin: [f32; 3],
    group: &m2_assets::terrain::TerrainGroup<'_>,
    uv: impl Fn([f32; 3]) -> [f32; 2],
) -> Piece {
    let pos = group.positions();
    let nrm = group
        .decl
        .find(DeclUsage::Normal, 0)
        .and_then(|e| group.vertices.read(e))
        .unwrap_or_default();
    let (mut positions, mut normals, mut uvs) = (Vec::new(), Vec::new(), Vec::new());
    for (i, p) in pos.iter().enumerate() {
        let world = [p[0] + origin[0], p[1] + origin[1], p[2] + origin[2]];
        let n = nrm
            .get(i)
            .map(|n| {
                Vec3::new(n[0], n[1], n[2])
                    .try_normalize()
                    .unwrap_or(Vec3::Y)
            })
            .unwrap_or(Vec3::Y);
        positions.extend(flip_v(world));
        normals.extend(flip_v(n.to_array()));
        uvs.extend(uv(world));
    }
    // Full ranges draw the whole group; fall back to the material ranges.
    let ranges = if group.full_ranges.is_empty() {
        &group.material_ranges
    } else {
        &group.full_ranges
    };
    let mut seen = HashSet::new();
    let mut indices = Vec::new();
    for r in ranges {
        for [a, b, c] in group.triangles(r) {
            let mut key = [a, b, c];
            key.sort_unstable();
            if seen.insert(key) {
                indices.extend([a, c, b]);
            }
        }
    }
    Piece {
        name,
        map,
        positions,
        normals,
        uvs,
        indices,
    }
}

/// Where model primitives go: the glTF buffers, images and materials of a
/// world export, and the materials already made for model textures.
struct Sink<'a> {
    g: &'a mut Gltf,
    images: &'a mut Vec<String>,
    image_sampler: &'a mut Vec<u8>,
    materials: &'a mut Vec<String>,
    by_texture: &'a mut HashMap<u32, usize>,
    untextured: &'a mut Option<usize>,
}

/// glTF primitives for `model`'s finest complete level of detail, split
/// into `groups` lists. `group(node)` gives the list for a segment on
/// `node` and the matrix taking model space into that list's space, or
/// `None` to leave the segment out. Also returns the triangle count.
fn model_primitives<R: Read + Seek>(
    wad: &mut WadArchive<R>,
    s: &mut Sink,
    model: &Model,
    groups: usize,
    group: &dyn Fn(u16) -> Option<(usize, Mat4)>,
) -> (Vec<Vec<String>>, usize) {
    let mut out = vec![Vec::new(); groups];
    let mut triangles = 0;
    let lod = (0..model.info.lod_count)
        .find(|&l| model.has_lod(l))
        .unwrap_or(0);
    for (segment, geom) in model.lod(lod) {
        let Some((list, post)) = group(segment.node) else {
            continue;
        };
        // Skinned positions are already in model space.
        let to_model = match geom.kind {
            GeometryKind::Skin => Mat4::IDENTITY,
            _ => model.node_to_model(segment.node).unwrap_or(Mat4::IDENTITY),
        };
        let to_group = post * to_model;
        for grp in &geom.groups {
            let read = |u| grp.decl.find(u, 0).and_then(|e| grp.vertices.read(e));
            let Some(pos) = read(DeclUsage::Position) else {
                continue;
            };
            let p: Vec<f32> = pos
                .iter()
                .flat_map(|v| {
                    flip_v(
                        to_group
                            .transform_point3(Vec3::new(v[0], v[1], v[2]))
                            .to_array(),
                    )
                })
                .collect();
            let mut attrs = format!(r#""POSITION":{}"#, s.g.floats(&p, 3, true, Some(34962)));
            if let Some(nrm) = read(DeclUsage::Normal) {
                let n: Vec<f32> = nrm
                    .iter()
                    .flat_map(|v| {
                        let n = to_group.transform_vector3(Vec3::new(v[0], v[1], v[2]));
                        flip_v(n.try_normalize().unwrap_or(Vec3::Y).to_array())
                    })
                    .collect();
                let _ = write!(
                    attrs,
                    r#","NORMAL":{}"#,
                    s.g.floats(&n, 3, false, Some(34962))
                );
            }
            if let Some(uv) = read(DeclUsage::TexCoord) {
                let t: Vec<f32> = uv.iter().flat_map(|v| [v[0], v[1]]).collect();
                let _ = write!(
                    attrs,
                    r#","TEXCOORD_0":{}"#,
                    s.g.floats(&t, 2, false, Some(34962))
                );
            }
            for range in &grp.material_ranges {
                let tris: Vec<u16> = grp
                    .triangles(range)
                    .into_iter()
                    .flat_map(|[a, b, c]| [a, c, b])
                    .collect();
                if tris.is_empty() {
                    continue;
                }
                triangles += tris.len() / 3;
                let ind = s.g.u16s(&tris, 1, Some(34963));
                let texture = model
                    .materials
                    .get(range.material as usize)
                    .and_then(|m| m.textures.first().copied())
                    .filter(|&t| t != 0);
                let material = match texture.and_then(|t| s.by_texture.get(&t).copied()) {
                    Some(m) => m,
                    None => {
                        let loaded =
                            texture.and_then(|t| load_texture(wad, t, 256).map(|x| (t, x)));
                        match loaded {
                            Some((t, (w, h, rgba, alpha))) => {
                                let view = s.g.view(&png(w, h, &rgba), None);
                                s.images.push(format!(
                                    r#"{{"bufferView":{view},"mimeType":"image/png","name":"{t:08x}"}}"#
                                ));
                                s.image_sampler.push(1);
                                s.materials.push(format!(
                                    r#"{{"name":"{t:08x}","pbrMetallicRoughness":{{"baseColorTexture":{{"index":{}}},"metallicFactor":0,"roughnessFactor":0.9}}{}}}"#,
                                    s.images.len() - 1,
                                    if alpha { r#","alphaMode":"MASK","alphaCutoff":0.5,"doubleSided":true"# } else { "" }
                                ));
                                s.by_texture.insert(t, s.materials.len() - 1);
                                s.materials.len() - 1
                            }
                            None => *s.untextured.get_or_insert_with(|| {
                                s.materials.push(r#"{"name":"untextured object","pbrMetallicRoughness":{"baseColorFactor":[0.6,0.6,0.6,1],"metallicFactor":0,"roughnessFactor":0.9}}"#.to_owned());
                                s.materials.len() - 1
                            }),
                        }
                    }
                };
                out[list].push(format!(
                    r#"{{"attributes":{{{attrs}}},"indices":{ind},"material":{material}}}"#
                ));
            }
        }
    }
    (out, triangles)
}

fn export_world<R: Read + Seek>(wad: &mut WadArchive<R>, o: &WorldOptions) -> AnyResult<Vec<u8>> {
    use m2_assets::layer::LAYER_TYPE;
    use m2_assets::terrain::{CELL_SIZE, LOW_RES_TYPE, TERRAIN_TYPE, colour_map_uv};
    use m2_assets::{Layer, LowResTile, TerrainCell};

    // The layout layer: positions of every cell and low-res tile.
    let mut cell_at: HashMap<u32, [f32; 3]> = HashMap::new();
    let mut tile_at: HashMap<u32, [f32; 3]> = HashMap::new();
    let (mut tiles, mut cells) = (HashMap::new(), HashMap::new());
    // Placed models: (model hash, position, rotation).
    let mut placed: Vec<(u32, [f32; 3], [f32; 4])> = Vec::new();
    // Scatter assets by the terrain cell sharing their block.
    let mut scatter_of: Vec<(u32, Vec<u8>)> = Vec::new();
    // Texture names, and the texture hashes of every model, to find vehicles.
    let mut texture_names: HashMap<u32, String> = HashMap::new();
    let mut model_textures: Vec<(u32, Vec<u32>)> = Vec::new();
    for block in 0..wad.index().blocks().len() {
        let is_static = wad.index().blocks()[block].path.contains("layers_static");
        let (mut block_cell, mut block_scatter) = (None, Vec::new());
        let decoded = wad.read_block(block)?;
        for (e, data) in BlockContents::parse(&decoded)?.iter() {
            match e.type_hash {
                TEXTURE_TYPE if o.vehicles > 0 => {
                    if let Some(n) = m2_assets::Ucfx::parse(data).ok().and_then(|u| u.name()) {
                        texture_names.insert(e.name_hash, n.to_ascii_lowercase());
                    }
                }
                MODEL_TYPE if o.vehicles > 0 && e.unk_0x08 == 0 => {
                    if let Ok(m) = Model::parse(data) {
                        model_textures.push((
                            e.name_hash,
                            m.materials
                                .iter()
                                .flat_map(|mt| mt.textures.iter().copied())
                                .collect(),
                        ));
                    }
                }
                LAYER_TYPE => {
                    let layer = Layer::parse(data)?;
                    let placements = layer.placements();
                    let wanted = match o.objects {
                        Objects::Static => is_static,
                        Objects::All => true,
                        Objects::None => false,
                    };
                    if wanted {
                        for (entity, model) in layer.hashes("ModelName") {
                            if let Some(p) = placements.get(&entity) {
                                placed.push((model, p.position, p.rotation));
                            }
                        }
                    }
                    for (out, component) in [
                        (&mut cell_at, "TerrainObject"),
                        (&mut tile_at, "LowResTerrainObject"),
                    ] {
                        for (entity, asset) in layer.hashes(component) {
                            if let Some(p) = placements.get(&entity) {
                                out.insert(asset, p.position);
                            }
                        }
                    }
                }
                LOW_RES_TYPE => {
                    tiles.insert(e.name_hash, data.to_vec());
                }
                TERRAIN_TYPE => {
                    cells.insert(e.name_hash, data.to_vec());
                    block_cell = Some(e.name_hash);
                }
                m2_assets::scatter::SCATTER_TYPE => block_scatter.push(data.to_vec()),
                _ => {}
            }
        }
        if let Some(cell) = block_cell {
            scatter_of.extend(block_scatter.into_iter().map(|s| (cell, s)));
        }
    }
    let near = |c: [f32; 3]| {
        o.detail.is_some_and(|[x, z]| {
            // Distance from the point to the cell's square.
            let dx = ((c[0] - x).abs() - CELL_SIZE / 2.0).max(0.0);
            let dz = ((c[2] - z).abs() - CELL_SIZE / 2.0).max(0.0);
            (dx * dx + dz * dz).sqrt() <= o.radius
        })
    };

    let mut pieces = Vec::new();
    let mut detailed = 0;
    for (hash, data) in &cells {
        let Some(&centre) = cell_at.get(hash) else {
            continue;
        };
        if !near(centre) {
            continue;
        }
        let cell = TerrainCell::parse(data)?;
        let map = cell.colour_map().unwrap_or(0);
        detailed += 1;
        for (pi, patch) in cell.patches.iter().enumerate() {
            let origin = [
                centre[0] + patch.offset[0],
                centre[1] + patch.offset[1],
                centre[2] + patch.offset[2],
            ];
            for (gi, g) in patch.groups.iter().enumerate() {
                pieces.push(piece(
                    format!("cell {hash:08x} patch {pi} group {gi}"),
                    map,
                    origin,
                    g,
                    |w| colour_map_uv(w[0] - centre[0], w[2] - centre[2]),
                ));
            }
        }
    }
    // Low-res tiles everywhere else, drawn with the island-wide map; a
    // detailed cell's tile has the same centre, so skip those.
    let detailed_centres: Vec<[f32; 3]> = cell_at.values().copied().filter(|c| near(*c)).collect();
    for (hash, data) in &tiles {
        let Some(&centre) = tile_at.get(hash) else {
            continue;
        };
        let covered = detailed_centres
            .iter()
            .any(|c| (c[0] - centre[0]).abs() < 1.0 && (c[2] - centre[2]).abs() < 1.0);
        if covered {
            continue;
        }
        let tile = LowResTile::parse(data)?;
        let map = tile
            .materials
            .first()
            .and_then(|m| m.textures.first().copied())
            .unwrap_or(0);
        // The island map follows the cell maps' axes at island scale.
        pieces.push(piece(
            format!("tile {hash:08x}"),
            map,
            centre,
            &tile.group,
            |w| {
                [
                    (ISLAND / 2.0 - w[0]) / ISLAND,
                    (ISLAND / 2.0 - w[2]) / ISLAND,
                ]
            },
        ));
    }

    let mut g = Gltf::default();
    // One material per colour map; the island map stays at full size, the
    // cell maps likewise (512 × 512).
    let mut material_of: HashMap<u32, usize> = HashMap::new();
    let mut materials = Vec::new();
    let mut images = Vec::new();
    for p in &pieces {
        if material_of.contains_key(&p.map) {
            continue;
        }
        let texture = load_texture(wad, p.map, 2048).map(|(w, h, rgba, _)| {
            let view = g.view(&png(w, h, &rgba), None);
            images.push(format!(
                r#"{{"bufferView":{view},"mimeType":"image/png","name":"{:08x}"}}"#,
                p.map
            ));
            images.len() - 1
        });
        materials.push(match texture {
            Some(i) => format!(
                r#"{{"name":"{:08x}","pbrMetallicRoughness":{{"baseColorTexture":{{"index":{i}}},"metallicFactor":0,"roughnessFactor":1}}}}"#,
                p.map
            ),
            None => r#"{"name":"untextured","pbrMetallicRoughness":{"baseColorFactor":[0.3,0.4,0.2,1],"metallicFactor":0,"roughnessFactor":1}}"#.to_owned(),
        });
        material_of.insert(p.map, materials.len() - 1);
    }
    let sea = materials.len();
    materials.push(r#"{"name":"sea","pbrMetallicRoughness":{"baseColorFactor":[0.10,0.30,0.42,0.75],"metallicFactor":0,"roughnessFactor":0.2},"alphaMode":"BLEND","doubleSided":true}"#.to_owned());

    let mut nodes = Vec::new();
    let mut meshes = Vec::new();
    for p in &pieces {
        if p.indices.is_empty() {
            continue;
        }
        let pos = g.floats(&p.positions, 3, true, Some(34962));
        let nrm = g.floats(&p.normals, 3, false, Some(34962));
        let uv = g.floats(&p.uvs, 2, false, Some(34962));
        let ind = g.u16s(&p.indices, 1, Some(34963));
        meshes.push(format!(
            r#"{{"primitives":[{{"attributes":{{"POSITION":{pos},"NORMAL":{nrm},"TEXCOORD_0":{uv}}},"indices":{ind},"material":{}}}]}}"#,
            material_of[&p.map]
        ));
        nodes.push(format!(
            r#"{{"name":"{}","mesh":{}}}"#,
            p.name,
            meshes.len() - 1
        ));
    }
    // Placed models: one glTF mesh per model at its finest complete level
    // of detail, one node per placement (named "obj ..." so the viewer can
    // hide distant ones). Object textures repeat, so they use sampler 1.
    // A model's collision (its PHY2 shapes) is a second mesh, placed as a
    // child node "col ..." of each of its placements.
    let mut image_sampler = vec![0u8; images.len()];
    let mut mesh_of_model: HashMap<u32, (Option<usize>, Option<usize>)> = HashMap::new();
    let mut material_of_texture: HashMap<u32, usize> = HashMap::new();
    let mut untextured = None;
    let mut collision_material = None;
    let mut child_nodes = std::collections::HashSet::new();
    let (mut object_count, mut object_triangles) = (0, 0);
    let (mut collided, mut collision_triangles) = (0, 0);
    for (model_hash, position, rotation) in &placed {
        let (mesh, col) = match mesh_of_model.get(model_hash) {
            Some(m) => *m,
            None => {
                let mut col = None;
                let built = (|| -> AnyResult<Option<usize>> {
                    let (base, chunks) = load_model(wad, *model_hash)?;
                    let mut model = Model::parse(&base)?;
                    for c in &chunks {
                        model.add_streamed(c)?;
                    }
                    let phy2 = m2_assets::Ucfx::parse(&base)?.top_level(*b"PHY2");
                    match phy2.map(m2_assets::Physics::parse).transpose() {
                        Ok(Some(p)) => {
                            let (mut pos, mut idx) = (Vec::new(), Vec::new());
                            for (k, s) in p.shapes.iter().enumerate() {
                                // A shape sits at its part's place in its
                                // body node's space.
                                let node = p
                                    .shape_node(k)
                                    .and_then(|n| model.node_to_model(n as u16))
                                    .unwrap_or(Mat4::IDENTITY);
                                let place = p.shape_place(k).map_or(Mat4::IDENTITY, |(at, q)| {
                                    let q = Quat::from_array(q);
                                    let q = if q.length() > 0.5 {
                                        q.normalize()
                                    } else {
                                        Quat::IDENTITY
                                    };
                                    Mat4::from_rotation_translation(q, Vec3::from(at))
                                });
                                let to_model = node * place;
                                let (v, t) = s.geometry.triangles(s.radius);
                                let first = (pos.len() / 3) as u32;
                                for x in v {
                                    pos.extend(flip_v(
                                        to_model.transform_point3(Vec3::from(x)).to_array(),
                                    ));
                                }
                                idx.extend(
                                    t.iter().flat_map(|&[a, b, c]| [a, c, b].map(|i| first + i)),
                                );
                            }
                            if !idx.is_empty() {
                                let material = *collision_material.get_or_insert_with(|| {
                                    materials.push(r#"{"name":"collision","pbrMetallicRoughness":{"baseColorFactor":[1,0.35,0.1,0.35],"metallicFactor":0,"roughnessFactor":1},"alphaMode":"BLEND","doubleSided":true}"#.to_owned());
                                    materials.len() - 1
                                });
                                let a = g.floats(&pos, 3, true, Some(34962));
                                let i = g.u32s(&idx, Some(34963));
                                meshes.push(format!(
                                    r#"{{"name":"col {model_hash:08x}","primitives":[{{"attributes":{{"POSITION":{a}}},"indices":{i},"material":{material}}}]}}"#
                                ));
                                col = Some((meshes.len() - 1, idx.len() / 3));
                            }
                        }
                        Ok(None) => {}
                        Err(e) => eprintln!("collision {model_hash:08x}: {e}"),
                    }
                    let mut sink = Sink {
                        g: &mut g,
                        images: &mut images,
                        image_sampler: &mut image_sampler,
                        materials: &mut materials,
                        by_texture: &mut material_of_texture,
                        untextured: &mut untextured,
                    };
                    let (lists, tris) =
                        model_primitives(wad, &mut sink, &model, 1, &|_| Some((0, Mat4::IDENTITY)));
                    object_triangles += tris;
                    let prims = &lists[0];
                    if prims.is_empty() {
                        return Ok(None);
                    }
                    meshes.push(format!(
                        r#"{{"name":"{model_hash:08x}","primitives":[{}]}}"#,
                        prims.join(",")
                    ));
                    Ok(Some(meshes.len() - 1))
                })();
                let mesh = built.unwrap_or_else(|e| {
                    eprintln!("model {model_hash:08x}: {e}");
                    None
                });
                let col = col.map(|(m, tris)| {
                    collision_triangles += tris;
                    m
                });
                mesh_of_model.insert(*model_hash, (mesh, col));
                (mesh, col)
            }
        };
        let Some(mesh) = mesh else { continue };
        object_count += 1;
        let children = match col {
            Some(col) => {
                collided += 1;
                nodes.push(format!(r#"{{"name":"col {model_hash:08x}","mesh":{col}}}"#));
                child_nodes.insert(nodes.len() - 1);
                format!(r#","children":[{}]"#, nodes.len() - 1)
            }
            None => String::new(),
        };
        nodes.push(format!(
            r#"{{"name":"obj {model_hash:08x}","mesh":{mesh},"translation":{:?},"rotation":{:?}{children}}}"#,
            flip_v(*position),
            flip_q(*rotation)
        ));
    }
    eprintln!(
        "{object_count} placed objects from {} models, {object_triangles} triangles",
        mesh_of_model.values().filter(|m| m.0.is_some()).count()
    );
    eprintln!(
        "collision: {collided} of {object_count} placed objects have PHY2 shapes ({} models, {collision_triangles} triangles); the viewer uses the rest's render triangles instead",
        mesh_of_model
            .values()
            .filter(|m| m.0.is_some() && m.1.is_some())
            .count()
    );

    // Vehicles: up to `o.vehicles` drivable ground vehicles parked in a row
    // north of the detail point, each a node "veh ..." holding a body mesh
    // and one node per wheel ("wheel N", at the wheel's centre) that the
    // viewer turns, with the driving data in the node's extras. Kinds come
    // from texture names and wheels from geometry (`m2_assets::vehicle`).
    let mut candidates: Vec<(usize, String, String, u32)> = Vec::new();
    for (hash, textures) in &model_textures {
        let names = textures
            .iter()
            .filter_map(|t| texture_names.get(t).map(String::as_str));
        let Some((kind, label)) = m2_assets::vehicle::kind_from_textures(names) else {
            continue;
        };
        // Ground vehicles and helicopters, in the order they are offered.
        let rank = match kind.as_str() {
            "car" => 0,
            "truck" => 1,
            "helicopter" if label.contains("huey") => 2,
            "apc" => 4,
            "tank" => 5,
            "bus" | "van" | "semi" => 6,
            "helicopter" => 7,
            "boat" | "plane" | "airstrike" | "vtol" | "towed" | "trailer" | "outboardmotor"
            | "winch" | "transport" | "motorcycle" => continue,
            // `global_veh_<model>` textures: civilian cars and trucks.
            _ => 3,
        };
        candidates.push((rank, label, kind, *hash));
    }
    candidates.sort();
    let base = o.detail.unwrap_or([0.0, 0.0]);
    let mut used = HashSet::new();
    let mut parked = Vec::new();
    let cursor = &mut (base[0] - 20.0);
    // One of each kind first, then the rest in order.
    let mut per_rank: HashMap<usize, usize> = HashMap::new();
    let mut failed = HashSet::new();
    let order = candidates
        .iter()
        .map(|c| (true, c))
        .chain(candidates.iter().map(|c| (false, c)));
    for (first_pass, (rank, label, kind, hash)) in order {
        if parked.len() >= o.vehicles {
            break;
        }
        if used.contains(label)
            || failed.contains(hash)
            || (first_pass && per_rank.get(rank).copied().unwrap_or(0) >= 1)
        {
            continue;
        }
        let built = (|| -> AnyResult<Option<String>> {
            let (base_bytes, chunks) = load_model(wad, *hash)?;
            let mut model = Model::parse(&base_bytes)?;
            for c in &chunks {
                model.add_streamed(c)?;
            }
            let flying = kind == "helicopter";
            let tracked = kind == "tank" || kind == "apc";
            let (wheels, rotors) = if flying {
                (Vec::new(), m2_assets::vehicle::rotors(&model))
            } else {
                (m2_assets::vehicle::find_wheels(&model), Vec::new())
            };
            if flying && !rotors.iter().any(|r| r.axis[1] == 1.0) {
                return Ok(None);
            }
            if !flying
                && !tracked
                && (!(4..=10).contains(&wheels.len())
                    || wheels.iter().any(|w| !(0.15..=1.3).contains(&w.radius)))
            {
                return Ok(None);
            }
            // Turning parts: the wheels, then the rotors.
            let spinners: Vec<(&[u16], [f32; 3])> = wheels
                .iter()
                .map(|w| (&w.nodes[..], w.centre))
                .chain(rotors.iter().map(|r| (&r.nodes[..], r.centre)))
                .collect();
            let spinner_of = |node: u16| {
                let mut n = Some(node);
                for _ in 0..=model.nodes.len() {
                    let cur = n?;
                    if let Some(i) = spinners.iter().position(|s| s.0.contains(&cur)) {
                        return Some(i);
                    }
                    n = model.nodes.get(usize::from(cur))?.parent;
                }
                None
            };
            let group = |node: u16| {
                Some(match spinner_of(node) {
                    Some(i) => (1 + i, Mat4::from_translation(-Vec3::from(spinners[i].1))),
                    None => (0, Mat4::IDENTITY),
                })
            };
            let mut sink = Sink {
                g: &mut g,
                images: &mut images,
                image_sampler: &mut image_sampler,
                materials: &mut materials,
                by_texture: &mut material_of_texture,
                untextured: &mut untextured,
            };
            let (lists, _) = model_primitives(wad, &mut sink, &model, 1 + spinners.len(), &group);
            if lists[0].is_empty() {
                return Ok(None);
            }
            let mut children = Vec::new();
            for (i, prims) in lists.iter().enumerate() {
                let mesh = if prims.is_empty() {
                    String::new()
                } else {
                    meshes.push(format!(r#"{{"primitives":[{}]}}"#, prims.join(",")));
                    format!(r#","mesh":{}"#, meshes.len() - 1)
                };
                nodes.push(match i {
                    0 => format!(r#"{{"name":"vbody"{mesh}}}"#),
                    _ if i <= wheels.len() => format!(
                        r#"{{"name":"wheel {}","translation":{:?}{mesh}}}"#,
                        i - 1,
                        flip_v(spinners[i - 1].1)
                    ),
                    _ => format!(
                        r#"{{"name":"rotor {}","translation":{:?}{mesh}}}"#,
                        i - 1 - wheels.len(),
                        flip_v(spinners[i - 1].1)
                    ),
                });
                child_nodes.insert(nodes.len() - 1);
                children.push(nodes.len() - 1);
            }
            // The front axle steers.
            let front = wheels
                .iter()
                .map(|w| w.centre[2])
                .fold(f32::NEG_INFINITY, f32::max);
            let wheel_json: Vec<String> = wheels
                .iter()
                .map(|w| {
                    format!(
                        r#"{{"radius":{},"width":{},"steer":{}}}"#,
                        w.radius,
                        w.width,
                        w.centre[2] >= front - 0.3
                    )
                })
                .collect();
            // Mass: the heaviest PHY2 body, in the game's units.
            let mass = m2_assets::Ucfx::parse(&base_bytes)?
                .top_level(*b"PHY2")
                .and_then(|d| m2_assets::Physics::parse(d).ok())
                .map(|p| p.bodies.iter().map(|b| b.mass).fold(0.0f32, f32::max))
                .filter(|m| m.is_finite() && *m > 0.0);
            // The driver's seat and door markers (see m2_assets::vehicle).
            use m2_assets::vehicle::{DRIVER_DOOR, DRIVER_SEAT};
            let marker = |h| {
                m2_assets::vehicle::marker(&model, h)
                    .map_or("null".to_owned(), |p| format!("{:?}", flip_v(p)))
            };
            let rotor_json: Vec<String> = rotors
                .iter()
                .map(|r| format!(r#"{{"axis":{:?}}}"#, r.axis))
                .collect();
            // In a row, side by side with 3 m between them.
            let b = &model.info.bounds;
            let width = b.max[0] - b.min[0];
            let at = [*cursor + width / 2.0, 0.0, base[1] + 12.0];
            *cursor += width + 3.0;
            nodes.push(format!(
                r#"{{"name":"veh {hash:08x}","translation":{:?},"children":{children:?},"extras":{{"vehicle":{{"model":"{hash:08x}","kind":"{kind}","name":"{label}","tracked":{tracked},"mass":{},"min":{:?},"max":{:?},"seat":{},"door":{},"flying":{flying},"wheels":[{}],"rotors":[{}]}}}}}}"#,
                flip_v(at),
                mass.map_or("null".to_owned(), |m| m.to_string()),
                flip_v(b.min),
                flip_v(b.max),
                marker(DRIVER_SEAT),
                marker(DRIVER_DOOR),
                wheel_json.join(","),
                rotor_json.join(",")
            ));
            Ok(Some(format!(
                "{label} ({kind}, {} wheels, {} rotors)",
                wheels.len(),
                rotors.len()
            )))
        })();
        match built {
            Ok(Some(desc)) => {
                used.insert(label.clone());
                *per_rank.entry(*rank).or_default() += 1;
                parked.push(desc);
            }
            Ok(None) => {
                failed.insert(*hash);
            }
            Err(e) => {
                failed.insert(*hash);
                eprintln!("vehicle {hash:08x}: {e}");
            }
        }
    }
    if o.vehicles > 0 {
        eprintln!(
            "{} vehicle models found by texture name; parked {}: {}",
            candidates.len(),
            parked.len(),
            parked.join(", ")
        );
    }

    // Vegetation in the detailed cells, GPU-instanced: one node per run of
    // a patch (named "veg ..." for the viewer's culling), carrying its
    // instances in EXT_mesh_gpu_instancing.
    let mirror = m2_math::glam::Mat3::from_diagonal(Vec3::new(1.0, 1.0, -1.0));
    let mut veg_material: HashMap<u32, usize> = HashMap::new();
    let (mut veg_nodes, mut veg_instances) = (0, 0);
    for (cell, data) in &scatter_of {
        let Some(&centre) = cell_at.get(cell) else {
            continue;
        };
        if !near(centre) {
            continue;
        }
        let s = match m2_assets::Scatter::parse(data) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("scatter in cell {cell:08x}: {e}");
                continue;
            }
        };
        let mesh_ids: Vec<Option<usize>> = s
            .meshes
            .iter()
            .map(|m| {
                let tris: Vec<u16> = m.triangles().into_iter().flat_map(|[a, b, c]| [a, c, b]).collect();
                if tris.is_empty() {
                    return None;
                }
                let p: Vec<f32> = m.positions().into_iter().flat_map(flip_v).collect();
                let n: Vec<f32> = m
                    .read(DeclUsage::Normal)
                    .iter()
                    .flat_map(|v| flip_v(Vec3::new(v[0], v[1], v[2]).try_normalize().unwrap_or(Vec3::Y).to_array()))
                    .collect();
                let t: Vec<f32> = m.read(DeclUsage::TexCoord).iter().flat_map(|v| [v[0], v[1]]).collect();
                let mut attrs = format!(r#""POSITION":{}"#, g.floats(&p, 3, true, Some(34962)));
                if n.len() == p.len() {
                    let _ = write!(attrs, r#","NORMAL":{}"#, g.floats(&n, 3, false, Some(34962)));
                }
                if t.len() * 3 == p.len() * 2 {
                    let _ = write!(attrs, r#","TEXCOORD_0":{}"#, g.floats(&t, 2, false, Some(34962)));
                }
                let ind = g.u16s(&tris, 1, Some(34963));
                let texture = m.material.textures.first().copied().filter(|&t| t != 0).unwrap_or(0);
                let material = *veg_material.entry(texture).or_insert_with(|| {
                    let image = load_texture(wad, texture, 256).map(|(w, h, rgba, _)| {
                        let view = g.view(&png(w, h, &rgba), None);
                        images.push(format!(
                            r#"{{"bufferView":{view},"mimeType":"image/png","name":"{texture:08x}"}}"#
                        ));
                        image_sampler.push(1);
                        images.len() - 1
                    });
                    materials.push(match image {
                        Some(i) => format!(
                            r#"{{"name":"veg {texture:08x}","pbrMetallicRoughness":{{"baseColorTexture":{{"index":{i}}},"metallicFactor":0,"roughnessFactor":1}},"alphaMode":"MASK","alphaCutoff":0.5,"doubleSided":true}}"#
                        ),
                        None => r#"{"name":"veg","pbrMetallicRoughness":{"baseColorFactor":[0.2,0.35,0.1,1],"metallicFactor":0,"roughnessFactor":1},"doubleSided":true}"#.to_owned(),
                    });
                    materials.len() - 1
                });
                meshes.push(format!(
                    r#"{{"name":"veg {:08x}","primitives":[{{"attributes":{{{attrs}}},"indices":{ind},"material":{material}}}]}}"#,
                    m.hash
                ));
                Some(meshes.len() - 1)
            })
            .collect();
        for patch in &s.patches {
            let first = usize::from(patch.first_run);
            for run in &s.runs[first..first + usize::from(patch.run_count)] {
                let Some(mesh) = mesh_ids.get(usize::from(run.mesh)).copied().flatten() else {
                    continue;
                };
                let (mut t, mut q, mut sc) = (Vec::new(), Vec::new(), Vec::new());
                let start = run.first as usize;
                for m in &s.instances[start..start + usize::from(run.count)] {
                    // Rows of R (rotation x uniform scale) and the offset t.
                    let r = m2_math::glam::Mat3::from_cols(
                        Vec3::new(m[0], m[1], m[2]),
                        Vec3::new(m[4], m[5], m[6]),
                        Vec3::new(m[8], m[9], m[10]),
                    )
                    .transpose();
                    let scale = r.x_axis.length();
                    if scale <= 0.0 || !scale.is_finite() {
                        continue;
                    }
                    let rot = Quat::from_mat3(&(mirror * r * mirror * (1.0 / scale))).normalize();
                    let pos = [
                        centre[0] + patch.centre[0] + m[3],
                        centre[1] + patch.centre[1] + m[7],
                        centre[2] + patch.centre[2] + m[11],
                    ];
                    t.extend(flip_v(pos));
                    q.extend(rot.to_array());
                    sc.extend([scale; 3]);
                }
                if t.is_empty() {
                    continue;
                }
                veg_instances += t.len() / 3;
                let (qa, sa) = (g.floats(&q, 4, false, None), g.floats(&sc, 3, false, None));
                // The node sits at the run's first instance, for culling;
                // instances are relative to it.
                let origin = [t[0], t[1], t[2]];
                let local: Vec<f32> = t
                    .chunks_exact(3)
                    .flat_map(|p| [p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]])
                    .collect();
                let la = g.floats(&local, 3, true, None);
                nodes.push(format!(
                    r#"{{"name":"veg {cell:08x}","mesh":{mesh},"translation":{origin:?},"extensions":{{"EXT_mesh_gpu_instancing":{{"attributes":{{"TRANSLATION":{la},"ROTATION":{qa},"SCALE":{sa}}}}}}}}}"#
                ));
                veg_nodes += 1;
            }
        }
    }
    eprintln!("{veg_nodes} vegetation runs, {veg_instances} instances");

    // Sea level water.
    let half = ISLAND / 2.0 + 4000.0;
    let water = g.floats(
        &[
            -half, o.sea, -half, half, o.sea, -half, half, o.sea, half, -half, o.sea, half,
        ],
        3,
        true,
        Some(34962),
    );
    let water_ind = g.u16s(&[0, 2, 1, 0, 3, 2], 1, Some(34963));
    meshes.push(format!(
        r#"{{"name":"sea","primitives":[{{"attributes":{{"POSITION":{water}}},"indices":{water_ind},"material":{sea}}}]}}"#
    ));
    nodes.push(format!(r#"{{"name":"sea","mesh":{}}}"#, meshes.len() - 1));
    eprintln!(
        "{} detailed cells, {} terrain pieces, {} vertices, {} images",
        detailed,
        pieces.len(),
        pieces.iter().map(|p| p.positions.len() / 3).sum::<usize>(),
        images.len()
    );
    let roots: Vec<usize> = (0..nodes.len())
        .filter(|i| !child_nodes.contains(i))
        .collect();
    let textures: Vec<String> = (0..images.len())
        .map(|i| {
            format!(
                r#"{{"source":{i},"sampler":{}}}"#,
                image_sampler.get(i).copied().unwrap_or(0)
            )
        })
        .collect();
    let json = format!(
        r#"{{"asset":{{"version":"2.0","generator":"m2-assets gltf example"}},"extensionsUsed":["EXT_mesh_gpu_instancing"],"scene":0,"scenes":[{{"name":"world","nodes":{roots:?}}}],"nodes":[{}],"meshes":[{}],"materials":[{}],"images":[{}],"textures":[{}],"samplers":[{{"magFilter":9729,"minFilter":9987,"wrapS":33071,"wrapT":33071}},{{"magFilter":9729,"minFilter":9987,"wrapS":10497,"wrapT":10497}}],"accessors":[{}],"bufferViews":[{}],"buffers":[{{"byteLength":{}}}]}}"#,
        nodes.join(","),
        meshes.join(","),
        materials.join(","),
        images.join(","),
        textures.join(","),
        g.accessors.join(","),
        g.views.join(","),
        g.bin.len().next_multiple_of(4)
    );
    Ok(g.glb(json))
}
