//! Photo mode: the world held still while a free camera flies about it,
//! and pictures of it saved as PNG files.
//!
//! **Invented, all of it.** The Xbox 360 game has no photo mode, so nothing
//! here comes from its code or data: this is a Beta addition of the port,
//! off unless the settings' `[enhanced] photo_mode` (or `--photo-mode`)
//! turns it on, and out of the game's own paths otherwise. Its keys, speeds,
//! limits, the on-screen help and where the pictures go are all choices
//! made here.
//!
//! - F10 (the [`Action::PhotoMode`] binding) enters and leaves it. While it
//!   is on nothing ticks: no physics, people, vehicles, scripts or
//!   mission, and the world's sounds and music wait as the pause menu
//!   holds them. Leaving gives back the view and HUD exactly as they were:
//!   the game's own camera and the player are never moved, the photo
//!   camera only stands in for them in [`crate::game::Game::view`].
//! - Flying: the movement keys (W A S D), Space and C up and down, the
//!   mouse or the look keys to turn, Shift five times faster, Ctrl five
//!   times slower, the mouse wheel the field of view, Q and E to roll
//!   (R levels it), `+`/`-` on the number row the base speed.
//! - H shows the game's HUD again (hidden by default), P hides her, F1
//!   the help, F12 saves a picture (without the help) to
//!   [`screenshot_dir`].

use std::path::{Path, PathBuf};

use m2_math::glam::{Mat4, Quat, Vec3};
use m2_render::View;

use crate::input::{Action, Input};

/// The base flying speed, metres a second, and its range.
const SPEED: f32 = 10.0;
const SPEED_RANGE: (f32, f32) = (0.5, 200.0);
/// Each `+`/`-` multiplies the base speed by this.
const SPEED_STEP: f32 = 1.5;
/// Shift's and Ctrl's factors.
const FAST: f32 = 5.0;
const SLOW: f32 = 0.2;
/// The field of view's range and a wheel notch's change, degrees.
const FOV_RANGE: (f32, f32) = (10.0, 120.0);
const FOV_STEP: f32 = 5.0;
/// How fast Q and E roll the camera, radians a second.
const ROLL_RATE: f32 = 1.0;
/// Pitch stops short of straight up and down, radians.
const PITCH_LIMIT: f32 = 1.55;
/// How long a saved picture's name stays on screen, seconds.
const SAVED_SHOWN: f32 = 3.0;

/// The photo camera: where it is, which way it looks, its roll and lens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhotoCamera {
    pub position: Vec3,
    /// Radians clockwise from `+z` seen from above (as
    /// [`crate::camera::FlyCamera::yaw`]).
    pub yaw: f32,
    /// Radians up from the horizon.
    pub pitch: f32,
    /// Radians about the view direction, clockwise as seen.
    pub roll: f32,
    /// The game's field of view, degrees (the picture's width at 4:3).
    pub fov: f32,
    /// The base flying speed, metres a second.
    pub speed: f32,
}

impl PhotoCamera {
    /// A camera where `view` is seen from, looking its way, level.
    pub fn from_view(view: &View, fov: f32) -> Self {
        let inv = view.view.inverse();
        let eye = inv.w_axis.truncate();
        // A right-handed look-to view looks down its -z.
        let forward = (-inv.z_axis.truncate()).normalize_or(Vec3::Z);
        Self {
            position: eye,
            yaw: forward.x.atan2(forward.z),
            pitch: forward.y.clamp(-1.0, 1.0).asin(),
            roll: 0.0,
            fov,
            speed: SPEED,
        }
    }

    pub fn forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(sy * cp, sp, cy * cp)
    }

    /// Level right (no roll), for strafing.
    pub fn right(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        Vec3::new(-cy, 0.0, sy)
    }

    /// The picture's up, rolled.
    pub fn up(&self) -> Vec3 {
        let forward = self.forward();
        let level = self.right().cross(forward).normalize_or(Vec3::Y);
        Quat::from_axis_angle(forward, self.roll) * level
    }

    /// One step of `dt` seconds with these controls.
    pub fn update(&mut self, c: &Controls, dt: f32) {
        self.yaw -= c.look.0;
        self.pitch = (self.pitch - c.look.1).clamp(-PITCH_LIMIT, PITCH_LIMIT);
        if c.speed_steps != 0 {
            self.speed =
                (self.speed * SPEED_STEP.powi(c.speed_steps)).clamp(SPEED_RANGE.0, SPEED_RANGE.1);
        }
        let mut speed = self.speed;
        if c.fast {
            speed *= FAST;
        }
        if c.slow {
            speed *= SLOW;
        }
        let motion = self.forward() * c.forward + self.right() * c.right + Vec3::Y * c.up;
        self.position += motion.normalize_or_zero() * speed * dt;
        self.fov = (self.fov + c.fov_steps as f32 * FOV_STEP).clamp(FOV_RANGE.0, FOV_RANGE.1);
        if c.level {
            self.roll = 0.0;
        } else {
            self.roll = (self.roll + c.roll * ROLL_RATE * dt)
                .clamp(-std::f32::consts::PI, std::f32::consts::PI);
        }
    }

    /// The view through it, with the game's near and far distances.
    pub fn view(&self, aspect: f32) -> View {
        let mut view = View::perspective(
            self.position,
            self.forward(),
            m2_engine::projection::fov_y(self.fov.to_radians(), aspect),
            aspect,
            m2_engine::projection::NEAR,
            m2_engine::projection::FAR,
        );
        view.view = Mat4::look_to_rh(self.position, self.forward(), self.up());
        view
    }
}

/// What the player asks of the photo camera in one step, read from the
/// keys ([`Controls::read`]) or made up by a check.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Controls {
    /// -1 to 1 each: back/forward, left/right, down/up.
    pub forward: f32,
    pub right: f32,
    pub up: f32,
    /// Turn, radians: (yaw right, pitch down).
    pub look: (f32, f32),
    pub fast: bool,
    pub slow: bool,
    /// Wheel notches: positive narrows the view (zooms in).
    pub fov_steps: i32,
    /// -1 rolls left, 1 right.
    pub roll: f32,
    /// Level the roll.
    pub level: bool,
    /// `+`/`-` presses: the base speed up or down.
    pub speed_steps: i32,
}

impl Controls {
    /// This step's controls from the keys and mouse.
    pub fn read(input: &mut Input, sensitivity: f32, invert_y: bool, mouse_look: bool) -> Self {
        let (dx, dy) = input.take_look(mouse_look);
        let invert = if invert_y { -1.0 } else { 1.0 };
        let mut fov_steps = 0;
        if input.take_pressed(Action::ZoomIn) {
            fov_steps -= 1;
        }
        if input.take_pressed(Action::ZoomOut) {
            fov_steps += 1;
        }
        let mut speed_steps = 0;
        if input.take_pressed(Action::PhotoFaster) {
            speed_steps += 1;
        }
        if input.take_pressed(Action::PhotoSlower) {
            speed_steps -= 1;
        }
        Self {
            forward: input.axis(Action::MoveBack, Action::MoveForward),
            right: input.axis(Action::MoveLeft, Action::MoveRight),
            up: input.axis(Action::MoveDown, Action::MoveUp),
            look: (dx as f32 * sensitivity, dy as f32 * sensitivity * invert),
            fast: input.held(Action::Sprint),
            slow: input.held(Action::Walk),
            fov_steps,
            roll: input.axis(Action::PhotoRollLeft, Action::PhotoRollRight),
            level: input.take_pressed(Action::Reload),
            speed_steps,
        }
    }
}

/// Photo mode while it is on.
#[derive(Clone, Debug, PartialEq)]
pub struct PhotoMode {
    pub camera: PhotoCamera,
    /// The game's HUD hidden (the default).
    pub hide_hud: bool,
    /// Her body hidden.
    pub hide_player: bool,
    /// The help shown (while the HUD is hidden).
    pub help: bool,
    /// A picture is being taken: the next frame is drawn without the help
    /// and copied.
    pub shot_pending: bool,
    /// Pictures saved so far.
    pub shots: u32,
    /// The last picture's file and how long its name has shown, seconds.
    pub saved: Option<(PathBuf, f32)>,
}

impl PhotoMode {
    /// Photo mode from the frame now seen: `view` (with its field of view).
    pub fn enter(view: &View, fov: f32) -> Self {
        Self {
            camera: PhotoCamera::from_view(view, fov),
            hide_hud: true,
            hide_player: false,
            help: true,
            shot_pending: false,
            shots: 0,
            saved: None,
        }
    }

    /// One step: the camera flies, the toggles toggle. Takes every
    /// one-shot press so none is left over for play when photo mode ends.
    pub fn step(&mut self, input: &mut Input, c: Controls, dt: f32) {
        self.camera.update(&c, dt);
        if input.take_pressed(Action::PhotoHud) {
            self.hide_hud = !self.hide_hud;
        }
        if input.take_pressed(Action::PhotoHidePlayer) {
            self.hide_player = !self.hide_player;
        }
        if input.take_pressed(Action::Pda) {
            self.help = !self.help;
        }
        if input.take_pressed(Action::PhotoShot) {
            self.shot_pending = true;
        }
        input.clear_pressed();
        if let Some((_, shown)) = &mut self.saved {
            *shown += dt;
            if *shown > SAVED_SHOWN {
                self.saved = None;
            }
        }
    }

    /// The help's lines, or none (hidden, or a picture being taken).
    pub fn overlay_lines(&self) -> Vec<String> {
        if self.shot_pending {
            return Vec::new();
        }
        let mut lines = Vec::new();
        if self.help {
            lines.extend(HELP.iter().map(|s| s.to_string()));
            lines.push(format!(
                "speed {:.1} m/s   field of view {:.0}   roll {:.0}",
                self.camera.speed,
                self.camera.fov,
                self.camera.roll.to_degrees()
            ));
        }
        if let Some((path, _)) = &self.saved {
            lines.push(format!("saved {}", path.display()));
        }
        lines
    }
}

/// The on-screen help (invented wording).
pub const HELP: [&str; 6] = [
    "PHOTO MODE (F10 to leave)",
    "W A S D move, Space/C up and down, mouse or I J K L turn",
    "Shift faster, Ctrl slower, + / - base speed",
    "wheel: field of view, Q/E roll, R level",
    "H game HUD, P hide her, F1 this help",
    "F12 save a picture",
];

/// Paints the help (and the last picture's name) at the top left.
pub fn paint_overlay(ui: &mut egui::Ui, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    let ctx = ui.ctx().clone();
    egui::Area::new(egui::Id::new("photo mode help"))
        .fixed_pos(egui::pos2(16.0, 16.0))
        .show(&ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_black_alpha(160))
                .inner_margin(10.0)
                .corner_radius(4.0)
                .show(ui, |ui| {
                    for l in lines {
                        ui.label(
                            egui::RichText::new(l)
                                .color(egui::Color32::WHITE)
                                .size(15.0),
                        );
                    }
                });
        });
}

/// Where pictures go: `M2_SCREENSHOTS` if set, else a `Screenshots` folder
/// under the user's data folder (`%LOCALAPPDATA%\m2-alpha` on Windows,
/// `$XDG_DATA_HOME/m2-alpha` or `~/.local/share/m2-alpha` elsewhere). `env`
/// reads an environment variable.
pub fn screenshot_dir(env: impl Fn(&str) -> Option<String>) -> PathBuf {
    if let Some(d) = env("M2_SCREENSHOTS").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    let base = env("LOCALAPPDATA")
        .or_else(|| env("XDG_DATA_HOME"))
        .map(PathBuf::from)
        .or_else(|| env("HOME").map(|h| Path::new(&h).join(".local").join("share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("m2-alpha").join("Screenshots")
}

/// A picture's file name: the date and time (UTC) and its number, so
/// pictures never overwrite each other.
pub fn shot_name(unix: u64, n: u32) -> String {
    let days = (unix / 86_400) as i64;
    let secs = unix % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "photo-{y:04}{m:02}{d:02}-{:02}{:02}{:02}-{n:03}.png",
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    )
}

/// Writes RGBA rows (top down) as a PNG (alpha dropped: the frame is
/// opaque).
pub fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> anyhow::Result<()> {
    use anyhow::Context as _;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
    }
    let file =
        std::fs::File::create(path).with_context(|| format!("writing {}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let rgb: Vec<u8> = rgba
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    let mut w = enc.write_header()?;
    w.write_image_data(&rgb)?;
    w.finish()?;
    Ok(())
}

/// `--photo-check` (offscreen, on foot): lets her land, enters photo mode
/// from her view, flies it 3 s forward and up while turning and rolling
/// (the world not ticked, as the app holds it), saves a picture beside
/// `out` (`<stem>-photo.png`), and leaves. PASS when the picture is a
/// PNG of the frame's size that isn't one flat colour, the photo camera
/// went over 20 m, her body and the game's camera didn't move while it
/// was on, and the view after leaving is the view before entering. Exits
/// 0 on a PASS, 1 on a FAIL.
pub fn check(
    backend: &mut m2_render::gpu::WgpuBackend,
    game: &mut crate::game::Game,
    fov: f32,
    aspect: f32,
    out: &Path,
) -> ! {
    const TICK: f32 = 1.0 / 60.0;
    // Land and let the ground around her stream in.
    for i in 0..240 {
        game.step(crate::player::Intent::default(), TICK);
        if i % 10 == 0 {
            let _ = crate::render(backend, game, fov, aspect, None);
        }
    }
    let before = game.view(fov, aspect);
    let body = game.scene.player.as_ref().map(|p| p.character.position);
    let camera = game.camera;
    let on_foot = game.on_foot;
    let mut photo = PhotoMode::enter(&before, fov);
    let start = photo.camera.position;
    let mut lines = Vec::new();
    // Entered: the same picture.
    let entered = photo.camera.view(aspect);
    let same_entry = entered.view.abs_diff_eq(before.view, 1e-2);
    photo.hide_player = true;
    game.scene.hide_player = true;
    game.photo = Some(photo);
    let fly = Controls {
        forward: 1.0,
        up: 0.4,
        look: (0.004, 0.001),
        roll: 0.2,
        fast: true,
        ..Default::default()
    };
    for i in 0..180 {
        if let Some(p) = &mut game.photo {
            p.camera.update(&fly, TICK);
        }
        if i % 10 == 0 {
            let _ = crate::render(backend, game, fov, aspect, None);
        }
    }
    let flown = game
        .photo
        .as_ref()
        .map_or(0.0, |p| p.camera.position.distance(start));
    let picture = out.with_file_name(format!(
        "{}-photo.png",
        out.file_stem()
            .map_or("shot".into(), |s| s.to_string_lossy())
    ));
    let mut saved = None;
    if crate::render(backend, game, fov, aspect, None).is_ok()
        && backend.request_capture()
        && let Some((w, h, rgba)) = backend.take_capture()
    {
        let flat = rgba.chunks_exact(4).all(|p| p[..3] == rgba[..3]);
        match write_png(&picture, w, h, &rgba) {
            Ok(()) => saved = Some((w, h, flat)),
            Err(e) => lines.push(format!("picture: {e:#}")),
        }
    }
    let png_ok = std::fs::read(&picture).is_ok_and(|b| b.starts_with(b"\x89PNG\r\n\x1a\n"));
    let held = game.scene.player.as_ref().map(|p| p.character.position) == body
        && game.camera == camera
        && game.on_foot == on_foot;
    // Leave: the view is the one before.
    game.photo = None;
    game.scene.hide_player = false;
    let after = game.view(fov, aspect);
    let restored = after.view == before.view && after.projection == before.projection;
    let size = backend.size();
    let picture_ok = png_ok && saved.is_some_and(|(w, h, flat)| (w, h) == size && !flat);
    lines.push(format!(
        "entered at {:.1}: same picture {same_entry}; flown {flown:.1} m; picture {} ({}); \
         her body and the game's camera held {held}; view restored {restored}",
        start,
        picture.display(),
        match saved {
            Some((w, h, flat)) => format!("{w}x{h}, flat {flat}, png {png_ok}"),
            None => "not taken".into(),
        }
    ));
    for l in &lines {
        eprintln!("photo-check: {l}");
    }
    let pass = same_entry && flown > 20.0 && picture_ok && held && restored;
    eprintln!("photo-check: {}", if pass { "PASS" } else { "FAIL" });
    crate::errlog::exit(if pass { 0 } else { 1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam() -> PhotoCamera {
        PhotoCamera {
            position: Vec3::new(10.0, 5.0, -3.0),
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            fov: 70.0,
            speed: SPEED,
        }
    }

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-4
    }

    #[test]
    fn entering_keeps_the_view_it_came_from() {
        let fly = crate::camera::FlyCamera {
            position: Vec3::new(100.0, -30.0, 2500.0),
            yaw: 0.7,
            pitch: -0.3,
        };
        let before = fly.view(70.0, 16.0 / 9.0);
        let p = PhotoMode::enter(&before, 70.0);
        assert!(close(p.camera.position, fly.position));
        assert!((p.camera.yaw - fly.yaw).abs() < 1e-4);
        assert!((p.camera.pitch - fly.pitch).abs() < 1e-4);
        // Its view is the same picture (to float precision 2.5 km out).
        let after = p.camera.view(16.0 / 9.0);
        assert!(after.view.abs_diff_eq(before.view, 1e-2));
        assert!(after.projection.abs_diff_eq(before.projection, 1e-4));
        assert!(p.hide_hud && p.help && !p.hide_player);
    }

    #[test]
    fn flying_moves_along_the_view_and_up_the_world() {
        let mut c = cam();
        c.update(
            &Controls {
                forward: 1.0,
                ..Default::default()
            },
            1.0,
        );
        assert!(close(c.position, Vec3::new(10.0, 5.0, 7.0)));
        c.update(
            &Controls {
                up: 1.0,
                fast: true,
                ..Default::default()
            },
            0.5,
        );
        assert!(close(c.position, Vec3::new(10.0, 30.0, 7.0)));
        c.update(
            &Controls {
                right: 1.0,
                slow: true,
                ..Default::default()
            },
            1.0,
        );
        // Right at yaw 0 is -x in the game's frame (as the free camera).
        assert!(close(c.position, Vec3::new(8.0, 30.0, 7.0)));
        // Diagonals are no faster.
        let mut d = cam();
        d.update(
            &Controls {
                forward: 1.0,
                right: 1.0,
                ..Default::default()
            },
            1.0,
        );
        assert!(((d.position - cam().position).length() - SPEED).abs() < 1e-3);
    }

    #[test]
    fn looking_turns_and_pitch_stops_short_of_straight_up() {
        let mut c = cam();
        c.update(
            &Controls {
                look: (-0.5, -10.0),
                ..Default::default()
            },
            0.1,
        );
        assert!((c.yaw - 0.5).abs() < 1e-6);
        assert_eq!(c.pitch, PITCH_LIMIT);
    }

    #[test]
    fn roll_turns_the_up_and_r_levels_it() {
        let mut c = cam();
        assert!(close(c.up(), Vec3::Y));
        c.update(
            &Controls {
                roll: 1.0,
                ..Default::default()
            },
            std::f32::consts::FRAC_PI_2,
        );
        assert!((c.roll - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        // A quarter turn about +z: up lies along the level right's axis.
        assert!(c.up().dot(Vec3::Y).abs() < 1e-4);
        assert!(c.up().x.abs() > 0.99);
        // The view rolls with it.
        let rolled = c.view(1.0);
        c.update(
            &Controls {
                level: true,
                ..Default::default()
            },
            0.1,
        );
        assert_eq!(c.roll, 0.0);
        assert!(!rolled.view.abs_diff_eq(c.view(1.0).view, 1e-3));
        assert!(close(c.up(), Vec3::Y));
    }

    #[test]
    fn field_of_view_and_speed_keep_to_their_ranges() {
        let mut c = cam();
        c.update(
            &Controls {
                fov_steps: -2,
                ..Default::default()
            },
            0.0,
        );
        assert_eq!(c.fov, 60.0);
        c.update(
            &Controls {
                fov_steps: -100,
                ..Default::default()
            },
            0.0,
        );
        assert_eq!(c.fov, FOV_RANGE.0);
        c.update(
            &Controls {
                fov_steps: 100,
                ..Default::default()
            },
            0.0,
        );
        assert_eq!(c.fov, FOV_RANGE.1);
        c.update(
            &Controls {
                speed_steps: 2,
                ..Default::default()
            },
            0.0,
        );
        assert!((c.speed - SPEED * 2.25).abs() < 1e-4);
        c.update(
            &Controls {
                speed_steps: -50,
                ..Default::default()
            },
            0.0,
        );
        assert_eq!(c.speed, SPEED_RANGE.0);
    }

    #[test]
    fn the_help_hides_for_the_picture() {
        let view = crate::camera::FlyCamera::looking_at(Vec3::ZERO, Vec3::Z).view(70.0, 1.0);
        let mut p = PhotoMode::enter(&view, 70.0);
        assert_eq!(p.overlay_lines()[0], HELP[0]);
        p.shot_pending = true;
        assert!(p.overlay_lines().is_empty());
        p.shot_pending = false;
        p.help = false;
        p.saved = Some((PathBuf::from("x.png"), 0.0));
        assert_eq!(p.overlay_lines(), ["saved x.png"]);
    }

    #[test]
    fn pictures_go_to_the_users_data_folder() {
        let env = |vars: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                vars.iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| v.to_string())
            }
        };
        assert_eq!(
            screenshot_dir(env(&[("LOCALAPPDATA", "C:/Users/u/AppData/Local")])),
            Path::new("C:/Users/u/AppData/Local/m2-alpha/Screenshots")
        );
        assert_eq!(
            screenshot_dir(env(&[("HOME", "/home/u")])),
            Path::new("/home/u/.local/share/m2-alpha/Screenshots")
        );
        assert_eq!(
            screenshot_dir(env(&[
                ("M2_SCREENSHOTS", "D:/shots"),
                ("LOCALAPPDATA", "x")
            ])),
            Path::new("D:/shots")
        );
        assert_eq!(shot_name(1_795_654_800, 7), "photo-20261126-010000-007.png");
    }

    #[test]
    fn pictures_are_pngs() {
        let dir = std::env::temp_dir().join(format!("m2-photo-{}", std::process::id()));
        let path = dir.join("p.png");
        let rgba: Vec<u8> = (0..4 * 3 * 2).map(|i| i as u8).collect();
        write_png(&path, 3, 2, &rgba).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
