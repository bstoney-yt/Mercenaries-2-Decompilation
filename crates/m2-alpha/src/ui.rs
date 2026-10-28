//! The HUD and the pause menu, drawn with egui over the finished frame.
//!
//! Health, ammo and weapon, the reticle, hit arcs and messages are the
//! game's own HUD ([`GameHud`]) once its scripts have started; until then,
//! or without the game's files, a stand-in of our own draws health, weapon
//! and a crosshair, and the mission's objectives and markers. The markers
//! are then the game's icons. What the player can do next and loading
//! lines are ours, in egui's bundled fonts.

use std::time::Duration;

use std::f32::consts::TAU;

use egui::{Align2, Color32, FontId, RawInput, Rect, Shape, Stroke, StrokeKind, pos2, vec2};
use egui_wgpu::{Renderer, RendererOptions, ScreenDescriptor};
use m2_render::gpu::OverlayTarget;
use m2_render::gpu::wgpu;

use m2_math::glam::{Mat4, Vec3};

use crate::combat::Weapon;
use crate::config::Config;
use crate::gamehud::{GameHud, HudState, flash_key};
use crate::mission::MissionView;
use crate::support::{SupportHud, WheelHud};

/// What the HUD shows this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HudInfo {
    /// On foot or driving (not flying the free camera).
    pub playing: bool,
    pub hp: f32,
    pub down: bool,
    pub weapon: Weapon,
    /// Her rifle's template name (`Carbine`, `Bullpup Rifle`).
    pub rifle: String,
    /// What she carries in the launcher's place, if anything.
    pub second: Option<Weapon>,
    /// What she carries in the launcher's place, by its template's name
    /// (`RPG`, `Stinger`); empty, by its kind.
    pub launcher: String,
    /// Grenades she has; or of the explosive she has chosen to throw
    /// (`explosive`: its template's name, empty for grenades).
    pub grenades: u32,
    pub explosive: String,
    /// The rounds of the weapon up when it runs out (the rifle's and the
    /// grenade launcher's): in its clip, and stored.
    pub rounds: Option<(u32, u32)>,
    /// What the use key takes from within reach, as the game words it
    /// ("Equip Grenade Launcher").
    pub prompt: Option<String>,
    /// A hijack's button prompt is up and the keyboard plays: the keys
    /// that press it ("K or flick the mouse down"), under the game's arrow
    /// ([`crate::hijack::key_hint`]).
    pub hijack_keys: Option<String>,
    /// Driving: the vehicle's name and speed (m/s), and whether it is
    /// wrecked.
    pub driving: Option<(String, f32, bool)>,
    /// The point the crosshair is on: on foot, or driving a vehicle with
    /// a gun.
    pub crosshair: Option<Vec3>,
    /// A vehicle in reach to get into.
    pub can_enter: Option<String>,
    /// Shown while the island or its people are still loading.
    pub loading: Option<String>,
    /// The mission's objectives and markers (`--mission`).
    pub mission: Option<MissionView>,
    /// World to clip, for placing markers.
    pub view_proj: Option<Mat4>,
    /// The on-screen touch controls to draw over play, once a finger has
    /// touched the screen ([`crate::touch`]).
    pub touch: Option<crate::touch::Picture>,
    /// What the game's HUD is told, if it runs.
    pub game: Option<HudState>,
    /// Her support: the item ready, the designator, the satellite's view,
    /// the timing wheel, the bomb's flash.
    pub support: SupportHud,
    /// The hero she is (the pause menu marks them).
    pub hero: crate::hero::Hero,
}

/// What the pause menu asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuAction {
    Resume,
    /// Write the settings file.
    SaveSettings,
    /// Save the game, or load the saved game.
    SaveGame,
    LoadGame,
    /// Play as another mercenary (also kept in the settings).
    Hero(crate::hero::Hero),
    /// Show the window another way (also kept in the settings).
    Display(crate::config::DisplayMode),
    Quit,
    /// Something done in the shop.
    Shop(ShopAction),
    /// Something done in the cheat menu.
    Cheat(crate::cheats::CheatAction),
}

/// What the player does in the shop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShopAction {
    /// Look at faction `i`'s shop ([`crate::mission::SHOPS`]).
    Faction(usize),
    /// Buy one of this.
    Buy(String),
    /// Every shop's items unlocked or not (the cheat).
    UnlockAll(bool),
    Close,
}

/// The shop up: whose, what it sells, and what the last purchase said.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShopScreen {
    pub faction: usize,
    pub view: crate::mission::ShopView,
    pub said: Option<String>,
    pub unlock_all: bool,
}

/// Seconds the controls hint shows at the start.
const HINT_TIME: f64 = 12.0;

/// The HUD and menu.
pub struct Hud {
    pub ctx: egui::Context,
    renderer: Option<Renderer>,
    /// Textures handed to the renderer and not freed (for `M2_PAINT_LOG`).
    uploaded: std::collections::HashSet<egui::TextureId>,
    /// The pause menu is open.
    pub menu: bool,
    /// The shop is up, in the pause menu's place.
    pub shop: Option<ShopScreen>,
    /// The cheat menu is up, in the pause menu's place.
    pub cheats: Option<crate::cheats::CheatScreen>,
    /// Tessellated and waiting to paint: shapes, texture changes, scale.
    frame: Option<(Vec<egui::ClippedPrimitive>, egui::TexturesDelta, f32)>,
    /// The game's own HUD, if its files are there.
    pub game: Option<GameHud>,
    /// The game's loading screen, if `Loading.wad` reads.
    pub loading: Option<crate::loading::LoadingScreen>,
    /// Each frame waits for the game's HUD to draw it (play run offscreen,
    /// faster than the HUD's scripts would keep up with).
    pub wait: bool,
    /// The developer's keys work (`--dev`, or a test launch): the menu
    /// names F5/F9 for saving and loading, and loads while the mission
    /// scripts run, though a load puts the world back without their state.
    pub dev: bool,
    /// The player's settings and saves for the game's own pause menu
    /// ([`crate::pause`]), set as it opens.
    pub pause_settings: crate::pause::Settings,
    /// Keys and mouse buttons for the pause menu's keyboard table, not yet
    /// passed on.
    pub pause_captured: Vec<crate::input::Control>,
}

impl std::fmt::Debug for Hud {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hud")
            .field("menu", &self.menu)
            .finish_non_exhaustive()
    }
}

impl Drop for Hud {
    fn drop(&mut self) {
        // Texture changes never painted are dropped on purpose.
        if let Some((_, delta, _)) = &mut self.frame {
            delta.clear();
        }
    }
}

impl Default for Hud {
    fn default() -> Self {
        Self::new()
    }
}

impl Hud {
    pub fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.set_visuals(egui::Visuals::dark());
        Self {
            ctx,
            renderer: None,
            uploaded: Default::default(),
            menu: false,
            shop: None,
            cheats: None,
            frame: None,
            game: None,
            loading: None,
            wait: false,
            dev: true,
            pause_settings: crate::pause::Settings::default(),
            pause_captured: Vec::new(),
        }
    }

    /// Steps the game's HUD for this frame, if it runs.
    fn feed(&mut self, input: &RawInput, info: &HudInfo) {
        let (Some(game), Some(state)) = (&mut self.game, &info.game) else {
            return;
        };
        let mut state = state.clone();
        state.size = input
            .screen_rect
            .map_or([0.0; 2], |r| [r.width(), r.height()]);
        state.dt = game.frame_time(input.time);
        // The menu is the game's own pause menu, but in the shop.
        state.pause = self.menu && self.shop.is_none();
        state.pause_settings = self.pause_settings.clone();
        state.pause_captured = std::mem::take(&mut self.pause_captured);
        // The pause menu takes the mouse.
        if state.pause {
            let at = input.events.iter().rev().find_map(|e| match e {
                egui::Event::PointerMoved(p) => Some(*p),
                _ => None,
            });
            (state.pointer, state.clicks) = crate::gamehud::pointer_input(&input.events, at);
        }
        let message = info.mission.as_ref().and_then(|m| m.game_message.as_ref());
        state.messages.extend(game.message(message));
        state.blips = blips(info);
        state.reticle = reticle(info);
        state.support = info.support.clone();
        state.cash = info.mission.as_ref().and_then(|m| m.cash);
        state.radar_regions = info
            .mission
            .as_ref()
            .map_or_else(Vec::new, |m| m.radar_regions.clone());
        state.tutorial = info.mission.as_ref().and_then(|m| m.game_tutorial.clone());
        if let Some(m) = &info.mission {
            state.hud_calls = game.hud_calls(&m.hud_calls);
            state.key_names = m.key_names.clone();
            state.pda_objectives = m.pda_objectives.clone();
            state.trespass = m.trespass.clone();
            state.vehicle_disguise = m.vehicle_disguise;
            state.disguise = m.disguise;
        }
        // The HUD's movies take them while they hold the controls (the
        // PDA, a fanfare).
        {
            state.keys = input
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key {
                        key,
                        pressed,
                        repeat: false,
                        ..
                    } => Some((flash_key(*key)?, *pressed)),
                    _ => None,
                })
                .collect();
        }
        if self.wait {
            game.update_and_wait(&self.ctx, state, Duration::from_secs(10));
        } else {
            game.update(&self.ctx, state);
        }
    }

    /// Runs the game's HUD `seconds` on `info`, waiting for each step (and
    /// up to `timeout` for its scripts to start): for screenshots, which
    /// draw one frame. `keys` (Flash key codes) are pressed half way and
    /// held to the end; `typed`, they are typed one by one from a quarter
    /// of the way, a third of a second apart (a menu walked through).
    pub fn settle(
        &mut self,
        info: &HudInfo,
        size: (u32, u32),
        seconds: f64,
        (keys, clicks): (&[i32], &[(usize, [f32; 2])]),
        typed: bool,
        timeout: Duration,
    ) {
        let (Some(game), Some(state)) = (&mut self.game, &info.game) else {
            return;
        };
        let mut state = state.clone();
        state.size = [size.0 as f32, size.1 as f32];
        let message = info.mission.as_ref().and_then(|m| m.game_message.as_ref());
        state.messages.extend(game.message(message));
        state.blips = blips(info);
        state.reticle = reticle(info);
        state.support = info.support.clone();
        state.cash = info.mission.as_ref().and_then(|m| m.cash);
        state.radar_regions = info
            .mission
            .as_ref()
            .map_or_else(Vec::new, |m| m.radar_regions.clone());
        state.tutorial = info.mission.as_ref().and_then(|m| m.game_tutorial.clone());
        if let Some(m) = &info.mission {
            state.hud_calls = game.hud_calls(&m.hud_calls);
            state.key_names = m.key_names.clone();
            state.pda_objectives = m.pda_objectives.clone();
            state.trespass = m.trespass.clone();
            state.vehicle_disguise = m.vehicle_disguise;
            state.disguise = m.disguise;
        }
        let step = 1.0 / 30.0;
        game.update_and_wait(
            &self.ctx,
            HudState {
                dt: 0.0,
                ..state.clone()
            },
            timeout,
        );
        // Hurts and messages once; then time passes.
        state.hurts.clear();
        state.messages.clear();
        state.hud_calls.clear();
        state.dt = step;
        let steps = (seconds / step).ceil() as usize;
        for i in 0..steps {
            if !game.ready() {
                break;
            }
            // Held from half way to the last step (the PDA's map moves
            // while its keys are held).
            state.pointer = None;
            state.clicks.clear();
            state.keys = if typed {
                const GAP: usize = 10;
                match i.checked_sub(steps / 4) {
                    // A click (key -1): the pointer there, then the
                    // button down and up.
                    Some(j) if j / GAP < keys.len() && keys[j / GAP] == -1 => {
                        if let Some(&(_, p)) = clicks.iter().find(|(n, _)| *n == j / GAP)
                            && j % GAP < 3
                        {
                            state.pointer = Some(p);
                            if j % GAP > 0 {
                                state.clicks.push(j % GAP == 1);
                            }
                        }
                        Vec::new()
                    }
                    Some(j) if j / GAP < keys.len() && j % GAP < 2 => {
                        vec![(keys[j / GAP], j % GAP == 0)]
                    }
                    _ => Vec::new(),
                }
            } else {
                match i.checked_sub(steps / 2) {
                    Some(0) => keys.iter().map(|&k| (k, true)).collect(),
                    Some(_) if i + 1 == steps => keys.iter().map(|&k| (k, false)).collect(),
                    _ => Vec::new(),
                }
            };
            game.update_and_wait(&self.ctx, state.clone(), Duration::from_secs(10));
        }
    }

    /// Lays out this frame's HUD (and menu, if open) for `input`, editing
    /// `config` from the menu's settings. Returns what the menu asked for
    /// and egui's requests to the window (cursor, clipboard).
    pub fn build(
        &mut self,
        input: RawInput,
        info: &HudInfo,
        config: &mut Config,
    ) -> (Option<MenuAction>, egui::PlatformOutput) {
        let mut action = None;
        let menu = self.menu;
        self.feed(&input, info);
        let game = self.game.as_ref().filter(|g| g.ready());
        // Without the developer's keys, no load while the scripts run: it
        // would put the world back without their state.
        let load = self.dev || info.mission.is_none();
        let dev = self.dev;
        let shop = self.shop.as_ref();
        let cheats = self.cheats.as_ref();
        // The game's own pause menu, once its scripts run: ours keeps the
        // alpha's own settings beside it.
        let own = self.game.as_ref().is_some_and(GameHud::pause_available);
        // The loading screen: up while the island loads and over the
        // scripts' fades to black.
        let mut loading = self.loading.as_mut();
        if let Some(screen) = loading.as_deref_mut() {
            let black = info.mission.as_ref().map_or(0.0, |m| m.black);
            screen.update(info.loading.is_some(), black, input.time.unwrap_or(0.0));
            for line in screen.take_log() {
                eprintln!("{line}");
            }
        }
        let covered = loading
            .as_deref()
            .is_some_and(crate::loading::LoadingScreen::showing);
        let output = self.ctx.run_ui(input, |ui| {
            hud(ui, info, game, covered);
            if let (false, false, Some(touch)) = (menu, covered, &info.touch) {
                let ctx = ui.ctx().clone();
                let painter = ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Foreground,
                    "touch controls".into(),
                ));
                crate::touch::paint(touch, &painter, ctx.content_rect(), ctx.pixels_per_point());
            }
            if let Some(screen) = loading.as_deref_mut() {
                let ctx = ui.ctx().clone();
                let painter = ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Foreground,
                    "loading screen".into(),
                ));
                screen.paint(&ctx, &painter);
            }
            if let (true, Some(cheats)) = (menu, cheats) {
                action = cheat_window(ui.ctx(), cheats).map(MenuAction::Cheat);
            } else if let (true, Some(shop)) = (menu, shop) {
                action = shop_window(ui.ctx(), shop).map(MenuAction::Shop);
            } else if menu {
                action = pause_menu(ui.ctx(), config, info.hero, dev, load, own);
            }
        });
        let ppp = output.pixels_per_point;
        let shapes = self.ctx.tessellate(output.shapes, ppp);
        // Texture changes accumulate until painted.
        let delta = match self.frame.take() {
            Some((_, mut old, _)) => {
                old.append(output.textures_delta);
                old
            }
            None => output.textures_delta,
        };
        self.frame = Some((shapes, delta, ppp));
        (action, output.platform_output)
    }

    /// Lays out a screen of its own in place of the HUD (the movies and
    /// the front end, [`crate::flow`]), for [`paint`](Self::paint).
    pub fn build_screen(
        &mut self,
        input: RawInput,
        screen: impl FnMut(&mut egui::Ui),
    ) -> egui::PlatformOutput {
        let output = self.ctx.run_ui(input, screen);
        let ppp = output.pixels_per_point;
        let shapes = self.ctx.tessellate(output.shapes, ppp);
        let delta = match self.frame.take() {
            Some((_, mut old, _)) => {
                old.append(output.textures_delta);
                old
            }
            None => output.textures_delta,
        };
        self.frame = Some((shapes, delta, ppp));
        output.platform_output
    }

    /// Paints what [`build`](Self::build) laid out.
    pub fn paint(&mut self, t: OverlayTarget<'_>) {
        let Some((shapes, mut delta, ppp)) = self.frame.take() else {
            return;
        };
        let renderer = self.renderer.get_or_insert_with(|| {
            let mut r = Renderer::new(t.device, t.format, RendererOptions::default());
            crate::multiply::Resources::install(&mut r.callback_resources, t.device, t.format);
            r
        });
        for (id, images) in &delta.set {
            for image in images {
                renderer.update_texture(t.device, t.queue, *id, image);
            }
            self.uploaded.insert(*id);
        }
        if std::env::var_os("M2_PAINT_LOG").is_some() {
            log_primitives(&shapes, &delta, &self.uploaded, t.format, t.size);
        }
        let screen = ScreenDescriptor {
            size_in_pixels: [t.size.0, t.size.1],
            pixels_per_point: ppp,
        };
        let extra = renderer.update_buffers(t.device, t.queue, t.encoder, &shapes, &screen);
        if !extra.is_empty() {
            t.queue.submit(extra);
        }
        {
            let pass = t.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hud"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: t.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            renderer.render(&mut pass.forget_lifetime(), &shapes, &screen);
        }
        for id in &delta.free {
            self.uploaded.remove(id);
            renderer.free_texture(id);
        }
        delta.clear();
    }
}

/// Test squares (`M2_PAINT_TEST` set), 100 px each along the top from
/// x 20: opaque magenta, opaque black, black at alpha 0.75, mid grey;
/// to see whether solid fills reach the picture at all.
fn paint_test(painter: &egui::Painter) {
    let colours = [
        Color32::from_rgb(255, 0, 255),
        Color32::BLACK,
        Color32::from_rgba_unmultiplied(0, 0, 0, 192),
        Color32::from_gray(128),
    ];
    for (i, c) in colours.into_iter().enumerate() {
        let x = 20.0 + 120.0 * i as f32;
        painter.rect_filled(
            egui::Rect::from_min_size(pos2(x, 120.0), vec2(100.0, 100.0)),
            0.0,
            c,
        );
    }
}

/// What goes to the GPU (`M2_PAINT_LOG` set): each mesh's texture, its
/// vertices, bounds, clip and colours, to find HUD fills lost on the way.
fn log_primitives(
    shapes: &[egui::ClippedPrimitive],
    delta: &egui::TexturesDelta,
    uploaded: &std::collections::HashSet<egui::TextureId>,
    format: wgpu::TextureFormat,
    size: (u32, u32),
) {
    let set: Vec<_> = delta.set.keys().copied().collect();
    eprintln!(
        "paint: {} primitives to {format:?} {size:?}; textures set {set:?}",
        shapes.len()
    );
    for p in shapes {
        if let egui::epaint::Primitive::Mesh(m) = &p.primitive {
            let b = m.calc_bounds();
            let alpha = m.vertices.iter().map(|v| v.color.a());
            let (lo, hi) = alpha.fold((255, 0), |(lo, hi), a| (lo.min(a), hi.max(a)));
            let dark = m
                .vertices
                .iter()
                .filter(|v| v.color.r() < 16 && v.color.a() > 64)
                .count();
            eprintln!(
                "paint: {:?}{} {} verts {} indices, bounds {:.0?}..{:.0?}, clip {:.0?}..{:.0?}, alpha {lo}..{hi}, {dark} dark",
                m.texture_id,
                if uploaded.contains(&m.texture_id) {
                    ""
                } else {
                    " (NOT UPLOADED)"
                },
                m.vertices.len(),
                m.indices.len(),
                b.min,
                b.max,
                p.clip_rect.min,
                p.clip_rect.max,
            );
        }
    }
}

/// Input for a frame with no window events (screenshots): the screen size
/// in pixels, at one pixel per point.
pub fn headless_input(width: u32, height: u32, time: f64) -> RawInput {
    RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            pos2(0.0, 0.0),
            vec2(width as f32, height as f32),
        )),
        time: Some(time),
        ..RawInput::default()
    }
}

/// Where the reticle goes: the crosshair's point on the screen, -1..=1
/// across and up it. None while she is down, aims at nothing (in a
/// vehicle without a gun) or the point is behind the camera.
fn reticle(info: &HudInfo) -> Option<[f32; 2]> {
    if info.down {
        return None;
    }
    let (at, vp) = info.crosshair.zip(info.view_proj)?;
    let clip = vp * at.extend(1.0);
    if clip.w <= 0.01 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    Some([ndc.x.clamp(-1.0, 1.0), ndc.y.clamp(-1.0, 1.0)])
}

/// The mission's markers, for the game HUD's minimap.
fn blips(info: &HudInfo) -> Vec<([f32; 3], String, [u8; 4])> {
    info.mission.as_ref().map_or_else(Vec::new, |m| {
        m.markers
            .iter()
            .map(|m| (m.at.to_array(), m.icon.clone(), m.colour))
            .collect()
    })
}

/// The HUD for `info`; `covered`: the game's loading screen is up, in
/// place of the fade and the loading line.
fn hud(ui: &mut egui::Ui, info: &HudInfo, game: Option<&GameHud>, covered: bool) {
    let ctx = ui.ctx().clone();
    let screen = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::background());
    let text = |at, align, s: &str, size, colour| {
        // A shadow keeps text readable over bright sky.
        painter.text(
            at + vec2(1.5, 1.5),
            align,
            s,
            FontId::proportional(size),
            Color32::from_black_alpha(180),
        );
        painter.text(at, align, s, FontId::proportional(size), colour);
    };
    let white = Color32::from_gray(235);
    flash(&ctx, screen, info.support.flash);
    // The scripts' fade to black (`MrxGui.GlobalFadeToBlack`), over all,
    // unless its movie (the loading screen) plays it.
    if let Some(black) = info
        .mission
        .as_ref()
        .map(|m| m.black)
        .filter(|&b| b > 0.0 && !covered)
    {
        ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, "fade".into()))
            .rect_filled(
                screen,
                0.0,
                Color32::from_black_alpha((black.min(1.0) * 255.0) as u8),
            );
    }
    if let Some(loading) = info.loading.as_ref().filter(|_| !covered) {
        text(
            screen.center_top() + vec2(0.0, 24.0),
            Align2::CENTER_TOP,
            loading,
            18.0,
            white,
        );
    }
    let time = ctx.input(|i| i.time);
    if let Some(view) = info.support.satellite {
        // The game's own view (`MrxGuiSatelliteLayout`) once its HUD runs,
        // with the objective under it; ours until then.
        match game.filter(|_| info.playing) {
            Some(game) => {
                game.paint(&painter);
                satellite_objective(&painter, screen, info, &text);
            }
            None => satellite(&painter, screen, info, view, &text),
        }
        return;
    }
    // The PDA over the world: only the game's HUD (its movie) shows, as
    // the wiki's picture of it has it.
    if info.support.pda
        && info.playing
        && let Some(game) = game
    {
        game.paint(&painter);
        return;
    }
    // Ours, until the game's HUD runs.
    if time < HINT_TIME && info.playing && game.is_none() {
        text(
            screen.center_top() + vec2(0.0, 56.0),
            Align2::CENTER_TOP,
            "Click to look around. WASD move, Shift sprint, Space jump, E get in, \
             left button fire, right button aim, 1/2 weapons, G grenade, B bash, Esc menu",
            15.0,
            white,
        );
    }
    if info.playing
        && let Some(game) = game
    {
        // The scripts' markers in the world, under the HUD.
        if let (Some(m), Some(vp)) = (&info.mission, info.view_proj) {
            let here = info.game.as_ref().and_then(|s| s.position).map(Vec3::from);
            ground_discs(&painter, screen, vp, m, here);
            world_markers(&painter, screen, vp, m, game);
        }
        game.paint(&painter);
        if std::env::var_os("M2_PAINT_TEST").is_some() {
            paint_test(&painter);
        }
    }
    if let Some(m) = &info.mission {
        // The game's HUD shows the messages in its message buffer, and
        // the markers as the game draws them. Nor, when its fanfare call
        // failed in its scripts, the fanfare.
        let dropped = game.is_some_and(|g| g.dropped_fanfare());
        mission(
            &painter,
            screen,
            info.view_proj,
            m,
            game.is_none(),
            dropped,
            &text,
        );
    }
    if !info.playing {
        text(
            screen.left_bottom() + vec2(20.0, -20.0),
            Align2::LEFT_BOTTOM,
            "Free camera (V to play)",
            16.0,
            white,
        );
        return;
    }
    if game.is_none() {
        stand_in(&painter, screen, info, &text);
    }
    // What the use key does here: with a pad, after its Y (the layout's
    // `Action`) as the game draws it.
    let use_prompt = |what: &str| {
        let at = screen.center() + vec2(0.0, 60.0);
        let pad = info.game.as_ref().is_some_and(|s| s.pad);
        if let Some(game) = game.filter(|_| pad) {
            let size = 17.0;
            let width = ctx.fonts_mut(|f| {
                f.layout_no_wrap(what.to_owned(), FontId::proportional(size), white)
                    .size()
                    .x
            });
            let icon = size * 1.6;
            let left = at.x - (icon + 6.0 + width) / 2.0;
            let r = egui::Rect::from_min_size(egui::pos2(left, at.y - 4.0), vec2(icon, icon));
            game.paint_texture(&painter, crate::pad::Button::Y.texture(), r, Color32::WHITE);
            text(
                egui::pos2(left + icon + 6.0, at.y),
                Align2::LEFT_TOP,
                what,
                size,
                white,
            );
        } else {
            text(at, Align2::CENTER_TOP, &format!("E: {what}"), 17.0, white);
        }
    };
    if let Some(keys) = &info.hijack_keys {
        // Under the game's own arrow (the HUD's `MrxGuiHudActionHijack`).
        text(
            screen.center() + vec2(0.0, 60.0),
            Align2::CENTER_TOP,
            keys,
            17.0,
            white,
        );
    }
    if let Some(prompt) = &info.prompt {
        use_prompt(prompt);
    } else if let Some(name) = &info.can_enter
        && info.driving.is_none()
    {
        use_prompt(&format!(
            "get in the {}",
            name.trim_start_matches("global_veh_")
        ));
    }
    if info.down {
        text(
            screen.center(),
            Align2::CENTER_CENTER,
            "Down! Back on your feet in a moment.",
            26.0,
            Color32::from_rgb(240, 90, 80),
        );
    }
    // The game's HUD has its own support box; the bomb's countdown and the
    // PDA are ours with either.
    if info.driving.is_none() {
        support(screen, &info.support, game.is_none(), &text);
    }
}

/// Our own health bar, weapon line and crosshair, until the game's HUD
/// runs.
fn stand_in<T: Fn(egui::Pos2, Align2, &str, f32, Color32)>(
    painter: &egui::Painter,
    screen: egui::Rect,
    info: &HudInfo,
    text: &T,
) {
    let white = Color32::from_gray(235);
    // Health, bottom left.
    let bar =
        egui::Rect::from_min_size(screen.left_bottom() + vec2(24.0, -44.0), vec2(220.0, 16.0));
    painter.rect_filled(bar.expand(2.0), 3.0, Color32::from_black_alpha(160));
    let frac = (info.hp / 100.0).clamp(0.0, 1.0);
    let colour = if frac < 0.3 {
        Color32::from_rgb(220, 50, 40)
    } else {
        Color32::from_rgb(90, 200, 90)
    };
    painter.rect_filled(
        egui::Rect::from_min_size(bar.min, vec2(bar.width() * frac, bar.height())),
        2.0,
        colour,
    );
    text(
        bar.left_top() + vec2(0.0, -6.0),
        Align2::LEFT_BOTTOM,
        "Health",
        14.0,
        white,
    );

    match &info.driving {
        Some((label, speed, wrecked)) => {
            let name = label.trim_start_matches("global_veh_");
            let line = if *wrecked {
                format!("{name}: wrecked (E to get out)")
            } else {
                format!("{name}  {:.0} km/h", speed.abs() * 3.6)
            };
            text(
                screen.right_bottom() + vec2(-24.0, -28.0),
                Align2::RIGHT_BOTTOM,
                &line,
                18.0,
                white,
            );
            // Its gun's crosshair, where the gun aims.
            if let Some(at) = info
                .crosshair
                .zip(info.view_proj)
                .and_then(|(p, vp)| to_screen(vp, p, screen))
            {
                crosshair(painter, at);
            }
        }
        None => {
            let name = |w: Weapon| match w {
                Weapon::Rifle => "rifle",
                Weapon::Rpg => "RPG",
                Weapon::GrenadeLauncher => "grenade launcher",
            };
            let launcher = |w: Weapon| match (&info.launcher, w) {
                (n, _) if !n.is_empty() => n.clone(),
                (_, Weapon::GrenadeLauncher) => "Grenade launcher".to_owned(),
                _ => "RPG".to_owned(),
            };
            let up = match (info.weapon, info.rounds) {
                (Weapon::Rifle, Some((clip, stored))) => {
                    format!("{} {clip} | {stored}", info.rifle)
                }
                (Weapon::Rifle, None) => info.rifle.clone(),
                (w, Some((clip, stored))) => {
                    format!("{} {clip} | {stored}", launcher(w))
                }
                (w, None) => launcher(w),
            };
            let other = match (info.weapon, info.second) {
                (Weapon::Rifle, Some(_)) if !info.launcher.is_empty() => {
                    format!("   ·   2 {}", info.launcher)
                }
                (Weapon::Rifle, Some(w)) => format!("   ·   2 {}", name(w)),
                (Weapon::Rifle, None) => String::new(),
                _ => "   ·   1 rifle".to_owned(),
            };
            // Holding the designator, it is what is up.
            let (up, other) = match &info.support.designator {
                Some(designator) => (designator.clone(), "   ·   1/2 put it away".to_owned()),
                None => (up, other),
            };
            text(
                screen.right_bottom() + vec2(-24.0, -28.0),
                Align2::RIGHT_BOTTOM,
                &format!(
                    "{up}{other}   ·   G {} ({})",
                    if info.explosive.is_empty() {
                        "grenade"
                    } else {
                        &info.explosive
                    },
                    info.grenades
                ),
                18.0,
                white,
            );
            // Where she aims, a little above the middle.
            if !info.down {
                let at = info
                    .crosshair
                    .zip(info.view_proj)
                    .and_then(|(p, vp)| to_screen(vp, p, screen))
                    .unwrap_or(screen.center());
                crosshair(painter, at);
            }
        }
    }
}

/// The support box above the health bar (if `boxed`: the game's HUD has
/// its own): the item ready to call (its name, how many, whether free,
/// how it is called in) and the keys for the step the call is at; and
/// while a bomb is on its way, how long.
fn support<T: Fn(egui::Pos2, Align2, &str, f32, Color32)>(
    screen: egui::Rect,
    s: &SupportHud,
    boxed: bool,
    text: &T,
) {
    let white = Color32::from_gray(235);
    if let Some(left) = s.inbound {
        text(
            screen.center_top() + vec2(0.0, 130.0),
            Align2::CENTER_TOP,
            &format!("Bomb inbound: {:.0} s", left.ceil()),
            20.0,
            Color32::from_rgb(240, 120, 60),
        );
    }
    // The game's HUD draws its own PDA.
    if s.pda && boxed {
        pda(screen, s, text);
    }
    let Some((name, method, count, free)) = s.item.as_ref().filter(|_| boxed) else {
        return;
    };
    let at = screen.left_bottom() + vec2(24.0, -78.0);
    let free = if *free { " FREE" } else { "" };
    text(
        at,
        Align2::LEFT_BOTTOM,
        &format!("{count}{free}   ·   {method}"),
        14.0,
        Color32::from_rgb(110, 220, 110),
    );
    text(
        at + vec2(0.0, -18.0),
        Align2::LEFT_BOTTOM,
        name,
        16.0,
        white,
    );
    let keys = if s.menu {
        "E: use support item   ·   T: close"
    } else {
        "T: support menu   ·   P: PDA"
    };
    text(
        at + vec2(0.0, -40.0),
        Align2::LEFT_BOTTOM,
        keys,
        13.0,
        Color32::from_gray(200),
    );
}

/// The PDA, a stand-in until the game's own screens are drawn: what she
/// has in her stockpile, and the key to close it.
fn pda<T: Fn(egui::Pos2, Align2, &str, f32, Color32)>(
    screen: egui::Rect,
    s: &SupportHud,
    text: &T,
) {
    let gold = Color32::from_rgb(255, 200, 0);
    let white = Color32::from_gray(235);
    let at = screen.center() + vec2(-200.0, -110.0);
    text(at, Align2::LEFT_TOP, "PDA", 24.0, gold);
    text(
        at + vec2(0.0, 44.0),
        Align2::LEFT_TOP,
        "Stockpile",
        17.0,
        gold,
    );
    let stock = match &s.item {
        Some((name, method, count, free)) => {
            let free = if *free { "  FREE" } else { "" };
            format!("{name}   x{count}{free}   ·   {method}")
        }
        None => "Nothing to call in".to_owned(),
    };
    text(at + vec2(0.0, 70.0), Align2::LEFT_TOP, &stock, 16.0, white);
    text(
        at + vec2(0.0, 200.0),
        Align2::LEFT_TOP,
        "P: close the PDA",
        14.0,
        Color32::from_gray(200),
    );
}

/// The bomb's flash over everything, `strength` 1 at its brightest.
fn flash(ctx: &egui::Context, screen: egui::Rect, strength: f32) {
    if strength <= 0.0 {
        return;
    }
    let alpha = (255.0 * strength.min(1.0)) as u8;
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("flash"),
    ))
    .rect_filled(
        screen,
        0.0,
        Color32::from_rgba_unmultiplied(255, 248, 220, alpha),
    );
}

/// Ruler ticks every this many metres, longer every fifth; the rulers are
/// this wide (points).
const TICK: f32 = 10.0;
const RULER: f32 = 20.0;

/// The satellite's view, as round 93's videos show it (its grey and
/// grain are the post pass's): rulers along the top (x) and the left (z,
/// north up) marking where the view's middle is, a crosshair, the operation's time and cost, the
/// objective, the keys, and once the target is confirmed the timing
/// wheel. `view` is the view's middle on the ground, seconds since it
/// opened and the cost ($K).
fn satellite<T: Fn(egui::Pos2, Align2, &str, f32, Color32)>(
    painter: &egui::Painter,
    screen: egui::Rect,
    info: &HudInfo,
    (middle, open, cost): (Vec3, f32, f32),
    text: &T,
) {
    let white = Color32::from_gray(235);
    let ink = Color32::from_white_alpha(200);
    // Its grey and grain are the post pass's (`game::satellite_look`).

    // The rulers, and on each the view's middle.
    // Short, as round 95 saw them in the videos: a third of the width
    // along the top, 40% of the height a sixth of the way in from the
    // left, their ticks unnumbered.
    let top = Rect::from_center_size(
        pos2(screen.center().x, screen.top() + screen.height() * 0.06),
        vec2(screen.width() / 3.0, RULER),
    );
    let left = Rect::from_center_size(
        pos2(screen.left() + screen.width() * 0.17, screen.center().y),
        vec2(RULER, screen.height() * 0.4),
    );
    for r in [top, left] {
        painter.rect_filled(r, 0.0, Color32::from_black_alpha(150));
    }
    let centre = screen.center();
    if let Some(vp) = info.view_proj {
        let first = |v: f32| (v / TICK).round() as i32;
        for k in first(middle.x) - 60..=first(middle.x) + 60 {
            let x = k as f32 * TICK;
            let Some(p) = project(vp, Vec3::new(x, middle.y, middle.z), screen) else {
                continue;
            };
            if !(top.left()..=top.right()).contains(&p.x) {
                continue;
            }
            let long = if k % 5 == 0 { 12.0 } else { 6.0 };
            painter.line_segment(
                [pos2(p.x, top.bottom() - long), pos2(p.x, top.bottom())],
                Stroke::new(1.0, ink),
            );
        }
        for k in first(middle.z) - 60..=first(middle.z) + 60 {
            let z = k as f32 * TICK;
            let Some(p) = project(vp, Vec3::new(middle.x, middle.y, z), screen) else {
                continue;
            };
            if !(left.top()..=left.bottom()).contains(&p.y) {
                continue;
            }
            let long = if k % 5 == 0 { 12.0 } else { 6.0 };
            painter.line_segment(
                [pos2(left.right() - long, p.y), pos2(left.right(), p.y)],
                Stroke::new(1.0, ink),
            );
        }
    }
    // Triangles on the rulers point at the middle, its coordinates beside
    // them.
    let tri = |a: egui::Pos2, b: egui::Pos2, c: egui::Pos2, colour: Color32| {
        painter.add(Shape::convex_polygon(vec![a, b, c], colour, Stroke::NONE));
    };
    let x = centre.x;
    tri(
        pos2(x, top.bottom() + 1.0),
        pos2(x - 7.0, top.bottom() + 12.0),
        pos2(x + 7.0, top.bottom() + 12.0),
        white,
    );
    text(
        pos2(x, top.bottom() + 14.0),
        Align2::CENTER_TOP,
        &format!("{:.2}", middle.x),
        14.0,
        white,
    );
    let y = centre.y;
    tri(
        pos2(left.right() + 1.0, y),
        pos2(left.right() + 12.0, y - 7.0),
        pos2(left.right() + 12.0, y + 7.0),
        white,
    );
    text(
        pos2(left.right() + 15.0, y),
        Align2::LEFT_CENTER,
        &format!("{:.2}", middle.z),
        14.0,
        white,
    );
    // North, up, just above the left ruler.
    let north = pos2(left.center().x, left.top() - 30.0);
    text(north, Align2::CENTER_TOP, "N", 13.0, white);
    tri(
        north + vec2(0.0, 15.0),
        north + vec2(-5.0, 24.0),
        north + vec2(5.0, 24.0),
        white,
    );

    // The crosshair: a small plus, open in the middle.
    let stroke = Stroke::new(2.0, Color32::from_white_alpha(230));
    for d in [
        vec2(1.0, 0.0),
        vec2(-1.0, 0.0),
        vec2(0.0, 1.0),
        vec2(0.0, -1.0),
    ] {
        painter.line_segment([centre + d * 5.0, centre + d * 16.0], stroke);
    }

    // The operation's time (minutes, seconds and hundredths, as the
    // videos count it) and cost, in boxes below and right of the middle.
    let hundredths = (open.max(0.0) * 100.0) as u32;
    let readouts = [
        (
            "OPERATION TIME",
            format!(
                "{:02}:{:02}:{:02}",
                hundredths / 6000,
                hundredths / 100 % 60,
                hundredths % 100
            ),
        ),
        ("OPERATION COST", format!("${cost:.1} K")),
    ];
    for (i, (label, value)) in readouts.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(
                screen.left() + screen.width() * 0.6,
                screen.top() + screen.height() * (0.7 + i as f32 * 0.12),
            ),
            vec2(screen.width() * 0.15, 34.0),
        );
        painter.rect_filled(r, 2.0, Color32::from_black_alpha(150));
        painter.rect_stroke(
            r,
            2.0,
            Stroke::new(1.0, Color32::from_white_alpha(150)),
            StrokeKind::Inside,
        );
        text(
            r.left_top() + vec2(0.0, -3.0),
            Align2::LEFT_BOTTOM,
            label,
            11.0,
            ink,
        );
        text(r.center(), Align2::CENTER_CENTER, value, 18.0, white);
    }

    // The objective, bottom left; the keys, right of the middle.
    satellite_objective(painter, screen, info, text);
    let keys: &[&str] = if info.support.wheel.is_some() {
        &["T: back"]
    } else {
        &[
            "WASD or the mouse: move view",
            "Fire or E: confirm target",
            "T: back",
        ]
    };
    for (i, k) in keys.iter().enumerate() {
        text(
            pos2(screen.right() - 24.0, centre.y - 20.0 + i as f32 * 20.0),
            Align2::RIGHT_CENTER,
            k,
            14.0,
            white,
        );
    }

    if let Some(w) = &info.support.wheel {
        wheel(
            painter,
            centre,
            screen.height().min(screen.width()) * 0.13,
            w,
        );
        text(
            centre + vec2(0.0, screen.height() * 0.2),
            Align2::CENTER_TOP,
            "Fire as the marker passes through each grey wedge",
            17.0,
            white,
        );
    }
}

/// The mission's objective at the bottom left of the satellite's view,
/// in gold after a triangle pointing down, as the videos show it.
fn satellite_objective<T: Fn(egui::Pos2, Align2, &str, f32, Color32)>(
    painter: &egui::Painter,
    screen: egui::Rect,
    info: &HudInfo,
    text: &T,
) {
    let Some(objective) = info.mission.as_ref().and_then(|m| m.objectives.first()) else {
        return;
    };
    let gold = Color32::from_rgb(255, 200, 0);
    // The triangle drawn: not every font has the glyph.
    let at = screen.left_bottom() + vec2(24.0, -32.0);
    painter.add(Shape::convex_polygon(
        vec![at, at + vec2(12.0, 0.0), at + vec2(6.0, 9.0)],
        gold,
        Stroke::NONE,
    ));
    text(
        screen.left_bottom() + vec2(42.0, -24.0),
        Align2::LEFT_BOTTOM,
        objective,
        17.0,
        gold,
    );
}

/// The timing wheel centred on `c`, `r` across at its full size: a
/// translucent disc with rings and spokes, its grey wedges (green once
/// pressed in), the marker on the rim, and a check or a cross for the last
/// press.
fn wheel(painter: &egui::Painter, c: egui::Pos2, r: f32, w: &WheelHud) {
    // Clockwise from the top, on the screen (y down).
    let dir = |a: f32| vec2(a.sin(), -a.cos());
    let r = r * w.size.clamp(0.0, 1.0);
    if r < 1.0 {
        return;
    }
    let ring = Stroke::new(1.5, Color32::from_white_alpha(150));
    let lit = w.mark == Some(true);
    painter.circle_filled(
        c,
        r,
        if lit {
            Color32::from_rgba_unmultiplied(90, 210, 90, 70)
        } else {
            Color32::from_white_alpha(40)
        },
    );
    for f in [1.0, 0.68, 0.36] {
        painter.circle_stroke(c, r * f, ring);
    }
    for k in 0..12 {
        let d = dir(k as f32 * TAU / 12.0);
        painter.line_segment(
            [c + d * r * 0.36, c + d * r],
            Stroke::new(1.0, Color32::from_white_alpha(90)),
        );
    }
    for (mid, hit) in w.sectors {
        let mut points = vec![c];
        points
            .extend((0..=12).map(|k| c + dir(mid - w.wedge + 2.0 * w.wedge * k as f32 / 12.0) * r));
        let fill = if hit {
            Color32::from_rgba_unmultiplied(60, 150, 60, 170)
        } else {
            Color32::from_rgba_unmultiplied(60, 60, 60, 190)
        };
        painter.add(Shape::convex_polygon(points, fill, Stroke::NONE));
    }
    let m = c + dir(w.angle) * r;
    painter.circle_filled(m, 12.0, Color32::from_rgb(70, 190, 70));
    painter.circle_stroke(m, 12.0, Stroke::new(2.0, Color32::WHITE));
    match w.mark {
        Some(true) => {
            let s = r * 0.3;
            let green = Stroke::new(7.0, Color32::from_rgb(70, 200, 70));
            painter.line_segment([c + vec2(-0.9, 0.0) * s, c + vec2(-0.3, 0.6) * s], green);
            painter.line_segment([c + vec2(-0.3, 0.6) * s, c + vec2(0.9, -0.7) * s], green);
        }
        Some(false) => {
            let s = r * 0.25;
            let red = Stroke::new(7.0, Color32::from_rgb(220, 50, 40));
            painter.line_segment([c + vec2(-s, -s), c + vec2(s, s)], red);
            painter.line_segment([c + vec2(-s, s), c + vec2(s, -s)], red);
        }
        None => {}
    }
}

/// The mission's objectives (top left), its newest message (top centre),
/// tutorial line (left) and markers over its targets, if `message` (no
/// game HUD to show them). The contract's end shows if `message`, or if
/// `dropped` (the game HUD's fanfare call failed).
fn mission<T: Fn(egui::Pos2, Align2, &str, f32, Color32)>(
    painter: &egui::Painter,
    screen: egui::Rect,
    view_proj: Option<Mat4>,
    m: &MissionView,
    message: bool,
    dropped: bool,
    text: &T,
) {
    let white = Color32::from_gray(235);
    let gold = Color32::from_rgb(255, 200, 0);
    let mut at = screen.left_top() + vec2(20.0, 20.0);
    // Ours, as the rest, until the game's HUD runs (which has no such
    // list on screen).
    if let Some(status) = m.status.as_ref().filter(|_| message) {
        text(at, Align2::LEFT_TOP, status, 15.0, white);
        at.y += 22.0;
    }
    if !m.objectives.is_empty() && message {
        text(at, Align2::LEFT_TOP, "Objectives", 16.0, gold);
        at.y += 22.0;
        for o in &m.objectives {
            text(at, Align2::LEFT_TOP, &format!("•  {o}"), 15.0, white);
            at.y += 20.0;
        }
    }
    // The game's HUD has its own tutorial box.
    if let Some(line) = m.tutorial.as_ref().filter(|_| message) {
        tutorial(painter, screen, line);
    }
    // The game's HUD has its own fanfare (`Hud.Fanfare`), unless its call
    // failed there. Ours stays up while the fanfare waits for Enter, as the
    // game's does.
    if (m.contract_complete || m.fanfare_waiting) && (message || dropped) {
        text(
            screen.center() + vec2(0.0, -60.0),
            Align2::CENTER_CENTER,
            "Contract complete",
            40.0,
            gold,
        );
        if m.fanfare_waiting {
            text(
                screen.center() + vec2(0.0, -20.0),
                Align2::CENTER_CENTER,
                "Enter to continue",
                18.0,
                white,
            );
        }
    }
    if let Some((done, line)) = m.message.as_ref().filter(|_| message) {
        let head = if *done {
            "Objective complete"
        } else {
            "New objective"
        };
        text(
            screen.center_top() + vec2(0.0, 90.0),
            Align2::CENTER_TOP,
            &format!("{head}: {line}"),
            20.0,
            gold,
        );
    }
    // The game's HUD draws the markers' icons ([`world_markers`]).
    let Some(vp) = view_proj.filter(|_| message) else {
        return;
    };
    for marker in &m.markers {
        let Some(p) = to_screen(vp, marker.at, screen) else {
            continue;
        };
        let [r, g, b, _] = marker.colour;
        let colour = Color32::from_rgb(r, g, b);
        painter.circle_stroke(p, 9.0, Stroke::new(2.5, colour));
        text(
            p + vec2(0.0, -14.0),
            Align2::CENTER_BOTTOM,
            &marker.label(),
            14.0,
            colour,
        );
    }
}

/// How far off a disc on the ground is drawn, metres (invented).
const DISC_RANGE: f32 = 150.0;

/// The scripts' discs on the ground (`Marker.AddDisc`: the wardrobe's, a
/// delivery's drop-off): a ring of their radius in their colour, filled
/// faintly, flat at its height. Drawn over the world as the markers are,
/// so not hidden behind walls (inferred: the engine's own drawing of
/// them isn't known); one partly behind the camera isn't drawn.
fn ground_discs(
    painter: &egui::Painter,
    screen: egui::Rect,
    view_proj: Mat4,
    m: &MissionView,
    here: Option<Vec3>,
) {
    const SIDES: usize = 48;
    for &(at, radius, [r, g, b]) in &m.discs {
        if here.is_some_and(|h| h.distance(at) > DISC_RANGE) {
            continue;
        }
        let points: Option<Vec<egui::Pos2>> = (0..SIDES)
            .map(|i| {
                let a = i as f32 / SIDES as f32 * std::f32::consts::TAU;
                project(
                    view_proj,
                    at + Vec3::new(a.cos(), 0.0, a.sin()) * radius,
                    screen,
                )
            })
            .collect();
        let Some(points) = points else { continue };
        painter.add(egui::Shape::convex_polygon(
            points,
            Color32::from_rgba_unmultiplied(r, g, b, 50),
            Stroke::new(2.0, Color32::from_rgba_unmultiplied(r, g, b, 200)),
        ));
    }
}

/// The scripts' markers (`Marker.AddBlip`) over the world, under the
/// game's HUD: each its icon (the HUD's texture) tinted its colour,
/// centred on its point, its alpha scaled by its fade from the camera
/// ([`MarkerView::alpha_from`]; from the 360 code: the marker renderer
/// measures from its view's camera position, the translation of the
/// camera-to-world matrix the view keeps). Its size is taken as the HUD's
/// 640x480 units (inferred: the call's units aren't known), and one off
/// the screen isn't drawn (nor pinned to its edge).
///
/// [`MarkerView::alpha_from`]: crate::mission::MarkerView::alpha_from
fn world_markers(
    painter: &egui::Painter,
    screen: egui::Rect,
    view_proj: Mat4,
    m: &MissionView,
    game: &GameHud,
) {
    let unit = screen.height() / m2_game::gui::VIRTUAL_SIZE.1;
    let eye = camera_position(view_proj);
    for marker in &m.markers {
        let fade = eye.map_or(1.0, |e| marker.alpha_from(e));
        if fade <= 0.0 {
            continue;
        }
        let Some(p) = project(view_proj, marker.at, screen) else {
            continue;
        };
        let rect = Rect::from_center_size(p, vec2(marker.size, marker.size) * unit);
        let [r, g, b, a] = marker.colour;
        let a = (f32::from(a) * fade).round() as u8;
        let colour = Color32::from_rgba_unmultiplied(r, g, b, a);
        game.paint_texture(painter, &marker.icon, rect, colour);
    }
}

/// A tutorial line the scripts put up, in a box on the left, wrapped.
fn tutorial(painter: &egui::Painter, screen: egui::Rect, line: &str) {
    let width = (screen.width() * 0.3).clamp(220.0, 380.0);
    let galley = painter.layout(
        line.to_owned(),
        FontId::proportional(15.0),
        Color32::from_gray(240),
        width,
    );
    let at = screen.left_top() + vec2(20.0, screen.height() * 0.36);
    let back = Rect::from_min_size(at, galley.size()).expand(10.0);
    painter.rect_filled(back, 4.0, Color32::from_black_alpha(160));
    painter.rect_filled(
        Rect::from_min_size(back.min, vec2(4.0, back.height())),
        0.0,
        Color32::from_rgb(255, 200, 0),
    );
    painter.galley(at, galley, Color32::from_gray(240));
}

/// A crosshair centred on `c`.
fn crosshair(painter: &egui::Painter, c: egui::Pos2) {
    let stroke = Stroke::new(2.0, Color32::from_white_alpha(220));
    for (a, b) in [
        (vec2(-12.0, 0.0), vec2(-4.0, 0.0)),
        (vec2(4.0, 0.0), vec2(12.0, 0.0)),
        (vec2(0.0, -12.0), vec2(0.0, -4.0)),
        (vec2(0.0, 4.0), vec2(0.0, 12.0)),
    ] {
        painter.line_segment([c + a, c + b], stroke);
    }
}

/// The camera's position, from its view-projection: the one point it maps
/// to clip-space w = 0 with x = y = 0 (none for an orthographic one).
fn camera_position(view_proj: Mat4) -> Option<Vec3> {
    let eye = view_proj.inverse() * m2_math::glam::Vec4::Z;
    (eye.w.abs() > f32::EPSILON).then(|| eye.truncate() / eye.w)
}

/// Where a world point lands on the screen, if in front of the camera.
fn to_screen(view_proj: Mat4, world: Vec3, screen: egui::Rect) -> Option<egui::Pos2> {
    let clip = view_proj * world.extend(1.0);
    if clip.w <= 0.01 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    // Off to the side: pinned to the screen's edge.
    let x = ndc.x.clamp(-0.95, 0.95);
    let y = ndc.y.clamp(-0.9, 0.9);
    Some(pos2(
        screen.left() + (x + 1.0) * 0.5 * screen.width(),
        screen.top() + (1.0 - y) * 0.5 * screen.height(),
    ))
}

/// Where a world point lands on the screen, on it or off, if in front of
/// the camera.
fn project(view_proj: Mat4, world: Vec3, screen: egui::Rect) -> Option<egui::Pos2> {
    let clip = view_proj * world.extend(1.0);
    if clip.w <= 0.01 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    Some(pos2(
        screen.left() + (ndc.x + 1.0) * 0.5 * screen.width(),
        screen.top() + (1.0 - ndc.y) * 0.5 * screen.height(),
    ))
}

/// The shop: a tab for each faction's, her cash, and what it sells, with
/// a button to buy each that is unlocked and that she can pay for. The
/// look is this game's own (the game's shop screen isn't drawn); what is
/// sold, and at what price, is the scripts' (`MrxShop`).
fn shop_window(ctx: &egui::Context, shop: &ShopScreen) -> Option<ShopAction> {
    let mut action = None;
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Middle,
        egui::Id::new("dim"),
    ))
    .rect_filled(ctx.content_rect(), 0.0, Color32::from_black_alpha(120));
    egui::Window::new("Shop")
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.set_min_width(560.0);
            ui.horizontal_wrapped(|ui| {
                for (i, (_, name)) in crate::mission::SHOPS.iter().enumerate() {
                    if ui.selectable_label(shop.faction == i, *name).clicked() && shop.faction != i
                    {
                        action = Some(ShopAction::Faction(i));
                    }
                }
            });
            ui.separator();
            ui.label(format!(
                "Cash ${}    Fuel {:.0}",
                thousands(shop.view.cash),
                shop.view.fuel
            ));
            if let Some(said) = &shop.said {
                ui.label(said);
            }
            egui::ScrollArea::vertical()
                .max_height(420.0)
                .show(ui, |ui| {
                    egui::Grid::new("shop items")
                        .striped(true)
                        .num_columns(4)
                        .show(ui, |ui| {
                            for item in &shop.view.items {
                                ui.label(&item.name).on_hover_text(&item.description);
                                ui.label(format!("${}", thousands(item.price)));
                                ui.label(format!("{} held", item.stock));
                                let can = item.unlocked
                                    && item.price <= shop.view.cash
                                    && item.stock < item.most.max(1);
                                let label = if item.unlocked { "Buy" } else { "Locked" };
                                if ui.add_enabled(can, egui::Button::new(label)).clicked() {
                                    action = Some(ShopAction::Buy(item.id.clone()));
                                }
                                ui.end_row();
                            }
                        });
                });
            ui.separator();
            ui.horizontal(|ui| {
                let mut all = shop.unlock_all;
                if ui.checkbox(&mut all, "Unlock everything (cheat)").changed() {
                    action = Some(ShopAction::UnlockAll(all));
                }
                if ui.button("Close").clicked() {
                    action = Some(ShopAction::Close);
                }
            });
        });
    action
}

/// The test cheat menu ([`crate::cheats`]): a page each for weapons,
/// vehicles and support (the game's own lists, with a filter) and one for
/// her helpers.
fn cheat_window(
    ctx: &egui::Context,
    cheats: &crate::cheats::CheatScreen,
) -> Option<crate::cheats::CheatAction> {
    use crate::cheats::{CheatAction as A, Tab, shown};
    let mut action = None;
    // Plain black goes under everything egui draws (it would hide the
    // window drawn on the same layer order).
    let (order, dim) = if cheats.plain {
        (egui::Order::Background, Color32::BLACK)
    } else {
        (egui::Order::Middle, Color32::from_black_alpha(120))
    };
    ctx.layer_painter(egui::LayerId::new(order, egui::Id::new("dim")))
        .rect_filled(ctx.content_rect(), 0.0, dim);
    egui::Window::new("Cheats (test)")
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.set_min_width(560.0);
            ui.horizontal(|ui| {
                for tab in Tab::ALL {
                    if ui.selectable_label(cheats.tab == tab, tab.name()).clicked()
                        && cheats.tab != tab
                    {
                        action = Some(A::Tab(tab));
                    }
                }
            });
            ui.separator();
            if cheats.tab != Tab::Player {
                ui.horizontal(|ui| {
                    ui.label("Filter");
                    let mut filter = cheats.filter.clone();
                    if ui.text_edit_singleline(&mut filter).changed() {
                        action = Some(A::Filter(filter));
                    }
                });
            }
            if let Some(said) = &cheats.said {
                ui.label(said);
            }
            let f = cheats.filter.as_str();
            egui::ScrollArea::vertical()
                .max_height(420.0)
                .show(ui, |ui| match cheats.tab {
                    Tab::Weapons => {
                        if cheats.weapons.is_empty() {
                            ui.label("No weapon templates (the island's data isn't loaded).");
                        }
                        egui::Grid::new("cheat weapons")
                            .striped(true)
                            .num_columns(2)
                            .show(ui, |ui| {
                                for name in cheats.weapons.iter().filter(|n| shown(n, f)) {
                                    ui.label(name);
                                    if ui.button("Give").clicked() {
                                        action = Some(A::Weapon(name.clone()));
                                    }
                                    ui.end_row();
                                }
                            });
                    }
                    Tab::Vehicles => {
                        if cheats.vehicles.is_empty() {
                            ui.label("No vehicle templates (the island's data isn't loaded).");
                        }
                        egui::Grid::new("cheat vehicles")
                            .striped(true)
                            .num_columns(3)
                            .show(ui, |ui| {
                                for v in cheats
                                    .vehicles
                                    .iter()
                                    .filter(|v| shown(&v.name, f) || shown(v.kind, f))
                                {
                                    ui.label(&v.name);
                                    ui.label(v.kind);
                                    if ui.button("Spawn").clicked() {
                                        action = Some(A::Vehicle(v.name.clone()));
                                    }
                                    ui.end_row();
                                }
                            });
                    }
                    Tab::Support => {
                        if cheats.support.is_empty() {
                            ui.label("No support catalogue until the mission scripts run.");
                        }
                        egui::Grid::new("cheat support")
                            .striped(true)
                            .num_columns(4)
                            .show(ui, |ui| {
                                for s in cheats.support.iter().filter(|s| {
                                    shown(&s.name, f) || shown(&s.key, f) || shown(&s.kind, f)
                                }) {
                                    ui.label(&s.name).on_hover_text(&s.key);
                                    ui.label(&s.kind);
                                    if ui.button("Stock").clicked() {
                                        action = Some(A::StockSupport(s.key.clone()));
                                    }
                                    if ui.button("Call in").clicked() {
                                        action = Some(A::CallSupport(s.key.clone()));
                                    }
                                    ui.end_row();
                                }
                            });
                    }
                    Tab::Player => {
                        let mut god = cheats.toggles.god;
                        if ui
                            .checkbox(&mut god, "God mode (health kept whole)")
                            .changed()
                        {
                            action = Some(A::God(god));
                        }
                        let mut ammo = cheats.toggles.ammo;
                        if ui
                            .checkbox(&mut ammo, "Infinite ammo (clips, reserves, explosives)")
                            .changed()
                        {
                            action = Some(A::Ammo(ammo));
                        }
                        if ui.button("Heal").clicked() {
                            action = Some(A::Heal);
                        }
                        let money = format!(
                            "Give ${} and {:.0} fuel",
                            thousands(crate::cheats::CASH),
                            crate::cheats::FUEL
                        );
                        if ui.button(money).clicked() {
                            action = Some(A::Money);
                        }
                        let ahead = format!("Teleport {:.0} m ahead", crate::cheats::AHEAD);
                        if ui.button(ahead).clicked() {
                            action = Some(A::TeleportAhead);
                        }
                        if ui.button("Teleport to the nearest objective").clicked() {
                            action = Some(A::TeleportObjective);
                        }
                    }
                });
            ui.separator();
            if ui.button("Close").clicked() {
                action = Some(A::Close);
            }
        });
    action
}

/// A sum of money with its thousands apart: `1,250,000`.
fn thousands(n: f64) -> String {
    let digits = format!("{:.0}", n.abs());
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0.0 { format!("-{out}") } else { out }
}

/// The pause menu, with `playing` (the hero she is) marked under "Play
/// as". `dev`: the save and load buttons name their keys (which work only
/// then); `load`: a saved game can be loaded. `own`: the game's own pause
/// menu is up (the HUD's movie), and this is only the alpha's settings
/// beside it, folded in a corner (its dim and its Resume are the game's).
fn pause_menu(
    ctx: &egui::Context,
    config: &mut Config,
    playing: crate::hero::Hero,
    dev: bool,
    load: bool,
    own: bool,
) -> Option<MenuAction> {
    let mut action = None;
    if !own {
        // Dim the game behind the menu.
        ctx.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("dim"),
        ))
        .rect_filled(ctx.content_rect(), 0.0, Color32::from_black_alpha(120));
    }
    let window = if own {
        egui::Window::new("Alpha settings")
            .anchor(Align2::RIGHT_TOP, vec2(-16.0, 16.0))
            .collapsible(true)
            .default_open(false)
    } else {
        egui::Window::new("Paused")
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .collapsible(false)
    };
    window.resizable(false).show(ctx, |ui| {
        ui.set_min_width(300.0);
        ui.spacing_mut().item_spacing.y = 8.0;
        if !own && ui.button("Resume").clicked() {
            action = Some(MenuAction::Resume);
        }
        ui.horizontal(|ui| {
            let (save, load_label) = if dev {
                ("Save game (F5)", "Load game (F9)")
            } else {
                ("Save game", "Load game")
            };
            if ui.button(save).clicked() {
                action = Some(MenuAction::SaveGame);
            }
            if load && ui.button(load_label).clicked() {
                action = Some(MenuAction::LoadGame);
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Play as");
            for hero in crate::hero::Hero::ALL {
                if ui.selectable_label(playing == hero, hero.name()).clicked() && playing != hero {
                    config.hero = hero;
                    action = Some(MenuAction::Hero(hero));
                }
            }
        });
        ui.separator();
        ui.heading("Settings");
        ui.add(egui::Slider::new(&mut config.camera.fov, 40.0..=100.0).text("Field of view"));
        ui.add(
            egui::Slider::new(&mut config.camera.mouse_sensitivity, 0.0005..=0.01)
                .logarithmic(true)
                .text("Mouse sensitivity"),
        );
        ui.checkbox(&mut config.camera.invert_y, "Invert mouse Y");
        ui.checkbox(&mut config.window.vsync, "Vertical sync");
        ui.horizontal(|ui| {
            ui.label("Display");
            let now = config.window.mode;
            for mode in crate::config::DisplayMode::ALL {
                if ui.selectable_label(now == mode, mode.text()).clicked() && now != mode {
                    action = Some(MenuAction::Display(mode));
                }
            }
        });
        if ui.button("Save settings").clicked() {
            action = Some(MenuAction::SaveSettings);
        }
        ui.separator();
        ui.collapsing("Controls", |ui| {
            for line in include_str!("usage.txt")
                .lines()
                .skip_while(|l| !l.starts_with("On foot"))
            {
                ui.label(line);
            }
        });
        ui.separator();
        if ui.button("Quit to desktop").clicked() {
            action = Some(MenuAction::Quit);
        }
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_in_front_land_on_screen() {
        let screen = egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let vp = Mat4::perspective_rh(1.0, 800.0 / 600.0, 0.1, 100.0);
        let p = to_screen(vp, Vec3::new(0.0, 0.0, -10.0), screen).unwrap();
        assert!((p.x - 400.0).abs() < 0.5 && (p.y - 300.0).abs() < 0.5);
        assert!(to_screen(vp, Vec3::new(0.0, 0.0, 10.0), screen).is_none());
    }

    #[test]
    fn markers_fade_from_the_camera_not_from_her() {
        // A chase camera 6 m behind and 3 m over her: the marker's fade is
        // measured from the camera (from the 360 code), not from her.
        let eye = Vec3::new(10.0, 5.0, 26.0);
        let view = Mat4::look_at_rh(eye, Vec3::new(10.0, 2.0, 20.0), Vec3::Y);
        let vp = Mat4::perspective_rh(1.0, 800.0 / 600.0, 0.1, 1000.0) * view;
        let found = camera_position(vp).unwrap();
        assert!(found.distance(eye) < 1e-3, "{found}");
        let marker = crate::mission::MarkerView {
            at: Vec3::new(10.0, 2.0, 15.0),
            icon: "HUD_objective_destroy".into(),
            colour: [255, 0, 0, 255],
            near: 5.0,
            far: 175.0,
            flat: false,
            size: 32.0,
        };
        // 5 m from her: opaque from her, but the camera is ~11.4 m off.
        let her = Vec3::new(10.0, 2.0, 20.0);
        assert_eq!(marker.alpha_from(her), 1.0);
        assert!(marker.alpha_from(found) < 1.0);
        assert!(camera_position(Mat4::orthographic_rh(-1.0, 1.0, -1.0, 1.0, 0.1, 10.0)).is_none());
    }

    #[test]
    fn the_reticle_goes_where_she_aims() {
        let vp = Mat4::perspective_rh(1.0, 800.0 / 600.0, 0.1, 100.0);
        let mut info = HudInfo {
            crosshair: Some(Vec3::new(0.0, 0.0, -10.0)),
            view_proj: Some(vp),
            ..HudInfo::default()
        };
        let [x, y] = reticle(&info).unwrap();
        assert!(x.abs() < 1e-5 && y.abs() < 1e-5);
        // Above the camera's line: up the screen.
        info.crosshair = Some(Vec3::new(0.0, 2.0, -10.0));
        assert!(reticle(&info).unwrap()[1] > 0.1);
        info.down = true;
        assert_eq!(reticle(&info), None);
        info.down = false;
        info.crosshair = None;
        assert_eq!(reticle(&info), None);
    }

    #[test]
    fn hud_and_menu_lay_out() {
        let mut hud = Hud::new();
        let mut config = Config::default();
        let info = HudInfo {
            playing: true,
            hp: 40.0,
            driving: Some(("global_veh_hondacrx".into(), 12.0, false)),
            crosshair: Some(Vec3::new(0.0, 1.0, -40.0)),
            mission: Some(MissionView {
                black: 0.0,
                objectives: vec!["VzaCon001 objective 1".into()],
                tutorial: Some("[P] VzaCon001 tutorial 1\nline 2".into()),
                game_tutorial: Some("[pda] VzaCon001 tutorial 1\nline 2".into()),
                markers: vec![crate::mission::MarkerView {
                    at: Vec3::new(0.0, 0.0, -10.0),
                    icon: "HUD_objective_deliverable".into(),
                    colour: [255, 200, 0, 255],
                    near: 5.0,
                    far: 175.0,
                    flat: false,
                    size: 32.0,
                }],
                message: Some((false, "VzaCon001 objective 1".into())),
                game_message: None,
                radar_regions: Vec::new(),
                hud_calls: Vec::new(),
                pda_objectives: Vec::new(),
                status: None,
                contract_complete: true,
                ..MissionView::default()
            }),
            view_proj: Some(Mat4::perspective_rh(1.0, 1.333, 0.1, 100.0)),
            ..HudInfo::default()
        };
        let (action, _) = hud.build(headless_input(800, 600, 0.0), &info, &mut config);
        assert_eq!(action, None);
        hud.menu = true;
        let (action, _) = hud.build(headless_input(800, 600, 0.1), &info, &mut config);
        assert_eq!(action, None);
        let (shapes, delta, _) = hud.frame.as_ref().unwrap();
        assert!(!shapes.is_empty());
        // The font atlas arrives on the first frame and is kept until
        // painted.
        assert!(!delta.set.is_empty());
    }

    #[test]
    fn the_satellite_view_and_its_wheel_lay_out() {
        let mut hud = Hud::new();
        let mut config = Config::default();
        let middle = Vec3::new(-1472.0, -18.0, 2825.0);
        let view = crate::support::satellite_view(
            m2_math::glam::Vec2::new(middle.x, middle.z),
            middle.y,
            800.0 / 600.0,
        );
        let mut info = HudInfo {
            playing: true,
            hp: 80.0,
            view_proj: Some(view.projection * view.view),
            support: SupportHud {
                item: Some(("Satellite Guided Bomb".into(), "Satellite".into(), 1, true)),
                designator: Some("Satellite Designator".into()),
                satellite: Some((middle, 13.0, 0.0)),
                wheel: Some(WheelHud {
                    angle: 0.2,
                    sectors: [(0.0, true), (std::f32::consts::PI, false)],
                    wedge: crate::support::WEDGE,
                    mark: Some(true),
                    size: 1.0,
                }),
                flash: 0.5,
                ..SupportHud::default()
            },
            ..HudInfo::default()
        };
        // The view's middle is in the middle of the screen.
        let screen = egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let p = project(info.view_proj.unwrap(), middle, screen).unwrap();
        assert!((p.x - 400.0).abs() < 0.5 && (p.y - 300.0).abs() < 0.5);
        hud.build(headless_input(800, 600, 0.0), &info, &mut config);
        // Vertices tessellated: egui batches shapes into few meshes.
        let vertices = |hud: &Hud| -> usize {
            hud.frame
                .as_ref()
                .unwrap()
                .0
                .iter()
                .map(|p| match &p.primitive {
                    egui::epaint::Primitive::Mesh(m) => m.vertices.len(),
                    egui::epaint::Primitive::Callback(_) => 0,
                })
                .sum()
        };
        let viewing = vertices(&hud);
        assert!(viewing > 0);
        info.support.satellite = None;
        info.support.wheel = None;
        hud.build(headless_input(800, 600, 0.1), &info, &mut config);
        // The view's rulers and wheel are more shapes than the HUD on foot
        // (its grain is the post pass's).
        assert!(
            vertices(&hud) * 3 < viewing * 2,
            "{} {viewing}",
            vertices(&hud)
        );
    }
}
