//! Prints the characters' stance and action table
//! ([`m2_assets::animation_table::ACTIONS`]) of an archive from your own
//! installation, with every name hash the archive's scripts (or a short
//! list of guesses) name spelled out, and for each animation handle the
//! rows of the on-foot table that give it clips (and the first six of
//! the vehicle table).
//!
//! ```text
//! cargo run --release -p m2-assets --example actions -- <vz.wad> [stance|-] [names.txt] > actions.txt
//! ```
//!
//! With a stance (`Upright`, `InVehicle`, ...) only that stance's rows
//! are printed; with a second path, every candidate name is written
//! there, one per line. Prints names, hashes and counts only.

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::BufReader;
use std::process::ExitCode;

use m2_assets::animation_table::{self as table, ANY, AnimationTable, ColumnKind};
use m2_assets::lua::{Chunk, SCRIPT_TYPE};
use m2_assets::{BlockContents, Ucfx, WadArchive, name_hash};

/// Words the tables are likely keyed by that no script need name.
const GUESSES: &[&str] = &[
    "Upright",
    "InVehicle",
    "Swim",
    "Swimming",
    "Crouch",
    "Crouched",
    "Prone",
    "Ragdoll",
    "Idle",
    "Any",
    "None",
    "Default",
    "Driver",
    "Passenger",
    "Gunner",
    "rifle",
    "pistol",
    "rocket",
    "grenade",
    "golfclub",
    "box",
    "shotgun",
    "smg",
    "machinepistol",
    "launcher",
    "melee",
    "c4",
    "Hero",
    "Male",
    "Female",
    "mattias",
    "Jennifer",
    "Chris",
    // Found by joining up to three words of the scripts' names (round
    // 13); each matches a table hash, and reads as what it keys.
    "EquipLeftIdle",
    "EquipRightIdle",
    "EquipLeftRun",
    "EquipRightRun",
    "StowLeftIdle",
    "StowRightIdle",
    "StowLeftRun",
    "StowRightRun",
    "PickupWeapon",
    "EnterVehicle",
    "ExitVehicle",
    "EnterSwim",
    "ExitSwim",
    "Dive",
    "DiveLeft",
    "DiveRight",
    "DiveLeftProne",
    "DiveRightProne",
    "ReloadIdle",
    "ReloadRun",
    "ThrowGrenade",
    "ThrowGrenadeIdle",
    "ThrowGrenadeRun",
    "MeleeBash",
    "MeleeBashLow",
    "MeleeImpact",
    "MeleeGetup",
    "FireWeapon",
    "EnterHijack",
    "ExitHijack",
    "Fall",
    "FallImpactLight",
    "FallImpactHeavy",
    "FallImpactDeath",
    "Walk",
    "RunStop",
    "Die",
    "ClimbUp",
    "ClimbDown",
    "MountLadder",
    "DismountLadder",
    "GrabLadder",
    "OnLadder",
    "InAir",
    "Carrying",
    "CrouchCover",
    "UprightCoverLeft",
    "UprightCoverRight",
    "EnterCoverLeft",
    "EnterCoverRight",
    "ExitCover",
    "ExitCoverLeft",
    "ExitCoverRight",
    "VaultForwardLow",
    "VaultBackwardLow",
    "CallSupport",
    "CallSupportIdle",
    "CallSupportEnd",
    "SpeakGesture",
    "Fidget",
    "PrisonerIdle",
    "StaggerInPlace",
    "StaggerLong",
    "GrabGrenade",
    "AiGrabGrenade",
    "AiThrowGrenade",
    "DriveBy",
    "EnterSniper",
    "ExitSniper",
    "UpperBody",
    "FullBody",
    "Master",
];

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: actions <vz.wad> [stance]");
        return ExitCode::FAILURE;
    };
    let only = args.next().filter(|s| s != "-");
    let dump = args.next();
    match run(&path, only, dump) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(
    path: &str,
    only: Option<String>,
    dump: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut wad = WadArchive::open(BufReader::new(File::open(path)?))?;
    let mut names: HashMap<u32, String> = HashMap::new();
    let add = |s: &str, names: &mut HashMap<u32, String>| {
        if !s.is_empty() && s.len() < 120 {
            names.entry(name_hash(s)).or_insert_with(|| s.to_owned());
        }
    };
    for g in GUESSES {
        add(g, &mut names);
    }
    let mut tables: BTreeMap<u32, AnimationTable> = BTreeMap::new();
    for block in 0..wad.index().blocks().len() {
        let Ok(decoded) = wad.read_block(block) else {
            continue;
        };
        let Ok(contents) = BlockContents::parse(&decoded) else {
            continue;
        };
        for (entry, data) in contents.iter() {
            if let Some(n) = Ucfx::parse(data).ok().as_ref().and_then(|u| u.name()) {
                add(n, &mut names);
            }
            if entry.type_hash == SCRIPT_TYPE
                && let Ok(chunk) = Chunk::from_asset(data)
            {
                for s in chunk.main.strings() {
                    add(s, &mut names);
                    for part in s.split(['.', ':', '/', '\\', ' ']) {
                        add(part, &mut names);
                    }
                }
            }
            if entry.type_hash == table::ANIMATION_TABLE_TYPE
                && let Ok(t) = AnimationTable::parse(data)
            {
                tables.entry(entry.name_hash).or_insert(t);
            }
        }
    }
    let name = |h: u32| -> String {
        if h == ANY {
            "*".to_owned()
        } else {
            names.get(&h).cloned().unwrap_or_else(|| format!("{h:08x}"))
        }
    };
    println!("{} names, {} tables", names.len(), tables.len());
    if let Some(dump) = dump {
        let mut all: Vec<&str> = names.values().map(String::as_str).collect();
        all.sort_unstable();
        std::fs::write(dump, all.join("\n"))?;
    }
    for (h, t) in &tables {
        let cols: Vec<String> = t
            .columns
            .iter()
            .map(|c| format!("{}:{:?}", c.name, c.kind))
            .collect();
        println!("table {h:08x}: {} rows: {}", t.rows(), cols.join(", "));
    }
    let actions = tables.get(&table::ACTIONS).ok_or("no actions table")?;
    let chars = tables.get(&table::CHARACTER_ANIMATIONS);
    let cell = |t: &AnimationTable, row: usize, col: usize| -> String {
        let v = t.cell(row, col).unwrap_or(ANY);
        match t.columns[col].kind {
            ColumnKind::List => t
                .list(row, col)
                .iter()
                .map(|&x| name(x))
                .collect::<Vec<_>>()
                .join("|"),
            ColumnKind::Hash => name(v),
            _ => format!("{v}"),
        }
    };
    let stance_col = actions.column("Stance").ok_or("no Stance")?;
    let handles_col = actions.column("AnimationHandles");
    println!("\n== actions table ==");
    let vehicles = tables.get(&table::VEHICLE_ANIMATIONS);
    let mut per_vehicle_handle: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    if let Some(v) = vehicles
        && let Some(hc) = v.column(table::HANDLE)
    {
        for r in 1..v.rows() {
            per_vehicle_handle
                .entry(v.cell(r, hc).unwrap_or(ANY))
                .or_default()
                .push(r);
        }
    }
    let mut per_handle: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    if let Some(chars) = chars
        && let Some(hc) = chars.column(table::HANDLE)
    {
        for r in 1..chars.rows() {
            per_handle
                .entry(chars.cell(r, hc).unwrap_or(ANY))
                .or_default()
                .push(r);
        }
    }
    for row in 1..actions.rows() {
        if let Some(s) = &only
            && name(actions.cell(row, stance_col).unwrap_or(ANY)) != *s
        {
            continue;
        }
        let cells: Vec<String> = (0..actions.columns.len())
            .map(|c| format!("{}={}", actions.columns[c].name, cell(actions, row, c)))
            .collect();
        println!("A{row}: {}", cells.join(" "));
        let Some(hcol) = handles_col else { continue };
        let mut handles: Vec<u32> = actions.list(row, hcol).to_vec();
        if actions.columns[hcol].kind != ColumnKind::List {
            handles = actions.cell(row, hcol).into_iter().collect();
        }
        if let Some(v) = vehicles {
            for &h in &handles {
                let rows = per_vehicle_handle.get(&h).map_or(&[][..], Vec::as_slice);
                for &r in rows.iter().take(6) {
                    let cells: Vec<String> = (0..v.columns.len())
                        .filter(|&c| v.columns[c].name != table::HANDLE)
                        .map(|c| format!("{}={}", v.columns[c].name, cell(v, r, c)))
                        .collect();
                    println!("    {h:08x} V{r}: {}", cells.join(" "));
                }
                if rows.len() > 6 {
                    println!("    {h:08x} ... {} vehicle rows in all", rows.len());
                }
            }
        }
        let Some(chars) = chars else { continue };
        for h in handles {
            for &r in per_handle.get(&h).into_iter().flatten() {
                let cells: Vec<String> = (0..chars.columns.len())
                    .filter(|&c| chars.columns[c].name != table::HANDLE)
                    .map(|c| format!("{}={}", chars.columns[c].name, cell(chars, r, c)))
                    .collect();
                println!("    {h:08x} C{r}: {}", cells.join(" "));
            }
        }
    }
    Ok(())
}
