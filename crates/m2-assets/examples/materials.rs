//! Surveys how materials are built, to find what a renderer must do to
//! draw surfaces as the game does.
//!
//! ```text
//! cargo run -p m2-assets --release --example materials -- <vz.wad> [names or shader files...]
//! ```
//!
//! Prints, without dumping any asset data:
//!
//! - texture sizes, formats and flags;
//! - model materials by technique id: texture counts, flags, which kinds of
//!   texture (by name suffix and format) sit in each slot, the range of
//!   each of the 25 constants, and a few example materials by texture name;
//! - model primitive groups by effect hash, with the vertex elements they
//!   carry and the techniques of the materials drawn with them;
//! - terrain cell materials, terrain range shader hashes, terrain vertex
//!   elements and what their vertex colours look like (blend weights?);
//! - textures whose names mention roads or decals, and what uses them;
//! - model materials by flags word and by trailing word: techniques, the
//!   base texture's format, the opacity constant and example base textures
//!   (to read which bits mean blending, cut-outs or two-sided faces);
//! - ground scatter meshes by flags, technique and base texture: their
//!   fade distances, how flat they are, and example textures.
//!
//! Every extra argument is a file of candidate names: a text file is read
//! one name per line, anything else (`shader*.bin`) for its readable
//! strings. Each name is hashed with [`m2_assets::name_hash`] and matched
//! against every unknown id above, to name techniques and shaders.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;
use std::process::ExitCode;

use m2_assets::model::{MODEL_TYPE, Material};
use m2_assets::scatter::SCATTER_TYPE;
use m2_assets::terrain::{LOW_RES_TYPE, TERRAIN_TYPE};
use m2_assets::texture::INFO;
use m2_assets::vertex::{DeclUsage, VertexDecl};
use m2_assets::{
    BlockContents, LowResTile, Model, PixelFormat, Scatter, TerrainCell, TextureInfo, Ucfx,
    WadArchive, name_hash,
};

const TEXTURE_TYPE: u32 = 0xf011_157a;

type AnyResult<T> = Result<T, Box<dyn std::error::Error>>;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("usage: materials <vz.wad> [names or shader files...]");
        return ExitCode::FAILURE;
    };
    match run(path, &args[1..]) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// A texture's name and header.
struct Tex {
    name: String,
    info: TextureInfo,
}

/// Counts of things, printed most common first.
#[derive(Default)]
struct Tally<K: std::hash::Hash + Eq>(HashMap<K, usize>);

impl<K: std::hash::Hash + Eq + Clone + std::fmt::Display> Tally<K> {
    fn add(&mut self, k: K) {
        *self.0.entry(k).or_default() += 1;
    }

    fn top(&self, n: usize) -> String {
        let mut v: Vec<_> = self.0.iter().collect();
        v.sort_by(|a, b| {
            b.1.cmp(a.1)
                .then_with(|| a.0.to_string().cmp(&b.0.to_string()))
        });
        v.iter()
            .take(n)
            .map(|(k, c)| format!("{k}:{c}"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Per technique: what its materials hold.
#[derive(Default)]
struct Technique {
    count: usize,
    ntex: Tally<usize>,
    flags: Tally<String>,
    /// Per slot: name suffix and format of the texture there.
    slots: Vec<Tally<String>>,
    /// Per constant: min, max, distinct values seen (capped).
    constants: Vec<(f32, f32, Tally<String>)>,
    examples: Vec<String>,
    /// Effect hashes of groups drawn with these materials.
    effects: Tally<String>,
    /// Vertex signatures of those groups.
    decls: Tally<String>,
}

/// Per flags (or trailing) word: what its materials look like.
#[derive(Default)]
struct Usage {
    count: usize,
    techniques: Tally<String>,
    base: Tally<String>,
    opacity: Tally<String>,
    names: Vec<String>,
}

impl Usage {
    fn add(&mut self, textures: &HashMap<u32, Tex>, m: &Material, names: &HashMap<u32, String>) {
        self.count += 1;
        self.techniques
            .add(format!("{:08x}{}", m.technique, named(names, m.technique)));
        let base = m.textures.first().copied().unwrap_or(0);
        self.base.add(describe_slot(textures, base));
        self.opacity.add(format!("{}", m.constants[12]));
        if let Some(t) = textures.get(&base)
            && self.names.len() < 10
            && !self.names.contains(&t.name)
        {
            self.names.push(t.name.clone());
        }
    }

    fn print(&self, label: &str) {
        println!("{label}: {} materials", self.count);
        println!("    techniques [{}]", self.techniques.top(8));
        println!("    base [{}]", self.base.top(8));
        println!("    c12 [{}]", self.opacity.top(6));
        println!("    e.g. {}", self.names.join(", "));
    }
}

/// A texture name's last `_` token (`tt_sand01_n` -> `n`), or `-`.
fn suffix(name: &str) -> String {
    match name.rsplit_once('_') {
        Some((_, s)) if !s.is_empty() && s.len() <= 6 => s.to_ascii_lowercase(),
        _ => "-".into(),
    }
}

fn format_name(f: PixelFormat) -> String {
    format!("{f:?}")
}

/// A vertex declaration as `P N C T0 T1 W I Tg B` style letters.
fn decl_signature(decl: &VertexDecl) -> String {
    let mut s = Vec::new();
    for e in decl.elements() {
        let u = match e.usage {
            DeclUsage::Position => "P",
            DeclUsage::BlendWeight => "W",
            DeclUsage::BlendIndices => "I",
            DeclUsage::Normal => "N",
            DeclUsage::TexCoord => "T",
            DeclUsage::Tangent => "Tg",
            DeclUsage::Binormal => "B",
            DeclUsage::Color => "C",
            _ => "?",
        };
        s.push(format!("{u}{}:{:?}", e.usage_index, e.ty));
    }
    s.join(" ")
}

fn describe_slot(textures: &HashMap<u32, Tex>, hash: u32) -> String {
    if hash == 0 {
        return "empty".into();
    }
    match textures.get(&hash) {
        Some(t) => format!(
            "{}/{}{}",
            suffix(&t.name),
            format_name(t.info.format),
            if t.info.flags & 0x20 != 0 { "/f20" } else { "" }
        ),
        None => "missing".into(),
    }
}

fn texture_names(textures: &HashMap<u32, Tex>, m: &Material) -> String {
    m.textures
        .iter()
        .map(|&h| match textures.get(&h) {
            _ if h == 0 => "-".to_owned(),
            Some(t) => format!("{}({}x{})", t.name, t.info.width, t.info.height),
            None => format!("{h:08x}?"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

impl Technique {
    fn add(
        &mut self,
        textures: &HashMap<u32, Tex>,
        m: &Material,
        example: impl FnOnce() -> String,
    ) {
        self.count += 1;
        self.ntex.add(m.textures.len());
        self.flags.add(format!("{:#06x}", m.flags));
        if self.slots.len() < m.textures.len() {
            self.slots.resize_with(m.textures.len(), Tally::default);
        }
        for (i, &t) in m.textures.iter().enumerate() {
            self.slots[i].add(describe_slot(textures, t));
        }
        if self.constants.is_empty() {
            self.constants = (0..m.constants.len())
                .map(|_| (f32::MAX, f32::MIN, Tally::default()))
                .collect();
        }
        for (c, &v) in self.constants.iter_mut().zip(&m.constants) {
            c.0 = c.0.min(v);
            c.1 = c.1.max(v);
            if c.2.0.len() < 64 {
                c.2.add(format!("{v}"));
            }
        }
        if self.examples.len() < 4 {
            self.examples.push(example());
        }
    }

    fn print(&self, id: u32, names: &HashMap<u32, String>) {
        println!(
            "technique {id:08x}{} materials {} ntex [{}] flags [{}]",
            named(names, id),
            self.count,
            self.ntex.top(6),
            self.flags.top(6)
        );
        for (i, s) in self.slots.iter().enumerate() {
            println!("    slot {i}: {}", s.top(8));
        }
        for (i, (lo, hi, vals)) in self.constants.iter().enumerate() {
            if lo == hi {
                println!("    c{i:02} = {lo}");
            } else {
                println!("    c{i:02} {lo} .. {hi}  [{}]", vals.top(5));
            }
        }
        println!("    effects [{}]", self.effects.top(6));
        println!("    vertices [{}]", self.decls.top(4));
        for e in &self.examples {
            println!("    e.g. {e}");
        }
    }
}

fn named(names: &HashMap<u32, String>, h: u32) -> String {
    names
        .get(&h)
        .map(|n| format!(" \"{n}\""))
        .unwrap_or_default()
}

/// Candidate names from the given files.
fn candidates(files: &[String]) -> AnyResult<HashMap<u32, String>> {
    let mut out = HashMap::new();
    for f in files {
        let bytes = std::fs::read(f)?;
        let text_file = f.ends_with(".txt") || f.ends_with(".csv");
        let words: Vec<String> = if text_file {
            String::from_utf8_lossy(&bytes)
                .lines()
                .map(|l| l.trim().to_owned())
                .collect()
        } else {
            readable(&bytes, 3)
        };
        let before = out.len();
        for w in words {
            if w.is_empty() {
                continue;
            }
            // Try the name itself and, for paths or decorated names, its
            // pieces.
            let mut variants = vec![w.clone()];
            for sep in ['\\', '/', '.', ' ', ':'] {
                variants.extend(w.split(sep).filter(|p| p.len() >= 3).map(str::to_owned));
            }
            for v in variants {
                out.entry(name_hash(&v)).or_insert(v);
            }
        }
        eprintln!("{f}: {} candidate names", out.len() - before);
    }
    Ok(out)
}

/// Runs of printable ASCII at least `min` long.
fn readable(data: &[u8], min: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = Vec::new();
    for &b in data {
        if (0x20..0x7f).contains(&b) {
            cur.push(b);
        } else {
            if cur.len() >= min {
                out.push(String::from_utf8_lossy(&cur).into_owned());
            }
            cur.clear();
        }
    }
    if cur.len() >= min {
        out.push(String::from_utf8_lossy(&cur).into_owned());
    }
    out
}

fn run(path: &str, name_files: &[String]) -> AnyResult<()> {
    let names = candidates(name_files)?;
    let mut wad = WadArchive::open(BufReader::new(File::open(path)?))?;
    let mut textures: HashMap<u32, Tex> = HashMap::new();
    let mut models: HashMap<u32, Vec<u8>> = HashMap::new();
    let mut cells: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut tiles: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut scatters: Vec<Vec<u8>> = Vec::new();
    for block in 0..wad.index().blocks().len() {
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
        for (e, data) in contents.iter() {
            match e.type_hash {
                TEXTURE_TYPE => {
                    let Ok(u) = Ucfx::parse(data) else { continue };
                    let Some(info) = u.top_level(INFO) else {
                        continue;
                    };
                    let Ok(info) = TextureInfo::parse(info) else {
                        continue;
                    };
                    textures.insert(
                        e.name_hash,
                        Tex {
                            name: u.name().unwrap_or("?").to_owned(),
                            info,
                        },
                    );
                }
                // Resident assets only: materials live there.
                MODEL_TYPE if e.unk_0x08 == 0 => {
                    models.insert(e.name_hash, data.to_vec());
                }
                TERRAIN_TYPE => cells.push((e.name_hash, data.to_vec())),
                LOW_RES_TYPE => tiles.push((e.name_hash, data.to_vec())),
                SCATTER_TYPE => scatters.push(data.to_vec()),
                _ => {}
            }
        }
    }

    // Textures.
    let mut widths = Tally::default();
    let mut formats = Tally::default();
    let mut flags = Tally::default();
    let mut suffixes = Tally::default();
    for t in textures.values() {
        widths.add(format!("{}", t.info.width.max(t.info.height)));
        formats.add(format_name(t.info.format));
        flags.add(format!("{:#06x}", t.info.flags));
        suffixes.add(suffix(&t.name));
    }
    println!("== {} textures", textures.len());
    println!("largest side: {}", widths.top(12));
    println!("formats: {}", formats.top(12));
    println!("flags: {}", flags.top(12));
    println!("name suffixes: {}", suffixes.top(40));

    // Model materials.
    let mut techniques: BTreeMap<u32, Technique> = BTreeMap::new();
    let mut effect_ids: HashSet<u32> = HashSet::new();
    let mut format_ids: HashSet<u32> = HashSet::new();
    let mut material_names: HashSet<u32> = HashSet::new();
    let mut tails = Tally::default();
    let mut by_flags: BTreeMap<u16, Usage> = BTreeMap::new();
    let mut by_tail: BTreeMap<u32, Usage> = BTreeMap::new();
    let mut model_errors = 0;
    let mut road_users: HashMap<u32, usize> = HashMap::new();
    // Groups the renderer cannot lay out (so the alpha skips them).
    let mut unlaid: Tally<String> = Tally::default();
    let mut hashes: Vec<u32> = models.keys().copied().collect();
    hashes.sort_unstable();
    for hash in &hashes {
        let model = match Model::parse(&models[hash]) {
            Ok(m) => m,
            Err(_) => {
                model_errors += 1;
                continue;
            }
        };
        for m in &model.materials {
            material_names.insert(m.name_hash);
            tails.add(format!("{:08x}", m.surface));
            by_flags
                .entry(m.flags)
                .or_default()
                .add(&textures, m, &names);
            by_tail
                .entry(m.surface)
                .or_default()
                .add(&textures, m, &names);
            for &t in &m.textures {
                *road_users.entry(t).or_default() += 1;
            }
            techniques
                .entry(m.technique)
                .or_default()
                .add(&textures, m, || {
                    format!("model {hash:08x}: {}", texture_names(&textures, m))
                });
        }
        for slot in 0..model.info.slot_count as u8 {
            let Some(g) = model.geometry(slot) else {
                continue;
            };
            for group in &g.groups {
                effect_ids.insert(group.info.effect_hash);
                format_ids.insert(group.info.format_hash);
                let sig = decl_signature(&group.decl);
                if let Err(e) = group.layout() {
                    unlaid.add(format!("{sig} ({e})"));
                }
                let mut seen = HashSet::new();
                for r in &group.material_ranges {
                    let Some(m) = model.materials.get(r.material as usize) else {
                        continue;
                    };
                    if !seen.insert(m.technique) {
                        continue;
                    }
                    let t = techniques.entry(m.technique).or_default();
                    t.effects.add(format!(
                        "{:08x}{}",
                        group.info.effect_hash,
                        named(&names, group.info.effect_hash)
                    ));
                    t.decls.add(format!("{:?} {sig}", g.kind));
                }
            }
        }
    }
    println!(
        "\n== {} models ({} unparsed), {} techniques, {} material names, tails [{}]",
        hashes.len(),
        model_errors,
        techniques.len(),
        material_names.len(),
        tails.top(4)
    );
    println!("groups without a renderer layout: [{}]", unlaid.top(10));
    let mut order: Vec<_> = techniques.iter().collect();
    order.sort_by_key(|t| std::cmp::Reverse(t.1.count));
    for (id, t) in order {
        t.print(*id, &names);
    }
    println!("\n== model materials by flags");
    for (f, u) in &by_flags {
        u.print(&format!("flags {f:#06x}"));
    }
    println!("\n== model materials by trailing word");
    for (t, u) in &by_tail {
        u.print(&format!("tail {t:08x}{}", named(&names, *t)));
    }

    // Ground scatter.
    #[derive(Default)]
    struct Clumps {
        meshes: usize,
        instances: usize,
        fade: Tally<String>,
        unk: Tally<String>,
        shape: Tally<String>,
        names: Vec<String>,
    }
    let mut clumps: BTreeMap<String, Clumps> = BTreeMap::new();
    for data in &scatters {
        let Ok(sc) = Scatter::parse(data) else {
            continue;
        };
        let mut used = vec![0usize; sc.meshes.len()];
        for (mesh, _, _) in sc.placements() {
            if let Some(n) = used.get_mut(usize::from(mesh)) {
                *n += 1;
            }
        }
        for (m, n) in sc.meshes.iter().zip(used) {
            let base = m.material.textures.first().copied().unwrap_or(0);
            let key = format!(
                "flags {:#06x} {:08x}{} base {}",
                m.material.flags,
                m.material.technique,
                named(&names, m.material.technique),
                describe_slot(&textures, base)
            );
            let c = clumps.entry(key).or_default();
            c.meshes += 1;
            c.instances += n;
            c.fade.add(format!("{}-{}", m.fade[0], m.fade[1]));
            c.unk.add(format!("{:08x}/{}", m.unk_0x04, m.unk_0x10));
            let pos = m.positions();
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for p in &pos {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
            let (w, h) = ((hi[0] - lo[0]).max(hi[2] - lo[2]), hi[1] - lo[1]);
            c.shape.add(format!(
                "{} {}v {:.1}x{:.1}m",
                if h < 0.2 * w { "flat" } else { "upright" },
                pos.len(),
                w,
                h
            ));
            if let Some(t) = textures.get(&base)
                && c.names.len() < 6
                && !c.names.contains(&t.name)
            {
                c.names.push(t.name.clone());
            }
        }
    }
    println!("\n== ground scatter: {} assets", scatters.len());
    for (k, c) in &clumps {
        println!("{k}: {} meshes, {} instances", c.meshes, c.instances);
        println!("    fade [{}]", c.fade.top(6));
        println!("    0x04/0x10 [{}]", c.unk.top(6));
        println!("    shape [{}]", c.shape.top(6));
        println!("    e.g. {}", c.names.join(", "));
    }

    // Terrain.
    let mut tt: BTreeMap<u32, Technique> = BTreeMap::new();
    let mut shaders: Tally<String> = Tally::default();
    let mut tdecls: Tally<String> = Tally::default();
    let mut uv_examples = Vec::new();
    let mut colour_stats = (0usize, 0usize, [0f64; 4], [0usize; 4], 0usize);
    let mut terrain_ids: HashSet<u32> = HashSet::new();
    for (hash, data) in &cells {
        let Ok(c) = TerrainCell::parse(data) else {
            eprintln!("cell {hash:08x} unparsed");
            continue;
        };
        for m in &c.materials {
            terrain_ids.insert(m.material.technique);
            for &t in &m.material.textures {
                *road_users.entry(t).or_default() += 1;
            }
            tt.entry(m.material.technique)
                .or_default()
                .add(&textures, &m.material, || {
                    format!("cell {hash:08x}: {}", texture_names(&textures, &m.material))
                });
            if uv_examples.len() < 3 {
                uv_examples.push(format!("cell {hash:08x} uv rows: {:?}", m.uv));
            }
        }
        for g in c.patches.iter().flat_map(|p| &p.groups) {
            tdecls.add(decl_signature(&g.decl));
            for r in &g.material_ranges {
                let ntex = c
                    .materials
                    .get(r.material as usize)
                    .map_or(0, |m| m.material.textures.len());
                shaders.add(format!(
                    "{:08x}{}/ntex{ntex}",
                    r.shader_hash,
                    named(&names, r.shader_hash)
                ));
                terrain_ids.insert(r.shader_hash);
            }
            if let Some(e) = g.decl.find(DeclUsage::Color, 0)
                && let Some(cols) = g.vertices.read(e)
            {
                for c in cols {
                    colour_stats.0 += 1;
                    let sum: f32 = c.iter().sum();
                    let rgb: f32 = c[..3].iter().sum();
                    if (sum - 1.0).abs() < 0.03 {
                        colour_stats.1 += 1;
                    }
                    if (rgb - 1.0).abs() < 0.03 {
                        colour_stats.4 += 1;
                    }
                    for (i, &v) in c.iter().enumerate() {
                        colour_stats.2[i] += f64::from(v);
                        if v > 0.01 {
                            colour_stats.3[i] += 1;
                        }
                    }
                }
            }
        }
    }
    println!("\n== {} terrain cells", cells.len());
    println!("range shaders: {}", shaders.top(20));
    println!("vertex layouts: {}", tdecls.top(6));
    let n = colour_stats.0.max(1) as f64;
    println!(
        "vertex colours: {} vertices; rgba sums to 1: {:.1}%, rgb sums to 1: {:.1}%; mean [{:.3} {:.3} {:.3} {:.3}]; non-zero [{:.1}% {:.1}% {:.1}% {:.1}%]",
        colour_stats.0,
        100.0 * colour_stats.1 as f64 / n,
        100.0 * colour_stats.4 as f64 / n,
        colour_stats.2[0] / n,
        colour_stats.2[1] / n,
        colour_stats.2[2] / n,
        colour_stats.2[3] / n,
        100.0 * colour_stats.3[0] as f64 / n,
        100.0 * colour_stats.3[1] as f64 / n,
        100.0 * colour_stats.3[2] as f64 / n,
        100.0 * colour_stats.3[3] as f64 / n,
    );
    for e in &uv_examples {
        println!("{e}");
    }
    for (id, t) in &tt {
        t.print(*id, &names);
    }
    let mut tile_tech = Tally::default();
    for (_, data) in &tiles {
        if let Ok(t) = LowResTile::parse(data) {
            for m in &t.materials {
                tile_tech.add(format!(
                    "{:08x}/{}",
                    m.technique,
                    m.textures
                        .iter()
                        .map(|&h| describe_slot(&textures, h))
                        .collect::<Vec<_>>()
                        .join(",")
                ));
            }
        }
    }
    println!("low-res tiles: {} [{}]", tiles.len(), tile_tech.top(6));

    // Roads and decals.
    println!("\n== textures naming roads, decals, tracks, paths, asphalt");
    let mut listed: Vec<&Tex> = textures
        .values()
        .filter(|t| {
            let n = t.name.to_ascii_lowercase();
            [
                "road", "decal", "track", "path", "asphalt", "street", "pave", "tarmac", "dirt",
            ]
            .iter()
            .any(|k| n.contains(k))
        })
        .collect();
    listed.sort_by(|a, b| a.name.cmp(&b.name));
    for t in listed.iter().take(80) {
        println!(
            "  {} {}x{} {:?} used by {} materials",
            t.name,
            t.info.width,
            t.info.height,
            t.info.format,
            road_users.get(&name_hash(&t.name)).copied().unwrap_or(0)
        );
    }
    println!("  ({} in all)", listed.len());

    // Names for the unknown ids.
    if !names.is_empty() {
        println!("\n== candidate names matched");
        let groups: [(&str, Vec<u32>); 5] = [
            ("technique", techniques.keys().copied().collect()),
            ("effect", effect_ids.into_iter().collect()),
            ("format", format_ids.into_iter().collect()),
            ("terrain", terrain_ids.into_iter().collect()),
            ("material", material_names.into_iter().collect()),
        ];
        for (what, ids) in groups {
            let hits: Vec<String> = ids
                .iter()
                .filter_map(|id| names.get(id).map(|n| format!("{id:08x}={n}")))
                .collect();
            println!(
                "{what}: {} of {} named: {}",
                hits.len(),
                ids.len(),
                hits.join(" ")
            );
        }
    }
    Ok(())
}
