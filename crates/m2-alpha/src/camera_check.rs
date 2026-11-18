//! `--camera-check <kind>`: the pad's vehicle and turret cameras driven by
//! scripted stick input, offscreen, against the 360 numbers in their PRs'
//! recipes. It reads the game's template database for the records the
//! cameras use, feeds [`crate::pad::Pad`] a stick for a few seconds at a
//! sixtieth a frame, and reads the turn it hands the game. No world is
//! loaded and nothing is drawn.
//!
//! Prints `camera-check: PASS ...` or `camera-check: FAIL ...` per case
//! and `camera-check: RESULT PASS` (or `FAIL`) at the end.
//!
//! Kinds: `tank` (the tank camera, [`m2_engine::pad_camera::TankLook`]),
//! `turret` (an emplaced gun's, [`m2_engine::pad_camera::TurretLook`]),
//! `heli` (a helicopter pilot's, [`m2_engine::pad_camera::HeliLook`]),
//! `heli-follow` (that camera following the helicopter's turn, nose and
//! bank, [`m2_engine::pad_camera::HeliFollow`], with the helicopter's
//! motion scripted), `turret-limits` (the turret camera's eased input
//! dropped at a limit, and the M113 AA driver's turret camera).

use std::collections::HashMap;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_engine::Install;
use m2_engine::tuning::VehicleTuning;
use m2_sys::xinput::PadState;

use crate::input::Input;
use crate::pad::{Mode, Pad};

/// A sixtieth of a second, the engine's tick.
pub const DT: f64 = 1.0 / 60.0;

/// The template database's vehicle tunings and names.
pub struct Records {
    pub tunings: HashMap<u32, VehicleTuning>,
    pub names: HashMap<u32, String>,
    /// Each template's turret camera (an emplaced gun's).
    pub turrets: HashMap<u32, m2_engine::pad_camera::TurretLookTuning>,
}

impl Records {
    pub fn read(install: &Install) -> anyhow::Result<Self> {
        let mut source = install.open_world()?;
        let hash = {
            let index = source.archive().index();
            index
                .assets()
                .iter()
                .find(|a| index.type_hash(a) == TEMPLATE_DB_TYPE)
                .map(|a| a.name_hash)
                .ok_or_else(|| anyhow::anyhow!("no template database"))?
        };
        let data = source.asset(hash, TEMPLATE_DB_TYPE)?;
        let db = Layer::parse(&data)?;
        Ok(Self {
            tunings: m2_engine::tuning::read(&db, None),
            turrets: m2_engine::tuning::turret_cameras(&db),
            names: db
                .names()
                .into_iter()
                .map(|(k, v)| (k, v.to_owned()))
                .collect(),
        })
    }

    /// The first template (by id) whose name holds `part` (any case) and
    /// whose tuning `pick` takes.
    pub fn find<T>(
        &self,
        part: &str,
        pick: impl Fn(&VehicleTuning) -> Option<T>,
    ) -> Option<(String, T)> {
        let part = part.to_ascii_lowercase();
        let mut ids: Vec<_> = self.tunings.keys().copied().collect();
        ids.sort_unstable();
        ids.into_iter().find_map(|id| {
            let name = self.names.get(&id)?;
            if !name.to_ascii_lowercase().contains(&part) {
                return None;
            }
            Some((name.clone(), pick(&self.tunings[&id])?))
        })
    }
}

impl Records {
    /// The first template (by id) whose name holds `part` (any case) with
    /// a turret camera.
    pub fn turret(&self, part: &str) -> Option<(String, m2_engine::pad_camera::TurretLookTuning)> {
        let part = part.to_ascii_lowercase();
        let mut ids: Vec<_> = self.turrets.keys().copied().collect();
        ids.sort_unstable();
        ids.into_iter().find_map(|id| {
            let name = self.names.get(&id)?;
            name.to_ascii_lowercase()
                .contains(&part)
                .then(|| (name.clone(), self.turrets[&id]))
        })
    }
}

/// The checks' tally.
#[derive(Default)]
pub struct Tally {
    pub failed: usize,
    pub passed: usize,
}

impl Tally {
    /// `got` within `tolerance` (a share) of `want`, or both 0.
    pub fn near(&mut self, what: &str, got: f64, want: f64, tolerance: f64) {
        let pass = if want == 0.0 {
            got.abs() < 1e-6
        } else {
            ((got - want) / want).abs() <= tolerance
        };
        self.line(
            pass,
            &format!("{what}: {got:.2} (360: {want} ±{:.0}%)", tolerance * 100.0),
        );
    }

    pub fn line(&mut self, pass: bool, what: &str) {
        if pass {
            self.passed += 1;
        } else {
            self.failed += 1;
        }
        eprintln!(
            "camera-check: {} {what}",
            if pass { "PASS" } else { "FAIL" }
        );
    }

    pub fn missing(&mut self, what: &str) {
        self.line(false, &format!("{what}: no such record in the data"));
    }

    /// Prints the result line; whether every case passed.
    pub fn finish(self) -> bool {
        let pass = self.failed == 0 && self.passed > 0;
        eprintln!(
            "camera-check: RESULT {} ({} passed, {} failed)",
            if pass { "PASS" } else { "FAIL" },
            self.passed,
            self.failed
        );
        pass
    }
}

/// The right stick held at `stick` (raw, -1..1) for `seconds` in `mode`:
/// the pad's last frame's turn, degrees a second (yaw, pitch).
pub fn hold(pad: &mut Pad, mode: Mode, stick: [f32; 2], seconds: f64) -> (f64, f64) {
    let mut input = Input::default();
    let s = PadState {
        right_raw: stick,
        ..PadState::default()
    };
    let frames = (seconds / DT).round() as usize;
    let mut last = (0.0, 0.0);
    for _ in 0..frames {
        pad.feed(&mut input, s, mode, DT);
        last = input.take_pad_turn();
        input.take_pad_look();
    }
    (last.0.to_degrees() / DT, last.1.to_degrees() / DT)
}

/// Runs the checks of `kind`; whether all passed.
pub fn run(install: &Install, kind: &str) -> anyhow::Result<bool> {
    let records = Records::read(install)?;
    let mut t = Tally::default();
    match kind {
        "tank" => tank(&records, &mut t),
        "turret" => turret(&records, &mut t),
        "heli" => heli(&records, &mut t),
        "heli-follow" => heli_follow(&records, &mut t),
        "turret-limits" => turret_limits(&records, &mut t),
        other => anyhow::bail!(
            "--camera-check: no kind {other:?} (tank, turret, heli, heli-follow, turret-limits)"
        ),
    }
    Ok(t.finish())
}

/// #700's recipe: a LAV III at full stick 172 deg/s once eased, at 0.75
/// 95, at 0.4 nothing; an AMX-30 at full 69.
fn tank(records: &Records, t: &mut Tally) {
    let look = |v: &VehicleTuning| v.camera.and_then(|c| c.look);
    for (part, cases) in [
        ("lav", &[(1.0, 172.0), (0.75, 95.0), (0.4, 0.0)][..]),
        ("amx", &[(1.0, 69.0)][..]),
    ] {
        let Some((name, tuning)) = records.find(part, look) else {
            t.missing(&format!("tank {part}"));
            continue;
        };
        for &(x, want) in cases {
            let mut pad = Pad::default();
            pad.tank_tuning = Some(tuning);
            let (yaw, _) = hold(&mut pad, Mode::Tank, [x, 0.0], 3.0);
            t.near(
                &format!("tank {name} stick x {x}: yaw rate deg/s"),
                yaw.abs(),
                want,
                0.05,
            );
        }
    }
}

/// #705's recipe: an emplaced Quad 50 at x = 1.0 / 0.75 / 0.5 for 2 s,
/// about 115 / 63 / 12 deg/s once eased; its pitch held to 28.6 deg up
/// and 22.9 down.
fn turret(records: &Records, t: &mut Tally) {
    let Some((name, tuning)) = records.turret("quad") else {
        t.missing("turret quad");
        return;
    };
    for (x, want, tol) in [(1.0, 115.0, 0.05), (0.75, 63.0, 0.05), (0.5, 12.0, 0.15)] {
        let mut pad = Pad::default();
        pad.turret_tuning = Some(tuning);
        let (yaw, _) = hold(&mut pad, Mode::OnFoot, [x, 0.0], 2.0);
        t.near(
            &format!("turret {name} stick x {x}: yaw rate deg/s"),
            yaw.abs(),
            want,
            tol,
        );
    }
    let (low, high) = tuning.pitch_up();
    t.near(
        &format!("turret {name}: pitch up limit deg"),
        f64::from(high.to_degrees()),
        28.6,
        0.01,
    );
    t.near(
        &format!("turret {name}: pitch down limit deg"),
        f64::from(-low.to_degrees()),
        22.9,
        0.01,
    );
}

/// The right stick held at `stick` for `seconds` in a helicopter with the
/// view's pitch kept at `pitch` (radians up) each frame (`Some`) or let
/// move (`None`, from the pad's own): the last frame's pitch rate
/// (degrees a second) and the pitch reached (degrees).
fn heli_hold(
    tuning: m2_engine::pad_camera::HeliLookTuning,
    stick: [f32; 2],
    seconds: f64,
    start: f32,
    held: bool,
) -> (f64, f64) {
    let mut pad = Pad::default();
    pad.heli_tuning = Some(tuning);
    pad.pitch = start;
    let mut input = Input::default();
    let s = PadState {
        right_raw: stick,
        ..PadState::default()
    };
    let mut rate = 0.0;
    for _ in 0..(seconds / DT).round() as usize {
        if held {
            pad.pitch = start;
        }
        pad.feed(&mut input, s, Mode::Helicopter, DT);
        let (yaw, pitch) = input.take_pad_turn();
        input.take_pad_look();
        rate = (pitch.abs() + yaw.abs()).to_degrees() / DT;
    }
    (rate, f64::from(pad.pitch.to_degrees()))
}

/// #706's recipe: an MD-500's pilot camera, the stick up from its default
/// view (0.25 rad down) about 71 deg/s at first, stopping at 28.6 deg up;
/// down from level about 94 deg/s at first, stopping at 57.3 down; the
/// stick's x turns nothing.
fn heli(records: &Records, t: &mut Tally) {
    let Some((name, tuning)) = records
        .find("md500", |v| v.heli_camera)
        .or_else(|| records.find("md-500", |v| v.heli_camera))
    else {
        t.missing("heli md500");
        return;
    };
    let (rate, _) = heli_hold(tuning, [0.0, 1.0], 1.0, -0.25, true);
    t.near(
        &format!("heli {name} stick up from 0.25 rad down: deg/s at first"),
        rate,
        71.0,
        0.05,
    );
    let (_, reached) = heli_hold(tuning, [0.0, 1.0], 6.0, -0.25, false);
    t.near(
        &format!("heli {name} stick up held: pitch reached deg"),
        reached,
        28.6,
        0.02,
    );
    let (rate, _) = heli_hold(tuning, [0.0, -1.0], 1.0, 0.0, true);
    t.near(
        &format!("heli {name} stick down from level: deg/s at first"),
        rate,
        94.0,
        0.05,
    );
    let (_, reached) = heli_hold(tuning, [0.0, -1.0], 6.0, 0.0, false);
    t.near(
        &format!("heli {name} stick down held: pitch reached deg (down)"),
        -reached,
        57.3,
        0.02,
    );
    let (rate, _) = heli_hold(tuning, [1.0, 0.0], 1.0, 0.0, true);
    t.near(
        &format!("heli {name} stick x: camera turn deg/s"),
        rate,
        0.0,
        0.0,
    );
}

/// #711's recipe: an MD-500's view trails its turn, closing 10% of the
/// gap a sixtieth (`YawLag` 0.9); a nose 10 deg up lifts the view 7.5 deg
/// (`PitchAmp` 0.75); a 20 deg bank rolls it about 13 deg (`RollAmp`).
fn heli_follow(records: &Records, t: &mut Tally) {
    use m2_engine::pad_camera::HeliFollow;
    let Some((name, tuning)) = records.find("md-500", |v| v.heli_camera) else {
        t.missing("heli md-500");
        return;
    };
    let dt = DT as f32;
    let mut f = HeliFollow::default();
    f.step(&tuning, 0.0, 0.0, 0.0, dt);
    let (yaw, _, _) = f.step(&tuning, 1.0, 0.0, 0.0, dt);
    t.near(
        &format!("heli-follow {name}: share of a 1 rad turn closed in a sixtieth"),
        f64::from(yaw),
        0.10,
        0.02,
    );
    let mut f = HeliFollow::default();
    f.step(&tuning, 0.0, 0.0, 0.0, dt);
    let mut lift = 0.0;
    let mut roll = 0.0;
    for _ in 0..240 {
        let (_, l, r) = f.step(&tuning, 0.0, 10f32.to_radians(), 20f32.to_radians(), dt);
        lift = l;
        roll = r;
    }
    t.near(
        &format!("heli-follow {name}: view lift for a 10 deg nose up, deg"),
        f64::from(lift.to_degrees()),
        7.5,
        0.03,
    );
    t.near(
        &format!("heli-follow {name}: view roll for a 20 deg bank, deg"),
        f64::from(roll.to_degrees()),
        13.0,
        0.05,
    );
}

/// #713's recipe: at a limit, pushing on drops the axis's eased input to
/// 0 before the frame's step (0x824C9538, 0x824C9058), so it reads one
/// step from rest after it; reversed, it is non-zero on the first frame. And the M113 AA's driver's seat has its turret camera.
fn turret_limits(records: &Records, t: &mut Tally) {
    let Some((name, tuning)) = records.turret("quad") else {
        t.missing("turret quad");
        return;
    };
    let push = |pad: &mut Pad, stick: [f32; 2], frames: usize| {
        let mut input = Input::default();
        let s = PadState {
            right_raw: stick,
            ..PadState::default()
        };
        for _ in 0..frames {
            pad.feed(&mut input, s, Mode::OnFoot, DT);
            input.take_pad_turn();
            input.take_pad_look();
        }
    };
    let (_, high) = tuning.pitch_up();
    let mut pad = Pad::default();
    pad.turret_tuning = Some(tuning);
    pad.turret_base = Some(0.0);
    pad.yaw = 0.0;
    // Eased up to speed below the limit, then held at the limit pushing on.
    pad.pitch = 0.0;
    push(&mut pad, [0.0, 1.0], 60);
    let before = pad.turret_camera.pitch.value;
    pad.pitch = high;
    push(&mut pad, [0.0, 1.0], 1);
    let at = pad.turret_camera.pitch.value;
    // Dropped to 0 before the frame's step, then one step from rest:
    // (1 - keep) of the input.
    let one_step = 1.0 - tuning.pitch_keep[0];
    t.line(
        before > 0.5 && (at - one_step).abs() < 1e-4,
        &format!(
            "turret-limits {name}: at the pitch limit pushing on, eased input {before:.3} -> {at:.3} (360: dropped to 0, then one step {one_step:.3})"
        ),
    );
    push(&mut pad, [0.0, -1.0], 1);
    let back = pad.turret_camera.pitch.value;
    t.line(
        back != 0.0,
        &format!(
            "turret-limits {name}: reversed, first frame eased input {back:.3} (360: non-zero)"
        ),
    );
    match records.find("m113", |v| v.driver_turret.map(|_| ())) {
        Some((m113, ())) => t.line(
            true,
            &format!("turret-limits {m113}: driver's seat has its CameraTurret"),
        ),
        None => t.missing("turret-limits m113 driver's CameraTurret"),
    }
}
