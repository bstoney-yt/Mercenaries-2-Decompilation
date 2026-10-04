//! The HUD (health, weapon, crosshair, what the player can do next) and the
//! pause menu, drawn with egui over the finished frame.
//!
//! The HUD's look is our own, not the game's: the game's interface (Scaleform
//! movies) is not decoded. egui's bundled fonts are used.

use egui::{Align2, Color32, FontId, RawInput, Stroke, pos2, vec2};
use egui_wgpu::{Renderer, RendererOptions, ScreenDescriptor};
use m2_render::gpu::OverlayTarget;
use m2_render::gpu::wgpu;

use m2_math::glam::{Mat4, Vec3};

use crate::combat::Weapon;
use crate::config::Config;
use crate::mission::MissionView;

/// What the HUD shows this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HudInfo {
    /// On foot or driving (not flying the free camera).
    pub playing: bool,
    pub hp: f32,
    pub down: bool,
    pub weapon: Weapon,
    /// Driving: the vehicle's name and speed (m/s), and whether it is
    /// wrecked.
    pub driving: Option<(String, f32, bool)>,
    /// A vehicle in reach to get into.
    pub can_enter: Option<String>,
    /// Shown while the island or its people are still loading.
    pub loading: Option<String>,
    /// The mission's objectives and markers (`--mission`).
    pub mission: Option<MissionView>,
    /// World to clip, for placing markers.
    pub view_proj: Option<Mat4>,
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
    Quit,
}

/// Seconds the controls hint shows at the start.
const HINT_TIME: f64 = 12.0;

/// The HUD and menu.
pub struct Hud {
    pub ctx: egui::Context,
    renderer: Option<Renderer>,
    /// The pause menu is open.
    pub menu: bool,
    /// Tessellated and waiting to paint: shapes, texture changes, scale.
    frame: Option<(Vec<egui::ClippedPrimitive>, egui::TexturesDelta, f32)>,
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
            menu: false,
            frame: None,
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
        let output = self.ctx.run_ui(input, |ui| {
            hud(ui, info);
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
            renderer.free_texture(id);
        }
        delta.clear();
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

fn hud(ui: &mut egui::Ui, info: &HudInfo) {
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
    if time < HINT_TIME && info.playing {
        text(
            screen.center_top() + vec2(0.0, 56.0),
            Align2::CENTER_TOP,
            "Click to look around. WASD move, Shift sprint, Space jump, E get in, \
             left button fire, right button aim, 1/2 weapons, G grenade, Esc menu",
            15.0,
            white,
        );
    }
    if let Some(m) = &info.mission {
        mission(&painter, screen, info.view_proj, m, &text);
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
        }
        None => {
            let weapon = match info.weapon {
                Weapon::Rifle => "AK-47   ·   2 RPG   ·   G grenade",
                Weapon::Rpg => "RPG   ·   1 rifle   ·   G grenade",
            };
            text(
                screen.right_bottom() + vec2(-24.0, -28.0),
                Align2::RIGHT_BOTTOM,
                weapon,
                18.0,
                white,
            );
            if !info.down {
                // Crosshair.
                let c = screen.center();
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
        }
    }
    if let Some(name) = &info.can_enter
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
}

/// The mission's objectives (top left), its newest message (top centre)
/// and markers over its targets.
fn mission<T: Fn(egui::Pos2, Align2, &str, f32, Color32)>(
    painter: &egui::Painter,
    screen: egui::Rect,
    view_proj: Option<Mat4>,
    m: &MissionView,
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
    if m.contract_complete {
        text(
            screen.center() + vec2(0.0, -60.0),
            Align2::CENTER_CENTER,
            "Contract complete",
            40.0,
            gold,
        );
    }
    if let Some((done, line)) = &m.message {
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
    fn hud_and_menu_lay_out() {
        let mut hud = Hud::new();
        let mut config = Config::default();
        let info = HudInfo {
            playing: true,
            hp: 40.0,
            driving: Some(("global_veh_hondacrx".into(), 12.0, false)),
            mission: Some(MissionView {
                objectives: vec!["VzaCon001 objective 1".into()],
                markers: vec![(
                    Vec3::new(0.0, 0.0, -10.0),
                    "deliverable".into(),
                    [255, 200, 0, 255],
                )],
                message: Some((false, "VzaCon001 objective 1".into())),
                status: None,
                contract_complete: true,
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
}
