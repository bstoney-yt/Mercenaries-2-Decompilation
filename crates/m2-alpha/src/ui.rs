//! The HUD and the pause menu, drawn with egui over the finished frame.
//!
//! Health, ammo and weapon, the reticle, hit arcs and messages are the
//! game's own HUD ([`GameHud`]) once its scripts have started; until then,
//! or without the game's files, a stand-in of our own draws health, weapon
//! and a crosshair. What the player can do next, the mission's objectives
//! and markers, and loading lines are ours, in egui's bundled fonts.

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
    /// What she carries in the launcher's place, if anything.
    pub second: Option<Weapon>,
    /// Grenades she has.
    pub grenades: u32,
    /// The rounds of the weapon up when it runs out (the rifle's and the
    /// grenade launcher's): in its clip, and stored.
    pub rounds: Option<(u32, u32)>,
    /// What the use key takes from within reach, as the game words it
    /// ("Equip Grenade Launcher").
    pub prompt: Option<String>,
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
    /// What the game's HUD is told, if it runs.
    pub game: Option<HudState>,
    /// Her support: the item ready, the designator, the satellite's view,
    /// the timing wheel, the bomb's flash.
    pub support: SupportHud,
}

/// What the pause menu asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    Resume,
    /// Write the settings file.
    SaveSettings,
    /// Save the game, or load the saved game.
    SaveGame,
    LoadGame,
    /// Play as another mercenary (also kept in the settings).
    Hero(crate::hero::Hero),
    Quit,
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
    /// Tessellated and waiting to paint: shapes, texture changes, scale.
    frame: Option<(Vec<egui::ClippedPrimitive>, egui::TexturesDelta, f32)>,
    /// The game's own HUD, if its files are there.
    pub game: Option<GameHud>,
    /// Each frame waits for the game's HUD to draw it (play run offscreen,
    /// faster than the HUD's scripts would keep up with).
    pub wait: bool,
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
            frame: None,
            game: None,
            wait: false,
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
        let message = info.mission.as_ref().and_then(|m| m.game_message.as_ref());
        state.messages.extend(game.message(message));
        state.blips = blips(info);
        state.reticle = reticle(info);
        state.support = info.support.clone();
        state.radar_regions = info
            .mission
            .as_ref()
            .map_or_else(Vec::new, |m| m.radar_regions.clone());
        state.tutorial = info.mission.as_ref().and_then(|m| m.game_tutorial.clone());
        if let Some(m) = &info.mission {
            state.hud_calls = game.hud_calls(&m.hud_calls);
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
    /// draw one frame. `keys` (Flash key codes) are pressed half way.
    pub fn settle(
        &mut self,
        info: &HudInfo,
        size: (u32, u32),
        seconds: f64,
        keys: &[i32],
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
        state.radar_regions = info
            .mission
            .as_ref()
            .map_or_else(Vec::new, |m| m.radar_regions.clone());
        state.tutorial = info.mission.as_ref().and_then(|m| m.game_tutorial.clone());
        if let Some(m) = &info.mission {
            state.hud_calls = game.hud_calls(&m.hud_calls);
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
            state.keys = match i.checked_sub(steps / 2) {
                Some(0) => keys.iter().map(|&k| (k, true)).collect(),
                Some(1) => keys.iter().map(|&k| (k, false)).collect(),
                _ => Vec::new(),
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
        let output = self.ctx.run_ui(input, |ui| {
            hud(ui, info, game);
            if menu {
                action = pause_menu(ui.ctx(), config);
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
        let renderer = self
            .renderer
            .get_or_insert_with(|| Renderer::new(t.device, t.format, RendererOptions::default()));
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
            .map(|(at, label, colour)| (at.to_array(), label.clone(), *colour))
            .collect()
    })
}

fn hud(ui: &mut egui::Ui, info: &HudInfo, game: Option<&GameHud>) {
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
    // The scripts' fade to black (`MrxGui.GlobalFadeToBlack`), over all.
    if let Some(black) = info.mission.as_ref().map(|m| m.black).filter(|&b| b > 0.0) {
        ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, "fade".into()))
            .rect_filled(
                screen,
                0.0,
                Color32::from_black_alpha((black.min(1.0) * 255.0) as u8),
            );
    }
    if let Some(loading) = &info.loading {
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
    if time < HINT_TIME && info.playing {
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
        game.paint(&painter);
        if std::env::var_os("M2_PAINT_TEST").is_some() {
            paint_test(&painter);
        }
    }
    if let Some(m) = &info.mission {
        // The game's HUD shows the messages in its message buffer.
        mission(&painter, screen, info.view_proj, m, game.is_none(), &text);
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
    if game.is_some() {
        if let Some((_, _, true)) = &info.driving {
            text(
                screen.center() + vec2(0.0, 60.0),
                Align2::CENTER_TOP,
                "Wrecked: E to get out",
                17.0,
                white,
            );
        }
    } else {
        stand_in(&painter, screen, info, &text);
    }
    if let Some(prompt) = &info.prompt {
        text(
            screen.center() + vec2(0.0, 60.0),
            Align2::CENTER_TOP,
            &format!("E: {prompt}"),
            17.0,
            white,
        );
    } else if let Some(name) = &info.can_enter
        && info.driving.is_none()
    {
        text(
            screen.center() + vec2(0.0, 60.0),
            Align2::CENTER_TOP,
            &format!("E: get in the {}", name.trim_start_matches("global_veh_")),
            17.0,
            white,
        );
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
            let up = match (info.weapon, info.rounds) {
                (Weapon::Rifle, Some((clip, stored))) => format!("Carbine {clip} | {stored}"),
                (Weapon::Rifle, None) => "Carbine".to_owned(),
                (Weapon::Rpg, _) => "RPG".to_owned(),
                (Weapon::GrenadeLauncher, Some((clip, stored))) => {
                    format!("Grenade launcher {clip} | {stored}")
                }
                (Weapon::GrenadeLauncher, None) => "Grenade launcher".to_owned(),
            };
            let other = match (info.weapon, info.second) {
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
                &format!("{up}{other}   ·   G grenade ({})", info.grenades),
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

/// The mission's objectives (top left), its newest message (top centre)
/// and tutorial line (left) if `message` (no game HUD to show them), and
/// markers over its targets.
fn mission<T: Fn(egui::Pos2, Align2, &str, f32, Color32)>(
    painter: &egui::Painter,
    screen: egui::Rect,
    view_proj: Option<Mat4>,
    m: &MissionView,
    message: bool,
    text: &T,
) {
    let white = Color32::from_gray(235);
    let gold = Color32::from_rgb(255, 200, 0);
    let mut at = screen.left_top() + vec2(20.0, 20.0);
    if let Some(status) = &m.status {
        text(at, Align2::LEFT_TOP, status, 15.0, white);
        at.y += 22.0;
    }
    if !m.objectives.is_empty() {
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
    // The game's HUD has its own fanfare (`Hud.Fanfare`).
    if m.contract_complete && message {
        text(
            screen.center() + vec2(0.0, -60.0),
            Align2::CENTER_CENTER,
            "Contract complete",
            40.0,
            gold,
        );
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
    let Some(vp) = view_proj else { return };
    for (world, label, [r, g, b, _]) in &m.markers {
        let Some(p) = to_screen(vp, *world, screen) else {
            continue;
        };
        let colour = Color32::from_rgb(*r, *g, *b);
        painter.circle_stroke(p, 9.0, Stroke::new(2.5, colour));
        text(
            p + vec2(0.0, -14.0),
            Align2::CENTER_BOTTOM,
            label,
            14.0,
            colour,
        );
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

fn pause_menu(ctx: &egui::Context, config: &mut Config) -> Option<MenuAction> {
    let mut action = None;
    // Dim the game behind the menu.
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Middle,
        egui::Id::new("dim"),
    ))
    .rect_filled(ctx.content_rect(), 0.0, Color32::from_black_alpha(120));
    egui::Window::new("Paused")
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.set_min_width(300.0);
            ui.spacing_mut().item_spacing.y = 8.0;
            if ui.button("Resume").clicked() {
                action = Some(MenuAction::Resume);
            }
            ui.horizontal(|ui| {
                if ui.button("Save game (F5)").clicked() {
                    action = Some(MenuAction::SaveGame);
                }
                if ui.button("Load game (F9)").clicked() {
                    action = Some(MenuAction::LoadGame);
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Play as");
                for hero in crate::hero::Hero::ALL {
                    if ui
                        .selectable_label(config.hero == hero, hero.name())
                        .clicked()
                        && config.hero != hero
                    {
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
                markers: vec![(
                    Vec3::new(0.0, 0.0, -10.0),
                    "deliverable".into(),
                    [255, 200, 0, 255],
                )],
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
