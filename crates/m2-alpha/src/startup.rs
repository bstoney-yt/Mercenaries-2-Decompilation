//! The layers the game starts with. `layers_static` is many layers in one
//! block, and the game's start-up scripts load only some of them by name
//! (`vz_mar_village`), with state layers of their own
//! (`vz_state_alljob002_02_staging`). Free roam takes the island's
//! vehicles, people, traffic, zones and state layers' props from that set,
//! not from every static layer.
//!
//! The set is found by running the start-up scripts headless on a thread
//! of their own, as `--mission` does, and keeping only what they loaded.
//! From samples: the start-up scripts load layers from the campaign state
//! they're given. With `--load` they are handed the save's own script data
//! (as `--mission` does) and load that campaign's layers; without a save
//! they start a new game and load act 1's.

use crate::hero::Hero;
use crate::save::SaveGame;
use std::collections::HashSet;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use m2_assets::{WadArchive, name_hash};
use m2_engine::Install;
use m2_engine::world::member_stem;
use m2_game::mission::{GameData, Mission};
use m2_math::glam::Vec3;

/// Seconds of script time the start-up gets to load its layers.
const SETTLE: f64 = 5.0;
/// Script steps, seconds.
const STEP: f64 = 0.1;

/// An entity of a layer the start-up loaded that is a block of its own.
#[derive(Clone, Debug)]
pub struct LayerEntity {
    pub entity: u32,
    /// The template (`M151 Softtop (VZ)`), without the entity's id.
    pub name: String,
    pub position: Vec3,
    pub yaw: f32,
    /// The layer it is in (`vz_state_alljob002_02_staging`).
    pub layer: String,
}

/// What the start-up loaded.
#[derive(Clone, Debug, Default)]
pub struct StartupLayers {
    /// Name hashes of every layer loaded, as `layers_static`'s layers are
    /// keyed.
    pub hashes: HashSet<u32>,
    /// How many layers, for the log.
    pub count: usize,
    /// Their names (lower case), sorted.
    pub names: Vec<String>,
    /// The entities of the loaded layers that are blocks of their own
    /// (those inside `layers_static` are in the world index already).
    pub entities: Vec<LayerEntity>,
}

/// Running the start-up, then its result.
#[derive(Debug)]
pub enum Startup {
    Loading(Receiver<Result<StartupLayers, String>>),
    Ready(StartupLayers),
    /// The scripts could not run: everything static stands in.
    Failed,
}

impl Startup {
    /// Starts the scripts reading `install` in the background, from
    /// `save`'s campaign when there is one (`--load`), else a new game.
    pub fn start(install: &Install, hero: Hero, save: Option<&SaveGame>) -> Self {
        let scripts = saved_scripts(save);
        let wad = install.data.join("vz.wad");
        if scripts.is_some() {
            eprintln!("start-up: carrying on from the save's campaign");
        }
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let found = find(&wad, hero, scripts);
            // The game may have quit meanwhile.
            let _ = tx.send(found);
        });
        Self::Loading(rx)
    }

    /// Checks whether the set has arrived; logs it when it does.
    pub fn poll(&mut self) {
        let Self::Loading(rx) = self else { return };
        match rx.try_recv() {
            Ok(Ok(set)) => {
                eprintln!(
                    "start-up layers: {} ({} entities in state layers)",
                    set.count,
                    set.entities.len()
                );
                let acts: Vec<&str> = set
                    .names
                    .iter()
                    .filter(|n| n.contains("_act"))
                    .map(String::as_str)
                    .collect();
                eprintln!("start-up act layers: {} {}", acts.len(), acts.join(" "));
                *self = Self::Ready(set);
            }
            Ok(Err(e)) => {
                eprintln!("start-up layers: {e}; using every static layer");
                *self = Self::Failed;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                eprintln!("start-up layers: the scripts stopped; using every static layer");
                *self = Self::Failed;
            }
        }
    }

    pub fn loading(&self) -> bool {
        matches!(self, Self::Loading(_))
    }
}

/// The scripts' own data in `save`, which the start-up carries on from;
/// `None` (a new game) without a save, or for one from before it was kept.
fn saved_scripts(save: Option<&SaveGame>) -> Option<String> {
    save.and_then(|s| s.scripts.clone())
}

/// Reads the scripts' data from `wad` (`vz.wad`) and runs the start-up.
fn find(wad: &Path, hero: Hero, scripts: Option<String>) -> Result<StartupLayers, String> {
    File::open(wad)
        .map_err(|e| format!("{}: {e}", wad.display()))
        .and_then(|f| WadArchive::open(BufReader::new(f)).map_err(|e| e.to_string()))
        .and_then(|mut w| GameData::read(&mut w).map_err(|e| e.to_string()))
        .and_then(|data| run(data, hero, scripts).map_err(|e| e.to_string()))
}

/// Runs the start-up scripts, from the save's script data `scripts` when
/// given, and keeps the layers they loaded.
fn run(
    data: GameData,
    hero: Hero,
    scripts: Option<String>,
) -> Result<StartupLayers, m2_game::mission::MissionError> {
    let mut mission = Mission::new(data, hero.name(), [0.0; 3])?;
    if let Some(text) = scripts {
        mission.load_save(text);
    }
    mission.start();
    let mut t = 0.0;
    while t < SETTLE {
        mission.tick(STEP);
        t += STEP;
    }
    let mut names = mission.loaded_layers();
    names.sort();
    let mut set = StartupLayers {
        hashes: names.iter().map(|n| name_hash(n)).collect(),
        count: names.len(),
        names: names.clone(),
        entities: Vec::new(),
    };
    for name in &names {
        let Some(entities) = mission.layer(name) else {
            continue;
        };
        // `layers_static` itself is the whole block; the index has it.
        if m2_game::layers::LayerState::is_static(name) {
            continue;
        }
        set.entities.extend(entities.iter().map(|e| LayerEntity {
            entity: e.guid,
            name: member_stem(&e.name).to_owned(),
            position: Vec3::from(e.at),
            yaw: e.yaw,
            layer: name.clone(),
        }));
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NO_SCRIPTS: &str = r#"
version = 1
killed = []
vehicles = []

[player]
position = [1.0, 2.0, 3.0]
facing = 0.0
yaw = 0.0
pitch = 0.0
hp = 100.0
weapon = "rpg"
"#;

    #[test]
    fn the_start_up_carries_on_from_the_saves_scripts() {
        assert_eq!(saved_scripts(None), None);
        let old = SaveGame::from_toml(NO_SCRIPTS).unwrap();
        // From before the scripts' data was kept: a new game.
        assert_eq!(saved_scripts(Some(&old)), None);
        let text = "{[\"nAvailableCostumes\"]=1,}".to_owned();
        let save = SaveGame {
            scripts: Some(text.clone()),
            ..old
        };
        assert_eq!(saved_scripts(Some(&save)), Some(text));
    }

    /// Needs the game: `M2_INSTALL` (the folder holding `data\vz.wad`,
    /// default the usual install) and `M2_ACT2_SAVE`, an m2-alpha save
    /// from act 2. `cargo test -p m2-alpha -- --ignored act2`.
    #[test]
    #[ignore = "needs the game's vz.wad and an act-2 save"]
    fn an_act2_save_starts_with_act2_layers() {
        let root = std::env::var("M2_INSTALL").unwrap_or_else(|_| {
            r"C:\Program Files (x86)\EA Games\Mercenaries 2 World in Flames".into()
        });
        let install = Install::find(Path::new(&root)).expect("M2_INSTALL is no install");
        let path = std::env::var("M2_ACT2_SAVE").expect("M2_ACT2_SAVE names an act-2 save");
        let save = SaveGame::read(Path::new(&path)).unwrap();
        let hero = save.hero.unwrap_or_default();
        let wad = install.data.join("vz.wad");
        let act2 = |set: &StartupLayers| set.names.iter().any(|n| n.contains("_act2"));
        let new_game = find(&wad, hero, None).unwrap();
        assert!(!act2(&new_game), "a new game loads act 1's layers");
        let loaded = find(&wad, hero, saved_scripts(Some(&save))).unwrap();
        assert!(
            act2(&loaded),
            "no _act2 layer among {} from {path}",
            loaded.count
        );
    }
}
