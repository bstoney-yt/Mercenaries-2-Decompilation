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
//! hashes:<bank>[,<bank>..]  each sound in a line: index, name hash,
//!                           length; its name if found (for searching
//!                           names away from the install)
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
//! effects[:<word>,..]       every particle effect: name, the templates that
//!                           show it, emitters and textures; each setting's
//!                           values over all emitters; the emitters in full
//!                           of effects whose templates' names hold a word
//! spot:<x>,<z>[,<r>]        what stands near a spot (default 30 m): models
//!                           with their techniques and textures, and the
//!                           layer entities with their components' fields
//! atmospheres[:<word>,..]   every atmosphere region (area, centre) with
//!                           its records' time of day, sunlight, clouds,
//!                           top ambient light and haze; then the region
//!                           and record over each start point whose name
//!                           holds a word
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
//! pickups[:<word>,..]       every placed pickup (health, ammunition, a
//!                           weapon, cash), supply crate (what it leaves)
//!                           and emplaced gun, in the areas of the state
//!                           layers whose name holds a word (default
//!                           vzacon001,pmccon001): their box, 150 m wider,
//!                           over every layer; counts, then each one
//! templates:<word|id>[,..]  templates whose name holds a word (or whose
//!                           id is given, 8 hex digits), with their
//!                           components' fields (text fields not read), a
//!                           field naming another template followed by its
//!                           name; then the templates they name, likewise
//! ai[:<word>,..]           the AI side of the data: every component of the
//!                           template database (name, templates); those
//!                           whose name holds a word (default ai,
//!                           perception, squad, alarm, target, threat,
//!                           combat, cover, behav, sense, aware, attack,
//!                           faction) with their schema (field hash, name,
//!                           type, offset, bits), each field's most common
//!                           values, their distinct records (templates
//!                           sharing it); then the same components across
//!                           every layer, as field value counts
//! parents[:<word>,..]       how entities' Transform `Parent` matches the
//!                           template database (counts, and whether named
//!                           entities' parents carry their name), then
//!                           every entity of the layers holding a word
//!                           (default vzacon001,vza_con001) with its parent
//!                           template: name, model, components, and what
//!                           the alpha's world index makes of it
//! fleet[:<word>,..]         every vehicle template (any vehicle physics
//!                           component), or those whose name holds a word:
//!                           physics, model (in the archive? the texture
//!                           catalogue's label and kind; wheels the alpha
//!                           finds), placements in static and state
//!                           layers, health, damage key, seats, entrances
//!                           and hijack script, turrets, linked weapons,
//!                           engine bank, its other components, and what
//!                           the alpha does with it; one tab-separated
//!                           line each after a `fleet\t` head
//! arsenal[:<word>,..]       every weapon template (a `Weapon*` component),
//!                           or those whose name holds a word: the weapon
//!                           side's components and fields; per weapon a
//!                           tab-separated `arsenal\t` line (model and
//!                           whether archived, placements, weapon fields,
//!                           ammunition's fields, who names it through
//!                           which component); then `detail\t` dumps of
//!                           each and what it names, two links deep
//! components[:<word>,..]    the template database's components (all, or
//!                           those whose name holds a word): record count
//!                           and each field's name (or hash), type, width
//! scan:<hex>[,..]           every asset of every archive (and every loose
//!                           file under data) holding all the given words
//!                           (u32, little-endian): its block, type, name,
//!                           how often each word appears, and the bytes
//!                           around the first hit of the first word
//! dump:<type>[,<name>]      every asset of vz.wad of that type (and name
//!                           hash): each 4-byte word, 8 a line, as a name,
//!                           a float or hex
//! find:<hash>[,..]          names for hashes
//! hijack                    every template that can be hijacked: its
//!                           entrances' template, script, point, radius
//!                           and flag, and whether vz.wad has the script
//! names:<word>[,..]         every known name holding a word, with its
//!                           hash and the world archive's assets of that
//!                           name (type hash, block path); 400 at most
//! cues[:<name>,..]          which bank sounds the clips' sound events
//!                           (and the names given) play: each name tried
//!                           as a sound's, bare, numbered, lettered and
//!                           with each ground surface (`SoundMaterial`
//!                           templates' and common ones) before or after;
//!                           then the cues no sound answers, and how many
//!                           of each bank's sounds a cue answers
//! sounddb[:<word>,..]       the template database's sound side: the
//!                           components the sound materials have, each
//!                           field's values (how many set, distinct, bank
//!                           sounds); some materials in full (default the
//!                           ground's and people's; or every template
//!                           whose name holds a word), a bank sound shown
//!                           as bank#index and length; every component
//!                           whose values name bank sounds or clip cues;
//!                           templates named like a cue; and how many of
//!                           each bank's sounds the database names
//! soundkeys                 every distinct `SoundKey` the templates give
//!                           (hash, name), with how many templates give it
//!                           and one of them
//! strings:<word>[,..]       the scripts' string constants holding a word,
//!                           by script (hash, name if known, how many
//!                           strings it has, the globals it sets), each
//!                           cut at 100 characters
//! text:<word>[,..]          the English string table's entries holding a
//!                           word: key, its name if known, the text (cut
//!                           at 100 characters); 200 at most
//! soundnames[:<bank>,..]    names for banks' sounds (default fol_shared,
//!                           collision_shared, destruction_shared and the
//!                           resident stream 7871f925) built of a prefix,
//!                           one or two words (the sound materials', the
//!                           clips' sound events' and their parts, and
//!                           common ones) and an ending
//! steps[:<head>,..]         footstep sounds by name over every bank: a
//!                           head (`sfx_footstep`; more given), one or two
//!                           words (gaits, surfaces, the sound materials,
//!                           who steps) and an ending numbered and
//!                           lettered (`_01a`); each name found with two
//!                           endings or more, its bank and endings
//! vo:<line>[,..]            where scripted voice lines live (`VO.Cue`'s
//!                           `Fiona-In-Mission-Contract-Vz01-146`): every
//!                           cue table in English.wad holding the line's
//!                           name hash (block, table, category, volume,
//!                           distances, stream), and that block's sound
//!                           banks with the stream's sound length; then
//!                           the cue tables of blocks whose path holds
//!                           "vz01"/"vzacon" with their first cues
//! destruct[:<word>,..]      how the data describes what breaks: the
//!                           template database's components (records,
//!                           and placed entities whose template has
//!                           each), how placed entities name a template,
//!                           the fields of components named for damage
//!                           (health, debris, collapse...), and over
//!                           every placed model the state machines'
//!                           states and verbs, by name word (bld, sign,
//!                           fence, tree...); with words, the models so
//!                           named, each with its states and verbs
//! stam:<hash>[,<hash>..]     each model's state machine in full: per part
//!                           and state, the handler kind and its commands
//!                           (operands by name, node with its height, or
//!                           number), and every node's height and box
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
use m2_assets::lua::{Chunk, SCRIPT_TYPE};
use m2_assets::model::MODEL_TYPE;
use m2_assets::scatter::SCATTER_TYPE;
use m2_assets::sound::SOUND_BANK_TYPE;
use m2_assets::strings::STRINGS_TYPE;
use m2_assets::terrain::{LOW_RES_TYPE, TERRAIN_TYPE};
use m2_assets::vertex::DeclUsage;
use m2_assets::{
    BlockContents, Clip, Layer, Model, Scatter, SoundBank, StreamIndex, StringTable, TerrainCell,
    Texture,
};
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
/// A layer component's name, fields and records by entity (an entity may
/// have a component more than once: the first contract's supply crate has
/// two `SpawnOnDeath`s).
type ComponentRecords<'a> = (
    &'a str,
    Vec<m2_assets::layer::Field>,
    HashMap<u32, Vec<&'a [u8]>>,
);

/// A component's records by entity, each entity's in data order.
fn by_entity<'a>(c: &m2_assets::layer::Component<'a>) -> HashMap<u32, Vec<&'a [u8]>> {
    let mut out: HashMap<u32, Vec<&'a [u8]>> = HashMap::new();
    for (id, r) in c.records().unwrap_or_default() {
        out.entry(id).or_default().push(r);
    }
    out
}

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

/// Template fields whose names appear in no file: guessed, each hashing
/// to a field of the vehicle, weapon and camera components (the AMX-30's
/// values read sensibly under them).
const FIELD_NAMES: &[&str] = &[
    "MaxSpeed",
    "Mass",
    "Brake",
    "SteerSpeed",
    "SteerInertia",
    "SpringStrength",
    "SpringDamp",
    "SideFriction",
    "GravityFactor",
    "EngineTorque",
    "EngineInertia",
    "EngineOffsetY",
    "MinYaw",
    "MaxYaw",
    "maxYawSpeed",
    "maxYawAccel",
    "MinPitch",
    "MaxPitch",
    "maxPitchSpeed",
    "maxPitchAccel",
    "BoneYaw",
    "BonePitch",
    "YawThreshold",
    "PitchThreshold",
    "CameraOffset",
    "FocusOffset",
    "DefaultPitch",
    "YawSpeed",
    "PitchSpeed",
    "YawMin",
    "YawMax",
    "PitchMin",
    "PitchMax",
    "FarRange",
    "CameraBlendTime",
    "TurretName",
    "DefaultFOV",
    "MaxFOV",
    "FOVMaxSpeed",
    "RateOfFire",
    "iClipSize",
    "iRoundsPerReload",
    "iBulletsPerShot",
    "MaxAmmoReserve",
    "FireType",
    "AmmoTemplate",
    "MuzzleFlashTemplate",
    "MuzzleFlashHardpoint",
    "Velocity",
    "GravityScale",
    "Damage",
    "Radius",
    "MinDamage",
    "MaxDamage",
    "MaxForce",
    "MinForceFalloff",
    "InnerRadius",
    "OuterRadius",
    "DamageRadius",
    "ExplosionTemplate",
    "Explosion",
    "ImpactTemplate",
    "SpawnOnDeath",
    "Lifetime",
    "Fuse",
    "FuseTime",
    "CameraShake",
    "ShakeAmplitude",
    "ShakeDuration",
    "Template",
    "Speed",
    "Acceleration",
    "TurnRate",
    "HomingTemplate",
    "EffectTemplate",
    "ParticleTemplate",
    "SoundKey",
    "Armor",
    "DamageType",
    "DamageScale",
    "DamageMultiplier",
    "DamageModifier",
    "BulletDamage",
    "BulletScale",
    "ExplosionScale",
    "Penetration",
    "ArmorPiercing",
    "Resistance",
    "Fudge",
    "Scale",
    "Multiplier",
    "Spread",
    "fSpread",
    "MinSpread",
    "MaxSpread",
    "Accuracy",
    "Range",
    "MaxRange",
    "MinRange",
    "fRange",
    "ReloadTime",
    "fReloadTime",
    "MinRadius",
    "MaxRadius",
    "LockOn",
    "LockTime",
    "LockOnTime",
    "LockRange",
    "Zoom",
    "ZoomFOV",
    "Recoil",
    "ThrowSpeed",
    "ThrowVelocity",
    "LifeTime",
    "SpawnTemplate",
    "DamageKey",
    "WeaponType",
    "WeaponClass",
    "BurstCount",
    "BurstSize",
    "BurstDelay",
    "Overheat",
    "HeatPerShot",
    "CoolDown",
    "CoolRate",
    "SpinUp",
    "SpinUpTime",
    "Accel",
    "AccelTime",
    "MaxVelocity",
    "Bounce",
    "Restitution",
    "Friction",
    "Homing",
    "TracerTemplate",
    "Force",
    "BlastRadius",
    "ExplosionRadius",
    "Ammo",
    "AmmoType",
    "Icon",
    "HudIcon",
    "Scope",
    "bScope",
    "bAutomatic",
    "Automatic",
    "Silenced",
    "Penetrate",
    "Splash",
    "SplashDamage",
    "SplashRadius",
    "ImpulseScale",
    "Impulse",
    "MinVelocity",
    "Drag",
    "Seek",
    "SeekTime",
    "ArmTime",
    "ArmingTime",
    "MaxLifetime",
    "Timer",
    "StickTime",
    "Sticky",
    "MaxCount",
    "MaxCarry",
    "Count",
    "Amount",
    "Type",
];

/// Every name in the archives and data files; with `gather`, what the
/// slots, materials and fx sections need from `vz.wad` too.
fn harvest(install: &Install, gather: bool) -> Result<(Names, Gathered), Error> {
    let mut names = Names::default();
    for n in FIELD_NAMES {
        names.add(n);
    }
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
            "slots" | "materials" | "fx" | "effects" | "terrain" | "spot"
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
            "hashes" => hashes(&mut source, &names, arg),
            "soundkeys" => soundkeys(&mut source, &names),
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
            "effects" => effects(&mut source, &names, &got, arg),
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
            // Every entity with a component of that name, with all its
            // records (`having:FactionZone`).
            "having" => layer_entities_with(
                &mut source,
                &names,
                usize::MAX,
                Some(arg.as_str()),
                |_, _, _| true,
                |p| p.x,
            ),
            "collision" => collision(&mut source, &names, arg),
            "phy2words" => {
                if world.is_none() {
                    world = Some(WorldIndex::build(&mut source)?);
                }
                phy2_words(
                    &mut source,
                    world.as_ref().expect("just built"),
                    &names,
                    arg,
                )
            }
            "phy2census" => {
                if world.is_none() {
                    world = Some(WorldIndex::build(&mut source)?);
                }
                phy2_census(&mut source, world.as_ref().expect("just built"), &names)
            }
            "phy2rules" => phy2_rules(&mut source),
            "hijack" => {
                if world.is_none() {
                    world = Some(WorldIndex::build(&mut source)?);
                }
                hijackable(&mut source, world.as_ref().expect("just built"), &names);
                Ok(())
            }
            "vo" => vo_lines(&install, arg),
            "scan" => scan_words(&install, &names, arg),
            "dump" => dump_assets(&mut source, &names, arg),
            "engines" => engines(&mut source, &names, arg),
            "vehicle" => vehicles(&mut source, &mut lazy, &names, arg),
            "templates" => templates(&mut source, &names, arg),
            "ai" => ai_side(&mut source, &names, arg),
            "fleet" => fleet(&mut source, &mut lazy, &names, arg),
            "arsenal" => arsenal(&mut source, &names, arg),
            "components" => components(&mut source, &names, arg),
            "parents" => parents(&mut source, &names, arg),
            "cues" => cues(&mut source, &names, arg),
            "sounddb" => sounddb(&mut source, &names, arg),
            "soundnames" => soundnames(&mut source, &names, arg),
            "steps" => steps(&mut source, &names, arg),
            "destruct" => {
                if world.is_none() {
                    world = Some(WorldIndex::build(&mut source)?);
                }
                destruct(
                    &mut source,
                    world.as_ref().expect("just built"),
                    &names,
                    arg,
                )
            }
            "pickups" => pickups(&mut source, arg),
            "stam" => stam(&mut source, &names, arg),
            "strings" => script_strings(&mut source, &names, arg),
            "text" => texts(&mut source, &names, arg),
            "names" => names_holding(&mut source, &names, arg),
            "atmospheres" => {
                if world.is_none() {
                    world = Some(WorldIndex::build(&mut source)?);
                }
                atmospheres(world.as_ref().expect("just built"), &names, arg);
                Ok(())
            }
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

/// Every atmosphere region with its records (name, `LightTime`, sunlight
/// strength and colour, cloud tone and cover, the ambient cube's top
/// face), then, for each start point whose name holds one of `arg`'s
/// words, the region and record over it.
fn atmospheres(world: &WorldIndex, names: &Names, arg: &str) {
    let show = |a: &m2_engine::Atmosphere| {
        format!(
            "time {:.3} li {:.2} light {:.2},{:.2},{:.2} tone {:.2},{:.2},{:.2} cover {:.2} up {:.2},{:.2},{:.2} interp {:.1} dark {:.0} inscat {:.1} haze {:?}",
            a.light_time[0],
            a.light_intensity,
            a.light_color.x,
            a.light_color.y,
            a.light_color.z,
            a.tone.x,
            a.tone.y,
            a.tone.z,
            a.coverage,
            a.ambient_cube[2].x,
            a.ambient_cube[2].y,
            a.ambient_cube[2].z,
            a.interpolate_time,
            a.cloud_darkness,
            a.inscattering,
            a.haze_layers
        )
    };
    if let Some(a) = &world.atmosphere {
        println!("default: {}", show(a));
    }
    let regions = &world.atmosphere_regions;
    for r in regions {
        let n = r.outline.len().max(1) as f32;
        let c = r
            .outline
            .iter()
            .fold(m2_math::glam::Vec2::ZERO, |s, p| s + *p)
            / n;
        println!(
            "region {} ({:08x}) area {:.0} centre {:.0},{:.0}, default record {}",
            r.name,
            r.entity,
            r.area,
            c.x,
            c.y,
            r.default_setting()
        );
        for (i, (name, a)) in r.settings.iter().enumerate() {
            println!("  #{i} {} ({name:08x}): {}", names.show(*name), show(a));
        }
    }
    let words = words_of(arg);
    let mix = m2_engine::atmosphere::AtmosphereMix::default();
    for s in &world.starts {
        let lower = s.name.to_ascii_lowercase();
        if !words.iter().any(|w| lower.contains(w.as_str())) {
            continue;
        }
        let over = mix
            .over(regions, s.position)
            .map_or("the default".to_owned(), |(r, k)| {
                format!("{} #{k}", regions[r].name)
            });
        println!(
            "start {} at {:.0},{:.0},{:.0}: {over}",
            s.name, s.position.x, s.position.y, s.position.z
        );
    }
}

/// The words of a section's argument, lower case.
fn words_of(arg: &str) -> Vec<String> {
    arg.split(',')
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

/// `s` cut at `n` characters.
fn cut(s: &str, n: usize) -> String {
    match s.char_indices().nth(n) {
        Some((i, _)) => format!("{}..", &s[..i]),
        None => s.to_owned(),
    }
}

/// Every template that can be hijacked, with its entrances.
fn hijackable<R: Read + Seek>(source: &mut AssetSource<R>, world: &WorldIndex, names: &Names) {
    let db = &world.template_db;
    let index = source.archive().index();
    let scripts: HashSet<u32> = index
        .assets()
        .iter()
        .filter(|a| index.type_hash(a) == SCRIPT_TYPE)
        .map(|a| a.name_hash)
        .collect();
    let mut rows: Vec<(String, u32)> = db
        .hijack
        .keys()
        .map(|&id| (db.names.get(&id).cloned().unwrap_or_default(), id))
        .collect();
    rows.sort();
    println!("hijackable templates: {}", rows.len());
    for (name, id) in rows {
        let model = db.infos.get(&id).and_then(|i| i.model);
        println!(
            "  {id:08x} {name} model {}",
            model.map_or("-".into(), |m| format!("{m:08x}"))
        );
        for e in &db.hijack[&id] {
            let show =
                |h: Option<u32>| h.map_or("-".to_owned(), |h| format!("{h:08x} {}", names.show(h)));
            println!(
                "    entrance {:08x} {} script {:08x} {}{} point {} radius {} flag {:?} hardpoint {} name {}",
                e.entrance,
                db.names.get(&e.entrance).map_or("?", String::as_str),
                e.script,
                names.show(e.script),
                if scripts.contains(&e.script) {
                    ""
                } else {
                    " (no such script)"
                },
                show(e.point),
                e.radius,
                e.flag,
                show(e.hardpoint),
                show(e.name),
            );
        }
    }
}

/// Every known name holding a word, with the assets of that name.
fn names_holding<R: Read + Seek>(
    source: &mut AssetSource<R>,
    names: &Names,
    arg: &str,
) -> Result<(), Error> {
    let words = words_of(arg);
    let mut hits: Vec<(&str, u32)> = names
        .map
        .iter()
        .filter(|(_, n)| {
            let l = n.to_ascii_lowercase();
            words.iter().any(|w| l.contains(w))
        })
        .map(|(&h, n)| (n.as_str(), h))
        .collect();
    hits.sort_unstable();
    println!("names holding a word: {}", hits.len());
    let index = source.archive().index();
    let mut assets: HashMap<u32, Vec<String>> = HashMap::new();
    for a in index.assets() {
        let path = index
            .blocks()
            .get(usize::from(a.block))
            .map_or("?", |b| b.path.as_str());
        assets
            .entry(a.name_hash)
            .or_default()
            .push(format!("{:08x}@{path}", index.type_hash(a)));
    }
    for (n, h) in hits.iter().take(400) {
        let mut found = assets.get(h).cloned().unwrap_or_default();
        found.sort_unstable();
        found.dedup();
        found.truncate(4);
        println!("  {h:08x} {} {}", cut(n, 80), found.join(" "));
    }
    Ok(())
}

/// The scripts' string constants holding a word, by script.
fn script_strings<R: Read + Seek>(
    source: &mut AssetSource<R>,
    names: &Names,
    arg: &str,
) -> Result<(), Error> {
    let words = words_of(arg);
    let index = source.archive().index();
    let mut blocks: Vec<usize> = index
        .assets()
        .iter()
        .filter(|a| index.type_hash(a) == SCRIPT_TYPE)
        .map(|a| usize::from(a.block))
        .collect();
    blocks.sort_unstable();
    blocks.dedup();
    let (mut seen, mut found) = (HashSet::new(), 0);
    for b in blocks {
        let decoded = source.block(b)?;
        for (e, data) in BlockContents::parse(&decoded)?.iter() {
            if e.type_hash != SCRIPT_TYPE || !seen.insert(e.name_hash) {
                continue;
            }
            let Ok(chunk) = Chunk::from_asset(data) else {
                continue;
            };
            let strings = chunk.main.strings();
            let hits: Vec<String> = strings
                .iter()
                .filter(|s| {
                    let l = s.to_ascii_lowercase();
                    words.iter().any(|w| l.contains(w))
                })
                .map(|s| cut(s, 100))
                .collect();
            if hits.is_empty() {
                continue;
            }
            found += 1;
            let mut sets: Vec<String> = chunk
                .main
                .names(chunk.version)
                .into_iter()
                .filter(|(u, _)| *u == m2_assets::lua::NameUse::SetGlobal)
                .map(|(_, n)| n)
                .collect();
            sets.sort();
            sets.dedup();
            let more = sets.len().saturating_sub(12);
            sets.truncate(12);
            println!(
                "  script {:08x} {} ({} strings; sets {}{}):",
                e.name_hash,
                names.show(e.name_hash),
                strings.len(),
                sets.join(" "),
                if more > 0 {
                    format!(" and {more} more")
                } else {
                    String::new()
                }
            );
            for h in hits {
                println!("    {h}");
            }
        }
    }
    println!("scripts holding a word: {found} of {}", seen.len());
    Ok(())
}

/// The English string table's entries holding a word.
fn texts<R: Read + Seek>(
    source: &mut AssetSource<R>,
    names: &Names,
    arg: &str,
) -> Result<(), Error> {
    let words = words_of(arg);
    let english = m2_assets::name_hash("english");
    let index = source.archive().index();
    if !index
        .assets()
        .iter()
        .any(|a| index.type_hash(a) == STRINGS_TYPE && a.name_hash == english)
    {
        println!("no english string table");
        return Ok(());
    }
    let table = StringTable::parse(&source.asset(english, STRINGS_TYPE)?)?;
    let mut hits: Vec<(u32, &str)> = table
        .strings
        .iter()
        .filter(|(_, t)| {
            let l = t.to_lowercase();
            words.iter().any(|w| l.contains(w))
        })
        .map(|(&k, t)| (k, t.as_str()))
        .collect();
    hits.sort_unstable();
    println!(
        "entries holding a word: {} of {}",
        hits.len(),
        table.strings.len()
    );
    for (k, t) in hits.iter().take(200) {
        println!("  {k:08x} {} {}", names.show(*k), cut(t, 100));
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

/// Finds scripted voice lines (`VO.Cue`'s line names) in the cue tables of
/// `English.wad`, resident and per mission.
fn dump_assets<R: Read + Seek>(
    source: &mut AssetSource<R>,
    names: &Names,
    arg: &str,
) -> Result<(), Error> {
    let mut parts = arg.split(',').filter_map(hex);
    let Some(ty) = parts.next() else {
        println!("dump: no type");
        return Ok(());
    };
    let name = parts.next();
    let index = source.archive().index();
    let picked: Vec<u32> = index
        .assets()
        .iter()
        .filter(|a| index.type_hash(a) == ty && name.is_none_or(|n| n == a.name_hash))
        .map(|a| a.name_hash)
        .collect();
    for h in picked {
        let data = source.asset(h, ty)?;
        println!(
            "asset {ty:08x} {h:08x} {} ({} bytes)",
            names.show(h),
            data.len()
        );
        for (i, line) in data.chunks(32).enumerate() {
            let words: Vec<String> = line
                .chunks(4)
                .map(|c| {
                    let mut w = [0u8; 4];
                    w[..c.len()].copy_from_slice(c);
                    let v = u32::from_le_bytes(w);
                    let f = f32::from_le_bytes(w);
                    match names.get(v).filter(|_| v > 0xffff) {
                        Some(n) => n.to_owned(),
                        None if v == 0 => "0".to_owned(),
                        None if f.is_finite() && (1e-3..1e6).contains(&f.abs()) => format!("{f}"),
                        None => format!("#{v:x}"),
                    }
                })
                .collect();
            println!("  +{:04x} {}", i * 32, words.join(" "));
        }
    }
    Ok(())
}

fn scan_words(install: &Install, names: &Names, arg: &str) -> Result<(), Error> {
    let words: Vec<u32> = arg.split(',').filter_map(hex).collect();
    let report = |place: &str, data: &[u8]| {
        let counts: Vec<usize> = words
            .iter()
            .map(|w| {
                let b = w.to_le_bytes();
                data.windows(4).filter(|x| *x == b).count()
            })
            .collect();
        if counts.contains(&0) {
            return;
        }
        println!("  {place} ({} bytes): counts {counts:?}", data.len());
        let b = words[0].to_le_bytes();
        if let Some(at) = data.windows(4).position(|x| x == b) {
            let from = at.saturating_sub(64) & !3;
            let to = (at + 128).min(data.len());
            for (i, chunk) in data[from..to].chunks(16).enumerate() {
                let line: Vec<String> = chunk
                    .chunks(4)
                    .map(|c| {
                        let mut w = [0u8; 4];
                        w[..c.len()].copy_from_slice(c);
                        let v = u32::from_le_bytes(w);
                        let f = f32::from_le_bytes(w);
                        match names.get(v).filter(|_| v > 0xffff) {
                            Some(n) => format!("{v:08x}({n})"),
                            None if f.is_finite() && (1e-3..1e6).contains(&f.abs()) => {
                                format!("{v:08x}({f})")
                            }
                            None => format!("{v:08x}"),
                        }
                    })
                    .collect();
                println!("      +{:06x} {}", from + i * 16, line.join(" "));
            }
        }
    };
    if words.is_empty() {
        println!("scan: no words");
        return Ok(());
    }
    let mut wads = Vec::new();
    let mut loose = Vec::new();
    let mut stack = vec![install.data.clone()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = e.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("wad"))
            {
                wads.push(path);
            } else {
                loose.push(path);
            }
        }
    }
    for path in &loose {
        if let Ok(data) = std::fs::read(path) {
            report(&path.display().to_string(), &data);
        }
    }
    for path in &wads {
        let mut source = match open(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                continue;
            }
        };
        let paths: Vec<String> = source
            .archive()
            .index()
            .blocks()
            .iter()
            .map(|b| b.path.clone())
            .collect();
        for (b, p) in paths.iter().enumerate() {
            let Ok(decoded) = source.block(b) else {
                continue;
            };
            let Ok(contents) = BlockContents::parse(&decoded) else {
                report(&format!("{} {p} (whole block)", path.display()), &decoded);
                continue;
            };
            for (e, data) in contents.iter() {
                let place = format!(
                    "{} {p} type {:08x} {:08x} {}",
                    path.file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into()),
                    e.type_hash,
                    e.name_hash,
                    names.show(e.name_hash)
                );
                report(&place, data);
            }
        }
    }
    Ok(())
}

fn vo_lines(install: &Install, arg: &str) -> Result<(), Error> {
    let lines: Vec<(&str, u32)> = arg
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| (s, m2_assets::name_hash(s)))
        .collect();
    for (s, h) in &lines {
        println!("  {h:08x} {s}");
    }
    let wad = install.data.join("English.wad");
    let mut source = AssetSource::new(BufReader::new(File::open(&wad)?))?;
    let paths: Vec<String> = source
        .archive()
        .index()
        .blocks()
        .iter()
        .map(|b| b.path.clone())
        .collect();
    let mut tables = 0;
    for (b, path) in paths.iter().enumerate() {
        let Ok(decoded) = source.block(b) else {
            continue;
        };
        let contents = BlockContents::parse(&decoded)?;
        let mut found = Vec::new();
        let mut banks = Vec::new();
        let mission = {
            let p = path.to_ascii_lowercase();
            p.contains("vz01") || p.contains("vzacon")
        };
        for (e, data) in contents.iter() {
            match e.type_hash {
                m2_assets::voice::CUE_TABLE_TYPE => {
                    let Ok(t) = m2_assets::CueTable::parse(data) else {
                        continue;
                    };
                    tables += 1;
                    if mission {
                        let first: Vec<String> = t
                            .lines
                            .iter()
                            .take(6)
                            .map(|l| format!("{:08x}", l.cue))
                            .collect();
                        println!(
                            "  table {:08x} in {path}: {} lines {} groups; first {}",
                            t.hash,
                            t.lines.len(),
                            t.groups.len(),
                            first.join(" ")
                        );
                    }
                    for (s, h) in &lines {
                        for l in t.lines.iter().filter(|l| l.cue == *h) {
                            found.push(format!(
                                "{s}: table {:08x} line, category {:08x}, volume {:.2}, \
                                 {:.0}-{:.0} m, stream {}",
                                t.hash,
                                l.category,
                                l.volume,
                                l.min_distance,
                                l.max_distance,
                                l.stream
                            ));
                        }
                        for g in t.groups.iter().filter(|g| g.cue == *h) {
                            found.push(format!(
                                "{s}: table {:08x} group, category {:08x}, streams {:?}",
                                t.hash, g.category, g.streams
                            ));
                        }
                    }
                }
                SOUND_BANK_TYPE => {
                    if let Ok(bank) = SoundBank::parse(data) {
                        let secs: Vec<f32> = bank
                            .sounds
                            .iter()
                            .map(|s| s.frames as f32 / s.rate.max(1) as f32)
                            .collect();
                        banks.push((e.name_hash, secs));
                    } else if let Ok(i) = StreamIndex::parse(data) {
                        banks.push((e.name_hash, vec![-(i.streams.len() as f32)]));
                    }
                }
                _ => {}
            }
        }
        if found.is_empty() {
            continue;
        }
        println!("  block {b} {path}");
        for f in found {
            println!("    {f}");
        }
        for (h, secs) in banks {
            if secs.first().is_some_and(|s| *s < 0.0) {
                println!("    stream index {h:08x}: {} streams", -secs[0]);
            } else {
                let shown: Vec<String> = secs.iter().map(|s| format!("{s:.1}")).collect();
                println!(
                    "    bank {h:08x}: {} sounds, s: {}",
                    secs.len(),
                    shown.join(" ")
                );
            }
        }
    }
    println!("  cue tables read: {tables}");
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

fn hashes<R: Read + Seek>(
    source: &mut AssetSource<R>,
    names: &Names,
    arg: &str,
) -> Result<(), Error> {
    let banks: Vec<&str> = arg.split(',').filter(|b| !b.is_empty()).collect();
    let library = SoundLibrary::load(source, &banks)?;
    for bank in &banks {
        let mut sounds = library.bank(bank).to_vec();
        // A bank in a block of several (the resident foley, `fol_shared`)
        // is found by its own name instead.
        if sounds.is_empty() {
            let hash = m2_assets::name_hash(bank);
            let resident = SoundLibrary::load_banks(source, &[hash]);
            sounds = resident
                .bank_hash(hash)
                .map_or_else(Vec::new, |(_, s)| s.to_vec());
        }
        println!("bank {bank}: {} sounds", sounds.len());
        for (i, s) in sounds.iter().enumerate() {
            println!(
                "  #{i} {:08x} {:.2}{}",
                s.hash,
                s.seconds(),
                names
                    .get(s.hash)
                    .map(|n| format!(" {n}"))
                    .unwrap_or_default()
            );
        }
    }
    Ok(())
}

/// Ground surfaces a footstep's sound may be named for, besides those the
/// template database's `SoundMaterial (..)` templates name.
const SURFACES: [&str; 44] = [
    "dirt",
    "grass",
    "sand",
    "concrete",
    "asphalt",
    "tarmac",
    "road",
    "metal",
    "solidmetal",
    "hollowmetal",
    "metalgrate",
    "wood",
    "woodhollow",
    "water",
    "shallowwater",
    "deepwater",
    "puddle",
    "mud",
    "gravel",
    "rock",
    "stone",
    "tile",
    "carpet",
    "snow",
    "leaves",
    "foliage",
    "jungle",
    "default",
    "generic",
    "hard",
    "soft",
    "glass",
    "plastic",
    "rubber",
    "flesh",
    "body",
    "brick",
    "marble",
    "straw",
    "thatch",
    "earth",
    "ground",
    "bush",
    "pavement",
];

/// A cue's name with each ending tried: none, numbered (`_01`, `_1`,
/// `01`, `1`), lettered (`_a`, `a`), looped.
fn endings() -> Vec<String> {
    let mut out = vec![String::new(), "_lp".into(), "_loop".into()];
    for n in 1..=24 {
        out.push(format!("_{n:02}"));
        out.push(format!("_{n}"));
        out.push(format!("{n:02}"));
        out.push(format!("{n}"));
    }
    for c in 'a'..='h' {
        out.push(format!("_{c}"));
        out.push(format!("{c}"));
    }
    out.sort();
    out.dedup();
    out
}

/// The clips' sound events: (category, name) -> how many clips carry it.
fn sound_events<R: Read + Seek>(
    source: &mut AssetSource<R>,
) -> Result<BTreeMap<(String, String), usize>, Error> {
    let index = source.archive().index();
    let mut clip_blocks: Vec<usize> = index
        .assets()
        .iter()
        .filter(|a| index.type_hash(a) == ANIMATION_TYPE)
        .map(|a| usize::from(a.block))
        .collect();
    clip_blocks.sort_unstable();
    clip_blocks.dedup();
    let mut events: BTreeMap<(String, String), usize> = BTreeMap::new();
    for b in clip_blocks {
        let decoded = source.block(b)?;
        for (e, data) in BlockContents::parse(&decoded)?.iter() {
            if e.type_hash != ANIMATION_TYPE {
                continue;
            }
            let Ok(clip) = Clip::parse(data) else {
                continue;
            };
            let mut seen = HashSet::new();
            for ev in &clip.events {
                let key = (ev.category.clone(), ev.name.clone());
                if ev.category.to_ascii_lowercase().starts_with("sound") && seen.insert(key.clone())
                {
                    *events.entry(key).or_default() += 1;
                }
            }
        }
    }
    Ok(events)
}

/// Every bank's sounds, streamed banks' too.
#[derive(Default)]
struct BankSounds {
    /// Sound hash -> (bank, index, seconds) for each bank holding it.
    sounds: HashMap<u32, Vec<(u32, usize, f32)>>,
    /// Every bank with its number of sounds.
    banks: Vec<(u32, usize)>,
    /// A streamed bank's file (`ambience.pws`), by bank.
    streamed: HashMap<u32, String>,
}

impl BankSounds {
    /// A bank's name, or its stream file's.
    fn label(&self, names: &Names, bank: u32) -> String {
        names
            .get(bank)
            .map(str::to_owned)
            .or_else(|| self.streamed.get(&bank).cloned())
            .unwrap_or_else(|| "?".to_owned())
    }
}

fn bank_sounds<R: Read + Seek>(source: &mut AssetSource<R>) -> Result<BankSounds, Error> {
    let index = source.archive().index();
    let mut bank_blocks: Vec<usize> = index
        .assets()
        .iter()
        .filter(|a| index.type_hash(a) == SOUND_BANK_TYPE)
        .map(|a| usize::from(a.block))
        .collect();
    bank_blocks.sort_unstable();
    bank_blocks.dedup();
    let mut out = BankSounds::default();
    for b in bank_blocks {
        let decoded = source.block(b)?;
        for (e, data) in BlockContents::parse(&decoded)?.iter() {
            if e.type_hash != SOUND_BANK_TYPE {
                continue;
            }
            let sounds: Vec<(u32, f32)> = if let Ok(bank) = SoundBank::parse(data) {
                bank.sounds
                    .iter()
                    .map(|s| (s.hash, s.frames as f32 / s.rate.max(1) as f32))
                    .collect()
            } else if let Ok(index) = StreamIndex::parse(data) {
                out.streamed.insert(e.name_hash, index.file.clone());
                index
                    .streams
                    .iter()
                    .map(|s| (s.hash, s.seconds()))
                    .collect()
            } else {
                continue;
            };
            out.banks.push((e.name_hash, sounds.len()));
            for (i, (hash, secs)) in sounds.into_iter().enumerate() {
                out.sounds
                    .entry(hash)
                    .or_default()
                    .push((e.name_hash, i, secs));
            }
        }
    }
    Ok(out)
}

fn cues<R: Read + Seek>(
    source: &mut AssetSource<R>,
    names: &Names,
    arg: &str,
) -> Result<(), Error> {
    let index = source.archive().index();
    let db_hash = index
        .assets()
        .iter()
        .find(|a| index.type_hash(a) == TEMPLATE_DB_TYPE)
        .map(|a| a.name_hash);
    let mut events = sound_events(source)?;
    for w in arg.split(',').filter(|w| !w.is_empty()) {
        events.entry(("asked".into(), w.to_owned())).or_default();
    }
    // Surfaces: the template database's sound materials, and common ones.
    let mut materials: Vec<String> = Vec::new();
    if let Some(h) = db_hash {
        let data = source.asset(h, TEMPLATE_DB_TYPE)?;
        let db = Layer::parse(&data)?;
        for n in db.names().values() {
            if let Some(inner) = n
                .strip_prefix("SoundMaterial (")
                .and_then(|r| r.strip_suffix(')'))
            {
                materials.push(inner.to_owned());
            }
        }
    }
    materials.sort();
    materials.dedup();
    println!(
        "{} SoundMaterial templates: {}",
        materials.len(),
        materials.join(", ")
    );
    let mut surfaces: Vec<String> = materials.clone();
    surfaces.extend(SURFACES.iter().map(|s| (*s).to_owned()));
    surfaces.sort();
    surfaces.dedup();
    let library = bank_sounds(source)?;
    let (sounds, banks) = (&library.sounds, &library.banks);
    println!(
        "{} sound events in clips (+ {} asked); {} banks, {} sounds",
        events.keys().filter(|k| k.0 != "asked").count(),
        events.keys().filter(|k| k.0 == "asked").count(),
        banks.len(),
        sounds.values().map(Vec::len).sum::<usize>()
    );
    let ends = endings();
    let prefixes = ["", "fol_", "sfx_", "ahj_", "sfx_fol_"];
    let mut answered: HashSet<u32> = HashSet::new();
    let mut silent: Vec<String> = Vec::new();
    for ((category, name), clips) in &events {
        let lower = name.to_ascii_lowercase();
        // (sound hash, the name that hashed to it)
        let mut found: BTreeMap<u32, String> = BTreeMap::new();
        let mut try_name = |parts: &[&str]| {
            let h = hash_parts(parts);
            if sounds.contains_key(&h) {
                found.entry(h).or_insert_with(|| parts.concat());
            }
        };
        for p in prefixes {
            for e in &ends {
                try_name(&[p, &lower, e]);
                if category.contains("surface") || category == "asked" {
                    for s in &surfaces {
                        try_name(&[p, &lower, "_", s, e]);
                        try_name(&[p, s, "_", &lower, e]);
                        try_name(&[p, &lower, s, e]);
                    }
                }
            }
        }
        if found.is_empty() {
            silent.push(format!("{category}/{name}"));
            continue;
        }
        answered.extend(found.keys().copied());
        let library = &library;
        let shown: Vec<String> = found
            .iter()
            .take(8)
            .flat_map(|(h, n)| {
                sounds[h].iter().take(1).map(move |(bank, i, secs)| {
                    format!("{}#{i} {h:08x} {n} {secs:.2}s", library.label(names, *bank))
                })
            })
            .collect();
        println!(
            "  {category}/{name} ({clips} clips): {} sounds: {}{}",
            found.len(),
            shown.join("; "),
            if found.len() > 8 { "; ..." } else { "" }
        );
    }
    println!("{} cues no sound answers:", silent.len());
    for chunk in silent.chunks(8) {
        println!("  {}", chunk.join(", "));
    }
    println!("sounds a cue answers, per bank (those with any):");
    let mut per_bank: BTreeMap<u32, usize> = BTreeMap::new();
    for h in &answered {
        for (bank, _, _) in &sounds[h] {
            *per_bank.entry(*bank).or_default() += 1;
        }
    }
    for (bank, total) in banks {
        if let Some(n) = per_bank.get(bank) {
            println!(
                "  {:08x} {}: {n} of {total}",
                bank,
                library.label(names, *bank)
            );
        }
    }
    Ok(())
}

/// Heads a footstep sound's name may start with ([`steps`]).
const STEP_HEADS: [&str; 26] = [
    "sfx_footstep",
    "sfx_footsteps",
    "footstep",
    "footsteps",
    "sfx_fs",
    "fs",
    "sfx_step",
    "step",
    "sfx_ftstp",
    "ftstp",
    "sfx_fol_step",
    "fol_step",
    "sfx_fol_footstep",
    "fol_footstep",
    "sfx_foot",
    "foot",
    "sfx_fol",
    "fol",
    "sfx_hum_footstep",
    "hum_footstep",
    "sfx_hum",
    "hum",
    "sfx_fol_ftstp",
    "fol_ftstp",
    "sfx_fol_fs",
    "fol_fs",
];

/// Words a footstep sound's name may be built of besides the surfaces
/// ([`steps`]): gaits, who steps, sizes.
const STEP_WORDS: [&str; 58] = [
    "run",
    "walk",
    "sprint",
    "jog",
    "jumpland",
    "land",
    "landing",
    "jumptakeoff",
    "takeoff",
    "jump",
    "crouch",
    "scuff",
    "stop",
    "shuffle",
    "heavy",
    "light",
    "slow",
    "fast",
    "step",
    "steps",
    "footstep",
    "foot",
    "left",
    "right",
    "l",
    "r",
    "player",
    "plr",
    "jen",
    "jennifer",
    "mattias",
    "chris",
    "human",
    "hum",
    "npc",
    "ai",
    "male",
    "female",
    "boot",
    "boots",
    "shoe",
    "shoes",
    "barefoot",
    "soldier",
    "civ",
    "civilian",
    "hero",
    "char",
    "sml",
    "mid",
    "lrg",
    "small",
    "med",
    "large",
    "near",
    "close",
    "dist",
    "distant",
];

/// Footstep sounds by name over every bank (round 99: the clips' cues are
/// `footstep_run`, `_walk`, `_jumpland` and `_jumptakeoff`, of the
/// `sound_surface` kind; `fol_shared` holds
/// `sfx_footstep_jumpland_metal_01a`, and no ending [`cues`] tries is
/// like `_01a`): a head ([`STEP_HEADS`], and those `arg` gives), one or
/// two words ([`STEP_WORDS`], [`SURFACES`], the template database's sound
/// materials and their parts), and an ending numbered and lettered. A
/// name shows once two of its endings are found: one alone is as likely
/// chance, over thousands of sounds.
fn steps<R: Read + Seek>(
    source: &mut AssetSource<R>,
    names: &Names,
    arg: &str,
) -> Result<(), Error> {
    let index = source.archive().index();
    let db_hash = index
        .assets()
        .iter()
        .find(|a| index.type_hash(a) == TEMPLATE_DB_TYPE)
        .map(|a| a.name_hash);
    let mut words: Vec<String> = STEP_WORDS
        .iter()
        .chain(SURFACES.iter())
        .map(|w| (*w).to_owned())
        .collect();
    if let Some(h) = db_hash {
        let data = source.asset(h, TEMPLATE_DB_TYPE)?;
        let db = Layer::parse(&data)?;
        for n in db.names().values() {
            if let Some(inner) = n
                .strip_prefix("SoundMaterial (")
                .and_then(|r| r.strip_suffix(')'))
            {
                let lower = inner.to_ascii_lowercase();
                words.extend(
                    lower
                        .split(['_', ' '])
                        .filter(|p| p.len() >= 2)
                        .map(str::to_owned),
                );
                words.push(lower);
            }
        }
    }
    words.sort();
    words.dedup();
    let heads: Vec<String> = STEP_HEADS
        .iter()
        .map(|h| (*h).to_owned())
        .chain(arg.split(',').filter(|h| !h.is_empty()).map(str::to_owned))
        .collect();
    let mut ends = vec![String::new()];
    for n in 1..=12 {
        ends.push(format!("_{n:02}"));
        ends.push(format!("{n:02}"));
        ends.push(format!("_{n}"));
        for c in 'a'..='f' {
            ends.push(format!("_{n:02}{c}"));
            ends.push(format!("{n:02}{c}"));
        }
    }
    for c in 'a'..='f' {
        ends.push(format!("_{c}"));
        ends.push(format!("{c}"));
    }
    let library = bank_sounds(source)?;
    let sounds = &library.sounds;
    // Each name (its head and words) found, with the endings it was found
    // with and their sounds.
    let mut found: BTreeMap<String, Vec<(usize, u32)>> = BTreeMap::new();
    let mut stem = |h: u32, name: &dyn Fn() -> String| {
        for (k, e) in ends.iter().enumerate() {
            let x = fnv_end(fnv_on(h, e));
            if sounds.contains_key(&x) {
                found.entry(name()).or_default().push((k, x));
            }
        }
    };
    for head in &heads {
        let hh = fnv_on(fnv_on(FNV_START, head), "_");
        for a in &words {
            let ha = fnv_on(hh, a);
            stem(ha, &|| format!("{head}_{a}"));
            let hs = fnv_on(ha, "_");
            for b in words.iter().filter(|b| *b != a) {
                stem(fnv_on(hs, b), &|| format!("{head}_{a}_{b}"));
            }
        }
    }
    let alone = found.values().filter(|f| f.len() < 2).count();
    println!(
        "{} heads, {} words, {} endings over {} sounds in {} banks: {} names found with two \
         endings or more, {alone} with one (as likely chance)",
        heads.len(),
        words.len(),
        ends.len(),
        sounds.len(),
        library.banks.len(),
        found.len() - alone
    );
    for (name, hits) in found.iter().filter(|(_, f)| f.len() >= 2) {
        let mut banks: Vec<String> = hits
            .iter()
            .flat_map(|(_, h)| sounds[h].iter().map(|(b, _, _)| *b))
            .collect::<std::collections::BTreeSet<u32>>()
            .into_iter()
            .map(|b| format!("{b:08x} {}", library.label(names, b)))
            .collect();
        banks.truncate(3);
        let endings: Vec<&str> = hits
            .iter()
            .take(16)
            .map(|(k, _)| ends[*k].as_str())
            .collect();
        println!(
            "  {name}: {} ({}{}) in {}",
            hits.len(),
            endings.join(" "),
            if hits.len() > 16 { " ..." } else { "" },
            banks.join(", ")
        );
    }
    Ok(())
}

/// Words the sound side's field names are guessed from: alone and in
/// pairs (`FootstepWalk`), bare or with a type prefix (`fVolume`).
const SOUND_WORDS: [&str; 120] = [
    "Sound",
    "Sounds",
    "Set",
    "Material",
    "Surface",
    "Footstep",
    "Footsteps",
    "Foot",
    "Step",
    "Walk",
    "Run",
    "Sprint",
    "Land",
    "Jump",
    "Takeoff",
    "Scuff",
    "Slide",
    "Impact",
    "Impacts",
    "Hit",
    "Bullet",
    "Melee",
    "Body",
    "Fall",
    "Bounce",
    "Roll",
    "Scrape",
    "Break",
    "Destroy",
    "Destroyed",
    "Debris",
    "Collision",
    "Collide",
    "Crash",
    "Light",
    "Lite",
    "Medium",
    "Med",
    "Heavy",
    "Soft",
    "Hard",
    "Small",
    "Sml",
    "Large",
    "Lrg",
    "Min",
    "Max",
    "Velocity",
    "Speed",
    "Mass",
    "Volume",
    "Pitch",
    "Gain",
    "Bank",
    "Cue",
    "Name",
    "Type",
    "Parent",
    "Effect",
    "Particle",
    "Decal",
    "Explosion",
    "Fire",
    "Water",
    "Splash",
    "Wade",
    "Swim",
    "Vehicle",
    "Human",
    "Prop",
    "Terrain",
    "Weapon",
    "Projectile",
    "Threshold",
    "Delay",
    "Random",
    "Variation",
    "Count",
    "Index",
    "Id",
    "Group",
    "Priority",
    "Range",
    "Radius",
    "Falloff",
    "Distance",
    "Near",
    "Far",
    "Time",
    "Rate",
    "Low",
    "High",
    "Mid",
    "Default",
    "Base",
    "Loop",
    "Start",
    "Stop",
    "Idle",
    "Tire",
    "Skid",
    "Gravel",
    "Dirt",
    "Grass",
    "Sand",
    "Asphalt",
    "Metal",
    "Wood",
    "Stone",
    "Glass",
    "Fabric",
    "Foliage",
    "Rubber",
    "Flesh",
    "Gear",
    "Rustle",
    "Whoosh",
    "Punch",
    "Kick",
    "Shell",
];

/// Names for the sound side's fields: [`SOUND_WORDS`] alone and in
/// pairs, bare or with a type prefix, and the clips' sound events.
fn sound_field_names(events: &BTreeMap<(String, String), usize>) -> HashMap<u32, String> {
    let mut out = HashMap::new();
    let mut add = |parts: &[&str]| {
        out.entry(hash_parts(parts))
            .or_insert_with(|| parts.concat());
    };
    for p in ["", "f", "i", "b", "m_", "s", "n", "e", "u"] {
        for a in SOUND_WORDS {
            add(&[p, a]);
            for b in SOUND_WORDS {
                add(&[p, a, b]);
                add(&[p, a, "_", b]);
            }
        }
    }
    for (_, n) in events.keys() {
        add(&[n]);
    }
    out
}

/// The sound materials dumped when no word is given: the ground's, the
/// people's and a crate's.
const SOUND_MATERIALS: [&str; 12] = [
    "terrain",
    "terrain_dirt",
    "terrain_grass",
    "terrain_sand",
    "terrain_asphalt",
    "water_shallow",
    "hum_hero_jen",
    "hum_soldier",
    "Human",
    "prop_wood_crate",
    "bldg_stone",
    "prop_metal",
];

/// The template database's sound side: the components the sound
/// materials (`SoundMaterial (..)`) have and how each field's values
/// fall; some materials in full, each value that is a bank's sound shown
/// as `bank#index length`; every component whose values name bank sounds
/// or the clips' sound events; and how many of each bank's sounds the
/// database names.
fn soundkeys<R: Read + Seek>(source: &mut AssetSource<R>, names: &Names) -> Result<(), Error> {
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
    let data = source.asset(db_hash, TEMPLATE_DB_TYPE)?;
    let db = Layer::parse(&data)?;
    let tnames = db.names();
    let key = m2_assets::name_hash("SoundKey");
    // Each value: how many templates give it, and the first by name.
    let mut found: BTreeMap<u32, (usize, &str)> = BTreeMap::new();
    for c in db.components.iter().filter(|c| c.name == "SoundKey") {
        let Some(f) = c.fields.iter().find(|f| f.name_hash == key) else {
            continue;
        };
        let at = usize::from(f.offset);
        for (id, r) in c.records().unwrap_or_default() {
            let Some(bytes) = r.get(at..at + 4) else {
                continue;
            };
            let v = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            if v == 0 {
                continue;
            }
            let name = tnames.get(&id).copied().unwrap_or("?");
            let e = found.entry(v).or_insert((0, name));
            e.0 += 1;
            e.1 = e.1.min(name);
        }
    }
    let mut list: Vec<(&str, u32, usize, &str)> = found
        .into_iter()
        .map(|(v, (n, t))| (names.show(v), v, n, t))
        .collect();
    list.sort();
    println!("{} sound keys", list.len());
    for (name, v, n, t) in list {
        println!("  {v:08x} {name} {n} ({t})");
    }
    Ok(())
}

fn sounddb<R: Read + Seek>(
    source: &mut AssetSource<R>,
    names: &Names,
    arg: &str,
) -> Result<(), Error> {
    let words: Vec<String> = arg
        .split(',')
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    let events = sound_events(source)?;
    let library = bank_sounds(source)?;
    let (sounds, banks) = (&library.sounds, &library.banks);
    let guessed = sound_field_names(&events);
    let cue_names: HashMap<u32, &str> = events
        .keys()
        .map(|(_, n)| (hash_parts(&[n]), n.as_str()))
        .collect();
    let index = source.archive().index();
    let blocks: HashMap<u32, String> = index
        .blocks()
        .iter()
        .map(|b| {
            let n = m2_engine::world::layer_name(&b.path);
            (m2_assets::name_hash(&n), n)
        })
        .collect();
    let bank_set: HashSet<u32> = banks.iter().map(|b| b.0).collect();
    let Some(db_hash) = index
        .assets()
        .iter()
        .find(|a| index.type_hash(a) == TEMPLATE_DB_TYPE)
        .map(|a| a.name_hash)
    else {
        println!("no template database");
        return Ok(());
    };
    let data = source.asset(db_hash, TEMPLATE_DB_TYPE)?;
    let db = Layer::parse(&data)?;
    let tnames = db.names();
    let records: Vec<ComponentRecords<'_>> = db
        .components
        .iter()
        .filter(|c| c.name != "Name")
        .map(|c| (c.name, c.fields.clone(), by_entity(c)))
        .collect();
    let field_name = |f: &m2_assets::layer::Field| -> String {
        names
            .get(f.name_hash)
            .map(str::to_owned)
            .or_else(|| guessed.get(&f.name_hash).cloned())
            .unwrap_or_else(|| format!("{:08x}", f.name_hash))
    };
    // What a word is, when it names something.
    let what = |v: u32| -> Option<String> {
        if let Some(list) = sounds.get(&v) {
            let (bank, i, secs) = list[0];
            return Some(format!(
                "{v:08x} sound {}#{i} {secs:.2}s{}",
                library.label(names, bank),
                if list.len() > 1 { " (+more banks)" } else { "" }
            ));
        }
        if let Some(n) = tnames.get(&v) {
            return Some(format!("{v:08x} template {n:?}"));
        }
        if bank_set.contains(&v) {
            return Some(format!("{v:08x} bank {}", library.label(names, v)));
        }
        if let Some(n) = blocks.get(&v) {
            return Some(format!("{v:08x} block {n}"));
        }
        if let Some(n) = cue_names.get(&v) {
            return Some(format!("{v:08x} cue {n}"));
        }
        names
            .get(v)
            .filter(|_| v > 0xffff)
            .map(|n| format!("{v:08x} {n}"))
    };
    // A field's value: `None` when zero.
    let value = |f: &m2_assets::layer::Field, rec: &[u8]| -> Option<(u32, String)> {
        let at = usize::from(f.offset);
        let bytes = rec.get(at..at + 4).or_else(|| rec.get(at..))?;
        let mut word = [0u8; 4];
        word[..bytes.len().min(4)].copy_from_slice(&bytes[..bytes.len().min(4)]);
        let v = u32::from_le_bytes(word);
        if f.bit_width > 0 && f.bit_width < 32 {
            let bits = (v >> f.bit_shift) & ((1u32 << f.bit_width) - 1);
            return (bits != 0).then(|| (bits, bits.to_string()));
        }
        if v == 0 {
            return None;
        }
        if let Some(w) = what(v) {
            return Some((v, w));
        }
        let float = f32::from_le_bytes(word);
        Some((
            v,
            if float.is_finite() && (1e-4..1e7).contains(&float.abs()) {
                format!("{float:.3}")
            } else {
                format!("{v:#x}")
            },
        ))
    };
    let show = |id: u32, name: &str| {
        println!("  {id:08x} {name:?}");
        for (cname, fields, recs) in &records {
            for rec in recs.get(&id).into_iter().flatten() {
                let values: Vec<String> = fields
                    .iter()
                    .filter_map(|f| value(f, rec).map(|(_, v)| format!("{}={v}", field_name(f))))
                    .collect();
                println!(
                    "    {cname}: {}",
                    if values.is_empty() {
                        "(all zero)".to_owned()
                    } else {
                        values.join(", ")
                    }
                );
            }
        }
    };
    // The sound materials, and the components they have.
    let mut materials: Vec<(&str, u32)> = tnames
        .iter()
        .filter(|(_, n)| n.starts_with("SoundMaterial ("))
        .map(|(&id, &n)| (n, id))
        .collect();
    materials.sort_unstable();
    println!(
        "{} sound materials; {} sound events in clips; {} banks, {} sounds",
        materials.len(),
        events.len(),
        banks.len(),
        sounds.len()
    );
    let ids: HashSet<u32> = materials.iter().map(|m| m.1).collect();
    println!("components the sound materials have:");
    for (cname, fields, recs) in &records {
        let mine: Vec<&[u8]> = recs
            .iter()
            .filter(|(id, _)| ids.contains(id))
            .flat_map(|(_, r)| r.iter().copied())
            .collect();
        if mine.is_empty() {
            continue;
        }
        println!(
            "  {cname}: {} of them ({} templates in all), {} fields",
            mine.len(),
            recs.len(),
            fields.len()
        );
        for f in fields {
            let vals: Vec<(u32, String)> = mine.iter().filter_map(|r| value(f, r)).collect();
            let distinct: HashSet<u32> = vals.iter().map(|v| v.0).collect();
            let sounding = vals.iter().filter(|v| sounds.contains_key(&v.0)).count();
            let templ = vals.iter().filter(|v| tnames.contains_key(&v.0)).count();
            let sample: Vec<&str> = vals.iter().take(3).map(|v| v.1.as_str()).collect();
            println!(
                "    {} (t{} @{}{}): {} set, {} distinct, {sounding} sounds, {templ} templates; e.g. {}",
                field_name(f),
                f.ty,
                f.offset,
                if f.bit_width > 0 {
                    format!(" bits {}+{}", f.bit_shift, f.bit_width)
                } else {
                    String::new()
                },
                vals.len(),
                distinct.len(),
                sample.join(" | ")
            );
        }
    }
    // Some in full: those holding a word, or the ground's and people's.
    let picked: Vec<(&str, u32)> = if words.is_empty() {
        SOUND_MATERIALS
            .iter()
            .filter_map(|m| {
                let want = format!("SoundMaterial ({m})");
                materials
                    .iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case(&want))
                    .copied()
            })
            .collect()
    } else {
        let mut p: Vec<(&str, u32)> = tnames
            .iter()
            .filter(|(_, n)| {
                let n = n.to_ascii_lowercase();
                words.iter().any(|w| n.contains(w.as_str()))
            })
            .map(|(&id, &n)| (n, id))
            .collect();
        p.sort_unstable();
        p
    };
    println!("{} templates in full:", picked.len());
    for &(name, id) in picked.iter().take(60) {
        show(id, name);
    }
    // Where the database names bank sounds or clip cues.
    println!("components whose values name bank sounds or clip cues:");
    let mut named_sounds: HashSet<u32> = HashSet::new();
    for (cname, fields, recs) in &records {
        let mut per_field: BTreeMap<String, (usize, usize, String)> = BTreeMap::new();
        for (id, rec) in recs
            .iter()
            .flat_map(|(id, r)| r.iter().map(move |r| (id, r)))
        {
            for f in fields {
                let Some((v, shown)) = value(f, rec) else {
                    continue;
                };
                let sound = sounds.contains_key(&v);
                let cue = cue_names.contains_key(&v);
                if !(sound || cue) {
                    continue;
                }
                if sound {
                    named_sounds.insert(v);
                }
                let e = per_field.entry(field_name(f)).or_insert_with(|| {
                    (
                        0,
                        0,
                        format!("{} {shown}", tnames.get(id).copied().unwrap_or("?")),
                    )
                });
                if sound {
                    e.0 += 1;
                } else {
                    e.1 += 1;
                }
            }
        }
        if per_field.is_empty() {
            continue;
        }
        println!("  {cname} ({} templates):", recs.len());
        for (f, (s, c, eg)) in &per_field {
            println!("    {f}: {s} sounds, {c} cues; e.g. {eg}");
        }
    }
    // Templates named like a clip's sound event.
    // Named exactly so first, then holding a (long) event name.
    let mut cued: Vec<(bool, &str, u32)> = tnames
        .iter()
        .filter_map(|(&id, &n)| {
            let lower = n.to_ascii_lowercase();
            let exact = events.keys().any(|(_, e)| e.eq_ignore_ascii_case(n));
            let holds = events
                .keys()
                .any(|(_, e)| e.len() >= 8 && lower.contains(&e.to_ascii_lowercase()));
            (exact || holds).then_some((!exact, n, id))
        })
        .collect();
    cued.sort_unstable();
    println!("{} templates named like a clip's sound event:", cued.len());
    for &(_, name, id) in cued.iter().take(20) {
        show(id, name);
    }
    println!("bank sounds the database names, per bank:");
    let mut per_bank: BTreeMap<u32, usize> = BTreeMap::new();
    for h in &named_sounds {
        for (bank, _, _) in &sounds[h] {
            *per_bank.entry(*bank).or_default() += 1;
        }
    }
    for (bank, total) in banks {
        if let Some(n) = per_bank.get(bank) {
            println!(
                "  {bank:08x} {}: {n} of {total}",
                library.label(names, *bank)
            );
        }
    }
    Ok(())
}

/// FNV-1a of `s` (each byte OR 0x20) continued from `h`: [`hash_parts`]
/// a part at a time.
fn fnv_on(mut h: u32, s: &str) -> u32 {
    for b in s.bytes() {
        h ^= u32::from(b | 0x20);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// The name hash of what [`fnv_on`] has taken in.
fn fnv_end(h: u32) -> u32 {
    (h ^ u32::from(b'*')).wrapping_mul(0x0100_0193)
}

const FNV_START: u32 = 0x811c_9dc5;

/// More words sound names may be built of.
const SOUND_NAME_WORDS: [&str; 64] = [
    "fs",
    "ft",
    "ftstp",
    "footstep",
    "footsteps",
    "foot",
    "step",
    "steps",
    "walk",
    "run",
    "sprint",
    "land",
    "jump",
    "jumpland",
    "takeoff",
    "scuff",
    "slide",
    "jen",
    "hero",
    "soldier",
    "boot",
    "boots",
    "shoe",
    "bare",
    "heel",
    "toe",
    "conc",
    "concrete",
    "mud",
    "tile",
    "rock",
    "leaves",
    "bush",
    "hollow",
    "solid",
    "generic",
    "default",
    "impact",
    "imp",
    "hit",
    "body",
    "bodyfall",
    "fall",
    "melee",
    "whoosh",
    "swish",
    "swoosh",
    "punch",
    "kick",
    "butt",
    "rifle",
    "gun",
    "knife",
    "bash",
    "lt",
    "hvy",
    "hv",
    "lite",
    "soft",
    "hard",
    "short",
    "long",
    "human",
    "hum",
];

/// Names for bank sounds built of words: a prefix, a word, optionally
/// `_` and another word, and an ending; the words are the sound
/// materials' names and their parts, the clips' sound events and their
/// parts, and [`SOUND_WORDS`] and [`SOUND_NAME_WORDS`].
fn soundnames<R: Read + Seek>(
    source: &mut AssetSource<R>,
    names: &Names,
    arg: &str,
) -> Result<(), Error> {
    let wanted: Vec<String> = if arg.is_empty() {
        vec![
            "fol_shared".into(),
            "collision_shared".into(),
            "destruction_shared".into(),
            "7871f925".into(),
        ]
    } else {
        arg.split(',')
            .filter(|w| !w.is_empty())
            .map(str::to_owned)
            .collect()
    };
    let events = sound_events(source)?;
    let library = bank_sounds(source)?;
    let index = source.archive().index();
    let db_hash = index
        .assets()
        .iter()
        .find(|a| index.type_hash(a) == TEMPLATE_DB_TYPE)
        .map(|a| a.name_hash);
    let mut vocab: Vec<String> = Vec::new();
    let mut take = |w: &str| {
        let w = w.to_ascii_lowercase();
        if !w.is_empty() {
            for part in w.split('_').filter(|p| p.len() >= 2) {
                vocab.push(part.to_owned());
            }
            vocab.push(w);
        }
    };
    if let Some(h) = db_hash {
        let data = source.asset(h, TEMPLATE_DB_TYPE)?;
        let db = Layer::parse(&data)?;
        for n in db.names().values() {
            if let Some(inner) = n
                .strip_prefix("SoundMaterial (")
                .and_then(|r| r.strip_suffix(')'))
            {
                take(inner);
            }
        }
    }
    for (_, n) in events.keys() {
        take(n);
    }
    for w in SOUND_WORDS.iter().chain(SOUND_NAME_WORDS.iter()) {
        take(w);
    }
    vocab.sort();
    vocab.dedup();
    let prefixes = [
        "",
        "fol_",
        "sfx_",
        "sfx_fol_",
        "col_",
        "sfx_col_",
        "imp_",
        "sfx_imp_",
        "fs_",
        "ftstp_",
        "footstep_",
        "hum_",
        "sfx_hum_",
        "jen_",
        "melee_",
        "sfx_melee_",
        "ahj_",
        "sfx_ahj_",
        "foley_",
        "phys_",
    ];
    let long_ends = endings();
    let short_ends: Vec<String> = [
        "", "_01", "_02", "_03", "_04", "_05", "_06", "01", "02", "_1", "_2", "_a", "_b", "a", "b",
    ]
    .iter()
    .map(|e| (*e).to_owned())
    .collect();
    // The banks asked for: by name, or by hash (a streamed bank).
    let banks: Vec<u32> = library
        .banks
        .iter()
        .map(|b| b.0)
        .filter(|&b| {
            wanted
                .iter()
                .any(|w| hex(w) == Some(b) || library.label(names, b).eq_ignore_ascii_case(w))
        })
        .collect();
    let targets: HashSet<u32> = library
        .sounds
        .iter()
        .filter(|(_, list)| list.iter().any(|(b, _, _)| banks.contains(b)))
        .map(|(&h, _)| h)
        .collect();
    let mut found: HashMap<u32, String> = HashMap::new();
    for &t in &targets {
        if let Some(n) = names.get(t) {
            found.insert(t, n.to_owned());
        }
    }
    for p in prefixes {
        let hp = fnv_on(FNV_START, p);
        for a in &vocab {
            let ha = fnv_on(hp, a);
            for e in &long_ends {
                let h = fnv_end(fnv_on(ha, e));
                if targets.contains(&h) {
                    found.entry(h).or_insert_with(|| format!("{p}{a}{e}"));
                }
            }
            let hs = fnv_on(ha, "_");
            for b in &vocab {
                let hb = fnv_on(hs, b);
                for e in &short_ends {
                    let h = fnv_end(fnv_on(hb, e));
                    if targets.contains(&h) {
                        found.entry(h).or_insert_with(|| format!("{p}{a}_{b}{e}"));
                    }
                }
            }
        }
    }
    println!(
        "{} words, {} prefixes: {} of {} sounds named",
        vocab.len(),
        prefixes.len(),
        found.len(),
        targets.len()
    );
    for &bank in &banks {
        let mut rows: Vec<(usize, u32, f32)> = library
            .sounds
            .iter()
            .flat_map(|(&h, list)| {
                list.iter()
                    .filter(move |(b, _, _)| *b == bank)
                    .map(move |&(_, i, secs)| (i, h, secs))
            })
            .collect();
        rows.sort_unstable_by_key(|r| r.0);
        let named = rows.iter().filter(|r| found.contains_key(&r.1)).count();
        println!(
            "bank {bank:08x} {}: {named} of {} named",
            library.label(names, bank),
            rows.len()
        );
        for (i, h, secs) in &rows {
            if let Some(n) = found.get(h) {
                println!("  #{i} {h:08x} {secs:.2}s {n}");
            }
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
            match &cpu.collision {
                Some(c) => {
                    let (lo, hi) = c.bounds();
                    println!(
                        "  no PHY2: {} visible triangles stand in, filling ({:.2} {:.2} {:.2}) to ({:.2} {:.2} {:.2})",
                        c.triangles, lo.x, lo.y, lo.z, hi.x, hi.y, hi.z
                    );
                }
                None => println!("  no PHY2, and nothing solid drawn: no collision"),
            }
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
        for (i, body) in p.bodies.iter().enumerate() {
            println!(
                "  body {i}: {}, mass {:.1} kg",
                if body.is_fixed() { "fixed" } else { "moves" },
                body.mass
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

/// The raw words of `PHY2` records, to find what tells shapes apart
/// (foliage from rock, a pole that breaks from a wall): for `arg`'s
/// models, how the alpha knocks each loose, each body's node (shown
/// intact, wrecked or both), unknown floats and words, and each shape's
/// nodes and record words (the vertex pool bounds, count and offset, and
/// the shape's own index left out); then, over every placed model, how
/// many come loose and how, shapes and bodies by whether their nodes show
/// intact or wrecked and their bodies move, and the values of each
/// unknown word with how many shapes or bodies of models named for
/// plants, rocks, poles... carry them (words with many values just
/// counted).
fn phy2_words<R: Read + Seek>(
    source: &mut AssetSource<R>,
    world: &WorldIndex,
    names: &Names,
    arg: &str,
) -> Result<(), Error> {
    use m2_assets::physics::{Geometry, Physics as Phy2};
    /// A model's PHY2, with which of its nodes are hidden intact and
    /// wrecked (none when it switches nothing).
    fn phy2_of<R: Read + Seek>(
        source: &mut AssetSource<R>,
        h: u32,
    ) -> Option<(Phy2, [Vec<bool>; 2])> {
        let data = source.model(h).ok()?;
        let m = data.parse().ok()?;
        let hidden = m.states.as_ref().map_or_else(Default::default, |s| {
            [s.hidden(&m.nodes, false), s.hidden(&m.nodes, true)]
        });
        let u = m2_assets::Ucfx::parse(&data.base).ok()?;
        let p = Phy2::parse(u.top_level(*b"PHY2")?).ok()?;
        Some((p, hidden))
    }
    /// Whether node `n` shows intact and wrecked: 0 both, 1 intact only,
    /// 2 wrecked only, 3 neither.
    fn shows(hidden: &[Vec<bool>; 2], n: u32) -> u32 {
        let off = |w: usize| hidden[w].get(n as usize).copied().unwrap_or(false);
        u32::from(off(0)) * 2 + u32::from(off(1))
    }
    const SHOWS: [&str; 4] = ["both", "intact", "wrecked", "neither"];
    /// How the alpha knocks the model loose: its pieces (node, kg), and
    /// whether it breaks into them.
    fn loose_of<R: Read + Seek>(source: &mut AssetSource<R>, h: u32) -> Option<String> {
        let data = source.model(h).ok()?;
        let cpu = m2_engine::CpuModel::from_data(&data, 0).ok()?;
        let loose = cpu.collision?.loose?;
        let pieces: Vec<String> = loose
            .pieces
            .iter()
            .map(|q| match q.node {
                Some(n) => format!("node {n} {:.1} kg", q.mass),
                None => format!("whole {:.1} kg", q.mass),
            })
            .collect();
        Some(format!(
            "{} ({}), {:.1} kg in all",
            if loose.breaks {
                "breaks into its pieces"
            } else {
                "goes as one"
            },
            pieces.join(", "),
            loose.mass
        ))
    }
    let kind = |g: &Geometry| match g {
        Geometry::Box { .. } => "box",
        Geometry::Sphere => "sphere",
        Geometry::Capsule { .. } => "capsule",
        Geometry::Convex { .. } => "convex",
        Geometry::Mesh { .. } => "mesh",
    };
    // Shape record words not yet understood.
    const SHAPE_WORDS: [usize; 15] = [0, 1, 2, 4, 11, 12, 13, 14, 15, 17, 19, 20, 21, 22, 23];
    for h in arg.split(',').filter_map(hex) {
        let Some((p, hidden)) = phy2_of(source, h) else {
            println!("{h:08x}: no PHY2");
            continue;
        };
        println!(
            "{h:08x} {}: header {:08x?}, {}",
            names.show(h),
            p.unk_0x14,
            if hidden[0] == hidden[1] {
                "no wreck"
            } else {
                "has a wreck"
            }
        );
        println!(
            "  knocked loose: {}",
            loose_of(source, h).unwrap_or_else(|| "never (no body moves)".into())
        );
        // The node a body sits on: its part's.
        let body_node = |b: usize| p.parts.iter().find(|q| q.body == Some(b)).map(|q| q.node);
        for (i, b) in p.bodies.iter().enumerate() {
            let node = body_node(i);
            println!(
                "  body {i} {} {:.1} kg on node {node:?} (shows {}): centre {:?} unk {:?} words {:08x?}",
                if b.is_fixed() { "fixed" } else { "moves" },
                b.mass,
                node.map_or("?", |n| SHOWS[shows(&hidden, n) as usize]),
                b.centre_of_mass,
                b.unk,
                b.unk_words
            );
        }
        for (k, shape) in p.shapes.iter().enumerate() {
            let words: Vec<String> = SHAPE_WORDS
                .iter()
                .map(|&w| format!("{w}:{:x}", shape.record[w]))
                .collect();
            let own = p.parts.get(shape.part).map(|q| q.node);
            println!(
                "  shape {k} {} on body {:?}, part node {own:?} (shows {}), body node {:?}: {}",
                kind(&shape.geometry),
                p.body_of(k),
                own.map_or("?", |n| SHOWS[shows(&hidden, n) as usize]),
                p.shape_node(k),
                words.join(" ")
            );
        }
        let links: Vec<String> = p
            .parts
            .iter()
            .enumerate()
            .filter(|(_, part)| part.unk_link != u32::MAX)
            .map(|(i, part)| format!("{i}:{:x}", part.unk_link))
            .collect();
        if !links.is_empty() {
            println!("  parts' unknown links: {}", links.join(" "));
        }
    }
    // Over every placed model: values of each unknown word, by the kind
    // of thing its model's name says it is.
    const KINDS: [&str; 14] = [
        "plant",
        "bush",
        "scrub",
        "tree",
        "rock",
        "pole",
        "fence",
        "sign",
        "lamp",
        "barrel",
        "crate",
        "wall",
        "sandbag",
        "barricade",
    ];
    let mut models: Vec<u32> = world
        .objects
        .iter()
        .map(|o| o.model)
        .chain(world.state_objects.values().map(|o| o.model))
        .collect();
    models.sort_unstable();
    models.dedup();
    // word -> value -> (count, per kind (+ other, unnamed)).
    type Tally = BTreeMap<usize, HashMap<u32, (usize, [usize; 16])>>;
    let mut shape_tally: Tally = BTreeMap::new();
    let mut body_tally: Tally = BTreeMap::new();
    let class = |h: u32| {
        names.get(h).map_or(15, |n| {
            KINDS.iter().position(|k| n.contains(k)).unwrap_or(14)
        })
    };
    // Shapes and bodies by whether their node shows intact or wrecked and
    // their body moves: [shows][moves].
    let mut shape_shows = [[0usize; 2]; 4];
    let mut body_shows = [[0usize; 2]; 4];
    let (mut wrecks, mut moving_wrecks) = (0, 0);
    // Placed models the alpha knocks loose: as one, into pieces.
    let (mut as_one, mut to_pieces) = (0, 0);
    for &h in &models {
        match loose_of(source, h) {
            Some(l) if l.starts_with("breaks") => to_pieces += 1,
            Some(_) => as_one += 1,
            None => {}
        }
        let Some((p, hidden)) = phy2_of(source, h) else {
            continue;
        };
        let c = class(h);
        let moves = |b: Option<usize>| {
            b.and_then(|b| p.bodies.get(b))
                .is_some_and(|b| !b.is_fixed())
        };
        if hidden[0] != hidden[1] {
            wrecks += 1;
            moving_wrecks += usize::from(p.bodies.iter().any(|b| !b.is_fixed()));
        }
        for (k, shape) in p.shapes.iter().enumerate() {
            let node = p.parts.get(shape.part).map_or(0, |q| q.node);
            let show = shows(&hidden, node);
            shape_shows[show as usize][usize::from(moves(p.body_of(k)))] += 1;
            // Pseudo-word 100: shows (0 both, 1 intact, 2 wrecked, 3
            // neither) plus 4 when its body moves.
            let words = SHAPE_WORDS
                .iter()
                .map(|&w| (w, shape.record[w]))
                .chain([(100, show + 4 * u32::from(moves(p.body_of(k))))]);
            for (w, v) in words {
                let e = shape_tally.entry(w).or_default().entry(v).or_default();
                e.0 += 1;
                e.1[c] += 1;
            }
        }
        for (i, b) in p.bodies.iter().enumerate() {
            let node = p
                .parts
                .iter()
                .find(|q| q.body == Some(i))
                .map_or(0, |q| q.node);
            let show = shows(&hidden, node);
            body_shows[show as usize][usize::from(!b.is_fixed())] += 1;
            let words = b
                .unk
                .iter()
                .map(|f| f.to_bits())
                .chain(b.unk_words.iter().copied())
                .chain([u32::from(b.is_fixed()), show]);
            for (w, v) in words.enumerate() {
                let e = body_tally.entry(w).or_default().entry(v).or_default();
                e.0 += 1;
                e.1[c] += 1;
            }
        }
    }
    let show = |what: &str, tally: &Tally| {
        for (w, values) in tally {
            if values.len() > 24 {
                println!("{what} word {w}: {} values", values.len());
                continue;
            }
            let mut v: Vec<(&u32, &(usize, [usize; 16]))> = values.iter().collect();
            v.sort_by_key(|(_, (n, _))| std::cmp::Reverse(*n));
            for (value, (n, by)) in v.iter().take(8) {
                let kinds: Vec<String> = by
                    .iter()
                    .enumerate()
                    .filter(|(_, k)| **k > 0)
                    .map(|(i, k)| {
                        let name = match i {
                            14 => "other",
                            15 => "unnamed",
                            _ => KINDS[i],
                        };
                        format!("{name} {k}")
                    })
                    .collect();
                println!(
                    "{what} word {w} = {value:08x} ({:.3}): {n} [{}]",
                    f32::from_bits(**value),
                    kinds.join(", ")
                );
            }
        }
    };
    println!("over {} placed models' PHY2:", models.len());
    println!("{wrecks} models with PHY2 have a wreck, {moving_wrecks} of them a body that moves");
    println!("knocked loose: {as_one} models go as one, {to_pieces} break into their pieces");
    for (what, counts) in [("shapes", shape_shows), ("bodies", body_shows)] {
        for (i, [fixed, moving]) in counts.iter().enumerate() {
            println!(
                "{what} on nodes shown {}: {fixed} fixed, {moving} moving",
                SHOWS[i]
            );
        }
    }
    show("shape", &shape_tally);
    show("body", &body_tally);
    Ok(())
}

/// Which placed models have `PHY2` collision and which stand in their
/// visible triangles (the alpha's fallback): the island's objects placed
/// by name (rocks, plants, fences), those placed by `ModelName`, and the
/// state layers' props. How many models and placements each, the
/// commonest without, those whose bodies move, and a tally by name word.
fn phy2_census<R: Read + Seek>(
    source: &mut AssetSource<R>,
    world: &WorldIndex,
    names: &Names,
) -> Result<(), Error> {
    use m2_assets::physics::Physics as Phy2;
    // Placements per model: by name, by ModelName, by the state layers.
    let mut placed: HashMap<u32, [usize; 3]> = HashMap::new();
    for o in &world.objects {
        placed.entry(o.model).or_default()[if o.named { 0 } else { 1 }] += 1;
    }
    for o in world.state_objects.values() {
        placed.entry(o.model).or_default()[2] += 1;
    }
    struct Seen {
        hash: u32,
        placed: [usize; 3],
        /// Bodies, fixed bodies, shapes; `None` without `PHY2`.
        phy2: Option<(usize, usize, usize)>,
        /// The moving bodies' mass, kg.
        mass: f32,
        size: f32,
    }
    let mut seen: Vec<Seen> = Vec::new();
    let mut unread = 0;
    for (&hash, &n) in &placed {
        let Ok(data) = source.model(hash) else {
            unread += 1;
            continue;
        };
        let Ok(m) = data.parse() else {
            unread += 1;
            continue;
        };
        let b = &m.info.bounds;
        let size = (Vec3::from(b.max) - Vec3::from(b.min)).length();
        let phy2 = m2_assets::Ucfx::parse(&data.base)
            .ok()
            .and_then(|u| u.top_level(*b"PHY2"))
            .and_then(|d| Phy2::parse(d).ok());
        seen.push(Seen {
            hash,
            placed: n,
            phy2: phy2.as_ref().map(|p| {
                (
                    p.bodies.len(),
                    p.bodies.iter().filter(|b| b.is_fixed()).count(),
                    p.shapes.len(),
                )
            }),
            mass: phy2.as_ref().map_or(0.0, |p| {
                p.bodies
                    .iter()
                    .filter(|b| !b.is_fixed())
                    .map(|b| b.mass)
                    .sum()
            }),
            size,
        });
    }
    println!("{} models placed, {} unreadable", placed.len(), unread);
    const HOW: [&str; 3] = ["by name", "by ModelName", "by the state layers"];
    for (k, how) in HOW.iter().enumerate() {
        let tally = |with: bool| {
            seen.iter()
                .filter(|s| s.placed[k] > 0 && s.phy2.is_some() == with)
                .fold((0, 0), |(m, p), s| (m + 1, p + s.placed[k]))
        };
        let (with, without) = (tally(true), tally(false));
        println!(
            "placed {how}: {} models ({} placements) with PHY2, {} ({}) without",
            with.0, with.1, without.0, without.1
        );
    }
    for (k, how) in HOW.iter().enumerate() {
        let mut without: Vec<&Seen> = seen
            .iter()
            .filter(|s| s.placed[k] > 0 && s.phy2.is_none())
            .collect();
        without.sort_by_key(|s| std::cmp::Reverse(s.placed[k]));
        println!("commonest without PHY2, placed {how}:");
        for s in without.iter().take(if k == 0 { 50 } else { 25 }) {
            println!(
                "  {:5} x {:08x} {}: {:.1} m",
                s.placed[k],
                s.hash,
                names.show(s.hash),
                s.size
            );
        }
    }
    let mut moving: Vec<&Seen> = seen
        .iter()
        .filter(|s| s.phy2.is_some_and(|(bodies, fixed, _)| fixed < bodies))
        .collect();
    moving.sort_by_key(|s| std::cmp::Reverse(s.placed.iter().sum::<usize>()));
    println!(
        "with a body that moves: {} models; commonest:",
        moving.len()
    );
    for s in moving.iter().take(30) {
        let (bodies, fixed, shapes) = s.phy2.unwrap_or_default();
        println!(
            "  {:5} x {:08x} {}: {bodies} bodies ({fixed} fixed), {shapes} shapes, {:.0} kg moving, {:.1} m, placed {:?}",
            s.placed.iter().sum::<usize>(),
            s.hash,
            names.show(s.hash),
            s.mass,
            s.size,
            s.placed
        );
    }
    const WORDS: [&str; 30] = [
        "plant", "bush", "shrub", "fern", "grass", "reed", "flower", "weed", "vine", "leaf",
        "hedge", "bamboo", "banana", "palm", "tree", "log", "stump", "rock", "boulder", "fence",
        "sandbag", "wall", "sign", "pole", "lamp", "crate", "barrel", "debris", "rubble", "cactus",
    ];
    println!("by name word (models, placements): with PHY2 / without");
    for w in WORDS {
        let tally = |with: bool| {
            seen.iter()
                .filter(|s| s.phy2.is_some() == with && names.show(s.hash).contains(w))
                .fold((0, 0), |(m, p), s| {
                    (m + 1, p + s.placed.iter().sum::<usize>())
                })
        };
        let (a, b) = (tally(true), tally(false));
        if a.0 + b.0 > 0 {
            println!("  {w}: {} ({}) / {} ({})", a.0, a.1, b.0, b.1);
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

/// The vehicle physics components, each with the kind the alpha's world
/// index gives it.
const VEHICLE_PHYSICS: [&str; 4] = [
    "_CarPhysicsV2",
    "_TankPhysics",
    "_BoatPhysics",
    "_HelicopterPhysics",
];
/// `EntranceLink`/`PhysicalLink`/`SeatLink`'s field naming the linked
/// template.
const LINK_FIELD: u32 = 0x5470_4554;
/// `EntranceParameters`' hijack script.
const HIJACK_SCRIPT: u32 = 0x2c5d_34dc;
/// `Health`'s hit points.
const HEALTH_FIELD: u32 = 0xd122_919d;
/// `SoundEffect`'s engine bank.
const BANK_FIELD: u32 = 0x14a6_7fa6;

/// The `u32` of a record's field, by the field's name hash.
fn field_u32(c: &m2_assets::layer::Component<'_>, rec: &[u8], name: u32) -> Option<u32> {
    let f = c.fields.iter().find(|f| f.name_hash == name)?;
    let at = usize::from(f.offset);
    let b = rec.get(at..at + 4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Every vehicle template with what the data gives it and what the alpha
/// makes of it, one tab-separated line each.
fn fleet<R: Read + Seek>(
    source: &mut AssetSource<R>,
    lazy: &mut Lazy,
    names: &Names,
    arg: &str,
) -> Result<(), Error> {
    let words = words_of(arg);
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
    let models: HashSet<u32> = index
        .assets()
        .iter()
        .filter(|a| index.type_hash(a) == MODEL_TYPE)
        .map(|a| a.name_hash)
        .collect();
    let scripts: HashSet<u32> = index
        .assets()
        .iter()
        .filter(|a| index.type_hash(a) == SCRIPT_TYPE)
        .map(|a| a.name_hash)
        .collect();
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
    let tnames = db.names();
    let model_of = db.hashes("ModelName");

    // Each template's components, and records of the ones read here.
    let mut comps: HashMap<u32, Vec<&str>> = HashMap::new();
    let mut unreadable: Vec<&str> = Vec::new();
    for c in &db.components {
        match c.records() {
            Some(recs) => {
                for (id, _) in recs {
                    comps.entry(id).or_default().push(c.name);
                }
            }
            None => unreadable.push(c.name),
        }
    }
    println!(
        "components not read per template (text fields): {}",
        unreadable.join(", ")
    );
    let recs = |name: &str| -> (
        Option<&m2_assets::layer::Component<'_>>,
        HashMap<u32, Vec<&[u8]>>,
    ) {
        match db.component(name) {
            Some(c) => (Some(c), by_entity(c)),
            None => (None, HashMap::new()),
        }
    };
    let mut kind_of: HashMap<u32, &str> = HashMap::new();
    for p in VEHICLE_PHYSICS {
        if let Some(c) = db.component(p) {
            for (id, _) in c.records().unwrap_or_default() {
                kind_of.insert(id, p);
            }
        }
    }
    let (health_c, health) = recs("Health");
    let (sound_c, sound) = recs("SoundEffect");
    let (entrance_c, entrances) = recs("EntranceLink");
    let (params_c, params) = recs("EntranceParameters");
    let (link_c, links) = recs("PhysicalLink");
    let (seat_c, seats) = recs("SeatLink");
    let (_, turrets) = recs("Turret");
    let (modkey_c, modkeys) = recs("ModifierKey");

    // Placements: by the entity's parent, else by its name's stem.
    let lower: HashMap<String, u32> = tnames
        .iter()
        .map(|(&id, n)| (n.to_ascii_lowercase(), id))
        .collect();
    let mut placed: HashMap<u32, [usize; 2]> = HashMap::new();
    for b in blocks {
        let decoded = source.block(b)?;
        let at = usize::from(!paths[b].contains("layers_static"));
        for (e, data) in BlockContents::parse(&decoded)?.iter() {
            if e.type_hash != LAYER_TYPE {
                continue;
            }
            let Ok(layer) = Layer::parse(data) else {
                continue;
            };
            let entity_names = layer.names();
            for (id, p) in layer.placements() {
                let by_stem = entity_names.get(&id).and_then(|n| {
                    lower.get(&m2_engine::world::member_stem(n).to_ascii_lowercase())
                });
                let t = if kind_of.contains_key(&p.parent) {
                    Some(p.parent)
                } else {
                    by_stem.copied()
                };
                if let Some(t) = t.filter(|t| kind_of.contains_key(t)) {
                    placed.entry(t).or_default()[at] += 1;
                }
            }
        }
    }

    let catalogue = lazy.catalogue(source)?.clone();
    let mut wheels: HashMap<u32, String> = HashMap::new();
    let show = |h: u32| -> String {
        tnames
            .get(&h)
            .map(|n| (*n).to_owned())
            .or_else(|| names.get(h).map(str::to_owned))
            .unwrap_or_else(|| format!("{h:08x}"))
    };
    let mut list: Vec<(&str, u32, &str)> = kind_of
        .iter()
        .map(|(&id, &k)| (tnames.get(&id).copied().unwrap_or(""), id, k))
        .filter(|(n, _, _)| {
            let n = n.to_ascii_lowercase();
            words.is_empty() || words.iter().any(|w| n.contains(w.as_str()))
        })
        .collect();
    list.sort_unstable_by_key(|&(n, id, k)| (k, n.to_ascii_lowercase(), id));
    println!(
        "fleet\tid\tname\tphysics\tmodel\tin archive\tcatalogue\twheels\tstatic\tstate\thealth\tdamage key\tseats\tentrances\thijack\tturrets\tweapons\tengine\talpha\tother components"
    );
    let mut by_kind: BTreeMap<&str, [usize; 4]> = BTreeMap::new();
    for (name, id, kind) in list {
        let model = model_of.get(&id).copied();
        let in_archive = model.is_some_and(|m| models.contains(&m));
        let entry = model.and_then(|m| catalogue.vehicles.iter().find(|e| e.model == m));
        let wheel = match (model, in_archive) {
            (Some(m), true) => wheels
                .entry(m)
                .or_insert_with(|| {
                    let e = entry.cloned().unwrap_or(CatalogueEntry {
                        model: m,
                        kind: "car".into(),
                        label: name.to_owned(),
                    });
                    match source.model(m).ok().and_then(|d| {
                        d.parse().ok().map(|md| {
                            VehicleSpec::from_model(&e, &md).map(|s| (s.wheels.len(), s.tracked))
                        })
                    }) {
                        Some(Some((n, t))) => format!("{n}{}", if t { " tracked" } else { "" }),
                        Some(None) => "none".into(),
                        None => "unreadable".into(),
                    }
                })
                .clone(),
            _ => "-".into(),
        };
        let [stat, state] = placed.get(&id).copied().unwrap_or_default();
        let hp = health_c
            .and_then(|c| {
                health
                    .get(&id)?
                    .first()
                    .and_then(|r| field_u32(c, r, HEALTH_FIELD))
            })
            .map_or("-".to_owned(), |v| format!("{}", f32::from_bits(v)));
        let modkey = modkey_c.map_or(String::new(), |c| {
            modkeys
                .get(&id)
                .into_iter()
                .flatten()
                .filter_map(|r| {
                    c.fields.first().and_then(|f| {
                        let at = usize::from(f.offset);
                        r.get(at..at + 4)
                            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    })
                })
                .map(&show)
                .collect::<Vec<_>>()
                .join(",")
        });
        let seat_names = seat_c.map_or(String::new(), |c| {
            seats
                .get(&id)
                .into_iter()
                .flatten()
                .map(|r| {
                    c.fields
                        .iter()
                        .filter(|f| f.bit_width == 0)
                        .filter_map(|f| {
                            let at = usize::from(f.offset);
                            let b = r.get(at..at + 4)?;
                            let v = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                            names.get(v).filter(|_| v > 0xffff).map(str::to_owned)
                        })
                        .collect::<Vec<_>>()
                        .join("/")
                })
                .collect::<Vec<_>>()
                .join(",")
        });
        let n_seats = seats.get(&id).map_or(0, Vec::len);
        let mut hijack = Vec::new();
        let n_entrances = entrances.get(&id).map_or(0, Vec::len);
        if let (Some(ec), Some(pc)) = (entrance_c, params_c) {
            for r in entrances.get(&id).into_iter().flatten() {
                let Some(t) = field_u32(ec, r, LINK_FIELD) else {
                    continue;
                };
                for pr in params.get(&t).into_iter().flatten() {
                    if let Some(s) = field_u32(pc, pr, HIJACK_SCRIPT).filter(|&s| s != 0) {
                        hijack.push(format!(
                            "{}{}",
                            show(s),
                            if scripts.contains(&s) {
                                ""
                            } else {
                                " (no script)"
                            }
                        ));
                    }
                }
            }
        }
        let weapons = link_c.map_or(String::new(), |c| {
            links
                .get(&id)
                .into_iter()
                .flatten()
                .filter_map(|r| field_u32(c, r, LINK_FIELD).filter(|&v| v != 0))
                .map(&show)
                .collect::<Vec<_>>()
                .join(", ")
        });
        let engine = sound_c
            .and_then(|c| {
                sound
                    .get(&id)?
                    .iter()
                    .find_map(|r| field_u32(c, r, BANK_FIELD).filter(|&v| v != 0))
            })
            .map_or("-".to_owned(), &show);
        // What the alpha does with it: cars and tanks drive, boats float
        // when a mission places them, helicopters stand as models.
        let alpha = match (kind, in_archive) {
            (_, false) => "nothing (no model)",
            ("_HelicopterPhysics", _) => "stands (not flown)",
            ("_BoatPhysics", _) => "floats (mission boats drive)",
            (_, _) if wheel == "none" => "stands (no wheels found)",
            _ => "drives",
        };
        let mut other: Vec<&str> = comps.get(&id).cloned().unwrap_or_default();
        other.sort_unstable();
        other.dedup();
        other.retain(|c| {
            !matches!(
                *c,
                "Name"
                    | "ModelName"
                    | "Health"
                    | "SoundEffect"
                    | "EntranceLink"
                    | "PhysicalLink"
                    | "SeatLink"
                    | "Turret"
                    | "ModifierKey"
            ) && !VEHICLE_PHYSICS.contains(c)
        });
        let k = by_kind.entry(kind).or_default();
        k[0] += 1;
        k[1] += usize::from(in_archive);
        k[2] += usize::from(stat + state > 0);
        k[3] += usize::from(alpha == "drives" || alpha.starts_with("floats"));
        println!(
            "fleet\t{id:08x}\t{name}\t{}\t{}\t{}\t{}\t{wheel}\t{stat}\t{state}\t{hp}\t{modkey}\t{n_seats} {seat_names}\t{n_entrances}\t{}\t{}\t{weapons}\t{engine}\t{alpha}\t{}",
            kind.trim_start_matches('_'),
            model.map_or("-".to_owned(), |m| format!("{m:08x}")),
            if in_archive { "yes" } else { "no" },
            entry.map_or("-".to_owned(), |e| format!("{} {}", e.kind, e.label)),
            hijack.join(","),
            turrets.get(&id).map_or(0, Vec::len),
            other.join(" ")
        );
    }
    println!("by physics: templates, with a model, placed, used by the alpha");
    for (k, [n, m, p, u]) in by_kind {
        println!("  {k}: {n} {m} {p} {u}");
    }
    // Catalogue models no vehicle template names.
    let named: HashSet<u32> = kind_of
        .keys()
        .filter_map(|id| model_of.get(id))
        .copied()
        .collect();
    let loose: Vec<String> = catalogue
        .vehicles
        .iter()
        .filter(|e| !named.contains(&e.model))
        .map(|e| format!("{:08x} {} {}", e.model, e.kind, e.label))
        .collect();
    println!(
        "catalogue vehicle models no vehicle template names: {}",
        loose.len()
    );
    for l in loose {
        println!("  {l}");
    }
    Ok(())
}

/// Every weapon template (one with a component whose name starts with
/// `Weapon`), or those whose name holds a word: first the weapon-side
/// components and their fields; then one tab-separated line per weapon
/// after an `arsenal\t` head (model and whether the archives hold it,
/// its weapon components' fields, its ammunition's fields and what that
/// spawns, placements, and which templates name it through which
/// component: heroes' and soldiers' `EquipmentLink`s, vehicles'
/// `PhysicalLink`s, pickups); then every weapon and every template it
/// names (two links deep) with all its fields, after `detail\t` heads.
fn arsenal<R: Read + Seek>(
    source: &mut AssetSource<R>,
    names: &Names,
    arg: &str,
) -> Result<(), Error> {
    let words = words_of(arg);
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
    let models: HashSet<u32> = index
        .assets()
        .iter()
        .filter(|a| index.type_hash(a) == MODEL_TYPE)
        .map(|a| a.name_hash)
        .collect();
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
    let tnames = db.names();
    let model_of = db.hashes("ModelName");
    let records: Vec<ComponentRecords<'_>> = db
        .components
        .iter()
        .filter(|c| c.name != "Name")
        .map(|c| (c.name, c.fields.clone(), by_entity(c)))
        .collect();
    let field_name = |h: u32| names.get(h).map_or(format!("{h:08x}"), str::to_owned);

    const SIDE: [&str; 20] = [
        "weapon",
        "ammo",
        "projectile",
        "explos",
        "grenade",
        "thrown",
        "melee",
        "pickup",
        "spawn",
        "damage",
        "missile",
        "rocket",
        "bullet",
        "burst",
        "lock",
        "equip",
        "physicallink",
        "inventory",
        "fire",
        "controller",
    ];
    println!("weapon-side components: name, records, fields (name or hash, type, bits)");
    for c in &db.components {
        let n = c.name.to_ascii_lowercase();
        if !SIDE.iter().any(|w| n.contains(w)) {
            continue;
        }
        let fields: Vec<String> = c
            .fields
            .iter()
            .map(|f| {
                let bits = if f.bit_width > 0 {
                    format!(":{}@{}", f.bit_width, f.bit_shift)
                } else {
                    String::new()
                };
                format!("{} t{}{bits}", field_name(f.name_hash), f.ty)
            })
            .collect();
        println!("  {} ({}): {}", c.name, c.count, fields.join(", "));
    }

    // A template's fields, per component, and the templates they name.
    let fields_of = |id: u32| -> Vec<(&str, String, Vec<u32>)> {
        let mut out = Vec::new();
        for (cname, fields, recs) in &records {
            for rec in recs.get(&id).into_iter().flatten() {
                let mut named = Vec::new();
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
                                named.push(v);
                                format!("{shown} -> {n:?}")
                            }
                            None => shown,
                        }
                    })
                    .collect();
                out.push((*cname, values.join(", "), named));
            }
        }
        out
    };
    // Who names whom, through which component.
    let mut named_by: HashMap<u32, Vec<(u32, &str)>> = HashMap::new();
    for (cname, fields, recs) in &records {
        for (&owner, rs) in recs {
            for rec in rs {
                for f in fields {
                    if f.ty == STRING_FIELD || f.bit_width != 0 {
                        continue;
                    }
                    let at = usize::from(f.offset);
                    let Some(b) = rec.get(at..at + 4) else {
                        continue;
                    };
                    let v = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                    if v != owner && tnames.contains_key(&v) {
                        named_by.entry(v).or_default().push((owner, *cname));
                    }
                }
            }
        }
    }
    let mut weapon_comps: HashMap<u32, Vec<&str>> = HashMap::new();
    for (cname, _, recs) in &records {
        if cname.to_ascii_lowercase().starts_with("weapon") {
            for &id in recs.keys() {
                weapon_comps.entry(id).or_default().push(cname);
            }
        }
    }
    // Placements by parent template, in static and state layers.
    let mut placed: HashMap<u32, [usize; 2]> = HashMap::new();
    for b in blocks {
        let decoded = source.block(b)?;
        let at = usize::from(!paths[b].contains("layers_static"));
        for (e, data) in BlockContents::parse(&decoded)?.iter() {
            if e.type_hash != LAYER_TYPE {
                continue;
            }
            let Ok(layer) = Layer::parse(data) else {
                continue;
            };
            for (_, p) in layer.placements() {
                placed.entry(p.parent).or_default()[at] += 1;
            }
        }
    }
    let show = |h: u32| -> String {
        tnames
            .get(&h)
            .map(|n| (*n).to_owned())
            .unwrap_or_else(|| format!("{h:08x}"))
    };
    let mut list: Vec<(&str, u32)> = weapon_comps
        .keys()
        .map(|&id| (tnames.get(&id).copied().unwrap_or(""), id))
        .filter(|(n, _)| {
            let n = n.to_ascii_lowercase();
            words.is_empty() || words.iter().any(|w| n.contains(w.as_str()))
        })
        .collect();
    list.sort_unstable_by_key(|&(n, id)| (n.to_ascii_lowercase(), id));
    println!("{} weapon templates", list.len());
    println!(
        "arsenal\tid\tname\tweapon components\tmodel\tin archive\tstatic\tstate\tweapon fields\tammo\tammo fields\tammo names\tnamed by"
    );
    let ammo_field = m2_assets::name_hash("AmmoTemplate");
    for &(name, id) in &list {
        let model = model_of.get(&id).copied().filter(|&m| m != 0);
        let in_archive = model.is_some_and(|m| models.contains(&m));
        let mine = fields_of(id);
        let mut comps = weapon_comps[&id].clone();
        comps.sort_unstable();
        comps.dedup();
        let wfields: Vec<String> = mine
            .iter()
            .filter(|(c, _, _)| c.to_ascii_lowercase().starts_with("weapon"))
            .map(|(c, v, _)| format!("{c}{{{v}}}"))
            .collect();
        // Its ammunition: the AmmoTemplate field of any weapon component.
        let ammo = records
            .iter()
            .filter(|(c, _, _)| c.to_ascii_lowercase().starts_with("weapon"))
            .find_map(|(_, fields, recs)| {
                let f = fields.iter().find(|f| f.name_hash == ammo_field)?;
                let rec = recs.get(&id)?.first()?;
                let at = usize::from(f.offset);
                let b = rec.get(at..at + 4)?;
                Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]])).filter(|&v| v != 0)
            });
        let (afields, anamed) = ammo.map_or((String::new(), String::new()), |a| {
            let f = fields_of(a);
            let names: Vec<String> = f
                .iter()
                .flat_map(|(c, _, n)| n.iter().map(move |&t| format!("{c}:{}", show(t))))
                .collect();
            (
                f.iter()
                    .map(|(c, v, _)| format!("{c}{{{v}}}"))
                    .collect::<Vec<_>>()
                    .join(" "),
                names.join(", "),
            )
        });
        let mut by: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for &(owner, c) in named_by.get(&id).into_iter().flatten() {
            by.entry(c).or_default().push(show(owner));
        }
        let by: Vec<String> = by
            .into_iter()
            .map(|(c, mut o)| {
                o.sort_unstable();
                o.dedup();
                let n = o.len();
                o.truncate(8);
                format!("{c} {n}: {}", o.join("; "))
            })
            .collect();
        let [stat, state] = placed.get(&id).copied().unwrap_or_default();
        println!(
            "arsenal\t{id:08x}\t{name}\t{}\t{}\t{}\t{stat}\t{state}\t{}\t{}\t{afields}\t{anamed}\t{}",
            comps.join(" "),
            model.map_or("-".to_owned(), |m| format!("{m:08x} {}", names.show(m))),
            if in_archive { "yes" } else { "no" },
            wfields.join(" "),
            ammo.map_or("-".to_owned(), &show),
            by.join(" | ")
        );
    }
    // Every weapon, and what it names two links deep, in full.
    let mut seen: HashSet<u32> = HashSet::new();
    let mut queue: Vec<(u32, u32)> = list.iter().map(|&(_, id)| (id, 0)).collect();
    queue.reverse();
    while let Some((id, depth)) = queue.pop() {
        if !seen.insert(id) {
            continue;
        }
        println!("detail\t{id:08x}\t{}\tdepth {depth}", show(id));
        for (c, v, named) in fields_of(id) {
            println!("  {c}: {v}");
            if depth < 2 {
                for t in named {
                    if !seen.contains(&t) {
                        queue.push((t, depth + 1));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Templates of the template database whose name holds one of the words,
/// each with every component it has and that component's fields. Text
/// fields are not read (a component with one is only listed).
fn components<R: Read + Seek>(
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
    println!("template database: {} components", db.components.len());
    for c in &db.components {
        let n = c.name.to_ascii_lowercase();
        if !words.is_empty() && !words.iter().any(|w| n.contains(w.as_str())) {
            continue;
        }
        let fields: Vec<String> = c
            .fields
            .iter()
            .map(|f| {
                let name = names
                    .get(f.name_hash)
                    .map_or(format!("{:08x}", f.name_hash), str::to_owned);
                format!("{name}(t{} @{} w{})", f.ty, f.offset, f.bit_width)
            })
            .collect();
        println!("  {} [{}]: {}", c.name, c.count, fields.join(", "));
    }
    Ok(())
}

/// What a placed template is to the player, for the `pickups` census.
fn pickup_kind(
    leavings: &m2_engine::pickup::Leavings,
    tnames: &HashMap<u32, &str>,
    t: u32,
) -> Option<&'static str> {
    use m2_engine::pickup::EQUIPMENT;
    let name = tnames.get(&t).copied().unwrap_or("");
    if let Some(p) = leavings.pickups.get(&t) {
        return Some(if name.contains("Cash") {
            "cash"
        } else if p.kind == EQUIPMENT {
            "weapon"
        } else if p.heals > 0 && p.gives.is_empty() {
            "health"
        } else {
            "ammo"
        });
    }
    let labels = leavings.labels.get(&t).map_or(&[][..], Vec::as_slice);
    if labels.contains(&m2_assets::name_hash("Emplacedweapon")) {
        return Some("emplaced");
    }
    // Whatever leaves a pickup when destroyed: a crate, labelled so or
    // not.
    let spawns = leavings.spawns.get(&t)?;
    spawns
        .iter()
        .any(|s| leavings.pickups.contains_key(s))
        .then_some("crate")
}

fn pickups<R: Read + Seek>(source: &mut AssetSource<R>, arg: &str) -> Result<(), Error> {
    let mut words: Vec<String> = arg
        .split(',')
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    if words.is_empty() {
        words = vec!["vzacon001".into(), "pmccon001".into()];
    }
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
    let tnames = db.names();
    let leavings = m2_engine::pickup::read(&db);
    // Every placed one: its layer, kind, template, entity and place.
    let mut rows: Vec<(String, &str, u32, u32, Vec3)> = Vec::new();
    // Each state layer's box.
    let mut boxes: HashMap<String, (Vec3, Vec3)> = HashMap::new();
    for b in blocks {
        let decoded = source.block(b)?;
        let layer = m2_engine::world::layer_name(&paths[b]);
        for (e, data) in BlockContents::parse(&decoded)?.iter() {
            if e.type_hash != LAYER_TYPE {
                continue;
            }
            let Ok(l) = Layer::parse(data) else {
                continue;
            };
            for (id, p) in l.placements() {
                let at = Vec3::from_array(p.position);
                if layer.contains("state") {
                    let b = boxes.entry(layer.clone()).or_insert((at, at));
                    *b = (b.0.min(at), b.1.max(at));
                }
                if let Some(kind) = pickup_kind(&leavings, &tnames, p.parent) {
                    rows.push((layer.clone(), kind, p.parent, id, at));
                }
            }
        }
    }
    const WIDER: f32 = 150.0;
    for w in &words {
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for (_, b) in boxes
            .iter()
            .filter(|(n, _)| n.replace('_', "").contains(w.as_str()))
        {
            lo = lo.min(b.0);
            hi = hi.max(b.1);
        }
        if lo.x > hi.x {
            println!("{w}: no state layer");
            continue;
        }
        let (lo, hi) = (lo - Vec3::splat(WIDER), hi + Vec3::splat(WIDER));
        // In the box, of the island's layers or the contract's own
        // (`vz_state_vza_con001_post` is VzaCon001's too), not another
        // contract's that shares the ground.
        let own =
            |layer: &str| !layer.contains("state") || layer.replace('_', "").contains(w.as_str());
        let inside: Vec<_> = rows
            .iter()
            .filter(|r| r.4.x >= lo.x && r.4.x <= hi.x && r.4.z >= lo.z && r.4.z <= hi.z)
            .filter(|r| own(&r.0))
            .collect();
        let mut tally: BTreeMap<&str, usize> = BTreeMap::new();
        for r in &inside {
            *tally.entry(r.1).or_default() += 1;
        }
        println!(
            "pickups\t{w}\tbox {:.0},{:.0}..{:.0},{:.0}\t{} placed\t{tally:?}",
            lo.x,
            lo.z,
            hi.x,
            hi.z,
            inside.len()
        );
        for (layer, kind, t, id, at) in inside {
            let leaves: Vec<&str> = leavings
                .spawns
                .get(t)
                .into_iter()
                .flatten()
                .map(|s| tnames.get(s).copied().unwrap_or("?"))
                .collect();
            println!(
                "placed\t{w}\t{kind}\t{t:08x}\t{}\t{layer}\t{id:#x}\t{:.1},{:.1},{:.1}\t{}",
                tnames.get(t).copied().unwrap_or("?"),
                at.x,
                at.y,
                at.z,
                leaves.join("; ")
            );
        }
    }
    Ok(())
}

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
        .map(|c| (c.name, c.fields.clone(), by_entity(c)))
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
            for rec in recs.get(&id).into_iter().flatten() {
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

/// Default words for [`ai_side`]: components whose name holds one.
const AI_WORDS: [&str; 13] = [
    "ai",
    "perception",
    "squad",
    "alarm",
    "target",
    "threat",
    "combat",
    "cover",
    "behav",
    "sense",
    "aware",
    "attack",
    "faction",
];

/// A layer component gathered over every layer: its fields, every record
/// (owned) and how many entities carry it.
type LayerGathered = (Vec<m2_assets::layer::Field>, Vec<Vec<u8>>, usize);

/// A component's schema, a line a field.
fn print_schema(names: &Names, fields: &[m2_assets::layer::Field]) {
    for f in fields {
        println!(
            "      field {:08x} {} t{} @{} bits {}<<{}",
            f.name_hash,
            names.show(f.name_hash),
            f.ty,
            f.offset,
            f.bit_width,
            f.bit_shift
        );
    }
}

/// Each field's values over `records`: how many distinct, and the most
/// common (eight at most) with their counts.
fn print_value_counts(names: &Names, fields: &[m2_assets::layer::Field], records: &[&[u8]]) {
    for f in fields.iter().filter(|f| f.ty != STRING_FIELD) {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for rec in records {
            let v = field_value(names, f, rec);
            let v = v.split_once('=').map_or(v.clone(), |(_, v)| v.to_owned());
            *counts.entry(v).or_default() += 1;
        }
        let mut counts: Vec<(String, usize)> = counts.into_iter().collect();
        counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let top: Vec<String> = counts
            .iter()
            .take(8)
            .map(|(v, n)| format!("{v} x{n}"))
            .collect();
        println!(
            "      {:08x} {}: {} distinct: {}",
            f.name_hash,
            names.show(f.name_hash),
            counts.len(),
            top.join(" | ")
        );
    }
}

/// The printable runs of `data` (4 bytes or more), distinct, at most 40.
fn print_texts(data: &[u8]) {
    let mut seen: Vec<String> = Vec::new();
    for run in data.split(|b| !(0x20..0x7f).contains(b)) {
        if run.len() >= 4 {
            let t = String::from_utf8_lossy(run).into_owned();
            if !seen.contains(&t) {
                seen.push(t);
            }
        }
    }
    let more = seen.len().saturating_sub(40);
    seen.truncate(40);
    println!(
        "      texts: {:?}{}",
        seen,
        if more > 0 {
            format!(" and {more} more")
        } else {
            String::new()
        }
    );
}

/// The AI side of the template database and the layers (see the
/// `ai` section above).
fn ai_side<R: Read + Seek>(
    source: &mut AssetSource<R>,
    names: &Names,
    arg: &str,
) -> Result<(), Error> {
    let mut words = words_of(arg);
    if words.is_empty() {
        words = AI_WORDS.iter().map(|w| (*w).to_owned()).collect();
    }
    let wanted = |name: &str| {
        let l = name.to_ascii_lowercase();
        words.iter().any(|w| l.contains(w.as_str()))
    };
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
    let mut blocks: Vec<usize> = index
        .assets()
        .iter()
        .filter(|a| index.type_hash(a) == LAYER_TYPE)
        .map(|a| usize::from(a.block))
        .collect();
    let data = source.asset(hash, TEMPLATE_DB_TYPE)?;
    let db = Layer::parse(&data)?;
    let tnames = db.names();
    let all: Vec<String> = db
        .components
        .iter()
        .map(|c| {
            let n = c.records().map_or(0, |r| r.len());
            format!("{}({n})", c.name)
        })
        .collect();
    println!("template database components: {}", all.join(" "));
    for c in db.components.iter().filter(|c| wanted(c.name)) {
        println!(
            "  db component {} {:08x}: {} groups, record {} bytes",
            c.name, c.hash, c.count, c.record_size
        );
        print_schema(names, &c.fields);
        let Some(recs) = c.records() else {
            println!("      (records not read: text fields)");
            print_texts(c.data);
            continue;
        };
        let only: Vec<&[u8]> = recs.iter().map(|r| r.1).collect();
        print_value_counts(names, &c.fields, &only);
        // Distinct records, with the templates sharing each.
        let mut distinct: Vec<(&[u8], Vec<u32>)> = Vec::new();
        for (id, rec) in &recs {
            match distinct.iter_mut().find(|d| d.0 == *rec) {
                Some(d) => d.1.push(*id),
                None => distinct.push((rec, vec![*id])),
            }
        }
        distinct.sort_by_key(|d| std::cmp::Reverse(d.1.len()));
        println!("      {} distinct records", distinct.len());
        for (rec, ids) in distinct.iter().take(30) {
            let who: Vec<String> = ids
                .iter()
                .take(4)
                .map(|id| format!("{id:08x} {:?}", tnames.get(id).copied().unwrap_or("")))
                .collect();
            let values: Vec<String> = c
                .fields
                .iter()
                .map(|f| field_value(names, f, rec))
                .collect();
            println!(
                "      x{} [{}]: {}",
                ids.len(),
                who.join(", "),
                values.join(", ")
            );
        }
    }
    // The same components across every layer.
    blocks.sort_unstable();
    blocks.dedup();
    let mut seen = HashSet::new();
    let mut gathered: BTreeMap<String, LayerGathered> = BTreeMap::new();
    for b in blocks {
        let decoded = source.block(b)?;
        for (e, data) in BlockContents::parse(&decoded)?.iter() {
            if e.type_hash != LAYER_TYPE || !seen.insert(e.name_hash) {
                continue;
            }
            let Ok(layer) = Layer::parse(data) else {
                continue;
            };
            for c in layer.components.iter().filter(|c| wanted(c.name)) {
                let entry = gathered
                    .entry(c.name.to_owned())
                    .or_insert_with(|| (c.fields.clone(), Vec::new(), 0));
                entry.2 += c.count as usize;
                if entry.0 != c.fields {
                    continue;
                }
                for (_, rec) in c.records().unwrap_or_default() {
                    entry.1.push(rec.to_vec());
                }
            }
        }
    }
    for (name, (fields, recs, count)) in &gathered {
        println!(
            "  layer component {name}: {count} entities, {} read",
            recs.len()
        );
        print_schema(names, fields);
        let only: Vec<&[u8]> = recs.iter().map(Vec::as_slice).collect();
        print_value_counts(names, fields, &only);
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
    layer_entities_with(source, names, most, None, keep, order)
}

/// [`layer_entities`], keeping only entities with a record of the named
/// component, when one is named.
fn layer_entities_with<R: Read + Seek>(
    source: &mut AssetSource<R>,
    names: &Names,
    most: usize,
    having: Option<&str>,
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
            let with: Option<std::collections::HashSet<u32>> = having.map(|h| {
                layer
                    .components
                    .iter()
                    .filter(|c| c.name.eq_ignore_ascii_case(h))
                    .flat_map(|c| c.records().unwrap_or_default())
                    .map(|(id, _)| id)
                    .collect()
            });
            let mut near: Vec<(f32, u32)> = placements
                .iter()
                .filter(|(id, p)| {
                    if with.as_ref().is_some_and(|w| !w.contains(id)) {
                        return false;
                    }
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
                .map(|c| (c.name, c.fields.clone(), by_entity(c)))
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
                    for rec in recs.get(&id).into_iter().flatten() {
                        let values: Vec<String> =
                            fields.iter().map(|f| field_value(names, f, rec)).collect();
                        println!("    {cname}: {}", values.join(", "));
                    }
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

/// Words the fields, states and verbs of breaking things may be named
/// of, alone, in pairs and with `Init…State`: the names are only matched
/// against hashes the data holds.
const DESTRUCT_WORDS: &[&str] = &[
    "Health",
    "Max",
    "Hit",
    "Points",
    "Damage",
    "Threshold",
    "Armor",
    "Armour",
    "Destroy",
    "Destroyed",
    "Destruction",
    "Debris",
    "Template",
    "Collapse",
    "Anim",
    "Sink",
    "Speed",
    "Accel",
    "Lean",
    "Angle",
    "Time",
    "Delay",
    "Radius",
    "Force",
    "Mass",
    "Effect",
    "Sound",
    "Key",
    "Explosion",
    "Fire",
    "Smoke",
    "Emitter",
    "Spawn",
    "State",
    "Msg",
    "Init",
    "Pristine",
    "Intact",
    "Damaged",
    "Wreck",
    "Wrecked",
    "Broken",
    "Break",
    "Chunk",
    "Piece",
    "Pieces",
    "Count",
    "Min",
    "Scale",
    "Shake",
    "Camera",
    "Object",
    "Create",
    "Root",
    "Node",
    "Start",
    "Stop",
    "Set",
    "Show",
    "Hide",
    "Play",
    "Fall",
    "Fallen",
    "Topple",
    "Crumble",
    "Burn",
    "Burning",
    "Dead",
    "Death",
    "Level",
    "Stage",
    "Type",
    "Class",
    "Material",
    "Resist",
    "Resistance",
    "Multiplier",
    "Vehicle",
    "Bullet",
    "Explosive",
    "Impact",
    "Building",
    "Prop",
    "Light",
    "Heavy",
    "Small",
    "Large",
    "Value",
    "Reward",
    "Cash",
    "Bounty",
    "Faction",
    "Physics",
    "Physical",
    "Enable",
    "Disable",
    "Model",
    "Name",
    "Partial",
    "Half",
    "Full",
    "Ruin",
    "Ruined",
    "Rubble",
    "Dust",
    "Decal",
    "Height",
    "Offset",
    "Ground",
    "Under",
    "Lifetime",
    "Life",
    "Fade",
    "Out",
    "Respawn",
    "Regen",
    "Invulnerable",
    "Indestructible",
    "Destructible",
    "Breakable",
    "Flags",
    "Ratio",
    "Percent",
    "Total",
    "Collapsed",
    "Destroying",
    "Collapsing",
    "Sinking",
    "Falling",
    "Dying",
    "Alive",
    "Normal",
    "Default",
    "Idle",
    "On",
    "Off",
    "Enter",
    "Exit",
    "Send",
    "Post",
    "Event",
    "Message",
    "Attach",
    "Detach",
    "Apply",
    "Kill",
    "Explode",
    "Hp",
    "Strength",
    "Tough",
    "Weak",
];

/// How the data describes what breaks; see the module docs.
/// The names known plus those the data's destruction words make.
fn destruct_names(names: &Names) -> Names {
    let mut names = Names {
        map: names.map.clone(),
    };
    for &a in DESTRUCT_WORDS {
        names.add(a);
        for p in ["f", "i", "b", "max", "min", "m_", "s", "n"] {
            names.add(&format!("{p}{a}"));
        }
        names.add(&format!("Init{a}State"));
        for &b in DESTRUCT_WORDS {
            for p in ["", "f", "i", "b", "max", "min"] {
                names.add(&format!("{p}{a}{b}"));
            }
            names.add(&format!("Init{a}{b}State"));
        }
    }
    for w in [
        "show",
        "hide",
        "SetState",
        "StartEmitter",
        "StopEmitter",
        "CreateObject",
        "SendMessage",
        "SendDamage",
        "CameraShake",
        "StartAnim",
        "StopAnim",
        "Kill",
        "Delay",
        "Wait",
        "Timer",
        "SetRootNode",
        "Sink",
        "SinkState",
        "Fade",
        "FadeOut",
        "Remove",
        "Translate",
        "Move",
        "PlaySound",
        "StartSound",
        "Collapse",
        "DestroyMsg",
        "OnEnter",
        "OnExit",
        "OnMessage",
        "OnTimer",
        "Time",
        "Duration",
    ] {
        names.add(w);
    }
    names
}

/// `stam:<hash>,..`: each model's state machine written out in full.
fn stam<R: Read + Seek>(
    source: &mut AssetSource<R>,
    names: &Names,
    arg: &str,
) -> Result<(), Error> {
    let names = destruct_names(names);
    let name = |h: u32| names.map.get(&h).cloned();
    for word in arg.split(',').filter(|w| !w.is_empty()) {
        let Ok(h) = u32::from_str_radix(word.trim_start_matches("0x"), 16) else {
            println!("{word}: not a hex hash");
            continue;
        };
        let Ok(data) = source.model(h) else {
            println!("{h:08x}: no such model");
            continue;
        };
        let Ok(m) = data.parse() else {
            println!("{h:08x}: doesn't parse");
            continue;
        };
        let height = |n: &m2_assets::model::Node| n.bounds_max[1] - n.bounds_min[1];
        println!(
            "model {h:08x} {}: {} nodes",
            name(h).unwrap_or_default(),
            m.nodes.len()
        );
        for (i, n) in m.nodes.iter().enumerate() {
            println!(
                "  node {i} {:08x} {} parent {:?} box {:.1?}..{:.1?} height {:.1}",
                n.name_hash,
                name(n.name_hash).unwrap_or_default(),
                n.parent,
                &n.bounds_min[..3],
                &n.bounds_max[..3],
                height(n)
            );
        }
        let node_of = |w: u32| m.nodes.iter().position(|n| n.name_hash == w);
        let operand = |w: u32| -> String {
            if let Some(i) = node_of(w) {
                return format!("node{i}({:.1} m)", height(&m.nodes[i]));
            }
            if let Some(n) = name(w) {
                return n;
            }
            let f = f32::from_bits(w);
            if w < 0x1_0000 {
                format!("{w}")
            } else if f.is_finite() && (1e-3..1e5).contains(&f.abs()) {
                format!("{f}f")
            } else {
                format!("{w:08x}")
            }
        };
        let Ok(u) = m2_assets::Ucfx::parse(&data.base) else {
            continue;
        };
        let secs = u.sections();
        let Some(at) = u.children(None).find(|&i| &secs[i].fourcc == b"STAM") else {
            println!("  no STAM");
            continue;
        };
        let last = (at + secs[at].descendants as usize).min(secs.len() - 1);
        for sec in &secs[at + 1..=last] {
            let tag = String::from_utf8_lossy(&sec.fourcc).into_owned();
            let Some(d) = sec.data else {
                println!("  {tag}");
                continue;
            };
            let w: Vec<u32> = d
                .chunks_exact(4)
                .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            match tag.as_str() {
                "CEXE" => {
                    let mut line = Vec::new();
                    let mut ops = Vec::new();
                    for pair in w.chunks_exact(2) {
                        match pair[0] {
                            1 => ops.push(operand(pair[1])),
                            2 => {
                                let verb = name(pair[1]).unwrap_or(format!("{:08x}", pair[1]));
                                line.push(format!("{verb}({})", ops.join(", ")));
                                ops.clear();
                            }
                            k => {
                                if !ops.is_empty() {
                                    line.push(format!("[{}]", ops.join(", ")));
                                    ops.clear();
                                }
                                line.push(format!("<{k}:{}>", operand(pair[1])));
                            }
                        }
                    }
                    if !ops.is_empty() {
                        line.push(format!("[{}]", ops.join(", ")));
                    }
                    if w.len() % 2 == 1 {
                        line.push(format!("+{}", operand(w[w.len() - 1])));
                    }
                    println!("      {}", line.join(" "));
                }
                _ => {
                    let ws: Vec<String> = w.iter().map(|&x| operand(x)).collect();
                    let indent = if tag == "STAT" {
                        "    "
                    } else if tag == "CHDR" {
                        "     "
                    } else {
                        "  "
                    };
                    println!("{indent}{tag} {}", ws.join(" "));
                }
            }
        }
    }
    Ok(())
}

fn destruct<R: Read + Seek>(
    source: &mut AssetSource<R>,
    world: &WorldIndex,
    names: &Names,
    arg: &str,
) -> Result<(), Error> {
    let names = &destruct_names(names);
    let words: Vec<String> = arg
        .split(',')
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    const KEYS: [&str; 30] = [
        "destr", "health", "damage", "debris", "collapse", "break", "wreck", "death", "dead",
        "die", "explo", "hit", "armo", "frag", "shatter", "sink", "burn", "fire", "smash", "crush",
        "topple", "build", "prop", "state", "effect", "emit", "smoke", "spawn", "kill", "rubble",
    ];
    let keyed = |n: &str| {
        let n = n.to_ascii_lowercase();
        KEYS.iter().any(|k| n.contains(k))
    };

    // The template database: each template's components.
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
    let model_hashes: HashSet<u32> = index
        .assets()
        .iter()
        .filter(|a| index.type_hash(a) == MODEL_TYPE)
        .map(|a| a.name_hash)
        .collect();
    let paths: Vec<String> = index.blocks().iter().map(|b| b.path.clone()).collect();
    let db_data = source.asset(db_hash, TEMPLATE_DB_TYPE)?;
    let db = Layer::parse(&db_data)?;
    let tnames: HashMap<u32, String> = db
        .names()
        .into_iter()
        .map(|(id, n)| (id, n.to_owned()))
        .collect();
    let mut by_lower: HashMap<String, u32> = HashMap::new();
    for (&id, n) in &tnames {
        let e = by_lower.entry(n.to_ascii_lowercase()).or_insert(id);
        *e = (*e).min(id);
    }
    let mut comps: HashMap<u32, Vec<&str>> = HashMap::new();
    for c in &db.components {
        for (id, _) in c.records().unwrap_or_default() {
            let v = comps.entry(id).or_default();
            if !v.contains(&c.name) {
                v.push(c.name);
            }
        }
    }
    let tmodels = db.hashes("ModelName");
    let skip = |id: u32| {
        comps.get(&id).is_some_and(|c| {
            c.iter().any(|n| {
                matches!(
                    *n,
                    "_CarPhysicsV2"
                        | "_TankPhysics"
                        | "_BoatPhysics"
                        | "_HelicopterPhysics"
                        | "_HumanPhysics"
                        | "Ai"
                        | "AiBehavior"
                )
            })
        })
    };

    // Every placed entity with a model.
    struct Row {
        is_static: bool,
        model: u32,
        stem: String,
        /// Its template and how it was found: 0 parent, 1 by name, 2 by
        /// model.
        template: Option<(u32, u8)>,
        own: Vec<String>,
    }
    let mut by_model: HashMap<u32, Vec<u32>> = HashMap::new();
    for (&t, &m) in &tmodels {
        by_model.entry(m).or_default().push(t);
    }
    let mut rows: Vec<Row> = Vec::new();
    let mut own_tally: BTreeMap<String, usize> = BTreeMap::new();
    let mut own_fields: HashMap<String, (Vec<m2_assets::layer::Field>, Vec<Vec<u8>>)> =
        HashMap::new();
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
            let entity_names = layer.names();
            let model_names = layer.hashes("ModelName");
            let mut own: HashMap<u32, Vec<String>> = HashMap::new();
            for c in &layer.components {
                if matches!(c.name, "Transform" | "Name") {
                    continue;
                }
                for (id, r) in c.records().unwrap_or_default() {
                    own.entry(id).or_default().push(c.name.to_owned());
                    *own_tally.entry(c.name.to_owned()).or_default() += 1;
                    if keyed(c.name) {
                        let slot = own_fields
                            .entry(c.name.to_owned())
                            .or_insert_with(|| (c.fields.clone(), Vec::new()));
                        if slot.1.len() < 4000 {
                            slot.1.push(r.to_vec());
                        }
                    }
                }
            }
            for (id, p) in layer.placements() {
                let name = entity_names.get(&id).copied().unwrap_or("");
                let stem = m2_engine::world::member_stem(name).to_ascii_lowercase();
                let template = if tnames.contains_key(&p.parent) {
                    Some((p.parent, 0))
                } else {
                    by_lower
                        .get(&stem)
                        .or_else(|| by_lower.get(&format!("_{stem}")))
                        .map(|&t| (t, 1))
                };
                let model = model_names
                    .get(&id)
                    .copied()
                    .or_else(|| {
                        let h = m2_assets::name_hash(&stem);
                        model_hashes.contains(&h).then_some(h)
                    })
                    .or_else(|| template.and_then(|(t, _)| tmodels.get(&t).copied()));
                let Some(model) = model else { continue };
                let template = template.or_else(|| {
                    by_model
                        .get(&model)
                        .and_then(|t| t.iter().min())
                        .map(|&t| (t, 2))
                });
                if template.is_some_and(|(t, _)| skip(t)) {
                    continue;
                }
                rows.push(Row {
                    is_static,
                    model,
                    stem,
                    template,
                    own: own.remove(&id).unwrap_or_default(),
                });
            }
        }
    }
    let how = |k: u8| {
        rows.iter()
            .filter(|r| r.template.map(|t| t.1) == Some(k))
            .count()
    };
    println!(
        "placed entities with a model (no vehicles, people): {} ({} static), template by parent {}, by name {}, by model {}, none {}",
        rows.len(),
        rows.iter().filter(|r| r.is_static).count(),
        how(0),
        how(1),
        how(2),
        rows.iter().filter(|r| r.template.is_none()).count()
    );

    // Components: of templates, and placed entities whose template has it.
    println!(
        "template database: {} templates, {} components; per component: records, templates, placed entities (distinct templates), * = named for damage",
        tnames.len(),
        db.components.len()
    );
    let mut placed_by: HashMap<&str, (usize, HashSet<u32>)> = HashMap::new();
    for r in &rows {
        if let Some((t, _)) = r.template {
            for c in comps.get(&t).into_iter().flatten() {
                let e = placed_by.entry(c).or_default();
                e.0 += 1;
                e.1.insert(t);
            }
        }
    }
    let mut line = Vec::new();
    for c in &db.components {
        let recs = c.records().unwrap_or_default();
        let templates: HashSet<u32> = recs.iter().map(|r| r.0).collect();
        let (n, ts) = placed_by.get(c.name).map_or((0, 0), |e| (e.0, e.1.len()));
        line.push(format!(
            "{}{} {}/{}/{}({})",
            if keyed(c.name) { "*" } else { "" },
            c.name,
            recs.len(),
            templates.len(),
            n,
            ts
        ));
        if line.len() == 4 {
            println!("  {}", line.join("; "));
            line.clear();
        }
    }
    if !line.is_empty() {
        println!("  {}", line.join("; "));
    }
    println!("layer entity components (all layers), count:");
    let own_line: Vec<String> = own_tally
        .iter()
        .map(|(n, k)| format!("{}{n} {k}", if keyed(n) { "*" } else { "" }))
        .collect();
    for chunk in own_line.chunks(6) {
        println!("  {}", chunk.join("; "));
    }

    // The damage-named components' fields, each with its commonest values.
    let summarise = |fields: &[m2_assets::layer::Field], recs: &[&[u8]]| {
        for f in fields {
            let mut seen: HashMap<String, usize> = HashMap::new();
            for rec in recs {
                let v = field_value(names, f, rec);
                let v = v.split_once('=').map_or(v.clone(), |(_, v)| v.to_owned());
                *seen.entry(v).or_default() += 1;
            }
            let mut top: Vec<(usize, String)> = seen.into_iter().map(|(v, n)| (n, v)).collect();
            top.sort_unstable_by(|a, b| b.cmp(a));
            let fname = names
                .get(f.name_hash)
                .map_or(format!("{:08x}", f.name_hash), str::to_owned);
            let shown: Vec<String> = top
                .iter()
                .take(5)
                .map(|(n, v)| format!("{v} x{n}"))
                .collect();
            println!(
                "      {fname} (t{}{}) {} values: {}",
                f.ty,
                if f.bit_width > 0 {
                    format!(" bits {}+{}", f.bit_shift, f.bit_width)
                } else {
                    String::new()
                },
                top.len(),
                shown.join(", ")
            );
        }
    };
    println!("template components named for damage, fields:");
    for c in db.components.iter().filter(|c| keyed(c.name)) {
        let recs = c.records().unwrap_or_default();
        let mut sample: Vec<&str> = recs
            .iter()
            .filter_map(|r| tnames.get(&r.0).map(String::as_str))
            .collect();
        sample.sort_unstable();
        sample.dedup();
        println!(
            "  {} ({} records) e.g. {:?}",
            c.name,
            recs.len(),
            &sample[..sample.len().min(6)]
        );
        let raw: Vec<&[u8]> = recs.iter().map(|r| r.1).collect();
        summarise(&c.fields, &raw);
    }
    println!("layer components named for damage, fields:");
    let mut own_keys: Vec<&String> = own_fields.keys().collect();
    own_keys.sort_unstable();
    for k in own_keys {
        let (fields, recs) = &own_fields[k];
        println!("  {k} ({} records read)", recs.len());
        let raw: Vec<&[u8]> = recs.iter().map(Vec::as_slice).collect();
        summarise(fields, &raw);
    }

    // The models: their state machines' states and verbs.
    #[derive(Default)]
    struct Seen {
        states: Vec<u32>,
        verbs: Vec<u32>,
        wrecked: bool,
        chunks: usize,
        loose: Option<bool>,
    }
    let stam_of = |data: &m2_engine::assets::ModelData| -> Option<(Vec<u32>, Vec<u32>)> {
        let u = m2_assets::Ucfx::parse(&data.base).ok()?;
        let secs = u.sections();
        let at = u.children(None).find(|&i| &secs[i].fourcc == b"STAM")?;
        let last = (at + secs[at].descendants as usize).min(secs.len() - 1);
        let (mut states, mut verbs) = (Vec::new(), Vec::new());
        for sec in &secs[at + 1..=last] {
            let Some(d) = sec.data else { continue };
            let w: Vec<u32> = d
                .chunks_exact(4)
                .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            match &sec.fourcc {
                b"STAT" => {
                    if let Some(&s) = w.first()
                        && !states.contains(&s)
                    {
                        states.push(s);
                    }
                }
                b"CEXE" => {
                    for pair in w.chunks_exact(2) {
                        if pair[0] == 2 && !verbs.contains(&pair[1]) {
                            verbs.push(pair[1]);
                        }
                    }
                }
                _ => {}
            }
        }
        Some((states, verbs))
    };
    let mut placements: HashMap<u32, usize> = HashMap::new();
    for r in &rows {
        *placements.entry(r.model).or_default() += 1;
    }
    for o in &world.objects {
        placements.entry(o.model).or_default();
    }
    let mut seen: HashMap<u32, Seen> = HashMap::new();
    for &h in placements.keys() {
        let Ok(data) = source.model(h) else { continue };
        let mut s = Seen::default();
        if let Some((states, verbs)) = stam_of(&data) {
            s.states = states;
            s.verbs = verbs;
        }
        if let Ok(m) = data.parse()
            && let Some(st) = &m.states
        {
            s.wrecked = st.machines.iter().any(|mm| {
                mm.states
                    .iter()
                    .any(|x| x.0 == m2_assets::model::STATE_WRECKED)
            });
            s.chunks = st.chunks(&m.nodes).len();
        }
        s.loose = m2_engine::CpuModel::from_data(&data, 0)
            .ok()
            .and_then(|c| c.collision)
            .and_then(|c| c.loose)
            .map(|l| l.breaks);
        seen.insert(h, s);
    }
    println!(
        "{} placed models read of {}; with STAM {}, a wrecked state {}, chunks {}, loose {} (breaking {})",
        seen.len(),
        placements.len(),
        seen.values().filter(|s| !s.states.is_empty()).count(),
        seen.values().filter(|s| s.wrecked).count(),
        seen.values().filter(|s| s.chunks > 0).count(),
        seen.values().filter(|s| s.loose.is_some()).count(),
        seen.values().filter(|s| s.loose == Some(true)).count()
    );
    let tally = |pick: &dyn Fn(&Seen) -> &Vec<u32>, what: &str| {
        let mut t: HashMap<u32, (usize, usize)> = HashMap::new();
        for (h, s) in &seen {
            for &x in pick(s) {
                let e = t.entry(x).or_default();
                e.0 += 1;
                e.1 += placements.get(h).copied().unwrap_or(0);
            }
        }
        let mut t: Vec<(usize, usize, u32)> = t.into_iter().map(|(x, (m, p))| (m, p, x)).collect();
        t.sort_unstable_by(|a, b| b.cmp(a));
        println!("{what}: {} distinct (models, placements)", t.len());
        for chunk in t.chunks(4).take(20) {
            let shown: Vec<String> = chunk
                .iter()
                .map(|(m, p, x)| format!("{x:08x} {} {m}/{p}", names.show(*x)))
                .collect();
            println!("  {}", shown.join("; "));
        }
    };
    tally(&|s| &s.states, "states");
    tally(&|s| &s.verbs, "verbs");

    // By name word.
    const CATS: [&str; 40] = [
        "bld",
        "house",
        "shack",
        "hut",
        "church",
        "tower",
        "sign",
        "billboard",
        "fence",
        "wall",
        "gate",
        "tree",
        "palm",
        "plant",
        "bush",
        "lamp",
        "pole",
        "light",
        "tank",
        "barrel",
        "crate",
        "bridge",
        "antenna",
        "rock",
        "pier",
        "dock",
        "tent",
        "statue",
        "shop",
        "station",
        "gas",
        "hangar",
        "bunker",
        "sandbag",
        "container",
        "car",
        "debris",
        "ruin",
        "silo",
        "radio",
    ];
    let mut model_stem: HashMap<u32, &str> = HashMap::new();
    for r in &rows {
        model_stem.entry(r.model).or_insert(r.stem.as_str());
    }
    let damage_template = |t: u32| comps.get(&t).is_some_and(|c| c.iter().any(|n| keyed(n)));
    println!(
        "by name word: placements, models; models with STAM / wrecked state / chunks / loose; placements whose template has a damage component, or own one"
    );
    for cat in CATS {
        let of: Vec<&Row> = rows.iter().filter(|r| r.stem.contains(cat)).collect();
        let models: HashSet<u32> = of.iter().map(|r| r.model).collect();
        let count = |f: &dyn Fn(&Seen) -> bool| {
            models.iter().filter(|h| seen.get(h).is_some_and(f)).count()
        };
        println!(
            "  {cat}: {} placed, {} models; {} / {} / {} / {}; {} / {}",
            of.len(),
            models.len(),
            count(&|s| !s.states.is_empty()),
            count(&|s| s.wrecked),
            count(&|s| s.chunks > 0),
            count(&|s| s.loose.is_some()),
            of.iter()
                .filter(|r| r.template.is_some_and(|(t, _)| damage_template(t)))
                .count(),
            of.iter().filter(|r| r.own.iter().any(|n| keyed(n))).count()
        );
    }

    // The models the words name, in full.
    if !words.is_empty() {
        let mut picked: Vec<(&str, u32)> = model_stem
            .iter()
            .filter(|(_, s)| words.iter().any(|w| s.contains(w.as_str())))
            .map(|(&h, &s)| (s, h))
            .collect();
        picked.sort_unstable();
        println!("{} models match", picked.len());
        for (stem, h) in picked.into_iter().take(60) {
            let Some(s) = seen.get(&h) else { continue };
            let r = rows.iter().find(|r| r.model == h);
            let template = r.and_then(|r| r.template);
            let show = |v: &[u32]| {
                v.iter()
                    .map(|x| format!("{x:08x} {}", names.show(*x)))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            println!(
                "  {h:08x} {stem} x{}: wrecked {}, chunks {}, loose {:?}; template {}",
                placements.get(&h).copied().unwrap_or(0),
                s.wrecked,
                s.chunks,
                s.loose,
                template.map_or("none".to_owned(), |(t, k)| format!(
                    "{t:08x} {:?} (by {}) [{}]",
                    tnames.get(&t).map_or("", String::as_str),
                    ["parent", "name", "model"][usize::from(k)],
                    comps.get(&t).map_or(String::new(), |c| c.join(" "))
                ))
            );
            println!("    states: {}", show(&s.states));
            println!("    verbs: {}", show(&s.verbs));
        }
    }
    Ok(())
}

/// Every particle effect: its name, emitters and their textures; then,
/// per place in the particle settings list, the name hash, kind and the
/// range of its values over all emitters, and the same for transforms
/// and forces.
fn effects<R: Read + Seek>(
    source: &mut AssetSource<R>,
    names: &Names,
    got: &Gathered,
    arg: &str,
) -> Result<(), Error> {
    let words: Vec<String> = arg
        .split(',')
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    use m2_assets::effect::{self, Attr, EFFECT_TYPE};
    let paths: Vec<String> = source
        .archive()
        .index()
        .blocks()
        .iter()
        .map(|b| b.path.clone())
        .collect();
    let mut all = Vec::new();
    for (b, p) in paths.iter().enumerate() {
        if !p.to_ascii_lowercase().contains("effects") {
            continue;
        }
        let decoded = source.block(b)?;
        let Ok(contents) = BlockContents::parse(&decoded) else {
            continue;
        };
        for (e, data) in contents.iter() {
            if e.type_hash != EFFECT_TYPE {
                continue;
            }
            match effect::parse(data) {
                Ok(fx) => all.push((e.name_hash, fx)),
                Err(err) => println!("  {:08x} {err}", e.name_hash),
            }
        }
    }
    let set: HashSet<u32> = all.iter().map(|(h, _)| *h).collect();
    let users = effect_users(source, names, &set)?;
    let (mut tex, mut tex_found) = (0, 0);
    for (h, fx) in &all {
        println!(
            "{:08x} {} emitters {} forces {} shapes {}",
            h,
            names.show(*h),
            fx.emitters.len(),
            fx.forces.len(),
            fx.shapes.len()
        );
        let used = users.get(h).cloned().unwrap_or_default();
        if !used.is_empty() {
            println!(
                "    used by {}: {}",
                used.len(),
                used.iter().take(4).cloned().collect::<Vec<_>>().join(" | ")
            );
        }
        let detail = !words.is_empty()
            && used.iter().any(|u| {
                words
                    .iter()
                    .any(|w| u.to_ascii_lowercase().contains(w.as_str()))
            });
        if detail {
            effect_detail(names, fx);
        }
        for e in &fx.emitters {
            for &t in &e.textures {
                tex += 1;
                if got.textures.contains_key(&t) {
                    tex_found += 1;
                }
            }
            let t: Vec<String> = e
                .textures
                .iter()
                .take(3)
                .map(|&t| texture_line(names, got, t))
                .collect();
            println!("    tex {}: {}", e.textures.len(), t.join(" | "));
        }
    }
    println!(
        "effects {} named {}",
        all.len(),
        all.iter().filter(|(h, _)| names.get(*h).is_some()).count()
    );
    println!("emitter textures {tex}, found in vz.wad {tex_found}");
    let stats = |label: &str, lists: Vec<&Vec<Attr>>| {
        // By name where named; keyed settings by place.
        let mut by: BTreeMap<String, (usize, BTreeMap<String, usize>)> = BTreeMap::new();
        for list in lists {
            for (i, a) in list.iter().enumerate() {
                let key = match a.name {
                    Some(n) => format!("{n:08x} {}", names.show(n)),
                    None => format!("keyed@{i:02}"),
                };
                let v = if a.name.is_none() {
                    format!("{:?}", a.keys)
                } else if a.kind == 1 {
                    format!("k{} f{:x} {}", a.kind, a.flags, a.float())
                } else {
                    format!("k{} f{:x} {:#x}", a.kind, a.flags, a.int())
                };
                let s = by.entry(key).or_default();
                s.0 += 1;
                *s.1.entry(v).or_default() += 1;
            }
        }
        println!("-- {label}: {} settings", by.len());
        for (k, (n, vals)) in by {
            let mut v: Vec<_> = vals.into_iter().collect();
            v.sort_by_key(|e| std::cmp::Reverse(e.1));
            let top: Vec<String> = v.iter().take(8).map(|(s, c)| format!("{s} x{c}")).collect();
            println!("  {k} n{n} distinct {}: {}", v.len(), top.join("; "));
        }
    };
    stats(
        "particle",
        all.iter()
            .flat_map(|(_, f)| f.emitters.iter().map(|e| &e.particle))
            .collect(),
    );
    stats(
        "transform",
        all.iter()
            .flat_map(|(_, f)| f.emitters.iter().map(|e| &e.transform))
            .collect(),
    );
    stats(
        "force",
        all.iter().flat_map(|(_, f)| f.forces.iter()).collect(),
    );
    Ok(())
}

/// Which template fields name effects: per (component, field) how many,
/// then each effect's users (template, component, field).
fn effect_users<R: Read + Seek>(
    source: &mut AssetSource<R>,
    names: &Names,
    effects: &HashSet<u32>,
) -> Result<HashMap<u32, Vec<String>>, Error> {
    let index = source.archive().index();
    let Some(db_hash) = index
        .assets()
        .iter()
        .find(|a| index.type_hash(a) == TEMPLATE_DB_TYPE)
        .map(|a| a.name_hash)
    else {
        return Ok(HashMap::new());
    };
    let db_data = source.asset(db_hash, TEMPLATE_DB_TYPE)?;
    let db = Layer::parse(&db_data)?;
    let tnames: HashMap<u32, String> = db
        .names()
        .into_iter()
        .map(|(id, n)| (id, n.to_owned()))
        .collect();
    let mut fields: BTreeMap<String, usize> = BTreeMap::new();
    let mut users: HashMap<u32, Vec<String>> = HashMap::new();
    for c in &db.components {
        let Some(records) = c.records() else {
            continue;
        };
        for (id, r) in records {
            for f in &c.fields {
                let at = usize::from(f.offset);
                let Some(b) = r.get(at..at + 4) else {
                    continue;
                };
                let v = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                if effects.contains(&v) {
                    let field = format!("{}.{}", c.name, names.show(f.name_hash));
                    *fields.entry(field.clone()).or_default() += 1;
                    let t = tnames.get(&id).map_or("?", String::as_str);
                    users.entry(v).or_default().push(format!("{t} {field}"));
                }
            }
        }
    }
    println!("-- template fields naming effects");
    for (f, n) in fields {
        println!("  {f} x{n}");
    }
    Ok(users)
}

/// Setting names solved from their hashes (`sizevar` and the rest).
const EFFECT_SETTING_NAMES: &[&str] = &[
    "sizevar",
    "speedvar",
    "spreadvar",
    "massvar",
    "posy",
    "rotx",
    "roty",
    "rotz",
    "life",
    "lifevar",
    "rate",
    "ratevar",
    "emissiondir",
    "forcegravity",
    "forcedrag",
    "forcewind",
    "inheritvel",
    "decay",
    "forcegravityvar",
    "forcedragvar",
    "forcewindvar",
    "inheritvelvar",
    "scale",
    "radial",
];

fn setting_name(names: &Names, h: u32) -> String {
    EFFECT_SETTING_NAMES
        .iter()
        .find(|n| hash_parts(&[n]) == h)
        .map_or_else(
            || names.get(h).map_or(format!("{h:08x}"), str::to_owned),
            |n| (*n).to_owned(),
        )
}

/// One effect's emitters in full: settings, colour keys, textures.
fn effect_detail(names: &Names, fx: &m2_assets::effect::Effect) {
    let show = |list: &[m2_assets::effect::Attr]| {
        list.iter()
            .enumerate()
            .map(|(i, a)| match a.name {
                Some(n) if a.kind & 1 == 1 => format!(
                    "{}={}{}",
                    setting_name(names, n),
                    a.float(),
                    if a.flags != 0 {
                        format!("/f{}", a.flags)
                    } else {
                        String::new()
                    }
                ),
                Some(n) => format!(
                    "{}=#{:x}{}",
                    setting_name(names, n),
                    a.int(),
                    if a.kind != 0 {
                        format!("/k{:x}", a.kind)
                    } else {
                        String::new()
                    }
                ),
                None => format!("@{i}={:?}", a.keys),
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    for (i, e) in fx.emitters.iter().enumerate() {
        println!(
            "    emitter {i} shape {:02x?} tex {} first {:08x?}",
            e.shape_ref,
            e.textures.len(),
            e.textures.first()
        );
        println!("      trfm {}", show(&e.transform));
        println!("      ptyp {}", show(&e.particle));
        let keys: Vec<String> = [0usize, 10, 25, 50, 75, 99]
            .iter()
            .filter_map(|&k| e.colour.get(k))
            .map(|c| format!("{:?} {:.2} {:.2}", c.rgba(), c.intensity, c.unknown_6))
            .collect();
        println!("      colr {}", keys.join(" | "));
    }
    for f in &fx.forces {
        println!("    force {}", show(f));
    }
}
