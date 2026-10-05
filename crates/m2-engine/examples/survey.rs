//! Surveys what the alpha still knows only by hash or by guess, now the
//! name hash is solved: whose voice each voice set is, what each engine
//! sound is, the skeleton's bone names and what hangs off the hands, the
//! weapons' markers, which clips throw, jump and land (by their events),
//! what each material texture slot holds, how a terrain cell layers its
//! textures (and which way its scatter leans), the effect textures, and
//! what stands near a spot.
//!
//! ```text
//! cargo run --release -p m2-engine --example survey -- "<install dir>" <section>...
//! ```
//!
//! Sections, several at once (the name dictionary is built once):
//!
//! ```text
//! voices                    voice sets with their names; every vo_ name found
//! people                    the person models' roles
//! banks                     every sound bank: block, sounds, seconds
//! sounds:<bank>[,<bank>..]  each sound: length, and loudness (dB) and
//!                           zero crossings (a pitch guide, per second)
//!                           over its tenths; its name if found
//! rig[:<model>]             a skeleton's nodes (default Jennifer): name,
//!                           parent, rest place; what hangs off the hands
//! weapons                   the hand weapons' nodes and bounds
//! clips[:<model>[,<word>..]] clips fitting a skeleton: per block, per
//!                           event name, named clips, and every clip in
//!                           blocks whose path holds a word (default
//!                           jennifer, weapon, resident)
//! slots                     across all models, what each material texture
//!                           slot holds (name endings), per technique
//! materials:<model|label>[,..] each material's technique, flags, constants
//!                           and textures (name, size, format)
//! terrain:<x>,<z>           the terrain cell there: materials, textures,
//!                           UV transforms, vertex layout, colour ranges per
//!                           material, and its scatter's lean vs the ground
//! fx[:<word>,..]            texture names holding effect words
//! spot:<x>,<z>[,<r>]        what stands near a spot (default 30 m): models
//!                           with their techniques and textures, and the
//!                           layer entities with their components' fields
//! entities:<word>[,..]      every entity of the layers whose name holds a
//!                           word, or whose own name does, with its
//!                           components' fields (400 a layer at most)
//! collision:<model>[,..]    a model's collision: each PHY2 shape (kind,
//!                           node, radius, box in model space, sizes) and
//!                           part (node, body, shape, place), or the
//!                           visible geometry it falls back to
//! phy2rules                 across all models, how many collision shapes
//!                           land inside their model by each way of
//!                           placing them (node, part places)
//! engines[:<bank>,..]       engine banks (default all) by name hash: each
//!                           sound's length, level, steadiness, zero
//!                           crossings and pitch, what the alpha plays
//!                           from it, and which templates name it
//! vehicle:<word|model>[,..] vehicles whose label holds a word: what the
//!                           alpha makes of them (wheels, tracked, turret),
//!                           and every node with its geometry box and how
//!                           it fares in the wheel tests
//! templates:<word|id>[,..]  templates whose name holds a word (or whose
//!                           id is given, 8 hex digits), with their
//!                           components' fields (text fields not read), a
//!                           field naming another template followed by its
//!                           name; then the templates they name, likewise
//! parents[:<word>,..]       how entities' Transform `Parent` matches the
//!                           template database (counts, and whether named
//!                           entities' parents carry their name), then
//!                           every entity of the layers holding a word
//!                           (default vzacon001,vza_con001) with its parent
//!                           template: name, model, components, and what
//!                           the alpha's world index makes of it
//! find:<hash>[,..]          names for hashes
//! ```
//!
//! Names come from every NUL-terminated string in the four archives and
//! the data folder's other files (shader*.bin), plus bone-name patterns
//! (`Bone_RHand`). Prints names, hashes, counts, sizes and measurements
//! only: no sound, text or picture data.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::path::Path;

use m2_assets::animation::ANIMATION_TYPE;
use m2_assets::layer::{LAYER_TYPE, STRING_FIELD, TEMPLATE_DB_TYPE};
use m2_assets::model::MODEL_TYPE;
use m2_assets::scatter::SCATTER_TYPE;
use m2_assets::sound::SOUND_BANK_TYPE;
use m2_assets::terrain::{LOW_RES_TYPE, TERRAIN_TYPE};
use m2_assets::vertex::DeclUsage;
use m2_assets::{BlockContents, Clip, Layer, Model, Scatter, SoundBank, TerrainCell, Texture};
use m2_engine::anim::{BoundClip, Skeleton};
use m2_engine::assets::TEXTURE_TYPE;
use m2_engine::audio::SoundLibrary;
use m2_engine::vehicle::{Catalogue, CatalogueEntry, VehicleSpec, catalogue};
use m2_engine::{AssetSource, Install, Voices, WorldIndex};
use m2_math::glam::Vec3;

/// Jennifer.
const DEFAULT_MODEL: u32 = 0x15d8_9832;
const RIGHT_HAND: u32 = 0xd34f_7713;
const LEFT_HAND: u32 = 0xeb6f_1b2d;

type Error = Box<dyn std::error::Error>;
/// A model's materials: (technique, textures) each.
type ModelMaterials = (u32, Vec<(u32, Vec<u32>)>);
/// A clip: hash, block, seconds, speed, events (time, name, category).
type ClipRow = (u32, usize, f32, f32, Vec<(f32, String, String)>);
/// Per event (category, name): the clips with it (hash, time, seconds).
type EventUses = BTreeMap<(String, String), Vec<(u32, f32, f32)>>;
/// A layer component's name, fields and records by entity.
type ComponentRecords<'a> = (
    &'a str,
    Vec<m2_assets::layer::Field>,
    HashMap<u32, &'a [u8]>,
);

/// FNV-1a over the parts' bytes, each OR 0x20, then `*`: the name hash
/// of the parts joined, without joining them.
fn hash_parts(parts: &[&str]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for p in parts {
        for b in p.bytes() {
            h ^= u32::from(b | 0x20);
            h = h.wrapping_mul(0x0100_0193);
        }
    }
    h ^= u32::from(b'*');
    h.wrapping_mul(0x0100_0193)
}

/// Names by hash.
#[derive(Default)]
struct Names {
    map: HashMap<u32, String>,
}

impl Names {
    fn add(&mut self, s: &str) {
        if (2..=120).contains(&s.len()) {
            self.map
                .entry(hash_parts(&[s]))
                .or_insert_with(|| s.to_owned());
        }
    }

    /// A run of text: itself, its words, and the parts of paths in it.
    fn add_text(&mut self, text: &str) {
        let text = text.trim();
        if text.len() < 2 || text.len() > 160 || !text.bytes().any(|b| b.is_ascii_alphabetic()) {
            return;
        }
        self.add(text);
        for token in text.split(|c: char| !(c.is_ascii_alphanumeric() || "_.-/\\:".contains(c))) {
            if token.len() < 2 || token == text {
                continue;
            }
            self.add(token);
            for part in token.split(['.', '/', '\\', ':']) {
                if part.len() >= 2 && part != token {
                    self.add(part);
                }
            }
        }
    }

    /// The printable runs in `data` at least `min` long; with `nul`, only
    /// runs a NUL ends (C strings).
    fn scan(&mut self, data: &[u8], nul: bool, min: usize) {
        let mut start = None;
        for (i, &b) in data.iter().enumerate() {
            let printable = (0x20..0x7f).contains(&b);
            match (printable, start) {
                (true, None) => start = Some(i),
                (false, Some(s)) => {
                    if i - s >= min
                        && (!nul || b == 0)
                        && let Ok(text) = std::str::from_utf8(&data[s..i])
                    {
                        self.add_text(text);
                    }
                    start = None;
                }
                _ => {}
            }
        }
    }

    fn get(&self, hash: u32) -> Option<&str> {
        self.map.get(&hash).map(String::as_str)
    }

    fn show(&self, hash: u32) -> &str {
        self.get(hash).unwrap_or("?")
    }

    /// Short identifier-like names, to build longer ones from.
    fn words(&self) -> Vec<&str> {
        let mut w: Vec<&str> = self
            .map
            .values()
            .map(String::as_str)
            .filter(|s| {
                (2..=28).contains(&s.len())
                    && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            })
            .collect();
        w.sort_unstable();
        w.dedup();
        w
    }

    /// Names for `targets` (those not already known) built as prefix +
    /// word + suffix.
    fn resolve(
        &self,
        targets: &HashSet<u32>,
        prefixes: &[&str],
        suffixes: &[&str],
    ) -> HashMap<u32, String> {
        let mut out = HashMap::new();
        let open: HashSet<u32> = targets
            .iter()
            .copied()
            .filter(|t| !self.map.contains_key(t))
            .collect();
        if open.is_empty() {
            return out;
        }
        for w in self.words() {
            for p in prefixes {
                for s in suffixes {
                    let h = hash_parts(&[p, w, s]);
                    if open.contains(&h) {
                        out.entry(h).or_insert_with(|| format!("{p}{w}{s}"));
                    }
                }
            }
        }
        out
    }
}

/// What the pass over `vz.wad` gathers besides names.
#[derive(Default)]
struct Gathered {
    /// Texture hash: (width, height, format).
    textures: HashMap<u32, (u16, u16, String)>,
    /// Per model: its materials' (technique, textures).
    materials: Vec<ModelMaterials>,
}

fn open(path: &Path) -> Result<AssetSource<BufReader<File>>, Error> {
    let file = File::open(path)?;
    let mut s = AssetSource::new(BufReader::new(file))?;
    s.set_cache_budget(48 << 20);
    Ok(s)
}

/// Every name in the archives and data files; with `gather`, what the
/// slots, materials and fx sections need from `vz.wad` too.
fn harvest(install: &Install, gather: bool) -> Result<(Names, Gathered), Error> {
    let mut names = Names::default();
    let mut got = Gathered::default();
    let started = std::time::Instant::now();
    for wad in ["vz.wad", "English.wad", "shell.wad", "Loading.wad"] {
        let path = install.data.join(wad);
        if !path.is_file() {
            continue;
        }
        let mut source = open(&path)?;
        let paths: Vec<String> = source
            .archive()
            .index()
            .blocks()
            .iter()
            .map(|b| b.path.clone())
            .collect();
        for (b, p) in paths.iter().enumerate() {
            names.add_text(p);
            let decoded = match source.block(b) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("block {p}: {e}");
                    continue;
                }
            };
            let Ok(contents) = BlockContents::parse(&decoded) else {
                continue;
            };
            for (e, data) in contents.iter() {
                match e.type_hash {
                    TEXTURE_TYPE => {
                        if let Ok(t) = Texture::parse(data) {
                            if let Some(n) = t.name {
                                names.add_text(n);
                            }
                            if gather && wad == "vz.wad" && e.unk_0x08 == 0 {
                                got.textures.entry(e.name_hash).or_insert((
                                    t.info.width,
                                    t.info.height,
                                    format!("{:?}", t.info.format),
                                ));
                            }
                        }
                    }
                    SOUND_BANK_TYPE | TERRAIN_TYPE | LOW_RES_TYPE => {}
                    MODEL_TYPE => {
                        if gather
                            && wad == "vz.wad"
                            && e.unk_0x08 == 0
                            && let Ok(m) = Model::parse(data)
                        {
                            got.materials.push((
                                e.name_hash,
                                m.materials
                                    .iter()
                                    .map(|mt| (mt.technique, mt.textures.clone()))
                                    .collect(),
                            ));
                        }
                        names.scan(data, true, 4);
                    }
                    _ => names.scan(data, true, 3),
                }
            }
        }
        eprintln!(
            "names: {} after {wad} ({:.0} s)",
            names.map.len(),
            started.elapsed().as_secs_f32()
        );
    }
    if let Ok(dir) = std::fs::read_dir(&install.data) {
        for entry in dir.flatten() {
            let path = entry.path();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if !matches!(ext.as_str(), "bin" | "ini" | "txt" | "xml" | "cfg") {
                continue;
            }
            if entry.metadata().map_or(true, |m| m.len() > 64 << 20) {
                continue;
            }
            if let Ok(data) = std::fs::read(&path) {
                names.scan(&data, false, 4);
            }
        }
    }
    // Bone names: `Bone_` and the side, then the part.
    let parts = [
        "Root",
        "Pelvis",
        "Hips",
        "Spine",
        "Spine1",
        "Spine2",
        "Spine3",
        "Spine4",
        "Chest",
        "Neck",
        "Neck1",
        "Neck2",
        "Head",
        "HeadNub",
        "HeadEnd",
        "Jaw",
        "Eye",
        "Eyes",
        "EyeLid",
        "Brow",
        "Lip",
        "Tongue",
        "Ponytail",
        "Ponytail1",
        "Ponytail2",
        "Hair",
        "Hair1",
        "Hair2",
        "Clavicle",
        "Shoulder",
        "UpperArm",
        "Arm",
        "Bicep",
        "ForeArm",
        "Forearm",
        "Elbow",
        "Wrist",
        "Hand",
        "Palm",
        "Thumb",
        "Thumb1",
        "Thumb2",
        "Thumb3",
        "Index",
        "Index1",
        "Index2",
        "Index3",
        "Middle",
        "Middle1",
        "Middle2",
        "Middle3",
        "Ring",
        "Ring1",
        "Ring2",
        "Pinky",
        "Pinky1",
        "Pinky2",
        "Finger",
        "Finger0",
        "Finger1",
        "Finger2",
        "Finger3",
        "Finger4",
        "Finger01",
        "Finger02",
        "Finger11",
        "Finger12",
        "Fingers",
        "Thigh",
        "UpLeg",
        "UpperLeg",
        "Leg",
        "Knee",
        "Calf",
        "Shin",
        "LowerLeg",
        "Ankle",
        "Foot",
        "Heel",
        "Toe",
        "Toe0",
        "Toe1",
        "Toes",
        "Ball",
        "Weapon",
        "Weapon1",
        "Prop",
        "Prop1",
        "Attach",
        "Grip",
        "Gun",
        "Holster",
        "Holster1",
        "Back",
        "BackPack",
        "Belt",
        "Pouch",
        "Hip",
        "Butt",
        "Breast",
        "Skirt",
        "Coat",
        "Twist",
        "ArmTwist",
        "ForeArmTwist",
        "ForearmTwist",
        "UpperArmTwist",
        "Helmet",
        "Hat",
        "Cap",
        "Mask",
        "Visor",
        "Camera",
        "Cam",
        "Aim",
        "Target",
        "Look",
        "LookAt",
        "Effect",
        "Fx",
        "Muzzle",
        "Flash",
        "Shell",
        "Eject",
        "Magazine",
        "Mag",
        "Clip",
        "Stock",
        "Barrel",
        "Scope",
        "Sight",
        "Trigger",
        "Handle",
        "Strap",
        "Sling",
        "Launcher",
        "Rocket",
        "Grenade",
        "Pin",
        "Spoon",
    ];
    for side in ["", "R", "L", "Right", "Left", "R_", "L_", "Rt", "Lt"] {
        for p in parts {
            for pre in ["Bone_", "Bone", "Node_", "Marker_", ""] {
                for suf in [
                    "", "1", "2", "3", "_1", "_2", "Nub", "_Nub", "End", "_End", "Twist",
                ] {
                    names.add(&format!("{pre}{side}{p}{suf}"));
                }
            }
        }
    }
    eprintln!(
        "names: {} in all ({:.0} s)",
        names.map.len(),
        started.elapsed().as_secs_f32()
    );
    Ok((names, got))
}

/// The catalogue (vehicles, people, weapons), read once.
struct Lazy {
    catalogue: Option<Catalogue>,
}

impl Lazy {
    fn catalogue<R: Read + Seek>(
        &mut self,
        source: &mut AssetSource<R>,
    ) -> Result<&Catalogue, Error> {
        if self.catalogue.is_none() {
            self.catalogue = Some(catalogue(source)?);
        }
        Ok(self.catalogue.as_ref().expect("just set"))
    }
}

fn hex(s: &str) -> Option<u32> {
    u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok()
}

fn floats(s: &str) -> Vec<f32> {
    s.split(',').filter_map(|v| v.trim().parse().ok()).collect()
}

fn main() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: survey <install dir> <section>... (see the source for the sections)");
        std::process::exit(2);
    };
    assert_eq!(hash_parts(&["vo_", "jen"]), m2_assets::name_hash("vo_jen"));
    let sections: Vec<(String, String)> = args[1..]
        .iter()
        .map(|s| match s.split_once(':') {
            Some((a, b)) => (a.to_ascii_lowercase(), b.to_owned()),
            None => (s.to_ascii_lowercase(), String::new()),
        })
        .collect();
    let install = Install::find(Path::new(root))?;
    let gather = sections.iter().any(|(s, _)| {
        matches!(
            s.as_str(),
            "slots" | "materials" | "fx" | "terrain" | "spot"
        )
    });
    let (names, got) = harvest(&install, gather)?;
    let mut source = install.open_world()?;
    let mut lazy = Lazy { catalogue: None };
    let mut world: Option<WorldIndex> = None;
    for (section, arg) in &sections {
        println!();
        println!("== {section} {arg}");
        let result = match section.as_str() {
            "voices" => voices(&install, &names),
            "people" => lazy.catalogue(&mut source).map(|c| {
                println!("people roles: {}", c.people.len());
                for p in &c.people {
                    println!("  {:08x} {} soldier {}", p.model, p.role, p.soldier);
                }
            }),
            "banks" => banks(&mut source, &names),
            "sounds" => sounds(&mut source, &names, arg),
            "rig" => rig(&mut source, &names, hex(arg).unwrap_or(DEFAULT_MODEL)),
            "weapons" => weapons(&mut source, &mut lazy, &names),
            "clips" => clips(&mut source, &names, arg),
            "slots" => {
                slots(&names, &got);
                Ok(())
            }
            "materials" => materials(&mut source, &mut lazy, &names, &got, arg),
            "terrain" => {
                let p = floats(arg);
                if world.is_none() {
                    world = Some(WorldIndex::build(&mut source)?);
                }
                terrain(
                    &mut source,
                    world.as_ref().expect("just built"),
                    &names,
                    &got,
                    p.first().copied().unwrap_or(0.0),
                    p.get(1).copied().unwrap_or(0.0),
                )
            }
            "fx" => {
                fx(&names, &got, arg);
                Ok(())
            }
            "spot" => {
                let p = floats(arg);
                if world.is_none() {
                    world = Some(WorldIndex::build(&mut source)?);
                }
                spot(
                    &mut source,
                    world.as_ref().expect("just built"),
                    &names,
                    &got,
                    p.first().copied().unwrap_or(0.0),
                    p.get(1).copied().unwrap_or(0.0),
                    p.get(2).copied().unwrap_or(30.0),
                )
            }
            "entities" => {
                // Layers whose name holds a word, or entities whose name
                // does; at most 400 a layer.
                let words: Vec<String> = arg
                    .split(',')
                    .filter(|w| !w.is_empty())
                    .map(str::to_ascii_lowercase)
                    .collect();
                layer_entities(
                    &mut source,
                    &names,
                    400,
                    |layer, name, _| {
                        let name = name.to_ascii_lowercase();
                        words.iter().any(|w| layer.contains(w) || name.contains(w))
                    },
                    |p| p.x,
                )
            }
            "collision" => collision(&mut source, &names, arg),
            "phy2rules" => phy2_rules(&mut source),
            "engines" => engines(&mut source, &names, arg),
            "vehicle" => vehicles(&mut source, &mut lazy, &names, arg),
            "templates" => templates(&mut source, &names, arg),
            "parents" => parents(&mut source, &names, arg),
            "find" => {
                for h in arg.split(',').filter_map(hex) {
                    println!("  {h:08x} {}", names.show(h));
                }
                Ok(())
            }
            other => {
                println!("no section {other}");
                Ok(())
            }
        };
        if let Err(e) = result {
            println!("{section}: {e}");
        }
    }
    Ok(())
}

fn voices(install: &Install, names: &Names) -> Result<(), Error> {
    let voices = Voices::load(install)?;
    let targets: HashSet<u32> = voices.sets.iter().map(|t| t.hash).collect();
    let built = names.resolve(
        &targets,
        &["vo_", "vo_vz", "vo_civ", ""],
        &[
            "", "_01", "_02", "_03", "01", "02", "_1", "_2", "_m", "_f", "_male", "_female",
        ],
    );
    println!("voice sets: {}", voices.sets.len());
    for t in &voices.sets {
        let name = names
            .get(t.hash)
            .map(str::to_owned)
            .or_else(|| built.get(&t.hash).cloned())
            .unwrap_or_else(|| "?".to_owned());
        println!(
            "  {:08x} {:5} lines {:4} groups  {name}",
            t.hash,
            t.lines.len(),
            t.groups.len()
        );
    }
    let mut vo: Vec<&str> = names
        .map
        .values()
        .map(String::as_str)
        .filter(|s| {
            s.len() < 48
                && s.get(..3).is_some_and(|p| p.eq_ignore_ascii_case("vo_"))
                && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
        .collect();
    vo.sort_unstable_by_key(|s| s.to_ascii_lowercase());
    vo.dedup_by_key(|s| s.to_ascii_lowercase());
    println!("vo_ names: {} (* = a voice set)", vo.len());
    for s in vo {
        let h = hash_parts(&[s]);
        println!(
            "  {h:08x} {s}{}",
            if targets.contains(&h) { " *" } else { "" }
        );
    }
    Ok(())
}

fn banks<R: Read + Seek>(source: &mut AssetSource<R>, names: &Names) -> Result<(), Error> {
    let index = source.archive().index();
    let mut list: Vec<(usize, String)> = index
        .assets()
        .iter()
        .filter(|a| index.type_hash(a) == SOUND_BANK_TYPE)
        .map(|a| {
            (
                usize::from(a.block),
                index.blocks()[usize::from(a.block)].path.clone(),
            )
        })
        .collect();
    list.sort();
    list.dedup();
    for (b, path) in list {
        let decoded = source.block(b)?;
        for (e, data) in BlockContents::parse(&decoded)?.iter() {
            if e.type_hash != SOUND_BANK_TYPE {
                continue;
            }
            match SoundBank::parse(data) {
                Ok(bank) => {
                    let seconds: f32 = bank
                        .sounds
                        .iter()
                        .map(|s| s.frames as f32 / s.rate.max(1) as f32)
                        .sum();
                    println!(
                        "  {path}  {:08x} {}  {} sounds {seconds:.0} s",
                        e.name_hash,
                        names.show(e.name_hash),
                        bank.sounds.len()
                    );
                }
                Err(err) => println!("  {path}  {:08x}: {err}", e.name_hash),
            }
        }
    }
    Ok(())
}

fn sounds<R: Read + Seek>(
    source: &mut AssetSource<R>,
    names: &Names,
    arg: &str,
) -> Result<(), Error> {
    let banks: Vec<&str> = if arg.is_empty() {
        vec![
            "veh_jeep",
            "veh_tankbig",
            "veh_tankmed",
            "veh_semi",
            "veh_8cylinderold",
            "veh_4cylindernew",
        ]
    } else {
        arg.split(',').collect()
    };
    let library = SoundLibrary::load(source, &banks)?;
    for bank in &banks {
        let sounds = library.bank(bank);
        let stem = bank.trim_start_matches("veh_");
        let targets: HashSet<u32> = sounds.iter().map(|s| s.hash).collect();
        let (b_, s_) = (format!("{bank}_"), format!("{stem}_"));
        let built = names.resolve(
            &targets,
            &[&b_, &s_, "veh_", "sfx_", "snd_", "eng_", "engine_", ""],
            &[
                "", "_01", "_02", "_03", "_04", "01", "02", "03", "04", "1", "2", "3", "_1", "_2",
                "_3", "_lp", "_loop", "_l", "_r", ".wav",
            ],
        );
        println!("bank {bank}: {} sounds", sounds.len());
        for s in sounds {
            let ch = usize::from(s.channels.max(1));
            let frames = s.samples.len() / ch;
            if frames == 0 {
                println!("  {:08x} empty", s.hash);
                continue;
            }
            let tenth = (frames / 10).max(1);
            let mut loud = Vec::new();
            let mut zcr = Vec::new();
            for k in 0..10 {
                let (from, to) = (k * tenth, ((k + 1) * tenth).min(frames));
                if from >= to {
                    break;
                }
                let first: Vec<f64> = (from..to)
                    .map(|i| f64::from(s.samples[i * ch]) / 32768.0)
                    .collect();
                let rms = (first.iter().map(|x| x * x).sum::<f64>() / first.len() as f64).sqrt();
                loud.push(format!("{:>4.0}", 20.0 * rms.max(1e-5).log10()));
                let crossings = first
                    .windows(2)
                    .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
                    .count();
                let per_second = crossings as f64 * f64::from(s.rate) / first.len() as f64;
                zcr.push(format!("{per_second:>5.0}"));
            }
            let name = names
                .get(s.hash)
                .map(str::to_owned)
                .or_else(|| built.get(&s.hash).cloned())
                .unwrap_or_else(|| "?".to_owned());
            println!(
                "  {:08x} {}ch {:>5} Hz {:>6.2} s  dB [{}]  zc/s [{}]  {name}",
                s.hash,
                s.channels,
                s.rate,
                s.seconds(),
                loud.join(""),
                zcr.join("")
            );
        }
    }
    Ok(())
}

/// Names for a model's nodes: known, or built from bone patterns.
fn node_names(names: &Names, hashes: &[u32]) -> HashMap<u32, String> {
    let targets: HashSet<u32> = hashes.iter().copied().collect();
    let mut out = names.resolve(
        &targets,
        &[
            "Bone_",
            "Bone_R",
            "Bone_L",
            "Bone_R_",
            "Bone_L_",
            "Bone_Right",
            "Bone_Left",
            "Marker_",
            "Node_",
            "Fx_",
            "",
        ],
        &[
            "", "1", "2", "3", "0", "01", "02", "_1", "_2", "Nub", "End", "_End",
        ],
    );
    for &h in hashes {
        if let Some(n) = names.get(h) {
            out.insert(h, n.to_owned());
        }
    }
    out
}

fn rig<R: Read + Seek>(
    source: &mut AssetSource<R>,
    names: &Names,
    model: u32,
) -> Result<(), Error> {
    let data = source.model(model)?;
    let parsed = data.parse()?;
    let skeleton = Skeleton::from_nodes(&parsed.nodes);
    let rest = skeleton.model_matrices(&skeleton.rest_pose());
    let named = node_names(names, &skeleton.name_hashes);
    let name = |i: usize| {
        named
            .get(&skeleton.name_hashes[i])
            .map_or("?", String::as_str)
    };
    println!(
        "model {model:08x}: {} nodes, {} named",
        skeleton.len(),
        (0..skeleton.len())
            .filter(|&i| named.contains_key(&skeleton.name_hashes[i]))
            .count()
    );
    for (i, m) in rest.iter().enumerate() {
        let p = m.w_axis.truncate();
        println!(
            "  {i:3} {:08x} parent {:>3} at ({:6.3} {:6.3} {:6.3})  {}",
            skeleton.name_hashes[i],
            skeleton.parents[i].map_or("-".to_owned(), |p| p.to_string()),
            p.x,
            p.y,
            p.z,
            name(i)
        );
    }
    for (label, hash) in [("right hand", RIGHT_HAND), ("left hand", LEFT_HAND)] {
        let Some(hand) = skeleton.name_hashes.iter().position(|&h| h == hash) else {
            println!("{label}: not in this skeleton");
            continue;
        };
        let inv = rest[hand].inverse();
        println!(
            "{label} ({}): x axis {:.2}, y axis {:.2}, z axis {:.2} in model space; under it, in hand space:",
            name(hand),
            rest[hand].x_axis.truncate(),
            rest[hand].y_axis.truncate(),
            rest[hand].z_axis.truncate()
        );
        for (i, m) in rest.iter().enumerate() {
            let mut at = skeleton.parents[i];
            let mut depth = 1;
            while let Some(j) = at {
                if j == hand {
                    break;
                }
                at = skeleton.parents[j];
                depth += 1;
            }
            if at != Some(hand) {
                continue;
            }
            let p = inv.transform_point3(m.w_axis.truncate());
            println!(
                "  {}{i:3} {:08x} at ({:6.3} {:6.3} {:6.3})  {}",
                "  ".repeat(depth - 1),
                skeleton.name_hashes[i],
                p.x,
                p.y,
                p.z,
                name(i)
            );
        }
    }
    Ok(())
}

fn weapons<R: Read + Seek>(
    source: &mut AssetSource<R>,
    lazy: &mut Lazy,
    names: &Names,
) -> Result<(), Error> {
    let list = lazy.catalogue(source)?.weapons.clone();
    for (label, hash) in list {
        let data = source.model(hash)?;
        let m = data.parse()?;
        let b = &m.info.bounds;
        let hashes: Vec<u32> = m.nodes.iter().map(|n| n.name_hash).collect();
        let named = node_names(names, &hashes);
        println!(
            "{label} {hash:08x} {}: bounds ({:.3} {:.3} {:.3}) to ({:.3} {:.3} {:.3}), {} nodes",
            names.show(hash),
            b.min[0],
            b.min[1],
            b.min[2],
            b.max[0],
            b.max[1],
            b.max[2],
            m.nodes.len()
        );
        for (i, n) in m.nodes.iter().enumerate() {
            let at = m
                .node_to_model(i as u16)
                .map_or(Vec3::ZERO, |t| t.w_axis.truncate());
            println!(
                "  {i:3} {:08x} parent {:>3} at ({:6.3} {:6.3} {:6.3})  {}",
                n.name_hash,
                n.parent.map_or("-".to_owned(), |p| p.to_string()),
                at.x,
                at.y,
                at.z,
                named.get(&n.name_hash).map_or("?", String::as_str)
            );
        }
    }
    Ok(())
}

fn clips<R: Read + Seek>(
    source: &mut AssetSource<R>,
    names: &Names,
    arg: &str,
) -> Result<(), Error> {
    let mut parts = arg.split(',').filter(|s| !s.is_empty());
    let first = parts.next();
    let (model, words): (u32, Vec<String>) = match first.and_then(hex) {
        Some(h) if first.is_some_and(|f| f.len() == 8) => (h, parts.map(str::to_owned).collect()),
        _ => (
            DEFAULT_MODEL,
            first.into_iter().chain(parts).map(str::to_owned).collect(),
        ),
    };
    let words: Vec<String> = if words.is_empty() {
        ["jennifer", "weapon", "resident"]
            .map(str::to_owned)
            .to_vec()
    } else {
        words
    };
    let skeleton = Skeleton::from_nodes(&source.model(model)?.parse()?.nodes);
    let index = source.archive().index();
    let paths: Vec<String> = index.blocks().iter().map(|b| b.path.clone()).collect();
    let mut blocks: Vec<usize> = index
        .assets()
        .iter()
        .filter(|a| index.type_hash(a) == ANIMATION_TYPE)
        .map(|a| usize::from(a.block))
        .collect();
    blocks.sort_unstable();
    blocks.dedup();
    // (hash, block, duration, speed, events)
    let mut rows: Vec<ClipRow> = Vec::new();
    let mut unbound = 0;
    for b in blocks {
        let decoded = source.block(b)?;
        for (e, data) in BlockContents::parse(&decoded)?.iter() {
            if e.type_hash != ANIMATION_TYPE {
                continue;
            }
            let Ok(clip) = Clip::parse(data) else {
                continue;
            };
            let events: Vec<(f32, String, String)> = clip
                .events
                .iter()
                .map(|ev| (ev.time, ev.name.clone(), ev.category.clone()))
                .collect();
            match BoundClip::bind(e.name_hash, clip, &skeleton) {
                Some(bound) => rows.push((
                    e.name_hash,
                    b,
                    bound.info.duration,
                    bound.info.speed,
                    events,
                )),
                None => unbound += 1,
            }
        }
    }
    let targets: HashSet<u32> = rows.iter().map(|r| r.0).collect();
    let built = names.resolve(
        &targets,
        &[
            "",
            "jen_",
            "jennifer_",
            "hum_",
            "human_",
            "anim_",
            "a_",
            "pmc_",
            "female_",
        ],
        &[
            "", "_loop", "_start", "_end", "_in", "_out", "_01", "_02", "01", "02", "_lp",
        ],
    );
    let clip_name = |h: u32| {
        names
            .get(h)
            .map(str::to_owned)
            .or_else(|| built.get(&h).cloned())
    };
    println!(
        "{} clips fit {model:08x} ({unbound} others); {} named",
        rows.len(),
        rows.iter().filter(|r| clip_name(r.0).is_some()).count()
    );
    let mut per_block: BTreeMap<&str, (usize, usize, usize)> = BTreeMap::new();
    for r in &rows {
        let e = per_block.entry(paths[r.1].as_str()).or_default();
        e.0 += 1;
        e.1 += usize::from(!r.4.is_empty());
        e.2 += usize::from(clip_name(r.0).is_some());
    }
    println!("blocks (clips, with events, named):");
    for (p, (n, ev, named)) in &per_block {
        println!("  {n:5} {ev:5} {named:5}  {p}");
    }
    let mut per_event: EventUses = BTreeMap::new();
    for r in &rows {
        let mut seen = HashSet::new();
        for (t, n, c) in &r.4 {
            if seen.insert((n.clone(), c.clone())) {
                per_event
                    .entry((c.clone(), n.clone()))
                    .or_default()
                    .push((r.0, *t, r.2));
            }
        }
    }
    println!("events (category / name: clips; examples hash@time/duration):");
    for ((c, n), list) in &per_event {
        let examples: Vec<String> = list
            .iter()
            .take(6)
            .map(|(h, t, d)| format!("{h:08x}@{t:.2}/{d:.2}"))
            .collect();
        println!("  {c:>8} / {n}: {}  {}", list.len(), examples.join(" "));
    }
    println!("named clips:");
    for r in &rows {
        if let Some(n) = clip_name(r.0) {
            println!(
                "  {:08x} {:5.2} s {:4.1} m/s  {n}  [{}]",
                r.0, r.2, r.3, paths[r.1]
            );
        }
    }
    println!("clips in blocks holding {}:", words.join(", "));
    for r in &rows {
        let p = paths[r.1].to_ascii_lowercase();
        if !words.iter().any(|w| p.contains(&w.to_ascii_lowercase())) {
            continue;
        }
        let mut evs: Vec<String> = Vec::new();
        for (t, n, c) in &r.4 {
            let e = if c.is_empty() {
                format!("{n}@{t:.2}")
            } else {
                format!("{c}:{n}@{t:.2}")
            };
            if !evs.contains(&e) && evs.len() < 6 {
                evs.push(e);
            }
        }
        println!(
            "  {:08x} {:5.2} s {:4.1} m/s  {}  {}  {}",
            r.0,
            r.2,
            r.3,
            clip_name(r.0).unwrap_or_default(),
            evs.join(" "),
            paths[r.1]
        );
    }
    Ok(())
}

/// The ending of a texture name that says what it is (`_n`, `_spec`...).
fn ending(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let last = lower.rsplit('_').next().unwrap_or("");
    if last.len() <= 6 && !last.chars().all(|c| c.is_ascii_digit()) {
        format!("_{last}")
    } else {
        "(none)".to_owned()
    }
}

fn slots(names: &Names, got: &Gathered) {
    // Per technique: materials, models, fill patterns, and per slot the
    // endings of the textures in it.
    struct Tech {
        materials: usize,
        models: HashSet<u32>,
        patterns: HashMap<String, usize>,
        slots: Vec<HashMap<String, (usize, String)>>,
    }
    let mut techs: HashMap<u32, Tech> = HashMap::new();
    let mut global: Vec<HashMap<String, usize>> = Vec::new();
    let mut named = 0usize;
    let mut slots_total = 0usize;
    for (model, mats) in &got.materials {
        for (tech, textures) in mats {
            let t = techs.entry(*tech).or_insert_with(|| Tech {
                materials: 0,
                models: HashSet::new(),
                patterns: HashMap::new(),
                slots: Vec::new(),
            });
            t.materials += 1;
            t.models.insert(*model);
            let pattern: String = textures
                .iter()
                .map(|&x| if x == 0 { '.' } else { '#' })
                .collect();
            *t.patterns.entry(pattern).or_default() += 1;
            for (i, &tex) in textures.iter().enumerate() {
                if tex == 0 {
                    continue;
                }
                slots_total += 1;
                let end = match names.get(tex) {
                    Some(n) => {
                        named += 1;
                        ending(n)
                    }
                    None => "(unnamed)".to_owned(),
                };
                if t.slots.len() <= i {
                    t.slots.resize_with(i + 1, HashMap::new);
                }
                let e = t.slots[i]
                    .entry(end.clone())
                    .or_insert((0, names.show(tex).to_owned()));
                e.0 += 1;
                if global.len() <= i {
                    global.resize_with(i + 1, HashMap::new);
                }
                *global[i].entry(end).or_default() += 1;
            }
        }
    }
    println!(
        "{} models, {} techniques, {slots_total} filled slots ({named} named)",
        got.materials.len(),
        techs.len()
    );
    println!("per slot, the commonest endings:");
    for (i, g) in global.iter().enumerate() {
        let mut v: Vec<(&String, &usize)> = g.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1));
        let top: Vec<String> = v.iter().take(8).map(|(e, n)| format!("{e} {n}")).collect();
        println!("  slot {i}: {}", top.join(", "));
    }
    let mut list: Vec<(&u32, &Tech)> = techs.iter().collect();
    list.sort_by_key(|t| std::cmp::Reverse(t.1.materials));
    println!("techniques by materials (fill pattern: # = texture):");
    for (h, t) in list.iter().take(60) {
        let mut pats: Vec<(&String, &usize)> = t.patterns.iter().collect();
        pats.sort_by(|a, b| b.1.cmp(a.1));
        println!(
            "  {h:08x} {}  {} materials in {} models; patterns {}",
            names.show(**h),
            t.materials,
            t.models.len(),
            pats.iter()
                .take(3)
                .map(|(p, n)| format!("{p} {n}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        for (i, s) in t.slots.iter().enumerate() {
            if s.is_empty() {
                continue;
            }
            let mut v: Vec<(&String, &(usize, String))> = s.iter().collect();
            v.sort_by_key(|e| std::cmp::Reverse(e.1.0));
            println!(
                "      slot {i}: {}",
                v.iter()
                    .take(4)
                    .map(|(e, (n, ex))| format!("{e} {n} (e.g. {ex})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
}

fn texture_line(names: &Names, got: &Gathered, tex: u32) -> String {
    let size = got
        .textures
        .get(&tex)
        .map_or("not in vz.wad".to_owned(), |(w, h, f)| {
            format!("{w}x{h} {f}")
        });
    format!("{tex:08x} {} {size}", names.show(tex))
}

fn print_model_materials(names: &Names, got: &Gathered, model: &Model<'_>) {
    for (i, m) in model.materials.iter().enumerate() {
        let constants: Vec<String> = m.constants.iter().map(|c| format!("{c:.2}")).collect();
        println!(
            "    material {i} {:08x} {}: technique {:08x} {}, flags {:#06x}, tail {:08x}",
            m.name_hash,
            names.show(m.name_hash),
            m.technique,
            names.show(m.technique),
            m.flags,
            m.unk_tail
        );
        println!("      constants {}", constants.join(" "));
        for (s, &t) in m.textures.iter().enumerate() {
            if t != 0 {
                println!("      slot {s}: {}", texture_line(names, got, t));
            }
        }
    }
    // Vertex layouts and the groups' effect hashes, once each.
    let mut seen = HashSet::new();
    for (_, g) in model.lod(0) {
        for group in &g.groups {
            let layout: Vec<String> = group
                .decl
                .elements()
                .iter()
                .map(|e| format!("{:?}{}:{:?}", e.usage, e.usage_index, e.ty))
                .collect();
            let key = (group.info.effect_hash, layout.join(" "));
            if seen.insert(key.clone()) {
                println!(
                    "    group effect {:08x} {} format {:08x}: {}",
                    group.info.effect_hash,
                    names.show(group.info.effect_hash),
                    group.info.format_hash,
                    key.1
                );
            }
        }
    }
}

fn materials<R: Read + Seek>(
    source: &mut AssetSource<R>,
    lazy: &mut Lazy,
    names: &Names,
    got: &Gathered,
    arg: &str,
) -> Result<(), Error> {
    for item in arg.split(',').filter(|s| !s.is_empty()) {
        let hashes: Vec<u32> = match hex(item).filter(|_| item.len() == 8) {
            Some(h) => vec![h],
            None => {
                let c = lazy.catalogue(source)?;
                let word = item.to_ascii_lowercase();
                c.vehicles
                    .iter()
                    .filter(|v| v.label.contains(&word))
                    .map(|v| v.model)
                    .chain(
                        c.people
                            .iter()
                            .filter(|p| p.role.contains(&word))
                            .map(|p| p.model),
                    )
                    .take(3)
                    .collect()
            }
        };
        if hashes.is_empty() {
            println!("{item}: no model");
        }
        for h in hashes {
            let data = source.model(h)?;
            let m = data.parse()?;
            println!("model {h:08x} {}:", names.show(h));
            print_model_materials(names, got, &m);
        }
    }
    Ok(())
}

fn terrain<R: Read + Seek>(
    source: &mut AssetSource<R>,
    world: &WorldIndex,
    names: &Names,
    got: &Gathered,
    x: f32,
    z: f32,
) -> Result<(), Error> {
    let Some((&cell, &centre)) = world.cells.iter().min_by(|a, b| {
        let d = |c: &Vec3| (c.x - x).powi(2) + (c.z - z).powi(2);
        d(a.1).total_cmp(&d(b.1))
    }) else {
        println!("no cells");
        return Ok(());
    };
    let data = source.asset(cell, TERRAIN_TYPE)?;
    let parsed = TerrainCell::parse(&data)?;
    println!(
        "cell {cell:08x} at ({:.0} {:.0} {:.0}): {} materials, {} patches",
        centre.x,
        centre.y,
        centre.z,
        parsed.materials.len(),
        parsed.patches.len()
    );
    for (i, m) in parsed.materials.iter().enumerate() {
        println!(
            "  material {i}: technique {:08x} {}, flags {:#06x}",
            m.material.technique,
            names.show(m.material.technique),
            m.material.flags
        );
        let constants: Vec<String> = m
            .material
            .constants
            .iter()
            .map(|c| format!("{c:.2}"))
            .collect();
        println!("    constants {}", constants.join(" "));
        for (s, &t) in m.material.textures.iter().enumerate() {
            if t != 0 {
                println!("    slot {s}: {}", texture_line(names, got, t));
            }
        }
        for (l, uv) in m.uv.iter().enumerate() {
            if uv.iter().any(|v| *v != 0.0) {
                println!(
                    "    uv {l}: u = ({:.4} {:.4} {:.4} {:.4}), v = ({:.4} {:.4} {:.4} {:.4})",
                    uv[0], uv[1], uv[2], uv[3], uv[4], uv[5], uv[6], uv[7]
                );
            }
        }
    }
    // The patch under the spot: its layout, and per material range the
    // shader and the mean of each vertex colour.
    let local = (x - centre.x, z - centre.z);
    let patch = parsed
        .patches
        .iter()
        .min_by(|a, b| {
            let d = |p: &m2_assets::terrain::Patch<'_>| {
                (p.offset[0] - local.0).powi(2) + (p.offset[2] - local.1).powi(2)
            };
            d(a).total_cmp(&d(b))
        })
        .ok_or("no patches")?;
    println!(
        "patch at ({:.0} {:.0} {:.0}) in the cell: {} groups",
        patch.offset[0],
        patch.offset[1],
        patch.offset[2],
        patch.groups.len()
    );
    for (gi, g) in patch.groups.iter().enumerate() {
        let layout: Vec<String> = g
            .decl
            .elements()
            .iter()
            .map(|e| format!("{:?}{}:{:?}", e.usage, e.usage_index, e.ty))
            .collect();
        println!(
            "  group {gi}: {} vertices, unk {:x?}; {}",
            g.vertices.count,
            g.unk_info,
            layout.join(" ")
        );
        let colours: Vec<(u8, Vec<[f32; 4]>)> = g
            .decl
            .elements()
            .iter()
            .filter(|e| e.usage == DeclUsage::Color)
            .filter_map(|e| Some((e.usage_index, g.vertices.read(e)?)))
            .collect();
        for r in &g.material_ranges {
            let tris = g.triangles(r);
            let used: HashSet<u16> = tris.iter().flatten().copied().collect();
            let mut line = format!(
                "    range: shader {:08x} {} material {} {} triangles",
                r.shader_hash,
                names.show(r.shader_hash),
                r.material,
                tris.len()
            );
            for (k, c) in &colours {
                let mut sum = [0.0f32; 4];
                let mut hi = [0.0f32; 4];
                for &v in &used {
                    if let Some(px) = c.get(usize::from(v)) {
                        for j in 0..4 {
                            sum[j] += px[j];
                            hi[j] = hi[j].max(px[j]);
                        }
                    }
                }
                let n = used.len().max(1) as f32;
                line += &format!(
                    "; colour{k} mean ({:.2} {:.2} {:.2} {:.2}) max ({:.2} {:.2} {:.2} {:.2})",
                    sum[0] / n,
                    sum[1] / n,
                    sum[2] / n,
                    sum[3] / n,
                    hi[0],
                    hi[1],
                    hi[2],
                    hi[3]
                );
            }
            println!("{line}");
        }
    }
    // Scatter: which way the clumps lean against the ground under them,
    // read as rows (x' = m0 x + m1 y + m2 z + m3) or as columns.
    let mut ground: HashMap<(i32, i32), (Vec3, Vec3)> = HashMap::new();
    for p in &parsed.patches {
        for g in &p.groups {
            let Some(ne) = g.decl.find(DeclUsage::Normal, 0) else {
                continue;
            };
            let Some(normals) = g.vertices.read(ne) else {
                continue;
            };
            for (i, pos) in g.positions().iter().enumerate() {
                let w = Vec3::from(*pos) + Vec3::from(p.offset);
                let n = normals.get(i).map_or(Vec3::Y, |n| {
                    Vec3::new(n[0], n[1], n[2]).normalize_or(Vec3::Y)
                });
                ground.insert(
                    ((w.x / 2.0).floor() as i32, (w.z / 2.0).floor() as i32),
                    (w, n),
                );
            }
        }
    }
    let (mut rows_closer, mut cols_closer, mut tilted, mut total) = (0, 0, 0, 0);
    let (mut rows_angle, mut cols_angle) = (0.0f32, 0.0f32);
    let (mut rows_height, mut cols_height) = (0.0f32, 0.0f32);
    for data in source.assets_beside(cell, SCATTER_TYPE)? {
        let Ok(s) = Scatter::parse(&data) else {
            continue;
        };
        for (_, patch_centre, m) in s.placements() {
            total += 1;
            let at = Vec3::from(patch_centre) + Vec3::new(m[3], m[7], m[11]);
            let key = ((at.x / 2.0).floor() as i32, (at.z / 2.0).floor() as i32);
            let Some(&(g, n)) = ground.get(&key) else {
                continue;
            };
            let up_rows = Vec3::new(m[1], m[5], m[9]).normalize_or(Vec3::Y);
            let up_cols = Vec3::new(m[4], m[5], m[6]).normalize_or(Vec3::Y);
            if up_rows.angle_between(up_cols) < 0.05 {
                continue;
            }
            tilted += 1;
            let (ar, ac) = (up_rows.angle_between(n), up_cols.angle_between(n));
            rows_angle += ar;
            cols_angle += ac;
            if ar < ac {
                rows_closer += 1;
            } else {
                cols_closer += 1;
            }
            rows_height += (at.y - g.y).abs();
            cols_height += (at.y - g.y).abs();
        }
    }
    let t = tilted.max(1) as f32;
    println!(
        "scatter: {total} clumps, {tilted} tilted; up as read now (rows) is nearer the ground's \
         normal for {rows_closer}, as columns for {cols_closer}; mean angle to the normal \
         {:.1} vs {:.1} deg; mean height off the nearest ground point {:.2} m ({:.2})",
        (rows_angle / t).to_degrees(),
        (cols_angle / t).to_degrees(),
        rows_height / t,
        cols_height / t
    );
    Ok(())
}

fn fx(names: &Names, got: &Gathered, arg: &str) {
    let words: Vec<String> = if arg.is_empty() {
        [
            "muzzle", "flash", "smoke", "fire", "flame", "explo", "spark", "tracer", "blood",
            "dust", "debris", "glow", "impact", "splash", "fx_", "shell", "burn", "ember", "heat",
            "shock", "flare", "decal", "road", "asphalt", "track",
        ]
        .map(str::to_owned)
        .to_vec()
    } else {
        arg.split(',').map(str::to_ascii_lowercase).collect()
    };
    let mut list: Vec<(String, u32)> = got
        .textures
        .keys()
        .filter_map(|&h| {
            let n = names.get(h)?;
            let lower = n.to_ascii_lowercase();
            words
                .iter()
                .any(|w| lower.contains(w.as_str()))
                .then(|| (n.to_owned(), h))
        })
        .collect();
    list.sort();
    println!("{} textures named with {}", list.len(), words.join(", "));
    for (_, h) in list {
        println!("  {}", texture_line(names, got, h));
    }
}

fn spot<R: Read + Seek>(
    source: &mut AssetSource<R>,
    world: &WorldIndex,
    names: &Names,
    got: &Gathered,
    x: f32,
    z: f32,
    radius: f32,
) -> Result<(), Error> {
    let d = |p: Vec3| ((p.x - x).powi(2) + (p.z - z).powi(2)).sqrt();
    let mut objects: Vec<(f32, usize)> = world
        .objects
        .iter()
        .enumerate()
        .map(|(i, o)| (d(o.position), i))
        .filter(|(dist, _)| *dist < radius)
        .collect();
    objects.sort_by(|a, b| a.0.total_cmp(&b.0));
    println!("{} placed models within {radius} m:", objects.len());
    let mut shown = HashSet::new();
    for (dist, i) in objects {
        let o = &world.objects[i];
        println!(
            "  {:08x} {} {dist:.1} m at ({:.1} {:.1} {:.1}){}",
            o.model,
            names.show(o.model),
            o.position.x,
            o.position.y,
            o.position.z,
            if o.named { " (by name)" } else { "" }
        );
        if shown.insert(o.model)
            && let Ok(data) = source.model(o.model)
            && let Ok(m) = data.parse()
        {
            let b = &m.info.bounds;
            println!(
                "    bounds ({:.1} {:.1} {:.1}) to ({:.1} {:.1} {:.1}), flags {:#x}, {} materials",
                b.min[0],
                b.min[1],
                b.min[2],
                b.max[0],
                b.max[1],
                b.max[2],
                m.info.flags,
                m.materials.len()
            );
            print_model_materials(names, got, &m);
        }
    }
    // The layer entities near the spot, with their components' fields.
    layer_entities(source, names, usize::MAX, |_, _, p| d(p) < radius, d)
}

/// Each model's collision as the alpha builds it: its `PHY2` shapes
/// placed in model space, or else its visible triangles.
fn collision<R: Read + Seek>(
    source: &mut AssetSource<R>,
    names: &Names,
    arg: &str,
) -> Result<(), Error> {
    use m2_assets::physics::{Geometry, Physics as Phy2};
    for h in arg.split(',').filter_map(hex) {
        let data = match source.model(h) {
            Ok(d) => d,
            Err(e) => {
                println!("{h:08x}: {e}");
                continue;
            }
        };
        let m = data.parse()?;
        let b = &m.info.bounds;
        println!(
            "{h:08x} {}: bounds ({:.2} {:.2} {:.2}) to ({:.2} {:.2} {:.2}), {} nodes",
            names.show(h),
            b.min[0],
            b.min[1],
            b.min[2],
            b.max[0],
            b.max[1],
            b.max[2],
            m.nodes.len()
        );
        let phy2 = m2_assets::Ucfx::parse(&data.base)
            .ok()
            .and_then(|u| u.top_level(*b"PHY2"));
        let Some(p) = phy2.map(Phy2::parse) else {
            let cpu = m2_engine::CpuModel::from_data(&data, 0)?;
            println!(
                "  no PHY2: visible triangles stand in ({} collision)",
                if cpu.collision.is_some() { "a" } else { "no" }
            );
            continue;
        };
        let p = match p {
            Ok(p) => p,
            Err(e) => {
                println!("  PHY2 does not parse: {e}");
                continue;
            }
        };
        println!(
            "  PHY2: {} parts, {} bodies ({} fixed), {} shapes",
            p.parts.len(),
            p.bodies.len(),
            p.bodies.iter().filter(|b| b.is_fixed()).count(),
            p.shapes.len()
        );
        for (k, shape) in p.shapes.iter().enumerate() {
            let node = p.shape_node(k);
            let to_model = node
                .and_then(|n| u16::try_from(n).ok())
                .and_then(|n| m.node_to_model(n))
                .unwrap_or(m2_math::glam::Mat4::IDENTITY);
            let (v, t) = shape.geometry.triangles(shape.radius);
            let mut lo = Vec3::splat(f32::MAX);
            let mut hi = Vec3::splat(f32::MIN);
            for x in &v {
                let q = to_model.transform_point3(Vec3::from(*x));
                lo = lo.min(q);
                hi = hi.max(q);
            }
            let kind = match &shape.geometry {
                Geometry::Box { .. } => "box",
                Geometry::Sphere => "sphere",
                Geometry::Capsule { .. } => "capsule",
                Geometry::Convex { .. } => "convex",
                Geometry::Mesh { .. } => "mesh",
            };
            let node_name = node
                .and_then(|n| m.nodes.get(n as usize))
                .map_or("?", |n| names.show(n.name_hash));
            println!(
                "  shape {k}: {kind}, node {node:?} {node_name}, radius {:.3}, {} vertices, {} triangles, box ({:.2} {:.2} {:.2}) to ({:.2} {:.2} {:.2}), part {}",
                shape.radius,
                v.len(),
                t.len(),
                lo.x,
                lo.y,
                lo.z,
                hi.x,
                hi.y,
                hi.z,
                shape.part
            );
        }
        for (i, part) in p.parts.iter().enumerate() {
            let at = m
                .node_to_model(part.node as u16)
                .map(|n| n.w_axis.truncate());
            println!(
                "  part {i}: node {} {} at {:?}, body {:?}, shape {:?}, parent {:?}, position ({:.3} {:.3} {:.3}), rotation ({:.3} {:.3} {:.3} {:.3})",
                part.node,
                names.show(part.node_hash),
                at.map(|a| format!("({:.2} {:.2} {:.2})", a.x, a.y, a.z)),
                part.body,
                part.shape,
                part.parent,
                part.position[0],
                part.position[1],
                part.position[2],
                part.rotation[0],
                part.rotation[1],
                part.rotation[2],
                part.rotation[3]
            );
        }
    }
    Ok(())
}

/// Over every model's `PHY2`, how many shapes land inside the model's
/// bounds (0.25 m slack) when placed by each rule: the body part's node
/// (the alpha's), that node then the shape part's own place, that node
/// then the body part's place, the shape part's place alone, and the
/// shape part's own node then its place. Counts by shape kind, and how
/// many parts carry a place at all.
fn phy2_rules<R: Read + Seek>(source: &mut AssetSource<R>) -> Result<(), Error> {
    use m2_assets::physics::{Geometry, Physics as Phy2};
    use m2_math::glam::{Mat4, Quat};
    let index = source.archive().index();
    let mut models: Vec<u32> = index
        .assets()
        .iter()
        .filter(|a| index.type_hash(a) == MODEL_TYPE)
        .map(|a| a.name_hash)
        .collect();
    models.sort_unstable();
    models.dedup();
    const RULES: [&str; 5] = [
        "body node",
        "body node * shape part",
        "body node * body part",
        "shape part alone",
        "shape part's node * shape part",
    ];
    // Per kind: shapes, inside per rule.
    let mut tally: BTreeMap<&str, (usize, [usize; 5])> = BTreeMap::new();
    let (mut parts, mut placed_parts, mut with_phy2) = (0usize, 0usize, 0usize);
    let mut examples: Vec<String> = Vec::new();
    for h in models {
        let Ok(data) = source.model(h) else { continue };
        let Ok(m) = data.parse() else { continue };
        let Some(Ok(p)) = m2_assets::Ucfx::parse(&data.base)
            .ok()
            .and_then(|u| u.top_level(*b"PHY2"))
            .map(Phy2::parse)
        else {
            continue;
        };
        with_phy2 += 1;
        let b = &m.info.bounds;
        let (lo, hi) = (
            Vec3::from(b.min) - Vec3::splat(0.25),
            Vec3::from(b.max) + Vec3::splat(0.25),
        );
        let place = |part: &m2_assets::physics::Part| {
            Mat4::from_rotation_translation(
                Quat::from_array(part.rotation).normalize(),
                Vec3::from(part.position),
            )
        };
        let node = |n: u32| {
            u16::try_from(n)
                .ok()
                .and_then(|n| m.node_to_model(n))
                .unwrap_or(Mat4::IDENTITY)
        };
        for part in &p.parts {
            parts += 1;
            let q = Quat::from_array(part.rotation);
            if Vec3::from(part.position).length() > 1e-3 || (q.w.abs() - 1.0).abs() > 1e-3 {
                placed_parts += 1;
            }
        }
        for (k, shape) in p.shapes.iter().enumerate() {
            let kind = match &shape.geometry {
                Geometry::Box { .. } => "box",
                Geometry::Sphere => "sphere",
                Geometry::Capsule { .. } => "capsule",
                Geometry::Convex { .. } => "convex",
                Geometry::Mesh { .. } => "mesh",
            };
            let Some(own) = p.parts.get(shape.part) else {
                continue;
            };
            let body_part = p
                .body_of(k)
                .and_then(|body| p.parts.iter().find(|x| x.body == Some(body)));
            let body_node = p.shape_node(k).map_or(Mat4::IDENTITY, node);
            let rules = [
                body_node,
                body_node * place(own),
                body_node * body_part.map_or(Mat4::IDENTITY, place),
                place(own),
                node(own.node) * place(own),
            ];
            let (v, _) = shape.geometry.triangles(shape.radius);
            if v.is_empty() {
                continue;
            }
            let e = tally.entry(kind).or_default();
            e.0 += 1;
            for (r, to_model) in rules.iter().enumerate() {
                let (mut a, mut z) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                for x in &v {
                    let q = to_model.transform_point3(Vec3::from(*x));
                    a = a.min(q);
                    z = z.max(q);
                }
                let mid = (a + z) * 0.5;
                if mid.cmpge(lo).all() && mid.cmple(hi).all() {
                    e.1[r] += 1;
                }
            }
            if examples.len() < 12 && (Vec3::from(own.position).length() > 1e-3) {
                examples.push(format!(
                    "{h:08x} shape {k} {kind}: part {} node {} position ({:.2} {:.2} {:.2}) rotation ({:.2} {:.2} {:.2} {:.2}), body part node {:?}",
                    shape.part,
                    own.node,
                    own.position[0],
                    own.position[1],
                    own.position[2],
                    own.rotation[0],
                    own.rotation[1],
                    own.rotation[2],
                    own.rotation[3],
                    body_part.map(|b| b.node)
                ));
            }
        }
    }
    println!(
        "{with_phy2} models with PHY2; {parts} parts, {placed_parts} of them with a place (not identity)"
    );
    for (kind, (n, inside)) in &tally {
        let shown: Vec<String> = RULES
            .iter()
            .zip(inside)
            .map(|(r, k)| format!("{r} {k}"))
            .collect();
        println!(
            "  {kind}: {n} shapes; inside the bounds by {}",
            shown.join(", ")
        );
    }
    for e in examples {
        println!("  e.g. {e}");
    }
    Ok(())
}

/// The engine banks (all, or those named), read by name hash as the
/// template database names them: each sound's measurements as the alpha
/// takes them (length, level range over its tenths, steady or not, zero
/// crossings and repeating pitch), with its name if known, then what the
/// alpha would play (its loops, start and stop) and which templates name
/// the bank.
fn engines<R: Read + Seek>(
    source: &mut AssetSource<R>,
    names: &Names,
    arg: &str,
) -> Result<(), Error> {
    use m2_engine::audio::EngineSounds;
    const BANKS: [&str; 22] = [
        "veh_semi",
        "veh_bus",
        "veh_jeep",
        "veh_tankbig",
        "veh_tankmed",
        "veh_tanksmall",
        "veh_tankdiesel",
        "veh_8cylinderold",
        "veh_8cylinderold_cobra",
        "veh_8cylindertruck",
        "veh_8cylindernew",
        "veh_8cylinderluxury",
        "veh_8cylinderjapanese",
        "veh_4cylindernew",
        "veh_4cylinderold",
        "veh_4cylinderracing",
        "veh_largedieselnew",
        "veh_largedieselold",
        "veh_largegasnew",
        "veh_largegasold",
        "veh_largegascrappy",
        "veh_helicoptermedium",
    ];
    let banks: Vec<&str> = if arg.is_empty() {
        BANKS.to_vec()
    } else {
        arg.split(',').collect()
    };
    let hashes: Vec<u32> = banks.iter().map(|b| m2_assets::name_hash(b)).collect();
    let library = SoundLibrary::load_hashes(source, &hashes)?;
    // Which templates name each bank.
    let mut users: HashMap<u32, Vec<String>> = HashMap::new();
    if let Ok(index) = WorldIndex::build(source) {
        for (name, t) in &index.template_db.by_name {
            if let Some(bank) = t.engine {
                users.entry(bank).or_default().push(name.clone());
            }
        }
    }
    for (bank, hash) in banks.iter().zip(&hashes) {
        let Some((name, sounds)) = library.bank_hash(*hash) else {
            println!("bank {bank} ({hash:08x}): not found");
            continue;
        };
        let mut used = users.remove(hash).unwrap_or_default();
        used.sort();
        let shown = used.len().min(12);
        println!(
            "bank {name} ({hash:08x}): {} sounds; templates {}: {}{}",
            sounds.len(),
            used.len(),
            used[..shown].join(" | "),
            if used.len() > shown { " | ..." } else { "" }
        );
        for (i, s) in sounds.iter().enumerate() {
            let tenths = s.tenths_db();
            let (lo, hi) = tenths
                .iter()
                .fold((f32::MAX, f32::MIN), |(lo, hi), &d| (lo.min(d), hi.max(d)));
            println!(
                "  {i:2} {:08x} {:7.2} s, tenths {lo:6.1}..{hi:6.1} dB (first {:6.1}, last {:6.1}), {}, {:5.0} crossings/s, pitch {}  {}",
                s.hash,
                s.seconds(),
                tenths.first().copied().unwrap_or(-100.0),
                tenths.last().copied().unwrap_or(-100.0),
                if s.is_steady() { "steady" } else { "varies" },
                s.crossing_rate(),
                s.fundamental()
                    .map_or("-".to_owned(), |f| format!("{f:.0} Hz")),
                names.show(s.hash)
            );
        }
        let e = EngineSounds::pick(name, sounds);
        let loops: Vec<String> = e
            .loops
            .iter()
            .map(|(s, f)| {
                format!(
                    "{:08x} ({})",
                    s.hash,
                    f.map_or("-".to_owned(), |f| format!("{f:.0} Hz"))
                )
            })
            .collect();
        println!(
            "  plays {} loops{}: {}; start {:?}, stop {:?}",
            e.loops.len(),
            if e.named { " by name" } else { " measured" },
            loops.join(", "),
            e.start.map(|s| format!("{:08x}", s.hash)),
            e.stop.map(|s| format!("{:08x}", s.hash))
        );
    }
    Ok(())
}

/// Every layer entity `keep` (layer, entity name, place) keeps, with its
/// components' fields, at most `most` per layer, ordered by `order`.
/// Vehicles whose label (or kind) holds one of the words, or of a model
/// hash: the spec the alpha builds, the wheels found, then every node with
/// its own geometry's box and the wheel tests (`d` the box's larger side
/// seen from the side; round, thin, grounded; `off` how far the box's
/// middle is from the node's origin).
fn vehicles<R: Read + Seek>(
    source: &mut AssetSource<R>,
    lazy: &mut Lazy,
    names: &Names,
    arg: &str,
) -> Result<(), Error> {
    let words: Vec<String> = arg
        .split(',')
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    let list: Vec<CatalogueEntry> = lazy
        .catalogue(source)?
        .vehicles
        .iter()
        .filter(|v| {
            words.iter().any(|w| {
                v.label.contains(w.as_str())
                    || v.kind == *w
                    || (w.len() == 8 && hex(w) == Some(v.model))
            })
        })
        .cloned()
        .collect();
    println!("{} vehicles", list.len());
    let v3 = |v: Vec3| format!("({:.3} {:.3} {:.3})", v.x, v.y, v.z);
    let a3 = |v: [f32; 3]| format!("({:.3} {:.3} {:.3})", v[0], v[1], v[2]);
    for e in list.iter().take(12) {
        let data = source.model(e.model)?;
        let m = data.parse()?;
        let b = &m.info.bounds;
        println!();
        println!(
            "{:08x} {} {}: {} nodes, bounds {} to {}",
            e.model,
            e.kind,
            e.label,
            m.nodes.len(),
            a3(b.min),
            a3(b.max)
        );
        match VehicleSpec::from_model(e, &m) {
            None => println!("  spec: none (no wheels found)"),
            Some(spec) => {
                println!(
                    "  spec: {} wheels, tracked {}, {} road wheels, pivot {}, muzzle {}, seat {}, door {}",
                    spec.wheels.len(),
                    spec.tracked,
                    spec.road_wheels.len(),
                    spec.pivot.map_or("-".to_owned(), v3),
                    spec.muzzle.map_or("-".to_owned(), v3),
                    spec.seat.map_or("-".to_owned(), v3),
                    spec.door.map_or("-".to_owned(), v3),
                );
                for (i, w) in spec.wheels.iter().enumerate() {
                    println!(
                        "    wheel {i}: centre {} radius {:.3} width {:.3} steers {}",
                        v3(w.centre),
                        w.radius,
                        w.width,
                        w.steers
                    );
                }
            }
        }
        let found = m2_assets::vehicle::find_wheels(&m);
        println!("  found wheels: {}", found.len());
        for w in &found {
            println!(
                "    nodes {:?} centre {} radius {:.3} width {:.3}",
                w.nodes,
                a3(w.centre),
                w.radius,
                w.width
            );
        }
        let boxes = m2_assets::vehicle::node_boxes(&m);
        let floor = boxes.iter().map(|b| b.min[1]).fold(f32::INFINITY, f32::min);
        println!("  floor {floor:.3}, {} nodes with geometry", boxes.len());
        let hashes: Vec<u32> = m.nodes.iter().map(|n| n.name_hash).collect();
        let named = node_names(names, &hashes);
        for (i, n) in m.nodes.iter().enumerate() {
            let at = m
                .node_to_model(i as u16)
                .map_or(Vec3::ZERO, |t| t.w_axis.truncate());
            let mut line = format!(
                "  {i:3} {:08x} parent {:>3} at {}",
                n.name_hash,
                n.parent.map_or("-".to_owned(), |p| p.to_string()),
                v3(at)
            );
            if let Some(bx) = boxes.iter().find(|bx| usize::from(bx.node) == i) {
                let sz = [0, 1, 2].map(|a| bx.max[a] - bx.min[a]);
                let d = sz[1].max(sz[2]);
                let round = (sz[1] - sz[2]).abs() <= 0.15 * d;
                let thin = sz[0] <= 0.85 * d && sz[0] >= 0.03;
                let grounded = bx.min[1] <= floor + 0.1 * d + 0.05;
                let mid = Vec3::from([0, 1, 2].map(|a| 0.5 * (bx.min[a] + bx.max[a])));
                line += &format!(
                    " box {} to {} d {d:.3}{}{}{} off {:.3}",
                    a3(bx.min),
                    a3(bx.max),
                    if round { " round" } else { "" },
                    if thin { " thin" } else { "" },
                    if grounded { " grounded" } else { "" },
                    mid.distance(at)
                );
            }
            println!(
                "{line}  {}",
                named.get(&n.name_hash).map_or("?", String::as_str)
            );
        }
    }
    Ok(())
}

/// Templates of the template database whose name holds one of the words,
/// each with every component it has and that component's fields. Text
/// fields are not read (a component with one is only listed).
fn templates<R: Read + Seek>(
    source: &mut AssetSource<R>,
    names: &Names,
    arg: &str,
) -> Result<(), Error> {
    let words: Vec<String> = arg
        .split(',')
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    let index = source.archive().index();
    let Some(hash) = index
        .assets()
        .iter()
        .find(|a| index.type_hash(a) == TEMPLATE_DB_TYPE)
        .map(|a| a.name_hash)
    else {
        println!("no template database");
        return Ok(());
    };
    let data = source.asset(hash, TEMPLATE_DB_TYPE)?;
    let db = Layer::parse(&data)?;
    let tnames = db.names();
    println!(
        "template database: {} templates, {} components",
        tnames.len(),
        db.components.len()
    );
    let records: Vec<ComponentRecords<'_>> = db
        .components
        .iter()
        .filter(|c| c.name != "Name")
        .map(|c| {
            (
                c.name,
                c.fields.clone(),
                c.records().unwrap_or_default().into_iter().collect(),
            )
        })
        .collect();
    let texty: Vec<&str> = db
        .components
        .iter()
        .filter(|c| c.fields.iter().any(|f| f.ty == STRING_FIELD))
        .map(|c| c.name)
        .collect();
    println!("components with text (not read): {}", texty.join(", "));
    let ids: Vec<u32> = words
        .iter()
        .filter(|w| w.len() == 8)
        .filter_map(|w| hex(w))
        .collect();
    let mut picked: Vec<(&str, u32)> = tnames
        .iter()
        .filter(|(id, n)| {
            let n = n.to_ascii_lowercase();
            ids.contains(id) || words.iter().any(|w| n.contains(w.as_str()))
        })
        .map(|(&id, &n)| (n, id))
        .collect();
    picked.sort_unstable();
    println!("{} templates match", picked.len());
    // Each template's fields; a field naming another template gets its
    // name, and is gathered to show after.
    let mut named: Vec<u32> = Vec::new();
    let show = |id: u32, name: &str, named: &mut Vec<u32>| {
        println!("  {id:08x} {name:?}");
        for (cname, fields, recs) in &records {
            let Some(rec) = recs.get(&id) else {
                continue;
            };
            let values: Vec<String> = fields
                .iter()
                .map(|f| {
                    let shown = field_value(names, f, rec);
                    let at = usize::from(f.offset);
                    let other = (f.ty != STRING_FIELD && f.bit_width == 0)
                        .then(|| rec.get(at..at + 4))
                        .flatten()
                        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                        .filter(|v| *v != id)
                        .and_then(|v| tnames.get(&v).map(|n| (v, n)));
                    match other {
                        Some((v, n)) => {
                            if !named.contains(&v) {
                                named.push(v);
                            }
                            format!("{shown} -> {n:?}")
                        }
                        None => shown,
                    }
                })
                .collect();
            println!("    {cname}: {}", values.join(", "));
        }
    };
    let shown: Vec<u32> = picked.iter().take(150).map(|p| p.1).collect();
    for &(name, id) in picked.iter().take(150) {
        show(id, name, &mut named);
    }
    named.retain(|v| !shown.contains(v));
    if !named.is_empty() {
        println!("{} templates they name", named.len());
        let mut more = Vec::new();
        for id in named.into_iter().take(60) {
            show(id, tnames[&id], &mut more);
        }
    }
    Ok(())
}

/// How the layers' entities name their template through `Transform`'s
/// `Parent`, and the parents of the entities of some layers in full.
fn parents<R: Read + Seek>(
    source: &mut AssetSource<R>,
    names: &Names,
    arg: &str,
) -> Result<(), Error> {
    let words: Vec<String> = if arg.is_empty() {
        vec!["vzacon001".into(), "vza_con001".into()]
    } else {
        arg.split(',')
            .filter(|w| !w.is_empty())
            .map(str::to_ascii_lowercase)
            .collect()
    };
    let index = source.archive().index();
    let Some(db_hash) = index
        .assets()
        .iter()
        .find(|a| index.type_hash(a) == TEMPLATE_DB_TYPE)
        .map(|a| a.name_hash)
    else {
        println!("no template database");
        return Ok(());
    };
    let mut blocks: Vec<usize> = index
        .assets()
        .iter()
        .filter(|a| index.type_hash(a) == LAYER_TYPE)
        .map(|a| usize::from(a.block))
        .collect();
    blocks.sort_unstable();
    blocks.dedup();
    let paths: Vec<String> = index.blocks().iter().map(|b| b.path.clone()).collect();
    let db_data = source.asset(db_hash, TEMPLATE_DB_TYPE)?;
    let db = Layer::parse(&db_data)?;
    let tnames: HashMap<u32, String> = db
        .names()
        .into_iter()
        .map(|(id, n)| (id, n.to_owned()))
        .collect();
    let uniq: HashSet<u32> = db
        .uniq
        .get(4..)
        .unwrap_or_default()
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    let mut comps: HashMap<u32, Vec<&str>> = HashMap::new();
    for c in &db.components {
        for (id, _) in c.records().unwrap_or_default() {
            comps.entry(id).or_default().push(c.name);
        }
    }
    let tmodels = db.hashes("ModelName");
    println!(
        "template database: {} names, {} UNIQ ids",
        tnames.len(),
        uniq.len()
    );

    // Every layer's entities.
    struct Row {
        layer: String,
        is_static: bool,
        id: u32,
        name: String,
        parent: u32,
        at: [f32; 3],
        components: Vec<String>,
    }
    let mut rows: Vec<Row> = Vec::new();
    for b in blocks {
        let decoded = source.block(b)?;
        let is_static = paths[b].contains("layers_static");
        for (e, data) in BlockContents::parse(&decoded)?.iter() {
            if e.type_hash != LAYER_TYPE {
                continue;
            }
            let Ok(layer) = Layer::parse(data) else {
                continue;
            };
            let layer_name = names.show(e.name_hash).to_owned();
            let entity_names = layer.names();
            let mut own: HashMap<u32, Vec<String>> = HashMap::new();
            for c in &layer.components {
                if matches!(c.name, "Transform" | "Name") {
                    continue;
                }
                for (id, _) in c.records().unwrap_or_default() {
                    own.entry(id).or_default().push(c.name.to_owned());
                }
            }
            for (id, p) in layer.placements() {
                rows.push(Row {
                    layer: layer_name.clone(),
                    is_static,
                    id,
                    name: entity_names.get(&id).copied().unwrap_or("").to_owned(),
                    parent: p.parent,
                    at: p.position,
                    components: own.remove(&id).unwrap_or_default(),
                });
            }
        }
    }
    let entity_ids: HashSet<u32> = rows.iter().map(|r| r.id).collect();
    let count = |f: &dyn Fn(&Row) -> bool| rows.iter().filter(|r| f(r)).count();
    println!("entities: {}", rows.len());
    println!("  parent 0: {}", count(&|r| r.parent == 0));
    println!(
        "  parent with bit 31: {}",
        count(&|r| r.parent & 0x8000_0000 != 0)
    );
    println!(
        "  parent a template's id (names): {}",
        count(&|r| tnames.contains_key(&r.parent))
    );
    println!("  parent in UNIQ: {}", count(&|r| uniq.contains(&r.parent)));
    println!(
        "  parent an entity's id: {}",
        count(&|r| entity_ids.contains(&r.parent))
    );
    let mut distinct: HashMap<u32, usize> = HashMap::new();
    for r in &rows {
        *distinct.entry(r.parent).or_default() += 1;
    }
    println!("  distinct parents: {}", distinct.len());
    let mut top: Vec<(usize, u32)> = distinct.iter().map(|(&p, &n)| (n, p)).collect();
    top.sort_unstable_by(|a, b| b.cmp(a));
    for (n, p) in top.iter().take(15) {
        println!(
            "    {p:08x} x{n}: {:?}",
            tnames.get(p).map_or("(not a template)", String::as_str)
        );
    }
    // Entities whose name carries a template (`VZ Soldier 0x0012abcd`,
    // `_global_pile02 3`): is the parent that template?
    let mut same = 0;
    let mut differ: Vec<&Row> = Vec::new();
    let mut unnamed_parent = 0;
    for r in &rows {
        let stem = m2_engine::world::member_stem(&r.name);
        if stem == r.name.trim_start_matches('_') {
            continue;
        }
        match tnames.get(&r.parent) {
            Some(t) if t.eq_ignore_ascii_case(stem) => same += 1,
            Some(_) => differ.push(r),
            None => unnamed_parent += 1,
        }
    }
    println!(
        "named entities: parent is the named template {same}, another template {}, not a template {unnamed_parent}",
        differ.len()
    );
    for r in differ.iter().take(20) {
        println!(
            "    {:?} -> {:08x} {:?}",
            r.name,
            r.parent,
            tnames.get(&r.parent).map_or("", String::as_str)
        );
    }
    let missing: Vec<&Row> = rows
        .iter()
        .filter(|r| r.parent != 0 && !tnames.contains_key(&r.parent))
        .take(12)
        .collect();
    for r in missing {
        println!(
            "    not a template: {:?} in {} -> {:08x}",
            r.name, r.layer, r.parent
        );
    }

    // What a parent would add: props (a model, no vehicle physics, not a
    // person) and people, for entities with no ModelName.
    let vehicle = |id: u32| {
        comps.get(&id).is_some_and(|c| {
            c.iter().any(|n| {
                matches!(
                    *n,
                    "_CarPhysicsV2" | "_TankPhysics" | "_BoatPhysics" | "_HelicopterPhysics"
                )
            })
        })
    };
    let has = |id: u32, what: &str| comps.get(&id).is_some_and(|c| c.contains(&what));
    let person = |id: u32| {
        has(id, "_HumanPhysics") || ((has(id, "Ai") || has(id, "AiBehavior")) && !vehicle(id))
    };
    for is_static in [true, false] {
        let of =
            |r: &&Row| r.is_static == is_static && !r.components.iter().any(|c| c == "ModelName");
        let props = rows
            .iter()
            .filter(of)
            .filter(|r| tmodels.contains_key(&r.parent) && !vehicle(r.parent) && !person(r.parent))
            .count();
        let people = rows.iter().filter(of).filter(|r| person(r.parent)).count();
        println!(
            "{} layers, entities without ModelName: parent a prop with a model {props}, a person {people}",
            if is_static { "static" } else { "state" }
        );
    }
    println!(
        "templates with _HumanPhysics {}, with Ai {}, with AiBehavior {}",
        comps
            .values()
            .filter(|c| c.contains(&"_HumanPhysics"))
            .count(),
        comps.values().filter(|c| c.contains(&"Ai")).count(),
        comps.values().filter(|c| c.contains(&"AiBehavior")).count()
    );
    let mut people_templates: Vec<(&str, u32)> = tnames
        .iter()
        .filter(|&(&id, n)| {
            let n = n.to_ascii_lowercase();
            person(id)
                || ["soldier", "elite", "deathsquad", "gate", "carmona", "guard"]
                    .iter()
                    .any(|w| n.contains(w))
        })
        .map(|(&id, n)| (n.as_str(), id))
        .collect();
    people_templates.sort_unstable();
    println!(
        "person templates and names with soldier, elite, deathsquad, gate, carmona, guard: {}",
        people_templates.len()
    );
    for (n, id) in people_templates.iter().take(60) {
        let mut c = comps.get(id).cloned().unwrap_or_default();
        c.sort_unstable();
        println!(
            "    {id:08x} {n:?} person {} model {}: {}",
            person(*id),
            tmodels
                .get(id)
                .map_or("-".to_owned(), |m| format!("{m:08x} {}", names.show(*m))),
            c.join(" ")
        );
    }

    // The entities of the layers asked for, with their parents.
    let world = WorldIndex::build(source).ok();
    let mut human: HashMap<u32, bool> = HashMap::new();
    let mut wanted: Vec<&Row> = rows
        .iter()
        .filter(|r| {
            let l = r.layer.to_ascii_lowercase();
            words.iter().any(|w| l.contains(w.as_str()))
        })
        .collect();
    wanted.sort_by(|a, b| a.layer.cmp(&b.layer).then(a.name.cmp(&b.name)));
    if wanted.len() > 400 {
        println!("  ({} entities; the first 400)", wanted.len());
    }
    for r in wanted.into_iter().take(400) {
        let model = tmodels.get(&r.parent).copied();
        let skinned = model.map(|m| {
            *human.entry(m).or_insert_with(|| {
                source
                    .asset(m, MODEL_TYPE)
                    .ok()
                    .and_then(|d| Model::parse(&d).ok().map(|m| m.info.flags & 0x800 != 0))
                    .unwrap_or(false)
            })
        });
        let mut c = comps.get(&r.parent).cloned().unwrap_or_default();
        c.sort_unstable();
        let alpha = world.as_ref().map_or(String::new(), |w| {
            let mut out = Vec::new();
            if let Some(o) = w.state_objects.get(&r.id) {
                out.push(format!("prop {:08x}", o.model));
            }
            if let Some(s) = w.spawns.iter().find(|s| s.entity == r.id) {
                out.push(format!("spawn as {:?}", s.template));
            }
            out.join(", ")
        });
        println!(
            "  [{}] {:08x} at ({:.1} {:.1} {:.1}) {:?} own [{}] parent {:08x} {:?} model {}{} person {} [{}] alpha: {}",
            r.layer,
            r.id,
            r.at[0],
            r.at[1],
            r.at[2],
            r.name,
            r.components.join(" "),
            r.parent,
            tnames.get(&r.parent).map_or("", String::as_str),
            model.map_or("-".to_owned(), |m| format!("{m:08x} {}", names.show(m))),
            if skinned == Some(true) {
                " (skinned)"
            } else {
                ""
            },
            person(r.parent),
            c.join(" "),
            if alpha.is_empty() { "-" } else { &alpha }
        );
    }
    Ok(())
}

fn layer_entities<R: Read + Seek>(
    source: &mut AssetSource<R>,
    names: &Names,
    most: usize,
    keep: impl Fn(&str, &str, Vec3) -> bool,
    order: impl Fn(Vec3) -> f32,
) -> Result<(), Error> {
    let index = source.archive().index();
    let mut blocks: Vec<usize> = index
        .assets()
        .iter()
        .filter(|a| index.type_hash(a) == LAYER_TYPE)
        .map(|a| usize::from(a.block))
        .collect();
    blocks.sort_unstable();
    blocks.dedup();
    let paths: Vec<String> = index.blocks().iter().map(|b| b.path.clone()).collect();
    for b in blocks {
        let decoded = source.block(b)?;
        let layer_name = m2_engine::world::layer_name(&paths[b]);
        for (e, data) in BlockContents::parse(&decoded)?.iter() {
            if e.type_hash != LAYER_TYPE {
                continue;
            }
            let Ok(layer) = Layer::parse(data) else {
                continue;
            };
            let placements = layer.placements();
            let entity_names = layer.names();
            let mut near: Vec<(f32, u32)> = placements
                .iter()
                .filter(|(id, p)| {
                    let name = entity_names.get(id).copied().unwrap_or("");
                    keep(&layer_name, name, Vec3::from(p.position))
                })
                .map(|(&id, p)| (order(Vec3::from(p.position)), id))
                .collect();
            if near.is_empty() {
                continue;
            }
            near.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            println!(
                "layer {:08x} {} in {}: {} entities",
                e.name_hash,
                names.show(e.name_hash),
                paths[b],
                near.len()
            );
            let records: Vec<ComponentRecords<'_>> = layer
                .components
                .iter()
                .filter(|c| !matches!(c.name, "Transform" | "Name"))
                .map(|c| {
                    (
                        c.name,
                        c.fields.clone(),
                        c.records().unwrap_or_default().into_iter().collect(),
                    )
                })
                .collect();
            for (dist, id) in near.into_iter().take(most) {
                let p = placements[&id];
                println!(
                    "  {id:08x} {dist:.1} at ({:.1} {:.1} {:.1}) {:?}",
                    p.position[0],
                    p.position[1],
                    p.position[2],
                    entity_names.get(&id).copied().unwrap_or("")
                );
                for (cname, fields, recs) in &records {
                    let Some(rec) = recs.get(&id) else {
                        continue;
                    };
                    let values: Vec<String> =
                        fields.iter().map(|f| field_value(names, f, rec)).collect();
                    println!("    {cname}: {}", values.join(", "));
                }
            }
        }
    }
    Ok(())
}

/// A field of a stored record, named, as its bits, a hash with a name, or
/// a number.
fn field_value(names: &Names, f: &m2_assets::layer::Field, rec: &[u8]) -> String {
    let fname = names
        .get(f.name_hash)
        .map_or(format!("{:08x}", f.name_hash), str::to_owned);
    if f.ty == STRING_FIELD {
        return format!("{fname}=(text)");
    }
    let at = usize::from(f.offset);
    let Some(bytes) = rec.get(at..at + 4).or_else(|| rec.get(at..)) else {
        return format!("{fname}=(past the record)");
    };
    let mut word = [0u8; 4];
    word[..bytes.len().min(4)].copy_from_slice(&bytes[..bytes.len().min(4)]);
    let v = u32::from_le_bytes(word);
    if f.bit_width > 0 && f.bit_width < 32 {
        let bits = (v >> f.bit_shift) & ((1u32 << f.bit_width) - 1);
        return format!("{fname}={bits}");
    }
    let float = f32::from_le_bytes(word);
    let shown = if let Some(n) = names.get(v).filter(|_| v > 0xffff) {
        format!("{v:08x} {n}")
    } else if float.is_finite() && float != 0.0 && (1e-4..1e7).contains(&float.abs()) {
        format!("{float:.3}")
    } else {
        format!("{v:#x}")
    };
    format!("{fname}(t{})={shown}", f.ty)
}
