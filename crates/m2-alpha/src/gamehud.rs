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
//! Not drawn yet: the minimap's art (the engine draws it from the
//! terrain), the Flash pieces (the minimap's zones, pop-ups), animated
//! sprites and clock wipes, and the target-health pie slice (drawn
//! whole). The ammo the counters show is our own count (see
//! [`RIFLE_CLIP`]): the play has no ammo of its own yet.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, BufWriter, Write as _};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError, channel};
use std::time::{Duration, Instant};

use egui::{Color32, Mesh, TextureHandle, TextureOptions, pos2};
use m2_assets::font::FONT_TYPE;
use m2_assets::{Font, WadArchive, name_hash};
use m2_engine::{AssetSource, Install};
use m2_game::events::guid_value;
use m2_game::gui::Anchor;
use m2_game::gui_runtime::{self, DrawItem, TextSpan};
use m2_game::mission::{GameData, Mission};
use m2_math::glam::Vec3;
use mlua::{IntoLuaMulti, Lua, Value};

use crate::combat::{Fired, Weapon};
use crate::config::Config;
use crate::game::Game;

/// The fonts the HUD's text widgets use.
const FONTS: [&str; 2] = ["english_18", "english_20"];
/// The player's guid as the scripts know her (`Sys.StringToGuid("0x1")`).
const PLAYER: u32 = 1;
/// Guids standing for her guns (`uNewCurrentGunGuid`): the scripts only
/// ask their names.
const RIFLE_GUID: u32 = 0x100;
const RPG_GUID: u32 = 0x101;
/// The ammo the counters show. The play never runs out: the rifle's clip
/// counts down and starts again full; the stores stay as they are.
const RIFLE_CLIP: u32 = 30;
const RIFLE_STORED: u32 = 240;
const RPG_STORED: u32 = 4;
const GRENADES: u32 = 4;
/// The weapons' icons in the HUD's atlas (`uNewCurrentGun`).
const RIFLE_ICON: &str = "weapons_assault_rifle";
const RPG_ICON: &str = "weapons_rocket_launcher";
/// Seconds a message stays in the message buffer.
const MESSAGE_SECONDS: f64 = 6.0;
/// Seconds of play the scripts are given at most per frame.
const MAX_STEP: f64 = 0.25;
/// Largest texture size decoded, pixels.
const MAX_TEXTURE: u32 = 2048;

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
    /// The vehicle she is in, readable.
    pub vehicle: Option<String>,
    /// Hurts since the last state: damage and the direction it came from
    /// in degrees clockwise from where the camera looks.
    pub hurts: Vec<(f32, f32)>,
    /// Messages for the message buffer since the last state.
    pub messages: Vec<String>,
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
        let yaw = game.camera.yaw;
        let hurts = game
            .combat
            .take_hurts()
            .into_iter()
            .map(|(damage, from)| (damage, direction(at, yaw, from)))
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
            vehicle: vehicle.map(|v| readable_vehicle(&v.spec.label)),
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

/// Degrees clockwise (seen from above) from the camera's heading `yaw`
/// to `from`, as seen from `at`; 0 ahead, 90 to the right. 0 when either
/// is unknown.
fn direction(at: Option<Vec3>, yaw: f32, from: Option<Vec3>) -> f32 {
    let (Some(at), Some(from)) = (at, from) else {
        return 0.0;
    };
    let d = from - at;
    if d.x.abs() + d.z.abs() < 1e-3 {
        return 0.0;
    }
    // Yaw grows turning left (forward is (sin yaw, 0, cos yaw)).
    let left = d.x.atan2(d.z) - yaw;
    (-left).to_degrees().rem_euclid(360.0)
}

/// `"global_veh_hondacrx"` as `"hondacrx"`.
fn readable_vehicle(label: &str) -> String {
    label.trim_start_matches("global_veh_").replace('_', " ")
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
    last_message: Option<(bool, String)>,
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
    pub fn message(&mut self, message: Option<&(bool, String)>) -> Option<String> {
        if message == self.last_message.as_ref() {
            return None;
        }
        self.last_message = message.cloned();
        let (done, line) = message?;
        let head = if *done {
            "Objective complete"
        } else {
            "New objective"
        };
        Some(format!("{head}: {line}"))
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
}

struct Thread {
    mission: Mission,
    assets: AssetSource<BufReader<File>>,
    fonts: BTreeMap<String, Font>,
    /// Textures tried (sent, or found missing).
    tried: HashSet<u32>,
    told: Told,
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
        let mut prims = Vec::new();
        let mut want = Vec::new();
        for item in &items {
            self.prims(item, &mut prims, &mut want);
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
        let hp = (s.hp.round() as i32, s.in_vehicle);
        if self.told.hp != Some(hp) {
            self.told.hp = Some(hp);
            fire(lua, "GuiHealthUpdate", (hp.0, 100, hp.1))?;
        }
        if self.told.vehicle.as_ref() != Some(&s.vehicle) {
            self.told.vehicle = Some(s.vehicle.clone());
            let name = match &s.vehicle {
                Some(n) => Value::String(lua.create_string(n)?),
                None => Value::Boolean(false),
            };
            fire(lua, "GuiVehicleNameUpdate", (name, Value::Nil))?;
        }
        if self.told.weapon != Some(s.weapon) {
            self.told.weapon = Some(s.weapon);
            let (icon, guid) = match s.weapon {
                Weapon::Rifle => (RIFLE_ICON, RIFLE_GUID),
                Weapon::Rpg => (RPG_ICON, RPG_GUID),
            };
            let t = table(
                lua,
                [
                    ("uNewCurrentGun", Value::String(lua.create_string(icon)?)),
                    ("uNewCurrentGunGuid", guid_value(guid)),
                    ("bReticleCrosshair", Value::Boolean(true)),
                ],
            )?;
            fire(lua, "GuiWeaponEquippedUpdate", t)?;
        }
        let ammo = ammo(s.weapon, s.fired);
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
        for &(damage, direction) in &s.hurts {
            fire(lua, "GuiPlayerReceiveDamage", (direction, damage))?;
        }
        for m in &s.messages {
            let t = table(
                lua,
                [
                    ("sMessage", Value::String(lua.create_string(m)?)),
                    ("nDuration", Value::Number(MESSAGE_SECONDS)),
                ],
            )?;
            fire(lua, "TextBufferAddMessage", t)?;
        }
        Ok(())
    }

    /// The quads of one draw item; textures they use go in `want`.
    fn prims(&self, item: &DrawItem, out: &mut Vec<Prim>, want: &mut Vec<u32>) {
        match item {
            DrawItem::Quad {
                texture,
                uv,
                colour,
                rect,
                rotation,
                ..
            } => {
                let hash = name_hash(texture);
                want.push(hash);
                out.push(quad(Some(hash), *rect, *uv, *colour, *rotation));
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
            // Not drawn yet (see the module docs).
            DrawItem::Flash { .. } | DrawItem::Minimap { .. } | DrawItem::Movie { .. } => {}
        }
    }
}

/// Clip, in the clip, stored, grenades, grenades stored: as
/// `GuiAmmoUpdate` takes them.
fn ammo(weapon: Weapon, fired: Fired) -> [u32; 5] {
    match weapon {
        Weapon::Rifle => [
            RIFLE_CLIP,
            RIFLE_CLIP - fired.rifle % RIFLE_CLIP,
            RIFLE_STORED,
            GRENADES,
            0,
        ],
        Weapon::Rpg => [1, 1, RPG_STORED, GRENADES, 0],
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
    let mut lines: Vec<Vec<(&str, [f32; 4])>> = vec![Vec::new()];
    for s in spans {
        let c = s.colour.map_or(colour, |c| [c[0], c[1], c[2], colour[3]]);
        for (i, part) in s.text.split('\n').enumerate() {
            if i > 0 {
                lines.push(Vec::new());
            }
            if let Some(line) = lines.last_mut() {
                line.push((part, c));
            }
        }
    }
    for (n, line) in lines.iter().enumerate() {
        let width: f32 = line.iter().map(|p| font.text_width(p.0)).sum::<f32>() * size;
        let mut x = match justify {
            Anchor::Start => rect[0],
            Anchor::Centre => (rect[0] + rect[2] - width) / 2.0,
            Anchor::End => rect[2] - width,
        };
        let top = rect[1] + n as f32 * f32::from(font.line_height) * size;
        for (part, c) in line {
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

    #[test]
    fn hurts_come_from_the_side_they_came_from() {
        let at = Some(Vec3::ZERO);
        // Looking down +z (yaw 0): +x is to the left (yaw grows turning
        // left, forward is (sin yaw, 0, cos yaw), right is -x).
        let ahead = direction(at, 0.0, Some(Vec3::Z));
        let left = direction(at, 0.0, Some(Vec3::X));
        let right = direction(at, 0.0, Some(-Vec3::X));
        let behind = direction(at, 0.0, Some(-Vec3::Z));
        assert!(ahead.abs() < 0.01 || (ahead - 360.0).abs() < 0.01);
        assert!((right - 90.0).abs() < 0.01, "{right}");
        assert!((behind - 180.0).abs() < 0.01, "{behind}");
        assert!((left - 270.0).abs() < 0.01, "{left}");
        assert_eq!(direction(None, 0.0, Some(Vec3::X)), 0.0);
    }

    #[test]
    fn the_rifle_clip_counts_down_and_refills() {
        let fired = |rifle| Fired {
            rifle,
            ..Fired::default()
        };
        assert_eq!(ammo(Weapon::Rifle, fired(0))[1], 30);
        assert_eq!(ammo(Weapon::Rifle, fired(29))[1], 1);
        assert_eq!(ammo(Weapon::Rifle, fired(30))[1], 30);
        assert_eq!(ammo(Weapon::Rpg, fired(7))[..3], [1, 1, RPG_STORED]);
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
            messages: vec!["hi".into()],
            hp: 85.0,
            ..HudState::default()
        });
        assert!((a.dt - 0.3).abs() < 1e-9);
        assert_eq!(a.hurts.len(), 2);
        assert_eq!(a.messages, ["hi"]);
        assert_eq!(a.hp, 85.0);
    }
}
