//! Her binoculars: B (the pad's right stick pressed) puts them up on foot,
//! and the game's own binocular screen shows while she looks through them.
//!
//! The screen is the game's: the binocular and sniper scope layout
//! (`MrxGuiBinocularsLayout`, script `95708d95`) is built with the HUD,
//! and its handlers (`MrxGuiBinoculars`, script `cc386b17`) run on the
//! engine's events, which [`crate::gamehud`] fires from [`View`]:
//!
//! - `BinocularsEnter` `{ uPlayerGuid, bSniper = false }`: the screen shows,
//!   the HUD goes to sleep (`MrxGuiManager.ToggleHud(owner, false,
//!   "scope")`), and the frame closes in from 3x its size over 0.25 s
//!   (`_nIntroZoomScale`, `_nIntroZoomTime`) while the reticle fades in;
//! - `BinocularsUpdate` `{ uPlayerGuid, nCameraHeading, nPitch, nZoomLevel,
//!   FactionTexture }`: each meter's pointer moves along it, the
//!   heading's across the top, the pitch's down the left and the zoom's
//!   down the right; a number for the faction puts its badge away;
//! - `BinocularsExit` `{ uPlayerGuid }`: the frame opens out again, the
//!   HUD wakes.
//!
//! Where the handlers put the pointers (from the scripts), as a share of
//! the meter's length from its middle: the heading `nCameraHeading / 360`
//! (so -180..180 degrees spans the meter), the pitch `(nPitch - 6) / 100`
//! and the zoom `(nZoomLevel - 4) / -6` (up the meter as it grows). See
//! [`heading_meter`], [`pitch_meter`] and [`zoom_meter`].
//!
//! Inferred from those spans: the zoom levels are 1 to 7 (the ones whose
//! pointer stays on its meter; the scripts' other zoom readout writes
//! `"ZOOM x " .. nZoomLevel`, so a level is a magnification); the pitch is
//! degrees up; the heading is the one the compass gets (degrees, growing
//! turning left), wrapped to -180..180.
//!
//! The camera through them is the 360's binocular camera (class
//! 0x820EC8D8, [`m2_engine::binocular_camera`]): from 2x to 7x, starting
//! at 2x, zoomed continuously with the left stick's (here the movement
//! keys') up and down; the lens the game's field of view over the zoom;
//! the look at most 1/zoom rad/s, eased, held to 1.5 rad either side of
//! where they came up and to its pitch limits; from 2 m above her feet.
//!
//! Invented: a wheel notch zooms as a quarter second of full stick; the
//! mouse's turn is read as the stick's push (a half turn a second is
//! full); her body is left out; she stands and does not fire while they
//! are up. What the
//! reticle is on is told as [`Focus`]: from the 360 code (0x8243B250,
//! 0x82443160) a ray of [`FOCUS_RANGE`] from the camera, the object's
//! `FactionMarker` texture as `FactionTexture` and its `LocalizedName`
//! key as `sFocusName` (`[0x%08x]`). Inferred: the 2 m around the ray,
//! from the 2.0 the 360 hands that test. Invented: only vehicles and
//! people count, the two the Beta can name, and a soldier without a
//! template of his own shows his faction's badge and no name.
//!
//! Aiming a gun with a scope (`WeaponScope`: the sniper rifles', the
//! anti-material rifle's) shows the same screen with `bSniper` set, its
//! "Sniper Scope" tree, at the scope's magnification (its
//! `MinZoomLevel` to `MaxZoomLevel`; [`magnified`]), and she can fire
//! through it (gap-list sweep; the screen's sniper tree not compared with
//! footage: none found).

use m2_engine::binocular_camera::{BINOCULARS, BinocularCamera};

/// A mouse turn a second read as the stick pushed all the way, radians
/// (invented: the 360 reads the stick).
pub const LOOK_FULL: f32 = std::f32::consts::PI;
/// A wheel notch zooms as this long of full stick, seconds (invented).
pub const NOTCH: f32 = 0.25;

/// The binoculars up: their camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Binoculars {
    pub camera: BinocularCamera,
}

impl Binoculars {
    /// Put up while she looks along `heading` and `pitch` (radians).
    pub fn up(heading: f32, pitch: f32) -> Self {
        Self {
            camera: BinocularCamera::start(BINOCULARS, heading, pitch),
        }
    }

    /// How far they magnify now.
    pub fn zoom(&self) -> f32 {
        self.camera.zoom
    }

    /// A step of `dt` seconds with her look's turn `look` (radians this
    /// step, x turning the heading up, y up), the movement's up-down
    /// `axis` and `notches` of the wheel.
    pub fn step(&mut self, look: [f32; 2], axis: f32, notches: i32, dt: f32) {
        let full = (LOOK_FULL * dt).max(1e-6);
        // The stick's +x turns the heading down.
        self.camera.input = [
            (-look[0] / full).clamp(-1.0, 1.0),
            (look[1] / full).clamp(-1.0, 1.0),
        ];
        let wheel = notches as f32 * NOTCH / dt.max(1e-6);
        self.camera.zoom_axis = axis.clamp(-1.0, 1.0) + wheel;
        self.camera.step(dt);
        self.camera.zoom_axis = 0.0;
    }
}

/// `--binocular-check`: the binoculars driven the way the game drives
/// them ([`Binoculars::step`]) by a scripted look, zoom axis and wheel,
/// offscreen and without a world, against #724's 360 numbers. Prints
/// `binocular-check: PASS ...` or `FAIL ...` lines and
/// `binocular-check: RESULT PASS` (or `FAIL`); whether all passed.
pub fn check() -> bool {
    use m2_engine::binocular_camera::{PITCH_LIMITS, YAW_LIMIT};
    use m2_math::glam::Vec3;
    let dt = 1.0 / 60.0;
    let mut ok = true;
    let mut line = |pass: bool, what: String| {
        ok &= pass;
        eprintln!(
            "binocular-check: {} {what}",
            if pass { "PASS" } else { "FAIL" }
        );
    };
    let frames = |s: f32| (s / dt).round() as usize;
    let mut b = Binoculars::up(0.0, 0.0);
    line(
        b.zoom() == 2.0,
        format!("up: zoom {:.3} (360: 2.0, +0x600)", b.zoom()),
    );
    let lens = b.camera.fov(m2_engine::projection::FOV);
    line(
        (lens - 0.479_875).abs() < 1e-5,
        format!("up: lens {lens:.5} rad (360: 0.95975 / 2 = 0.47988, +0x5D8)"),
    );
    let eye = b.camera.eye(Vec3::ZERO, false);
    line(
        (eye - Vec3::new(0.0, 2.0, 0.2)).length() < 1e-5,
        format!("up: eye {eye:.2?} from her feet (360: 2 m up, 0.2 m ahead)"),
    );
    for _ in 0..frames(1.0) {
        b.step([0.0; 2], 1.0, 0, dt);
    }
    line(
        ((b.zoom() - 5.414) / 5.414).abs() < 0.01,
        format!(
            "zoom axis full 1 s: zoom {:.3} (360: 2 x (1 + 1/60)^60 = 5.41, about 2e; +-1%)",
            b.zoom()
        ),
    );
    for _ in 0..frames(2.0) {
        b.step([0.0; 2], 1.0, 0, dt);
    }
    line(
        b.zoom() == 7.0,
        format!("zoom axis full 3 s: zoom {:.3} (360: held at 7)", b.zoom()),
    );
    // The look turning right as fast as counts as full stick.
    let full = std::f32::consts::PI * dt;
    for _ in 0..frames(2.0) {
        b.step([-full, 0.0], 0.0, 0, dt);
    }
    let before = b.camera.yaw;
    b.step([-full, 0.0], 0.0, 0, dt);
    let rate = (before - b.camera.yaw) / dt;
    line(
        (rate - 1.0 / 7.0).abs() < 1e-3,
        format!("full right at 7x: {rate:.4} rad/s (360: 1/zoom = 0.1429)"),
    );
    for _ in 0..frames(60.0) {
        b.step([-full, 0.0], 0.0, 0, dt);
    }
    line(
        (b.camera.yaw + YAW_LIMIT).abs() < 1e-4,
        format!(
            "full right 60 s: heading {:.4} rad (360: held 1.5 from where they came up)",
            b.camera.yaw
        ),
    );
    for _ in 0..frames(60.0) {
        b.step([0.0, full], 0.0, 0, dt);
    }
    line(
        (b.camera.pitch - PITCH_LIMITS.1).abs() < 1e-5,
        format!(
            "full up 60 s: pitch {:.5} rad (360: held at 0.90425)",
            b.camera.pitch
        ),
    );
    eprintln!(
        "binocular-check: RESULT {}",
        if ok { "PASS" } else { "FAIL" }
    );
    ok
}

/// The lens, radians, for the game's field of view `fov` (radians)
/// magnified `zoom` times: the 360's binocular and scope camera divides
/// the angle itself (0x824BF700).
pub fn magnified(fov: f32, zoom: f32) -> f32 {
    fov / zoom.max(1e-4)
}

/// What the binocular screen is told each frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// What the reticle is on.
    pub focus: Option<Focus>,
    pub zoom: f32,
    /// Her gun's scope, not her binoculars (`bSniper`).
    pub sniper: bool,
    /// Degrees, growing turning left, -180..180 ([`wrap_degrees`]).
    pub heading: f32,
    /// Degrees up from the horizon.
    pub pitch: f32,
}

/// How far the reticle reaches, metres (the 360 code).
pub const FOCUS_RANGE: f32 = 200.0;
/// How near the reticle's line something must be, metres.
pub const FOCUS_REACH: f32 = 2.0;

/// The textures `FactionMarker` records name, by their name hashes
/// (`m2_assets::name_hash`). The data's ninth texture (5b9de759, on 15
/// templates) has no name found: no badge for those.
const MARKERS: [&str; 8] = [
    "HUD_faction_AN",
    "HUD_faction_CH",
    "HUD_faction_CV",
    "HUD_faction_GR",
    "HUD_faction_OC",
    "HUD_faction_PMC",
    "HUD_faction_PR",
    "HUD_faction_VZ",
];

/// The texture a `FactionMarker` names.
pub fn marker_texture(hash: u32) -> Option<&'static str> {
    MARKERS
        .into_iter()
        .find(|n| m2_assets::name_hash(n) == hash)
}

/// The badge of a faction as the alpha names it ([`crate::npc::faction_of`]).
pub fn faction_texture(faction: &str) -> Option<&'static str> {
    Some(match faction {
        "All" => "HUD_faction_AN",
        "Chi" => "HUD_faction_CH",
        "Gur" => "HUD_faction_GR",
        "Oil" => "HUD_faction_OC",
        "Pir" => "HUD_faction_PR",
        "Vza" => "HUD_faction_VZ",
        _ => return None,
    })
}

/// What the reticle is on, for the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Focus {
    pub texture: Option<&'static str>,
    /// Its name's string table key.
    pub name: Option<u32>,
}

/// How far along the ray from `eye` by `ahead` (a unit vector) `at` lies,
/// if it's within the reticle's reach.
pub fn along_ray(
    eye: m2_math::glam::Vec3,
    ahead: m2_math::glam::Vec3,
    at: m2_math::glam::Vec3,
) -> Option<f32> {
    let t = (at - eye).dot(ahead);
    ((0.0..=FOCUS_RANGE).contains(&t) && (eye + ahead * t).distance(at) <= FOCUS_REACH).then_some(t)
}

/// `degrees` brought into -180..180.
pub fn wrap_degrees(degrees: f32) -> f32 {
    let d = (degrees + 180.0).rem_euclid(360.0) - 180.0;
    if d < -180.0 { d + 360.0 } else { d }
}

/// Where `HandleHeadingUpdate` puts the heading's pointer: a share of the
/// meter's length right of its middle.
pub fn heading_meter(heading: f32) -> f32 {
    heading / 360.0
}

/// Where `HandleVertScrollUpdate` puts the pitch's pointer: a share of the
/// meter's length below its middle.
pub fn pitch_meter(pitch: f32) -> f32 {
    (pitch - 6.0) / 100.0
}

/// Where `HandleZoomUpdate` puts the zoom's pointer: a share of the
/// meter's length below its middle.
pub fn zoom_meter(zoom: f32) -> f32 {
    (zoom - 4.0) / -6.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_math::glam::Vec3;

    #[test]
    fn the_reticle_reaches_200_metres_and_2_either_side() {
        let (eye, ahead) = (Vec3::new(1.0, 2.0, 3.0), Vec3::X);
        let at = |x, y| along_ray(eye, ahead, eye + Vec3::new(x, y, 0.0));
        assert_eq!(at(50.0, 1.5), Some(50.0));
        assert_eq!(at(50.0, 2.5), None);
        assert_eq!(at(201.0, 0.0), None);
        assert_eq!(at(-5.0, 0.0), None);
    }

    #[test]
    fn the_markers_are_the_datas_textures() {
        // `FactionMarker`'s hashes in the template database.
        assert_eq!(marker_texture(0x9fad_a550), Some("HUD_faction_PMC"));
        assert_eq!(marker_texture(0x4434_32fc), Some("HUD_faction_VZ"));
        assert_eq!(marker_texture(0x5b9d_e759), None);
    }

    #[test]
    fn every_zoom_level_keeps_its_pointer_on_the_meter() {
        for z in 1..=7u8 {
            let at = zoom_meter(f32::from(z));
            assert!((-0.5..=0.5).contains(&at), "{z}: {at}");
        }
        // The meter spans 1 to 7; one more is off it. The binoculars'
        // camera keeps to 2 to 7 of it.
        assert_eq!(zoom_meter(1.0), 0.5);
        assert_eq!(zoom_meter(BINOCULARS.max), -0.5);
        assert!(zoom_meter(8.0) < -0.5);
        assert!(zoom_meter(0.0) > 0.5);
    }

    #[test]
    fn the_heading_meter_spans_a_whole_turn() {
        assert_eq!(heading_meter(wrap_degrees(180.0)), -0.5);
        assert_eq!(heading_meter(wrap_degrees(-90.0)), -0.25);
        assert_eq!(heading_meter(wrap_degrees(450.0)), 0.25);
        assert_eq!(pitch_meter(6.0), 0.0);
        assert_eq!(pitch_meter(56.0), 0.5);
    }

    #[test]
    fn they_come_up_at_2x_and_zoom_with_the_stick_and_wheel() {
        let dt = 1.0 / 60.0;
        let mut b = Binoculars::up(0.0, 0.0);
        assert_eq!(b.zoom(), 2.0);
        // Two notches in: half a second of full stick, about e^0.5.
        b.step([0.0; 2], 0.0, 2, dt);
        assert!(b.zoom() > 2.0 && b.zoom() < 7.0, "{}", b.zoom());
        for _ in 0..600 {
            b.step([0.0; 2], 1.0, 0, dt);
        }
        assert_eq!(b.zoom(), 7.0);
        // The mouse to the right: the heading goes down, at most 1/7 rad/s.
        for _ in 0..120 {
            b.step([-1.0, 0.0], 0.0, 0, dt);
        }
        let before = b.camera.yaw;
        b.step([-1.0, 0.0], 0.0, 0, dt);
        assert!(((before - b.camera.yaw) / dt - 1.0 / 7.0).abs() < 1e-3);
    }

    #[test]
    fn the_lens_divides_the_angle_by_the_zoom() {
        assert!((magnified(0.95975, 4.0) - 0.2399375).abs() < 1e-6);
    }
}
