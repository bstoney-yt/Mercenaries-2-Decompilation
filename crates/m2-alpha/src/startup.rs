//! The layers the game starts with. `layers_static` is many layers in one
//! block, and the game's start-up scripts load only some of them by name
//! (`vz_mar_village`), with state layers of their own
//! (`vz_state_alljob002_02_staging`). Free roam places the island's
//! vehicles from that set, not from every static layer.
//!
//! The set is found by running the start-up scripts headless on a thread
//! of their own, as `--mission` does, and keeping only what they loaded.

use std::collections::HashSet;
use std::fs::File;
use std::io::BufReader;
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
    /// Starts the scripts reading `install` in the background.
    pub fn start(install: &Install) -> Self {
        let wad = install.data.join("vz.wad");
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let found = File::open(&wad)
                .map_err(|e| format!("{}: {e}", wad.display()))
                .and_then(|f| WadArchive::open(BufReader::new(f)).map_err(|e| e.to_string()))
                .and_then(|mut w| GameData::read(&mut w).map_err(|e| e.to_string()))
                .and_then(|data| run(data).map_err(|e| e.to_string()));
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

/// Runs the start-up scripts and keeps the layers they loaded.
fn run(data: GameData) -> Result<StartupLayers, m2_game::mission::MissionError> {
    let mut mission = Mission::new(data, "Mattias", [0.0; 3])?;
    mission.start();
    let mut t = 0.0;
    while t < SETTLE {
        mission.tick(STEP);
        t += STEP;
    }
    let names = mission.loaded_layers();
    let mut set = StartupLayers {
        hashes: names.iter().map(|n| name_hash(n)).collect(),
        count: names.len(),
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
