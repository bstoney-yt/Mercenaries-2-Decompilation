//! The game's own HUD: its layout and handler scripts (health, ammo and
//! weapon, reticle, hit arcs, cash and fuel, messages) run in a Lua state
//! of their own on a thread, as [`m2_game::gui_runtime`] runs them, and
//! what they build is drawn with the game's textures and bitmap fonts.
//!
//! Each frame the play's state goes to the thread ([`HudState`]); the
//! thread fires the game's GUI events for what changed, steps the scripts
//! and sends back the screen as textured quads, with any texture they
//! need for the first time decoded. Drawing lags play by a frame.
//!
//! The minimap is drawn here, as the engine draws its own: the island's
//! map round her (see [`MAP_TEXTURE`]), turned with the camera, on the
//! radar's backdrop, with the mission's markers. Not drawn yet: the Flash
//! pieces (the minimap's zones, pop-ups), animated sprites and clock
//! wipes. The ammo the counters show is our own count (see
//! [`RIFLE_CLIP`]): the play has no ammo of its own yet.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, BufWriter, Write as _};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError, channel};
use std::time::{Duration, Instant};

use egui::{Color32, Mesh, TextureHandle, TextureOptions, pos2};
use m2_assets::font::FONT_TYPE;
use m2_assets::{Font, WadArchive, name_hash};
use m2_engine::world::ISLAND;
use m2_engine::{AssetSource, Install};
use m2_game::api::Object;
use m2_game::events::guid_value;
use m2_game::gui::Anchor;
use m2_game::gui_runtime::{self, DrawItem, TextSpan};
use m2_game::hud::ObjectiveMessage;
use m2_game::mission::{GameData, Mission};
use m2_math::glam::Vec3;
use mlua::{IntoLuaMulti, Lua, Value};

use crate::combat::{Combat, Fired, Picked, Rounds, Weapon};
use crate::config::Config;
use crate::game::Game;

/// The fonts the HUD's text widgets use.
const FONTS: [&str; 2] = ["english_18", "english_20"];
/// The player's guid as the scripts know her (`Sys.StringToGuid("0x1")`).
const PLAYER: u32 = 1;
/// Guids standing for her guns (`uNewCurrentGunGuid`): objects of the
/// scripts' world, for the gun box shows `Object.GetLocalizedName` of it.
/// Past the guids the scripts' world gives out (layer entities, templates,
/// spawns, markers), so they name no other object.
const RIFLE_GUID: u32 = 0x6000_0000;
const RPG_GUID: u32 = 0x6000_0001;
const LAUNCHER_GUID: u32 = 0x6000_0002;
/// The guns' names, as the game's English strings write them. Her rifle
/// is the first contract's carbine (template 80007212), as the footage's
/// gun box reads.
const RIFLE_NAME: &str = "Carbine";
const RPG_NAME: &str = "RPG";
const LAUNCHER_NAME: &str = "Grenade Launcher";
/// The ammo the counters show. The play never runs out: the rifle's clip,
/// the rockets stored and the grenades count down and start again full;
/// the rifle's store stays as it is.
const RIFLE_CLIP: u32 = 30;
const RIFLE_STORED: u32 = 240;
const RPG_STORED: u32 = 4;
const GRENADES: u32 = 4;
/// The weapons' icons (`uNewCurrentGun`): textures of their own, as the
/// game names them (`weapons_*`).
const RIFLE_ICON: &str = "weapons_m4";
const RPG_ICON: &str = "weapons_rpg";
const LAUNCHER_ICON: &str = "weapons_grenade_launcher_2";
/// Their reticles (`uReticleTexture`), `reticle_*` by the kind of round.
const RIFLE_RETICLE: &str = "reticle_bullets";
const RPG_RETICLE: &str = "reticle_rockets";
const LAUNCHER_RETICLE: &str = "reticle_grenade";
/// Seconds an objective message stays in the message buffer, as
/// `DisplayObjectiveMessage` asks.
const MESSAGE_SECONDS: f64 = 5.0;
/// An objective message's words and priority from its kind, icon index and
/// text id, as `MrxGuiInterface.DisplayObjectiveMessage` builds them: the
/// kind's colour, the icon's tag, `[objective.<prefix><status>]`, the text.
/// Nothing for a kind the scripts don't know.
const WORD_MESSAGE: &str = r#"
local kind, icon, text = ...
local c = MrxGuiInterface.tObjectiveMessageConfigs[kind]
if not c then return nil end
local tag = MrxUtil.GetInlineIconNameByIndex(icon) or ""
return c.sColor .. tag .. " [objective." .. c.sPrefix .. c.sStatus .. "]" .. " " .. text,
    c.nPriority or 1
"#;
/// Seconds of play the scripts are given at most per frame.
const MAX_STEP: f64 = 0.25;
/// Largest texture size decoded, pixels.
const MAX_TEXTURE: u32 = 2048;

/// The minimap's art: the engine draws the HUD's "VZ" map itself; the
/// island-wide map the distant terrain wears stands in, the island
/// (`ISLAND` metres square, centred on the origin) seen from above with
/// its roads, `+x` and `+z` towards its left and top edges.
const MAP_TEXTURE: &str = "vz_lrterrain";
/// The radar's backdrop (its `sBorderTexture`), with the north pointer on
/// its rim, `BORDER` virtual units across round a minimap 100 across.
const BORDER_TEXTURE: &str = "global_gui_hud_radar";
const BORDER: f32 = 154.0 / 100.0;
/// Her marker in the middle of the minimap.
const ARROW_TEXTURE: &str = "hud_objective_arrows";
/// Metres from the minimap's centre to its rim when the scripts set none
/// (`MinimapDataUpdateHandler` sets 150 at a walk, up to 400 at speed).
const DEFAULT_RANGE: f32 = 150.0;

/// What the HUD is told each frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HudState {
    /// Screen size in points.
    pub size: [f32; 2],
    /// Seconds since the last state.
    pub dt: f64,
    /// Health, 0..=100: hers, or the vehicle's she is in.
    pub hp: f32,
    pub in_vehicle: bool,
    pub weapon: Weapon,
    pub fired: Fired,
    /// Rounds picked up (a supply crate's rockets).
    pub picked: Picked,
    /// The grenade launcher's rounds, while she carries one.
    pub launcher: Option<Rounds>,
    /// The label of the vehicle she is in.
    pub vehicle: Option<String>,
    /// Which way the camera looks, degrees, growing as it turns left
    /// (`Player.GetCameraXZHeading`).
    pub heading: f32,
    /// Hurts since the last state: damage and the bearing it came from
    /// (see [`direction`]).
    pub hurts: Vec<(f32, f32)>,
    /// Messages for the message buffer since the last state.
    pub messages: Vec<ObjectiveMessage>,
    /// Who is under the reticle, as `GuiReticleUpdate` takes it: -1 an
    /// enemy, 0 anyone else; and their health (of 100).
    pub target: Option<(i32, i32)>,
    /// Where she is (world), and which way she faces: degrees, growing
    /// turning left, as the heading.
    pub position: Option<[f32; 3]>,
    pub facing: f32,
    /// Objective markers for the minimap: where, icon (`deliverable`...),
    /// colour (RGBA).
    pub blips: Vec<([f32; 3], String, [u8; 4])>,
    /// Where the reticle goes: the screen point her weapon (or her
    /// vehicle's gun) aims at, -1..=1 across and up the screen as
    /// `Gui.GetReticlePosition` answers. None hides it (down, or in a
    /// vehicle without a gun).
    pub reticle: Option<[f32; 2]>,
}

impl HudState {
    /// What the play shows now (the screen size and time step are the
    /// HUD's to fill in). Takes her hurts from `game`.
    pub fn of(game: &mut Game) -> Self {
        let player = game.scene.player.as_ref();
        let vehicle = player
            .and_then(|p| p.driving)
            .and_then(|i| game.scene.vehicles.get(i));
        let at = player.map(|p| p.character.position);
        // On foot, the person a shot fired now would hit: soldiers who
        // have turned on her are enemies.
        let people = &game.scene.people;
        let target = player
            .filter(|p| p.driving.is_none())
            .zip(game.scene.world.as_ref())
            .and_then(|(p, w)| Combat::aim_probe(&w.physics, people, p))
            .and_then(|(_, person)| people.npcs.get(person?))
            .filter(|n| n.alive)
            .map(|n| {
                let enemy = n.soldier && n.alert;
                (-i32::from(enemy), n.hp.round().clamp(0.0, 100.0) as i32)
            });
        let hurts = game
            .combat
            .take_hurts()
            .into_iter()
            .map(|(damage, from)| (damage, direction(at, from)))
            .collect();
        Self {
            hp: match vehicle {
                Some(v) if v.wrecked => 0.0,
                Some(_) => 100.0,
                None => game.combat.hp,
            },
            in_vehicle: vehicle.is_some(),
            weapon: game.combat.weapon,
            fired: game.combat.fired,
            picked: game.combat.picked,
            launcher: game.combat.launcher_rounds,
            vehicle: vehicle.map(|v| v.spec.label.clone()),
            heading: game.camera.yaw.to_degrees(),
            target,
            position: at.map(|p| p.to_array()),
            facing: player.map_or(0.0, |p| p.facing.to_degrees()),
            hurts,
            ..Self::default()
        }
    }

    /// Folds a later state into this one: events add up, the rest is the
    /// later's.
    fn merge(&mut self, later: Self) {
        let dt = self.dt + later.dt;
        let mut hurts = std::mem::take(&mut self.hurts);
        let mut messages = std::mem::take(&mut self.messages);
        hurts.extend(later.hurts.iter().copied());
        messages.extend(later.messages.iter().cloned());
        *self = Self {
            dt,
            hurts,
            messages,
            ..later
        };
    }
}

/// The bearing of `from` seen from `at`, as `GuiPlayerReceiveDamage`
/// takes it: degrees that, added to the camera's heading
/// ([`HudState::heading`]), give how far clockwise on screen from straight
/// ahead the hit arc points (`MrxGuiHudDamageIndicator` turns the arc to
/// direction + heading each frame; its art points up unturned). With
/// headings growing as the camera turns left, the bearing grows the other
/// way: 0 towards +z, 90 towards -x. 0 when either is unknown.
fn direction(at: Option<Vec3>, from: Option<Vec3>) -> f32 {
    let (Some(at), Some(from)) = (at, from) else {
        return 0.0;
    };
    let d = from - at;
    if d.x.abs() + d.z.abs() < 1e-3 {
        return 0.0;
    }
    // Heading grows turning left: forward is (sin h, 0, cos h).
    (-d.x.atan2(d.z)).to_degrees().rem_euclid(360.0)
}

/// What the vehicle box shows for a vehicle label: its name, and its
/// faction as the game's guid for it (`SetVehicleName(name, faction)`
/// picks the faction's icon; none shows the PMC's). Labels start with the
/// faction (`"vz_veh_truck_m151"` is the VZ's `"truck m151"`).
fn vehicle_name(label: &str) -> (String, Option<u32>) {
    let (faction, name) = label.split_once("_veh_").unwrap_or(("", label));
    // `MrxGuiHudVehicleDisguise.Init`'s faction guids.
    let guid = match faction {
        "al" => Some(0xbbc3_4ef4),
        "ch" => Some(0x4135_9cce),
        "oc" => Some(0xe947_b797),
        "gr" => Some(0xb10d_73ce),
        "pr" => Some(0xc182_15fe),
        "pmc" => Some(0x30e4_a26f),
        "civ" => Some(0xdcc8_b14d),
        "vz" => Some(0xb442_0059),
        _ => None,
    };
    (name.replace('_', " "), guid)
}

/// A textured (or, without texture, plain) quad in points: corners
/// clockwise from the top left, their UVs, and a colour (RGBA 0..=1) that
/// multiplies the texture.
#[derive(Clone, Debug, PartialEq)]
pub struct Prim {
    pub texture: Option<u32>,
    pub corners: [[f32; 2]; 4],
    pub uv: [[f32; 2]; 4],
    pub colour: [f32; 4],
}

/// RGBA8 pixels.
struct Image {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// What the thread sends back.
enum Output {
    Frame {
        prims: Vec<Prim>,
        images: Vec<(u32, Option<Image>)>,
        log: Vec<String>,
    },
    Failed(String),
}

enum Status {
    Loading,
    Ready,
    Failed,
}

/// The game's HUD, on the main thread: sends the play's state and paints
/// what comes back.
pub struct GameHud {
    tx: Sender<HudState>,
    rx: Receiver<Output>,
    status: Status,
    prims: Vec<Prim>,
    /// Decoded and waiting for an egui context to upload them.
    pending: Vec<(u32, Option<Image>)>,
    /// Textures by asset hash (`None`: missing in the archive).
    textures: HashMap<u32, Option<TextureHandle>>,
    /// The last objective message seen, to tell new ones.
    last_message: Option<ObjectiveMessage>,
    /// The time of the last frame (egui's input time).
    last_time: Option<f64>,
    log: Option<BufWriter<File>>,
}

impl std::fmt::Debug for GameHud {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GameHud")
            .field("prims", &self.prims.len())
            .field("textures", &self.textures.len())
            .finish_non_exhaustive()
    }
}

impl GameHud {
    /// Starts the HUD's scripts reading `install` in the background.
    pub fn start(install: &Install) -> Self {
        let (tx, thread_rx) = channel();
        let (thread_tx, rx) = channel();
        let data = install.data.clone();
        std::thread::Builder::new()
            .name("game hud".into())
            .spawn(move || run(&data, thread_rx, thread_tx))
            .expect("could not start the HUD thread");
        let path = Config::file_beside("m2-alpha-hud.log");
        let log = File::create(&path).ok().map(BufWriter::new);
        Self {
            tx,
            rx,
            status: Status::Loading,
            prims: Vec::new(),
            pending: Vec::new(),
            textures: HashMap::new(),
            last_message: None,
            last_time: None,
            log,
        }
    }

    /// The scripts are running and have drawn.
    pub fn ready(&self) -> bool {
        matches!(self.status, Status::Ready)
    }

    /// The scripts could not start: the old HUD stands in.
    pub fn failed(&self) -> bool {
        matches!(self.status, Status::Failed)
    }

    /// Seconds since the last frame, from egui's input `time`.
    pub fn frame_time(&mut self, time: Option<f64>) -> f64 {
        let dt = match (time, self.last_time) {
            (Some(now), Some(then)) => (now - then).clamp(0.0, MAX_STEP),
            _ => 0.0,
        };
        self.last_time = time.or(self.last_time);
        dt
    }

    /// An objective message for the message buffer, if new.
    pub fn message(&mut self, message: Option<&ObjectiveMessage>) -> Option<ObjectiveMessage> {
        if message == self.last_message.as_ref() {
            return None;
        }
        self.last_message = message.cloned();
        message.cloned()
    }

    /// Sends this frame's state and takes in what the thread has drawn.
    pub fn update(&mut self, ctx: &egui::Context, state: HudState) {
        if self.failed() {
            return;
        }
        let _ = self.tx.send(state);
        loop {
            match self.rx.try_recv() {
                Ok(out) => self.take(out),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if !self.failed() {
                        self.fail("the HUD thread stopped".into());
                    }
                    break;
                }
            }
        }
        self.upload(ctx);
    }

    /// Like [`update`](Self::update), but waits (up to `timeout`) for the
    /// thread to draw this state: for screenshots.
    pub fn update_and_wait(&mut self, ctx: &egui::Context, state: HudState, timeout: Duration) {
        if self.failed() {
            return;
        }
        let _ = self.tx.send(state);
        let until = Instant::now() + timeout;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(out) => {
                    let frame = matches!(out, Output::Frame { .. });
                    self.take(out);
                    if frame || self.failed() {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    self.fail("the HUD thread stopped".into());
                    break;
                }
            }
        }
        self.upload(ctx);
    }

    fn take(&mut self, out: Output) {
        match out {
            Output::Frame { prims, images, log } => {
                self.status = Status::Ready;
                self.prims = prims;
                self.pending.extend(images);
                self.write(log);
            }
            Output::Failed(why) => self.fail(why),
        }
    }

    fn fail(&mut self, why: String) {
        eprintln!("game HUD: {why}");
        self.write([why]);
        self.status = Status::Failed;
    }

    fn write(&mut self, lines: impl IntoIterator<Item = String>) {
        if let Some(log) = &mut self.log {
            for line in lines {
                let _ = writeln!(log, "{line}");
            }
            let _ = log.flush();
        }
    }

    fn upload(&mut self, ctx: &egui::Context) {
        for (hash, image) in self.pending.drain(..) {
            let handle = image.map(|i| {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [i.width as usize, i.height as usize],
                    &i.rgba,
                );
                // Tiled widgets repeat their texture.
                ctx.load_texture(
                    format!("hud {hash:08x}"),
                    image,
                    TextureOptions::LINEAR_REPEAT,
                )
            });
            self.textures.insert(hash, handle);
        }
    }

    /// Paints the HUD as last drawn.
    pub fn paint(&self, painter: &egui::Painter) {
        let mut mesh = Mesh::default();
        for p in &self.prims {
            let texture = match p.texture {
                Some(hash) => match self.textures.get(&hash) {
                    Some(Some(t)) => t.id(),
                    // Missing, or not uploaded yet.
                    _ => continue,
                },
                None => egui::TextureId::default(),
            };
            if mesh.texture_id != texture && !mesh.is_empty() {
                painter.add(std::mem::take(&mut mesh));
            }
            mesh.texture_id = texture;
            let [r, g, b, a] = p.colour.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
            let colour = Color32::from_rgba_unmultiplied(r, g, b, a);
            let base = mesh.vertices.len() as u32;
            for (c, uv) in p.corners.iter().zip(&p.uv) {
                let uv = match p.texture {
                    Some(_) => pos2(uv[0], uv[1]),
                    // The font atlas's white texel.
                    None => egui::epaint::WHITE_UV,
                };
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: pos2(c[0], c[1]),
                    uv,
                    color: colour,
                });
            }
            mesh.indices.extend([0, 1, 2, 0, 2, 3].map(|i| base + i));
        }
        if !mesh.is_empty() {
            painter.add(mesh);
        }
    }
}

/// The thread: reads the scripts, fonts and strings, starts the HUD, then
/// draws each state sent until the sender goes.
fn run(data: &std::path::Path, rx: Receiver<HudState>, tx: Sender<Output>) {
    let mut hud = match Thread::start(data) {
        Ok(h) => h,
        Err(e) => {
            let _ = tx.send(Output::Failed(e));
            return;
        }
    };
    while let Ok(mut state) = rx.recv() {
        // Behind: fold everything waiting into one step.
        while let Ok(later) = rx.try_recv() {
            state.merge(later);
        }
        let out = hud.frame(&state);
        if tx.send(out).is_err() {
            return;
        }
    }
}

/// What the HUD has been told, to fire only what changed.
#[derive(Default)]
struct Told {
    hp: Option<(i32, bool)>,
    weapon: Option<Weapon>,
    ammo: Option<[u32; 5]>,
    vehicle: Option<Option<String>>,
    target: Option<Option<(i32, i32)>>,
    reticle: Option<Option<[f32; 2]>>,
}

/// How far (in -1..=1 screen units, across plus up) the aim point moves
/// before the reticle is moved.
const RETICLE_STEP: f32 = 0.002;

struct Thread {
    mission: Mission,
    assets: AssetSource<BufReader<File>>,
    fonts: BTreeMap<String, Font>,
    /// Textures tried (sent, or found missing).
    tried: HashSet<u32>,
    told: Told,
    /// The reticle's widgets were listed in the log.
    reticle_logged: bool,
}

impl Thread {
    fn start(data: &std::path::Path) -> Result<Self, String> {
        let wad = data.join("vz.wad");
        let open = || {
            File::open(&wad)
                .map(BufReader::new)
                .map_err(|e| format!("{}: {e}", wad.display()))
        };
        let game_data = WadArchive::open(open()?)
            .map_err(|e| e.to_string())
            .and_then(|mut w| GameData::read(&mut w).map_err(|e| e.to_string()))?;
        let mut assets = AssetSource::new(open()?).map_err(|e| e.to_string())?;
        let mut fonts = BTreeMap::new();
        for name in FONTS {
            match assets
                .asset(name_hash(name), FONT_TYPE)
                .map_err(|e| e.to_string())
                .and_then(|a| Font::parse(&a).map_err(|e| e.to_string()))
            {
                Ok(f) => {
                    fonts.insert(name.to_owned(), f);
                }
                Err(e) => eprintln!("game HUD: font {name}: {e}"),
            }
        }
        let strings = game_data.strings.clone();
        let mut mission =
            Mission::new(game_data, "Mattias", [0.0; 3]).map_err(|e| e.to_string())?;
        let screen = gui_runtime::screen(mission.lua()).ok_or("no GUI runtime")?;
        {
            let mut s = screen.borrow_mut();
            s.set_strings(strings);
            for (name, f) in &fonts {
                s.set_font(name, f.clone());
            }
        }
        // Her guns, for the gun box to name.
        {
            let mut world = mission.world().borrow_mut();
            for (guid, name) in [
                (RIFLE_GUID, RIFLE_NAME),
                (RPG_GUID, RPG_NAME),
                (LAUNCHER_GUID, LAUNCHER_NAME),
            ] {
                world.insert(guid, Object::new(name, [0.0; 3]));
            }
        }
        // As the player-character module does; her guid is userdata, or
        // the scripts never own the widgets and no event reaches them.
        mission
            .run(
                "MrxGuiManager.CreateGui(Sys.StringToGuid(\"0x1\"))",
                "create gui",
            )
            .map_err(|e| e.to_string())?;
        Ok(Self {
            mission,
            assets,
            fonts,
            tried: HashSet::new(),
            told: Told::default(),
            reticle_logged: false,
        })
    }

    fn frame(&mut self, state: &HudState) -> Output {
        let mut log = Vec::new();
        if let Err(e) = self.feed(state) {
            log.push(format!("event: {e}"));
        }
        let dt = state.dt.min(MAX_STEP);
        if dt > 0.0 {
            self.mission.tick(dt);
            gui_runtime::tick(self.mission.lua(), dt);
        }
        let Some(screen) = gui_runtime::screen(self.mission.lua()) else {
            return Output::Failed("no GUI runtime".into());
        };
        let [w, h] = state.size;
        let items = screen.borrow().draw_list(w, h);
        log.extend(screen.borrow_mut().take_log());
        log.extend(self.mission.take_log());
        // The scripts' own commands (sounds, camera) are not the HUD's.
        let _ = self.mission.take_commands();
        // No aim point: the reticle's widgets are left out.
        let hidden = match state.reticle {
            Some(_) => HashSet::new(),
            None => reticle_widgets(&screen.borrow()),
        };
        if !hidden.is_empty() && !self.reticle_logged {
            self.reticle_logged = true;
            let s = screen.borrow();
            let mut names: Vec<&str> = hidden
                .iter()
                .filter_map(|&id| s.widget(id).map(|w| w.name.as_str()))
                .collect();
            names.sort_unstable();
            log.push(format!("reticle hidden: {}", names.join(", ")));
        }
        let mut prims = Vec::new();
        let mut want = Vec::new();
        for item in items.iter().filter(|i| !hidden.contains(&widget_of(i))) {
            self.prims(item, state, &mut prims, &mut want);
        }
        let mut images = Vec::new();
        for hash in want {
            if self.tried.insert(hash) {
                let image = match self.assets.texture(hash, MAX_TEXTURE) {
                    Ok(t) => t.mips.into_iter().next().map(|rgba| Image {
                        width: t.width,
                        height: t.height,
                        rgba,
                    }),
                    Err(e) => {
                        log.push(format!("texture {hash:08x}: {e}"));
                        None
                    }
                };
                images.push((hash, image));
            }
        }
        Output::Frame { prims, images, log }
    }

    /// Fires the GUI events for what changed since the last state.
    fn feed(&mut self, s: &HudState) -> mlua::Result<()> {
        let lua = self.mission.lua();
        let first = self.told.hp.is_none();
        if first {
            for event in ["PlayerCashChanged", "PlayerFuelChanged"] {
                let t = table(
                    lua,
                    [("uGuid", guid_value(PLAYER)), ("nValue", Value::Integer(0))],
                )?;
                fire(lua, event, t)?;
            }
            let t = table(lua, [("nX", Value::Integer(0)), ("nY", Value::Integer(0))])?;
            fire(lua, "GuiReticlePositionChange", t)?;
        }
        if let Some(screen) = gui_runtime::screen(lua) {
            screen.borrow_mut().set_camera_heading(s.heading);
        }
        // Where she is for the minimap, each frame as the engine does: its
        // handler hands it on to `MinimapUpdate`, with the heading as the
        // rotation (the map turns with the camera).
        if let Some([x, y, z]) = s.position {
            fire(lua, "GuiMinimapUpdate", (x, y, z, s.heading))?;
        }
        let hp = (s.hp.round() as i32, s.in_vehicle);
        if self.told.hp != Some(hp) {
            self.told.hp = Some(hp);
            fire(lua, "GuiHealthUpdate", (hp.0, 100, hp.1))?;
        }
        if self.told.vehicle.as_ref() != Some(&s.vehicle) {
            self.told.vehicle = Some(s.vehicle.clone());
            let (name, faction) = match &s.vehicle {
                Some(label) => {
                    let (name, faction) = vehicle_name(label);
                    (
                        Value::String(lua.create_string(name)?),
                        faction.map_or(Value::Nil, guid_value),
                    )
                }
                None => (Value::Boolean(false), Value::Nil),
            };
            fire(lua, "GuiVehicleNameUpdate", (name, faction))?;
        }
        if self.told.weapon != Some(s.weapon) {
            self.told.weapon = Some(s.weapon);
            let (icon, guid, reticle) = match s.weapon {
                Weapon::Rifle => (RIFLE_ICON, RIFLE_GUID, RIFLE_RETICLE),
                Weapon::Rpg => (RPG_ICON, RPG_GUID, RPG_RETICLE),
                Weapon::GrenadeLauncher => (LAUNCHER_ICON, LAUNCHER_GUID, LAUNCHER_RETICLE),
            };
            // What `MrxGuiHudReticle.HandleReticleGunSwitchEvent` reads:
            // without a type it leaves the reticle as it was. No target
            // health type: that bar shows only on a target.
            let t = table(
                lua,
                [
                    ("uNewCurrentGun", Value::String(lua.create_string(icon)?)),
                    ("uNewCurrentGunGuid", guid_value(guid)),
                    ("sReticleType", Value::String(lua.create_string("Normal")?)),
                    (
                        "uReticleTexture",
                        Value::String(lua.create_string(reticle)?),
                    ),
                    ("bReticleCrosshair", Value::Boolean(true)),
                ],
            )?;
            fire(lua, "GuiWeaponEquippedUpdate", t)?;
        }
        let ammo = ammo(s.weapon, s.fired, s.picked, s.launcher);
        if self.told.ammo != Some(ammo) {
            self.told.ammo = Some(ammo);
            let names = [
                "PrimaryClipSize",
                "PrimaryCurrentAmmo",
                "PrimaryStoredAmmo",
                "ExplosivesCurrentAmmo",
                "ExplosivesStoredAmmo",
            ];
            let t = lua.create_table()?;
            for (k, v) in names.iter().zip(ammo) {
                t.raw_set(*k, v)?;
            }
            fire(lua, "GuiAmmoUpdate", t)?;
        }
        // The reticle sits where she aims, as the game's camera tells it.
        let moved = match (self.told.reticle, s.reticle) {
            (Some(Some(a)), Some(b)) => (a[0] - b[0]).abs() + (a[1] - b[1]).abs() > RETICLE_STEP,
            (Some(a), b) => a.is_some() != b.is_some(),
            (None, _) => true,
        };
        if moved {
            self.told.reticle = Some(s.reticle);
            if let Some([x, y]) = s.reticle {
                if let Some(screen) = gui_runtime::screen(lua) {
                    screen.borrow_mut().set_reticle(x, y);
                }
                let t = table(
                    lua,
                    [
                        ("nX", Value::Number(x.into())),
                        ("nY", Value::Number(y.into())),
                    ],
                )?;
                fire(lua, "GuiReticlePositionChange", t)?;
            }
        }
        if self.told.target != Some(s.target) {
            self.told.target = Some(s.target);
            // (target, centre x, y, spread x, y, health, maximum): no
            // health fades the target-health arc (opaque as laid out);
            // a health shows that much of it.
            match s.target {
                Some((kind, hp)) => fire(
                    lua,
                    "GuiReticleUpdate",
                    (kind, Value::Nil, Value::Nil, 0, 0, hp, 100),
                )?,
                None => fire(lua, "GuiReticleUpdate", 0)?,
            }
        }
        for &(damage, direction) in &s.hurts {
            fire(lua, "GuiPlayerReceiveDamage", (direction, damage))?;
        }
        for m in &s.messages {
            // Worded as `MrxGuiInterface.DisplayObjectiveMessage` words it,
            // from the scripts' own tables: colour, icon, "[objective.
            // Objectiveadded]" and the objective's text id.
            let (text, priority): (Option<String>, Option<mlua::Integer>) = lua
                .load(WORD_MESSAGE)
                .set_name("=m2-hud message")
                .call((m.kind.as_str(), m.icon, m.text.as_str()))?;
            let Some(text) = text else {
                continue;
            };
            let priority = priority.unwrap_or(1);
            let t = table(
                lua,
                [
                    ("sMessage", Value::String(lua.create_string(text)?)),
                    ("nPriority", Value::Integer(priority)),
                    ("nDuration", Value::Number(MESSAGE_SECONDS)),
                    ("bClearBuffer", Value::Boolean(true)),
                    ("bAllowsAppends", Value::Boolean(false)),
                ],
            )?;
            fire(lua, "TextBufferAddMessage", t)?;
        }
        Ok(())
    }

    /// The quads of one draw item; textures they use go in `want`.
    fn prims(&self, item: &DrawItem, state: &HudState, out: &mut Vec<Prim>, want: &mut Vec<u32>) {
        match item {
            DrawItem::Quad {
                texture,
                uv,
                colour,
                rect,
                rotation,
                pie,
                ..
            } => {
                let hash = name_hash(texture);
                want.push(hash);
                match pie {
                    Some(pie) => out.extend(wedge(hash, *rect, *uv, *colour, *rotation, *pie)),
                    None => out.push(quad(Some(hash), *rect, *uv, *colour, *rotation)),
                }
            }
            DrawItem::Fill { colour, rect, .. } => {
                out.push(quad(None, *rect, [0.0, 0.0, 1.0, 1.0], *colour, 0.0));
            }
            DrawItem::Text {
                font,
                spans,
                rect,
                size,
                colour,
                justification,
                ..
            } => {
                if let Some(f) = self.fonts.get(&font.to_ascii_lowercase()) {
                    text(f, spans, *rect, *size, *colour, *justification, out, want);
                }
            }
            DrawItem::Minimap {
                rect,
                player,
                rotation,
                range,
                ..
            } => {
                if let Some(player) = player {
                    let view = MapView {
                        rect: *rect,
                        player: *player,
                        heading: *rotation,
                        range: range.unwrap_or(DEFAULT_RANGE),
                    };
                    minimap(&view, state, out, want);
                }
            }
            // Not drawn yet (see the module docs).
            DrawItem::Flash { .. } | DrawItem::Movie { .. } => {}
        }
    }
}

/// The widget a draw item belongs to.
fn widget_of(item: &DrawItem) -> u32 {
    match item {
        DrawItem::Quad { widget, .. }
        | DrawItem::Fill { widget, .. }
        | DrawItem::Text { widget, .. }
        | DrawItem::Flash { widget, .. }
        | DrawItem::Minimap { widget, .. }
        | DrawItem::Movie { widget, .. } => *widget,
    }
}

/// The reticle's widgets: those named for it, and everything under them.
fn reticle_widgets(screen: &gui_runtime::Screen) -> HashSet<u32> {
    let named = |id: u32| {
        screen
            .widget(id)
            .is_some_and(|w| w.name.to_ascii_lowercase().contains("reticle"))
    };
    screen
        .widgets()
        .map(|(id, _)| id)
        .filter(|&id| {
            let mut at = Some(id);
            // Up the parents (bounded, should a parent point back down).
            for _ in 0..64 {
                match at {
                    Some(a) if named(a) => return true,
                    Some(a) => at = screen.widget(a).and_then(|w| w.parent),
                    None => return false,
                }
            }
            false
        })
        .collect()
}

/// Clip, in the clip, stored, grenades, grenades stored: as
/// `GuiAmmoUpdate` takes them. Rounds picked up add to the stores; the
/// rockets count down from that and, run out, start again full. The
/// grenade launcher's are its own (`launcher`: in the clip, stored).
fn ammo(weapon: Weapon, fired: Fired, picked: Picked, launcher: Option<Rounds>) -> [u32; 5] {
    let grenades = GRENADES - fired.grenades % GRENADES;
    match weapon {
        Weapon::Rifle => [
            RIFLE_CLIP,
            RIFLE_CLIP - fired.rifle % RIFLE_CLIP,
            RIFLE_STORED + picked.rifle,
            grenades,
            0,
        ],
        Weapon::Rpg => {
            let stock = RPG_STORED + picked.rockets;
            [1, 1, stock - fired.rockets % stock, grenades, 0]
        }
        Weapon::GrenadeLauncher => {
            let r = launcher.unwrap_or_default();
            [
                Weapon::GrenadeLauncher.clip(),
                r.clip,
                r.stored,
                grenades,
                0,
            ]
        }
    }
}

fn fire(lua: &Lua, event: &str, args: impl IntoLuaMulti) -> mlua::Result<()> {
    gui_runtime::fire(lua, event, args).map(|_| ())
}

fn table<const N: usize>(lua: &Lua, fields: [(&str, Value); N]) -> mlua::Result<Value> {
    let t = lua.create_table()?;
    for (k, v) in fields {
        t.raw_set(k, v)?;
    }
    Ok(Value::Table(t))
}

/// A rectangle's quad, turned `degrees` clockwise about its centre.
fn quad(texture: Option<u32>, r: [f32; 4], uv: [f32; 4], colour: [f32; 4], degrees: f32) -> Prim {
    let corners = [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]];
    let corners = if degrees == 0.0 {
        corners
    } else {
        let (s, c) = degrees.to_radians().sin_cos();
        let (cx, cy) = ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
        corners.map(|[x, y]| {
            let (dx, dy) = (x - cx, y - cy);
            // y grows down the screen: this turns clockwise.
            [cx + dx * c - dy * s, cy + dx * s + dy * c]
        })
    };
    Prim {
        texture,
        corners,
        uv: [
            [uv[0], uv[1]],
            [uv[2], uv[1]],
            [uv[2], uv[3]],
            [uv[0], uv[3]],
        ],
        colour,
    }
}

/// The minimap as the scripts set it up: its circle on screen, where she
/// is, the camera's heading (the map turns with it: ahead is up) and the
/// metres to the rim.
struct MapView {
    rect: [f32; 4],
    player: [f32; 3],
    heading: f32,
    range: f32,
}

impl MapView {
    fn centre(&self) -> [f32; 2] {
        [
            (self.rect[0] + self.rect[2]) / 2.0,
            (self.rect[1] + self.rect[3]) / 2.0,
        ]
    }

    fn radius(&self) -> f32 {
        (self.rect[2] - self.rect[0]) / 2.0
    }

    /// Where world `(x, z)` shows: on screen, and whether inside the rim.
    fn screen(&self, x: f32, z: f32) -> ([f32; 2], bool) {
        let (s, c) = self.heading.to_radians().sin_cos();
        // Heading grows turning left: ahead is (sin h, cos h), right is
        // (-cos h, sin h).
        let (dx, dz) = (x - self.player[0], z - self.player[2]);
        let ahead = dx * s + dz * c;
        let right = -dx * c + dz * s;
        let k = self.radius() / self.range;
        let [cx, cy] = self.centre();
        (
            [cx + right * k, cy - ahead * k],
            ahead.hypot(right) <= self.range,
        )
    }

    /// The world `(x, z)` that shows at `angle` degrees clockwise from
    /// straight up, `along` (0..=1) of the way to the rim.
    fn world(&self, angle: f32, along: f32) -> [f32; 2] {
        let (hs, hc) = self.heading.to_radians().sin_cos();
        let (s, c) = angle.to_radians().sin_cos();
        let (ahead, right) = (c * along * self.range, s * along * self.range);
        [
            self.player[0] + ahead * hs - right * hc,
            self.player[2] + ahead * hc + right * hs,
        ]
    }
}

/// The map texture's coordinates of world `(x, z)`.
fn map_uv([x, z]: [f32; 2]) -> [f32; 2] {
    [(ISLAND / 2.0 - x) / ISLAND, (ISLAND / 2.0 - z) / ISLAND]
}

/// The minimap's quads: the radar's backdrop with north on its rim, the
/// island's map round her in the circle, the objective markers (at the rim
/// when farther) and her arrow.
fn minimap(view: &MapView, state: &HudState, out: &mut Vec<Prim>, want: &mut Vec<u32>) {
    let [cx, cy] = view.centre();
    let r = view.radius();
    let white = [1.0; 4];
    let full = [0.0, 0.0, 1.0, 1.0];
    let border = name_hash(BORDER_TEXTURE);
    want.push(border);
    let b = r * BORDER;
    // North (+z) is `heading` degrees clockwise from ahead.
    out.push(quad(
        Some(border),
        [cx - b, cy - b, cx + b, cy + b],
        full,
        white,
        view.heading,
    ));
    let map = name_hash(MAP_TEXTURE);
    want.push(map);
    const SIDES: usize = 48;
    let rim = |i: usize| i as f32 * 360.0 / SIDES as f32;
    let centre_uv = map_uv([view.player[0], view.player[2]]);
    for i in 0..SIDES {
        let point = |a: f32| {
            let (s, c) = a.to_radians().sin_cos();
            [cx + s * r, cy - c * r]
        };
        let (a, b) = (rim(i), rim(i + 1));
        let (ua, ub) = (map_uv(view.world(a, 1.0)), map_uv(view.world(b, 1.0)));
        out.push(Prim {
            texture: Some(map),
            corners: [[cx, cy], point(a), point(b), point(b)],
            uv: [centre_uv, ua, ub, ub],
            colour: white,
        });
    }
    for (at, icon, colour) in &state.blips {
        let ([x, y], inside) = view.screen(at[0], at[2]);
        // Farther ones sit on the rim, towards them.
        let [x, y] = if inside {
            [x, y]
        } else {
            let (dx, dy) = (x - cx, y - cy);
            let k = r / dx.hypot(dy).max(1e-3);
            [cx + dx * k, cy + dy * k]
        };
        let hash = name_hash(&format!("HUD_objective_{}", icon.replace(' ', "_")));
        want.push(hash);
        let s = r * 0.16;
        let c = colour.map(|v| f32::from(v) / 255.0);
        out.push(quad(Some(hash), [x - s, y - s, x + s, y + s], full, c, 0.0));
    }
    let arrow = name_hash(ARROW_TEXTURE);
    want.push(arrow);
    let (w, h) = (r * 0.09, r * 0.18);
    out.push(quad(
        Some(arrow),
        [cx - w, cy - h, cx + w, cy + h],
        full,
        white,
        view.heading - state.facing,
    ));
}

/// The part of a rectangle's quad in the wedge from its centre between
/// angles `pie` (degrees clockwise from straight up), turned `degrees`
/// clockwise: triangles, each a prim with its last corner doubled.
fn wedge(
    texture: u32,
    r: [f32; 4],
    uv: [f32; 4],
    colour: [f32; 4],
    degrees: f32,
    pie: [f32; 2],
) -> Vec<Prim> {
    let [from, to] = pie;
    let (w, h) = (r[2] - r[0], r[3] - r[1]);
    if to <= from || w <= 0.0 || h <= 0.0 {
        return Vec::new();
    }
    let centre = [(r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0];
    // Where a ray from the centre at angle `a` leaves the rectangle.
    let edge = |a: f32| {
        let (dx, dy) = {
            let (s, c) = a.to_radians().sin_cos();
            (s, -c)
        };
        let t = (w / 2.0 / dx.abs().max(1e-6)).min(h / 2.0 / dy.abs().max(1e-6));
        [centre[0] + dx * t, centre[1] + dy * t]
    };
    // The ends, the corners between them (where the edge bends) and every
    // 15 degrees between.
    let corner = w.atan2(h).to_degrees();
    let mut angles = vec![from, to];
    for turn in [-360.0, 0.0, 360.0] {
        for c in [corner, 180.0 - corner, 180.0 + corner, 360.0 - corner] {
            if turn + c > from && turn + c < to {
                angles.push(turn + c);
            }
        }
    }
    let mut a = from + 15.0;
    while a < to {
        angles.push(a);
        a += 15.0;
    }
    angles.sort_by(f32::total_cmp);
    let uv_at = |p: [f32; 2]| {
        [
            uv[0] + (p[0] - r[0]) / w * (uv[2] - uv[0]),
            uv[1] + (p[1] - r[1]) / h * (uv[3] - uv[1]),
        ]
    };
    let (s, c) = degrees.to_radians().sin_cos();
    // y grows down the screen: this turns clockwise.
    let turned = |[x, y]: [f32; 2]| {
        let (dx, dy) = (x - centre[0], y - centre[1]);
        [centre[0] + dx * c - dy * s, centre[1] + dx * s + dy * c]
    };
    angles
        .windows(2)
        .map(|pair| {
            let (p, q) = (edge(pair[0]), edge(pair[1]));
            let points = [centre, p, q, q];
            Prim {
                texture: Some(texture),
                corners: points.map(turned),
                uv: points.map(uv_at),
                colour,
            }
        })
        .collect()
}

/// Glyph quads for coloured `spans` in box `rect`: lines from its top,
/// each justified, `size` points per font pixel.
#[allow(clippy::too_many_arguments)]
fn text(
    font: &Font,
    spans: &[TextSpan],
    rect: [f32; 4],
    size: f32,
    colour: [f32; 4],
    justify: Anchor,
    out: &mut Vec<Prim>,
    want: &mut Vec<u32>,
) {
    // Each line's parts: text, or an inline icon a line high and square;
    // and the colour.
    type Part<'a> = (&'a str, Option<&'a str>, [f32; 4]);
    let mut lines: Vec<Vec<Part>> = vec![Vec::new()];
    for s in spans {
        let c = s.colour.map_or(colour, |c| [c[0], c[1], c[2], colour[3]]);
        if let Some(icon) = &s.icon {
            if let Some(line) = lines.last_mut() {
                line.push(("", Some(icon), c));
            }
            continue;
        }
        for (i, part) in s.text.split('\n').enumerate() {
            if i > 0 {
                lines.push(Vec::new());
            }
            if let Some(line) = lines.last_mut() {
                line.push((part, None, c));
            }
        }
    }
    let line_height = f32::from(font.line_height) * size;
    for (n, line) in lines.iter().enumerate() {
        let width: f32 = line
            .iter()
            .map(|p| match p.1 {
                Some(_) => line_height,
                None => font.text_width(p.0) * size,
            })
            .sum();
        let mut x = match justify {
            Anchor::Start => rect[0],
            Anchor::Centre => (rect[0] + rect[2] - width) / 2.0,
            Anchor::End => rect[2] - width,
        };
        let top = rect[1] + n as f32 * line_height;
        for (part, icon, c) in line {
            if let Some(icon) = icon {
                let hash = name_hash(icon);
                want.push(hash);
                let r = [x, top, x + line_height, top + line_height];
                out.push(quad(Some(hash), r, [0.0, 0.0, 1.0, 1.0], *c, 0.0));
                x += line_height;
                continue;
            }
            for code in part.encode_utf16() {
                if let Some(g) = font.glyph(code)
                    && let Some(m) = font.materials.get(usize::from(g.material))
                {
                    let y = top + f32::from(g.y_offset) * size;
                    let r = [
                        x,
                        y,
                        x + f32::from(g.width) * size,
                        y + f32::from(g.height) * size,
                    ];
                    want.push(m.texture);
                    out.push(quad(Some(m.texture), r, g.uv, *c, 0.0));
                }
                x += font.advance(code) * size;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where on screen the hit arc shows, clockwise from straight ahead:
    /// `HandleUpdateEvent` turns it by direction + heading, from the top
    /// where its art points.
    fn arc(heading: f32, from: Vec3) -> f32 {
        (direction(Some(Vec3::ZERO), Some(from)) + heading).rem_euclid(360.0)
    }

    #[test]
    fn hurts_come_from_the_side_they_came_from() {
        let near = |a: f32, b: f32| {
            let d = (a - b).rem_euclid(360.0);
            !(0.01..=359.99).contains(&d)
        };
        // Looking down +z (heading 0): +x is to the left (heading grows
        // turning left, forward is (sin h, 0, cos h), right is -x).
        assert!(near(arc(0.0, Vec3::Z), 0.0));
        assert!(near(arc(0.0, -Vec3::X), 90.0), "{}", arc(0.0, -Vec3::X));
        assert!(near(arc(0.0, -Vec3::Z), 180.0));
        assert!(near(arc(0.0, Vec3::X), 270.0));
        // Turned left to look down +x, what was on the left is ahead and
        // what was ahead is on the right.
        assert!(near(arc(90.0, Vec3::X), 0.0), "{}", arc(90.0, Vec3::X));
        assert!(near(arc(90.0, Vec3::Z), 90.0), "{}", arc(90.0, Vec3::Z));
        assert_eq!(direction(None, Some(Vec3::X)), 0.0);
    }

    #[test]
    fn counters_count_down_and_refill() {
        let fired = |rifle, rockets, grenades| Fired {
            rifle,
            rockets,
            grenades,
        };
        let none = Picked::default();
        assert_eq!(ammo(Weapon::Rifle, fired(0, 0, 0), none, None)[1], 30);
        assert_eq!(ammo(Weapon::Rifle, fired(29, 0, 0), none, None)[1], 1);
        assert_eq!(ammo(Weapon::Rifle, fired(30, 0, 0), none, None)[1], 30);
        assert_eq!(
            ammo(Weapon::Rpg, fired(0, 0, 0), none, None)[..3],
            [1, 1, RPG_STORED]
        );
        assert_eq!(
            ammo(Weapon::Rpg, fired(0, 1, 0), none, None)[2],
            RPG_STORED - 1
        );
        assert_eq!(ammo(Weapon::Rpg, fired(0, 4, 0), none, None)[2], RPG_STORED);
        assert_eq!(
            ammo(Weapon::Rifle, fired(0, 0, 1), none, None)[3],
            GRENADES - 1
        );
        // A crate's 20 rockets after three fired: 21 left.
        let crate_ = Picked {
            rockets: 20,
            ..Picked::default()
        };
        assert_eq!(
            ammo(Weapon::Rpg, fired(0, 3, 0), crate_, None)[2],
            RPG_STORED + 17
        );
        // The launcher's own rounds: "6 / 12" as the crate gives it.
        let launcher = Some(Rounds {
            clip: 6,
            stored: 12,
        });
        assert_eq!(
            ammo(Weapon::GrenadeLauncher, fired(0, 0, 0), none, launcher)[..3],
            [6, 6, 12]
        );
    }

    #[test]
    fn vehicles_are_named_with_their_faction() {
        assert_eq!(
            vehicle_name("vz_veh_truck_m151"),
            ("truck m151".to_owned(), Some(0xb442_0059))
        );
        assert_eq!(
            vehicle_name("global_veh_68valiant"),
            ("68valiant".to_owned(), None)
        );
    }

    #[test]
    fn the_minimap_turns_with_the_camera() {
        // Looking down +x (heading 90): +x is up the map and +z, which
        // was ahead, is to the right.
        let view = MapView {
            rect: [0.0, 0.0, 100.0, 100.0],
            player: [10.0, 0.0, 20.0],
            heading: 90.0,
            range: 100.0,
        };
        let ([x, y], inside) = view.screen(60.0, 20.0);
        assert!(
            inside && (x - 50.0).abs() < 1e-3 && (y - 25.0).abs() < 1e-3,
            "{x} {y}"
        );
        let ([x, y], _) = view.screen(10.0, 70.0);
        assert!(
            (x - 75.0).abs() < 1e-3 && (y - 50.0).abs() < 1e-3,
            "{x} {y}"
        );
        assert!(!view.screen(500.0, 20.0).1);
        // And back: what shows a quarter turn round, halfway out.
        let [wx, wz] = view.world(90.0, 0.5);
        let ([x, y], _) = view.screen(wx, wz);
        assert!(
            (x - 75.0).abs() < 1e-3 && (y - 50.0).abs() < 1e-3,
            "{x} {y}"
        );
        // The map's corners: +x and +z at its left and top.
        assert_eq!(map_uv([ISLAND / 2.0, ISLAND / 2.0]), [0.0, 0.0]);
    }

    #[test]
    fn pie_slices_keep_their_wedge() {
        // A quarter from straight up to the right: the top right quarter,
        // bent at the corner.
        let tris = wedge(
            1,
            [0.0, 0.0, 2.0, 2.0],
            [0.0, 0.0, 1.0, 1.0],
            [1.0; 4],
            0.0,
            [0.0, 90.0],
        );
        assert!(!tris.is_empty());
        for t in &tris {
            for [x, y] in t.corners {
                assert!(x > 1.0 - 1e-4 && y < 1.0 + 1e-4, "{x} {y}");
            }
        }
        let near = |c: [f32; 2]| (c[0] - 2.0).abs() + c[1].abs() < 1e-4;
        assert!(tris.iter().any(|t| t.corners.iter().any(|&c| near(c))));
        // The target-health arc's 130..230 lies under the centre.
        let arc = wedge(
            1,
            [0.0, 0.0, 2.0, 2.0],
            [0.0, 0.0, 1.0, 1.0],
            [1.0; 4],
            0.0,
            [130.0, 230.0],
        );
        assert!(
            arc.iter()
                .all(|t| t.corners.iter().all(|c| c[1] > 1.0 - 1e-4))
        );
        assert!(
            wedge(
                1,
                [0.0, 0.0, 2.0, 2.0],
                [0.0; 4],
                [1.0; 4],
                0.0,
                [90.0, 90.0]
            )
            .is_empty()
        );
    }

    #[test]
    fn quads_turn_clockwise() {
        let p = quad(
            None,
            [0.0, 0.0, 2.0, 2.0],
            [0.0, 0.0, 1.0, 1.0],
            [1.0; 4],
            90.0,
        );
        // The top left corner goes to the top right.
        let [x, y] = p.corners[0];
        assert!((x - 2.0).abs() < 1e-5 && y.abs() < 1e-5, "{x} {y}");
    }

    #[test]
    fn later_states_add_their_events() {
        let mut a = HudState {
            dt: 0.1,
            hurts: vec![(10.0, 0.0)],
            hp: 90.0,
            ..HudState::default()
        };
        a.merge(HudState {
            dt: 0.2,
            hurts: vec![(5.0, 90.0)],
            messages: vec![ObjectiveMessage {
                kind: "add".into(),
                icon: 5,
                text: "hi".into(),
            }],
            hp: 85.0,
            ..HudState::default()
        });
        assert!((a.dt - 0.3).abs() < 1e-9);
        assert_eq!(a.hurts.len(), 2);
        assert_eq!(a.messages.len(), 1);
        assert_eq!(a.hp, 85.0);
    }
}
