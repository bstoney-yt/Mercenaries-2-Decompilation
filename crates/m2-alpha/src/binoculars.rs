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
//! Invented: the start at 2x; the wheel (and Tab, the pad's right
//! bumper, which step up and round) changing it a level a notch; the
//! view from the camera's pivot above her head, with her body left out;
//! the narrower lens for a magnification ([`View::fov`]); the look slowed
//! by it; that she stands and does not fire while they are up. What the
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

/// The zoom levels the zoom meter spans.
pub const MIN_ZOOM: u8 = 1;
pub const MAX_ZOOM: u8 = 7;
/// The level they come up at (invented).
pub const START_ZOOM: u8 = 2;

/// The binoculars up: how far they magnify.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binoculars {
    pub zoom: u8,
}

impl Default for Binoculars {
    fn default() -> Self {
        Self { zoom: START_ZOOM }
    }
}

impl Binoculars {
    /// Zoom in (`by` > 0) or out a level per step, held to the meter.
    pub fn step(&mut self, by: i32) {
        let z = (i32::from(self.zoom) + by).clamp(i32::from(MIN_ZOOM), i32::from(MAX_ZOOM));
        self.zoom = z as u8;
    }

    /// Up a level, and from the top back to the bottom.
    pub fn cycle(&mut self) {
        self.zoom = if self.zoom >= MAX_ZOOM {
            MIN_ZOOM
        } else {
            self.zoom + 1
        };
    }

    /// The lens through them, degrees: `fov` (the plain view's) narrowed
    /// so what is seen is `zoom` times as large.
    pub fn fov(&self, fov: f32) -> f32 {
        let half = (fov.to_radians() * 0.5).tan() / f32::from(self.zoom.max(1));
        (2.0 * half.atan()).to_degrees()
    }

    /// How much the look is slowed: by the magnification, so a mouse
    /// count moves the picture as far as without them.
    pub fn look_scale(&self) -> f32 {
        1.0 / f32::from(self.zoom.max(1))
    }
}

/// The lens, degrees, for `fov` (the plain view's) magnified `zoom`
/// times: what is seen is `zoom` times as large.
pub fn magnified(fov: f32, zoom: f32) -> f32 {
    let half = (fov.to_radians() * 0.5).tan() / zoom.max(1.0);
    (2.0 * half.atan()).to_degrees()
}

/// What the binocular screen is told each frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// What the reticle is on.
    pub focus: Option<Focus>,
    pub zoom: u8,
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
        for z in MIN_ZOOM..=MAX_ZOOM {
            let at = zoom_meter(f32::from(z));
            assert!((-0.5..=0.5).contains(&at), "{z}: {at}");
        }
        // The ends of the range are the meter's ends; one more is off it.
        assert_eq!(zoom_meter(f32::from(MIN_ZOOM)), 0.5);
        assert_eq!(zoom_meter(f32::from(MAX_ZOOM)), -0.5);
        assert!(zoom_meter(f32::from(MAX_ZOOM + 1)) < -0.5);
        assert!(zoom_meter(f32::from(MIN_ZOOM - 1)) > 0.5);
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
    fn zoom_steps_and_cycles_within_the_meter() {
        let mut b = Binoculars::default();
        assert_eq!(b.zoom, START_ZOOM);
        b.step(-5);
        assert_eq!(b.zoom, MIN_ZOOM);
        b.step(20);
        assert_eq!(b.zoom, MAX_ZOOM);
        b.cycle();
        assert_eq!(b.zoom, MIN_ZOOM);
    }

    #[test]
    fn the_lens_magnifies_by_the_zoom() {
        let b = Binoculars { zoom: 4 };
        let fov = b.fov(60.0);
        let ratio = (30f32.to_radians().tan()) / (fov.to_radians() * 0.5).tan();
        assert!((ratio - 4.0).abs() < 1e-4, "{fov} {ratio}");
        assert!((Binoculars { zoom: 1 }.fov(60.0) - 60.0).abs() < 1e-3);
        assert_eq!(b.look_scale(), 0.25);
    }
}
