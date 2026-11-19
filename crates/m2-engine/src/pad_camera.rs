//! How the right stick turns the on-foot camera, as the 360 does it:
//! the stick's dead zone and rescale, the camera's response curves, its
//! per-axis input inertia, a base rate plus an extra rate that builds up
//! while the stick is held hard over, and the sensitivity setting, which
//! blends between three sets of those parameters.
//!
//! # Provenance
//!
//! From the 360 code:
//!
//! - **The stick** ([`stick`], 0x8285A770, for the right stick through
//!   0x8285A060 from XInput's `sThumbRX`/`sThumbRY` times 1/32767,
//!   0x8200A0D8): each axis loses 0.2 (0x82000A78) of its size and is
//!   scaled by 1.25 (0x820564E0); the result's length under 0.3
//!   (0x8205199C) is nothing, above it `min((r - 0.3) * 1.4286, 1)`
//!   (0x82077580) along the same direction. It is packed into four bytes
//!   (times 255, 0x820F8D80, truncated) that the input layer reads back
//!   as `(more - less) / 255` (0x8234C7D8, 0x820CB628).
//! - **The messages**: the player's input (0x8242E7A0) posts the stick's
//!   x as the look-yaw message (0x4BD465C7) and its y as look-pitch
//!   (0x574220AC); the on-foot camera (vtable 0x820F0290, built by
//!   0x824C2ED0) keeps them at +0x600 (negated: its yaw turns the other
//!   way) and +0x604 (0x824C5E18).
//! - **The curves** (0x824C5200): `sign(v) v^2` feeds the base rate and
//!   `sign(v) (1 - (1 - |v|)^2)` the extra rate ([`curves`]).
//! - **The input inertia** ([`Inertia`], 0x824C2DD0): the input, averaged
//!   with the last one, is approached by a fraction kept a sixtieth of a
//!   second ([`approach`], 0x824BB050) while there is input, another
//!   without; turning the other way it first falls to nothing.
//! - **The extra rate** ([`Extra`], 0x824C2C18): with the stick held the
//!   same way nearly all over (its curve squared at least 0.9604,
//!   0x820F8CA8) or pushed diagonally (both axes together at least 0.25,
//!   0x820DABDC, the other more than 0.01, 0x820C8114) for longer than
//!   the hold time (or once it already is over 0.1, 0x820BB468), the
//!   extra rate approaches its target; otherwise it falls back.
//! - **The step** (0x824C5200): the turn is the filtered input times the
//!   base rate, plus, while both agree in sign, the extra rate times the
//!   second curve, a step's time each (the extra faded in over a camera
//!   blend, `(+0x50C / +0x508)^3`, 1 once settled; and a base ramp at
//!   +0x750, 1 unless an untraced flag is set: both taken as 1 here).
//!   Yaw is radians; pitch is the camera's 0..1 place between its low and
//!   high offsets (+0x60C, clamped, 0x824C5520; the offsets blend
//!   quadratically, 0x824B95D0).
//! - **The presets** ([`PRESETS`], 0x824C6270): low, middle and high sets
//!   for yaw, pitch, aimed yaw and aimed pitch. Each step (0x824C3CC8)
//!   blends them by the sensitivity `s`: from low to middle over 0 to
//!   0.5, from middle to high over 0.5 to 1 ([`preset`]). `s` is the
//!   player's camera sensitivity (0.5 at start, 0x824BB0F0; copied to the
//!   camera at +0x4F8 by 0x824BB8E8) times the held item's camera row
//!   (+0x9E0, or +0x9E4 aiming, 0x824C3E98): 1 and 0.5 for most items, 0.25
//!   and 0 for those whose `HumanCameraModifier` `YawSpeed` is under 0.75
//!   ([`SENSITIVITY_ROW`]).
//!
//! - **The base ramp** ([`LookCamera::ramp`], 0x824C5200 at +0x750, 1 at
//!   start): approaches 1 keeping 0.75 a sixtieth, or 0.8 keeping 0.5
//!   while an aim-assist target is in view (0x824C42E0) and the stick is
//!   under 0.99 over (0x82B7D824); held at 0.99 or more for a quarter of
//!   a second (0x820DABDC) it is 1 at once.
//! - **The blend fade**: the extra rate times the cube of a camera
//!   blend's progress (+0x50C of +0x508, set by 0x824BEEA0), 1 with no
//!   blend ([`LookCamera::blend`]).
//! - **The item's row** ([`camera_row`], 0x824C3E98): the held item's
//!   `HumanCameraModifier` `RotationSpeed` (every weapon's is 3, by its
//!   yaw speed) picks a row by `YawSpeed`: over 1.25 (0x820BB644) row 1,
//!   under 0.75 row 2, else 0; the camera eases its multipliers to the
//!   row's (0x824C4038, keeping 0.5 a sixtieth, from 1 and 0.5).
//! - **The pitch** ([`pitch_at`]): the camera's place and the point it
//!   looks at are each a quadratic blend of three offsets by the 0..1
//!   (0x824B95D0 on +0x708/+0x714/+0x720 and +0x72C/+0x738/+0x744, eased
//!   to the stance's offsets by 0x824C3158), and the pitch is
//!   `-atan2(place.y - look.y, place.z - look.z)` (0x824C5520). Standing
//!   (0x82DE6D7C): place (y, z) 3.5, 1.4 / 2.1, 3.0 / 0.5, 1.25 and look
//!   2, 0 / 1.5, 0 / 2, 0, so -47.0 to +50.2 degrees; aiming (0x82DE6DC4)
//!   2.8, 1.25 / 1.75, 3.0 / 0.7, 0.9 and 1.85, -0.2 / 1.75, 0 / 1.8, 0,
//!   so -33.2 to +50.7.
//!
//! **Inferred**: that the pause menu's 0 to 100 is the camera sensitivity
//! times 100 (50 is 0.5, as the start value; the profile's
//! `gameSensitivity` is saved, but its use was not traced). **Not
//! ported**: aim assist, so the ramp's target is always 1 and the yaw's
//! aim-assist correction (0x824C5098, by the target's pull, 0x824BAB40)
//! nothing; the offsets' easing between stances.

/// A stance's camera offsets, (y, z) in the camera's frame: its place
/// and the point it looks at, low, middle and high.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Offsets {
    pub place: [[f32; 2]; 3],
    pub look: [[f32; 2]; 3],
}

/// Standing (0x82DE6D7C) and aiming in (0x82DE6DC4).
pub const STANDING: Offsets = Offsets {
    place: [[3.5, 1.4], [2.1, 3.0], [0.5, 1.25]],
    look: [[2.0, 0.0], [1.5, 0.0], [2.0, 0.0]],
};
pub const AIMING: Offsets = Offsets {
    place: [[2.8, 1.25], [1.75, 3.0], [0.7, 0.9]],
    look: [[1.85, -0.2], [1.75, 0.0], [1.8, 0.0]],
};

/// The pitch, radians up from level, at the camera's 0..1 `t`.
pub fn pitch_at(t: f32, o: &Offsets) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let blend = |p: &[[f32; 2]; 3], k: usize| {
        let a = p[0][k] + (p[1][k] - p[0][k]) * t;
        let b = p[1][k] + (p[2][k] - p[1][k]) * t;
        a + (b - a) * t
    };
    let dy = blend(&o.place, 0) - blend(&o.look, 0);
    let dz = blend(&o.place, 1) - blend(&o.look, 1);
    -dy.atan2(dz)
}

/// The 0..1 at which the camera pitches `pitch` (clamped to its range).
pub fn t_at(pitch: f32, o: &Offsets) -> f32 {
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..40 {
        let mid = (lo + hi) * 0.5;
        if pitch_at(mid, o) < pitch {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) * 0.5
}

/// Standing, the pitch's limits, radians ([`pitch_at`] 0 and 1).
pub const PITCH_RANGE: [f32; 2] = [-0.819_867_25, 0.876_058_04];

/// The row a held item's camera modifier picks (0x824C3E98): its
/// `RotationSpeed` category, or with 3 (by its yaw speed) its `YawSpeed`.
pub fn camera_row(rotation_speed: u32, yaw_speed: f32) -> usize {
    match rotation_speed {
        0..=2 => rotation_speed as usize,
        _ if yaw_speed > 1.25 => 1,
        _ if yaw_speed < 0.75 => 2,
        _ => 0,
    }
}

/// One set of an axis's parameters (0x824C6270's eight words, placed by
/// 0x824C3CC8).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Preset {
    /// The share of the gap kept each sixtieth with input, and without.
    pub keep: f32,
    pub keep_idle: f32,
    /// The base rate at full tilt (radians, or pitch's 0..1, a second).
    pub base: f32,
    /// The extra rate's target, and the shares kept going to it and
    /// falling back.
    pub extra: f32,
    pub extra_keep: f32,
    pub release_keep: f32,
    /// How long the stick is held over before the extra builds, seconds.
    pub hold: f32,
    /// The least the extra rises a second while it rises.
    pub min_rise: f32,
}

impl Preset {
    /// The words in the order 0x824C6270 writes them.
    const fn of(w: [f32; 8]) -> Self {
        Self {
            keep: w[0],
            keep_idle: w[1],
            base: w[2],
            extra: w[3],
            extra_keep: w[4],
            release_keep: w[5],
            hold: w[6],
            min_rise: w[7],
        }
    }

    fn lerp(a: &Self, b: &Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let l = |x: f32, y: f32| x * (1.0 - t) + y * t;
        Self {
            keep: l(a.keep, b.keep),
            keep_idle: l(a.keep_idle, b.keep_idle),
            base: l(a.base, b.base),
            extra: l(a.extra, b.extra),
            extra_keep: l(a.extra_keep, b.extra_keep),
            release_keep: l(a.release_keep, b.release_keep),
            hold: l(a.hold, b.hold),
            min_rise: l(a.min_rise, b.min_rise),
        }
    }
}

/// The axes the presets are for.
pub const YAW: usize = 0;
pub const PITCH: usize = 1;
pub const AIM_YAW: usize = 2;
pub const AIM_PITCH: usize = 3;

/// The presets, low, middle and high, by axis ([`YAW`], [`PITCH`],
/// [`AIM_YAW`], [`AIM_PITCH`]); 0x824C6270 at +0x7D4, +0x854, +0x8D4.
pub const PRESETS: [[Preset; 4]; 3] = [
    [
        Preset::of([0.99, 0.5, 1.0, 1.0, 0.975, 0.0, 0.0, 0.0]),
        Preset::of([0.99, 0.8, 0.2, 0.5, 0.975, 0.0, 0.0, 0.0]),
        Preset::of([0.963, 0.725, 0.5, 0.5, 0.975, 0.0, 0.0, 0.0]),
        Preset::of([0.975, 0.2, 0.5, 0.0, 0.975, 0.0, 0.0, 0.0]),
    ],
    [
        Preset::of([0.9, 0.5, 1.0, 4.0, 0.975, 0.0, 0.125, 0.0]),
        Preset::of([0.8, 0.2, 0.5, 0.5, 0.975, 0.0, 0.125, 0.0]),
        Preset::of([0.95, 0.5, 1.0, 2.0, 0.985, 0.0, 0.125, 0.0]),
        Preset::of([0.925, 0.2, 0.75, 0.0, 0.975, 0.0, 0.0, 0.0]),
    ],
    [
        Preset::of([0.9, 0.0, 2.0, 4.0, 0.975, 0.0, 0.125, 3.0]),
        Preset::of([0.85, 0.0, 1.0, 0.0, 0.975, 0.0, 0.0, 0.0]),
        Preset::of([0.925, 0.7, 1.6, 2.0, 0.925, 0.0, 0.125, 0.4]),
        Preset::of([0.9, 0.2, 0.8, 1.0, 0.975, 0.0, 0.0, 0.0]),
    ],
];

/// The held item's camera row, normal and aiming (0x824C3E98 from the
/// rows at +0x9F8, +0xA28 and +0xA58 of the camera): row 0 and 1 give 1
/// and 0.5, row 2 (a `YawSpeed` under 0.75) 0.25 and 0.
pub const SENSITIVITY_ROW: [[f32; 2]; 3] = [[1.0, 0.5], [1.0, 0.5], [0.25, 0.0]];

/// Axis `axis`'s parameters at sensitivity `s` (0..1).
pub fn preset(s: f32, axis: usize) -> Preset {
    let s = s.clamp(0.0, 1.0);
    if s > 0.5 {
        Preset::lerp(&PRESETS[1][axis], &PRESETS[2][axis], (s - 0.5) * 2.0)
    } else {
        Preset::lerp(&PRESETS[0][axis], &PRESETS[1][axis], s * 2.0)
    }
}

/// The engine's sixtieth of a second (0x820EB668).
const TICK: f32 = 0.016_666_668;

/// `x` moved toward `target` for `dt` seconds, keeping `keep` of the
/// gap each sixtieth (0x824BB050).
pub fn approach(mut x: f32, target: f32, keep: f32, mut dt: f32) -> f32 {
    if dt > 16.666_668 {
        return if keep <= 0.9999 { target } else { x };
    }
    let a = 1.0 - keep;
    while dt > TICK {
        if (target - x) * (target - x) < (keep * 1e-5) * (keep * 1e-5) {
            return target;
        }
        dt -= TICK;
        x += (target - x) * a;
    }
    let f = dt * 59.999_996;
    ((target - x) * a + x) * f + (1.0 - f) * x
}

/// A stick, raw -1..=1 each way (up positive), as the camera reads it
/// (see the module's provenance).
pub fn stick(x: f32, y: f32) -> [f32; 2] {
    let cut = |v: f32| (v.abs() - 0.2).max(0.0) * v.signum() * 1.25;
    let (fx, fy) = (cut(x), cut(y));
    let r = fx.hypot(fy);
    if r < 0.3 {
        return [0.0; 2];
    }
    let m = ((r - 0.3) * 1.428_571_5).min(1.0);
    let byte = |c: f32| (c.abs() * 255.0).trunc().copysign(c) / 255.0;
    [byte(fx / r * m), byte(fy / r * m)]
}

/// The two curves of an axis's input `v`: the base rate's and the extra
/// rate's.
pub fn curves(v: f32) -> (f32, f32) {
    let a = v.abs();
    (
        v.signum() * v * v,
        v.signum() * (1.0 - (1.0 - a) * (1.0 - a)),
    )
}

/// An axis's input inertia (0x824C2DD0).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Inertia {
    pub value: f32,
    last: f32,
}

impl Inertia {
    pub fn step(&mut self, input: f32, p: &Preset, dt: f32) {
        let mid = (input - self.last) * 0.5 + self.last;
        let (mut cur, mut dt) = (self.value, dt);
        let keep = if mid == 0.0 {
            p.keep_idle
        } else if cur * mid < 0.0 {
            // Turning back: falls to nothing first, then turns with what
            // is left of the step.
            let r = approach(cur, 0.0, p.keep_idle, dt);
            cur = 0.0;
            dt = if (mid - r).abs() <= 1e-4 {
                0.0
            } else {
                (1.0 - (r.abs() / (mid - r).abs()).clamp(0.0, 1.0)) * dt
            };
            p.keep
        } else {
            p.keep
        };
        self.value = approach(cur, mid, keep, dt);
        self.last = input;
    }
}

/// An axis's extra rate (0x824C2C18).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Extra {
    pub value: f32,
    held: f32,
    last: f32,
}

impl Extra {
    /// `c` the axis's first curve, `other` the other axis's input.
    pub fn step(&mut self, c: f32, other: f32, p: &Preset, dt: f32) {
        let cur = self.value;
        let c2 = c * c;
        let mut keep = p.release_keep;
        let mut target = 0.0;
        let mut holding = false;
        if 0.0 < c * self.last {
            if 0.960_400_04 <= c2 || (0.25 <= c2 + other * other && 0.01 < other * other) {
                self.held += dt;
                holding = true;
                if p.hold < self.held || 0.1 < cur {
                    target = p.extra;
                    keep = p.extra_keep;
                } else {
                    target = cur;
                }
            } else if 0.01 < c2 {
                keep = p.extra_keep;
            }
        }
        if !holding {
            self.held = 0.0;
        }
        let mut d = approach(cur, target, keep, dt) - cur;
        let least = p.min_rise * dt;
        if 0.0 < d && d < least {
            d = least.min(p.extra - cur);
        }
        self.value = cur + d;
        self.last = c;
    }
}

/// The on-foot camera's stick turn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LookCamera {
    /// By [`YAW`], [`PITCH`], [`AIM_YAW`], [`AIM_PITCH`].
    pub inertia: [Inertia; 4],
    pub extra: [Extra; 4],
    /// The base rate's ramp (+0x750) and how long the stick has been
    /// held over (+0x754).
    pub ramp: f32,
    held: f32,
    /// A camera blend's progress, 0..1 (1: none).
    pub blend: f32,
    /// The item row's multipliers as eased, normal and aiming.
    pub rows: [f32; 2],
}

impl Default for LookCamera {
    fn default() -> Self {
        Self {
            inertia: Default::default(),
            extra: Default::default(),
            ramp: 1.0,
            held: 0.0,
            blend: 1.0,
            rows: SENSITIVITY_ROW[0],
        }
    }
}

impl LookCamera {
    /// A step of `dt` seconds with the stick at `stick` (after
    /// [`stick`]), the camera sensitivity `sensitivity` (0..1), the held
    /// item's row ([`camera_row`]), aiming or not, an aim-assist target
    /// in view or not: the yaw turned (radians, towards the stick's +x)
    /// and the pitch moved (of the 0..1, towards its +y).
    pub fn step(
        &mut self,
        stick: [f32; 2],
        sensitivity: f32,
        row: usize,
        aiming: bool,
        target: bool,
        dt: f32,
    ) -> (f32, f32) {
        let want = SENSITIVITY_ROW[row.min(2)];
        for (r, w) in self.rows.iter_mut().zip(want) {
            *r = approach(*r, w, 0.5, dt);
        }
        let s = sensitivity * self.rows[usize::from(aiming)];
        let [x, y] = stick;
        // The base rate's ramp.
        let over = x.hypot(y);
        let (to, keep) = if target && over < 0.99 {
            (0.8, 0.5)
        } else {
            (1.0, 0.75)
        };
        self.ramp = approach(self.ramp, to, keep, dt);
        if over < 0.99 {
            self.held = 0.0;
        } else {
            self.held += dt;
            if self.held > 0.25 {
                self.ramp = 1.0;
            }
        }
        let fade = self.blend.clamp(0.0, 1.0).powi(3);
        let (yaw_c, yaw_c2) = curves(x);
        let (pitch_c, pitch_c2) = curves(y);
        let p: [Preset; 4] = std::array::from_fn(|axis| preset(s, axis));
        for (axis, (c, other)) in [(yaw_c, y), (yaw_c, y), (pitch_c, x), (pitch_c, x)]
            .into_iter()
            .enumerate()
        {
            let axis = [YAW, AIM_YAW, PITCH, AIM_PITCH][axis];
            self.inertia[axis].step(c, &p[axis], dt);
            self.extra[axis].step(c, other, &p[axis], dt);
        }
        let turn = |axis: usize, c2: f32| {
            let base = self.inertia[axis].value * p[axis].base * self.ramp * dt;
            let extra = self.extra[axis].value * c2 * dt;
            if 0.0 < base * extra {
                base + extra * fade
            } else {
                base
            }
        };
        let (yaw, pitch) = if aiming {
            (AIM_YAW, AIM_PITCH)
        } else {
            (YAW, PITCH)
        };
        (turn(yaw, yaw_c2), turn(pitch, pitch_c2))
    }
}

/// A car's (and a boat's) chase camera's stick look, as the 360's
/// (the `CameraCarPreset` camera, class 0x820F01F0, built by 0x824BFA10
/// from the seat's `CameraCarPresetLink` preset, 0x8243EAC0).
///
/// From the 360 code: its handler (0x824C0DF0) keeps the look-yaw input
/// (the stick's x, after [`stick`]) at +0x604 and the look-pitch (its y)
/// at +0x600. Each update (0x824C0518) eases each toward its input,
/// keeping 0.75 (0x820CAF1C) of the last, or past 0.1 (0x820BB468) of
/// input 0.825 for yaw (0x820F8CB0) and 0.85 for pitch (0x820D2F64): no
/// curve. The yaw turns at 2 radians a second times its eased input
/// (+0x668, 2.0 from 0x820C77A4 by 0x824BFA10; 0x824C07F8) and the pitch
/// offset at 1 (+0x66C), held to -1.2 (0x820F9364) to 0.25 (0x820DABDC).
///
/// **Inferred**: the easing taken a sixtieth of a second at a time (the
/// 360 eases once an update), and the pitch's sign (the stick's up looks
/// up, as on foot). **Not ported**: its own drift back behind the car
/// (`YawResetTime`, 0x824C02A8; the alpha's chase camera keeps its own)
/// and the first-person views.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CarLook {
    /// The eased yaw and pitch inputs (+0x630, +0x634).
    pub eased: [f32; 2],
}

/// The car camera's turn rates at full input, radians a second: yaw
/// (+0x668) and pitch (+0x66C).
pub const CAR_RATES: [f32; 2] = [2.0, 1.0];

/// The car camera's pitch limits, radians below and above level: its
/// pitch offset's (+0x610, 0x824C07F8: -1.2 and 0.25, 0x820F9364 and
/// 0x820DABDC), which is the view's pitch (the camera turns by its
/// negative, -(+0x62C + +0x610), the opposite way to the helicopter's):
/// -68.8 to +14.3 degrees.
pub const CAR_PITCH_LIMITS: [f32; 2] = [-1.2, 0.25];

/// The stick's yaw input under which the car camera takes none: its
/// square at most 0.1 (0x820BB468; 0x824C0AA0 drops it each update).
pub const CAR_YAW_THRESHOLD: f32 = 0.316_227_77;

/// The car camera's settings from its `CameraCarPreset` (placed by
/// 0x824BFA10: `YawResetTime` at +0x658, `DefaultPitch` +0x654,
/// `SpringStrength` +0x64C, `SpringDampening` +0x650).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CarPreset {
    pub yaw_reset_time: f32,
    pub default_pitch: f32,
    pub spring_strength: f32,
    pub spring_dampening: f32,
}

/// The look's fading back to the follow heading, seconds (+0x65C, 0.5,
/// 0x820D9EDC by 0x824BFA10).
pub const CAR_LOOK_FADE: f32 = 0.5;

/// The car camera's drift back behind the car (0x824C0AA0, 0x824C02A8,
/// 0x824B9B00, 0x824C07F8).
///
/// From the 360 code: a yaw input over [`CAR_YAW_THRESHOLD`] sets the look
/// timer (+0x63C) to the preset's `YawResetTime` plus [`CAR_LOOK_FADE`];
/// with none it runs down while the car goes faster than 2 m/s
/// (0x820C77A4), or while it is over the fade (parked it stalls a step
/// inside it: the 360's log, 0.47). The follow heading's target is the way
/// the car goes once it moves over 1 m/s (0x820BAEE8), else the view's own
/// heading (0x824C0040), so parked the look stays. While the timer runs
/// the follow heading is its target; once it is out, a spring pulls it there
/// (velocity += strength * the turn left - dampening * velocity, heading +=
/// velocity * dt, angles wrapped). The view's yaw is the follow heading
/// plus its own turn from it times the timer over the fade, clamped to 1,
/// applied to the view each update: free until the fade's half second,
/// then, compounding, back onto the follow heading within a fraction of
/// it (a boat's `YawResetTime` 6: swung back about 6.7 s after the stick).
///
/// **Inferred**: the spring stepped 30 times a second (the 360 steps it
/// once an update, its velocity's change not scaled by the time), the
/// fade's compounding taken a step of ours at a time (the 360's an update),
/// and the
/// way the car goes its heading (the 360's target turns to face back
/// while reversing, +0x644, not ported); the air time that softens the
/// spring (+0x640) taken as none.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CarFollow {
    pub timer: f32,
    pub follow: f32,
    velocity: f32,
    primed: bool,
    carry: f32,
}

impl CarFollow {
    /// The view's yaw after a step of `dt` seconds: `yaw` the view's yaw
    /// with this step's turn, `heading` the car's, `speed` its speed (m/s,
    /// either way), `held` whether the stick's yaw input is over
    /// [`CAR_YAW_THRESHOLD`].
    pub fn step(
        &mut self,
        p: &CarPreset,
        yaw: f32,
        heading: f32,
        speed: f32,
        held: bool,
        dt: f32,
    ) -> f32 {
        use std::f32::consts::{PI, TAU};
        let wrap = |a: f32| (a + PI).rem_euclid(TAU) - PI;
        if !self.primed {
            self.follow = heading;
            self.primed = true;
        }
        if held {
            self.timer = p.yaw_reset_time + CAR_LOOK_FADE;
        } else if speed.abs() > 2.0 || self.timer > CAR_LOOK_FADE {
            self.timer = (self.timer - dt).max(0.0);
        }
        // What the follow heading goes to (0x824C0040): the way the car
        // goes once it moves over 1 m/s, else the view's own heading, so
        // parked (the timer stalled a step inside the fade) nothing pulls
        // the view.
        let target = if speed.abs() > 1.0 { heading } else { yaw };
        if self.timer > 1e-4 {
            self.velocity = 0.0;
            self.follow = target;
        } else {
            const STEP: f32 = 1.0 / 30.0;
            self.carry += dt;
            while self.carry >= STEP {
                self.carry -= STEP;
                let off = wrap(target - self.follow);
                self.velocity += p.spring_strength * off - p.spring_dampening * self.velocity;
                self.follow = wrap(self.follow + self.velocity * STEP);
            }
        }
        let k = (self.timer / CAR_LOOK_FADE).clamp(0.0, 1.0);
        self.follow + wrap(yaw - self.follow) * k
    }
}

impl CarLook {
    /// A step of `dt` seconds with the stick at `stick` (after
    /// [`stick`]): the yaw turned (radians, towards the stick's +x) and
    /// the pitch moved (radians, towards its +y).
    pub fn step(&mut self, stick: [f32; 2], dt: f32) -> (f32, f32) {
        let keep = |v: f32, moving: f32| if v.abs() >= 0.1 { moving } else { 0.75 };
        let [x, y] = stick;
        // A yaw input under the threshold is none (0x824C0AA0).
        let x = if x * x > 0.1 { x } else { 0.0 };
        self.eased[0] = approach(self.eased[0], x, keep(x, 0.825), dt);
        self.eased[1] = approach(self.eased[1], y, keep(y, 0.85), dt);
        (
            CAR_RATES[0] * self.eased[0] * dt,
            CAR_RATES[1] * self.eased[1] * dt,
        )
    }
}

/// A tank seat's camera's stick look, as the 360's (the `CameraTank`
/// camera, class 0x820F0338, built by 0x824C8500 from the seat's record,
/// 0x8243EAC0): its `YawSpeed` and `PitchSpeed` and its input inertias
/// (`InputInertiaDefault`, `...DifferentSign`, `...NoInput`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TankLookTuning {
    pub yaw_speed: f32,
    pub pitch_speed: f32,
    pub keep: [f32; 3],
}

/// The tank camera's rate scale: its speeds are per thirtieth of a
/// second (0x820BB1F8, 30, in 0x824C86E0).
pub const TANK_RATE_SCALE: f32 = 30.0;

/// An axis's input inertia with three keeps (0x824B9BD8): with input of
/// the same sign as its value the first, of the other sign (or from
/// rest) the second (unless it is 0: then the value starts from 0 at the
/// first), with none after none the third.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Inertia3 {
    pub value: f32,
    idle: bool,
}

impl Inertia3 {
    pub fn step(&mut self, input: f32, keep: &[f32; 3], dt: f32) {
        let none = input == 0.0;
        let k = if none && self.idle {
            keep[2]
        } else if self.value * input <= 0.0 {
            if keep[1] == 0.0 {
                self.value = 0.0;
                keep[0]
            } else {
                keep[1]
            }
        } else {
            keep[0]
        };
        self.value = approach(self.value, input, k, dt);
        self.idle = none;
    }
}

/// A tank seat's camera's stick look (0x824C86E0).
///
/// From the 360 code: its handler (0x824C8AD0) keeps the look-yaw input
/// (the stick's x after [`stick`]) at +0x604 and the look-pitch (its y)
/// at +0x600; each update eases each through [`Inertia3`] with the
/// seat's keeps (+0x610, +0x628) and turns the yaw by `YawSpeed` (+0x644)
/// and the pitch by `PitchSpeed` (+0x648) times the eased input times
/// the time times 30 ([`TANK_RATE_SCALE`]): no curve. The pitch is held
/// to `MinPitch`..`MaxPitch` from the hull's (+0x64C, +0x650).
///
/// **Inferred**: the pitch's sign (the stick's up looks up, as on foot).
/// **Not ported**: the approach back inside the limits when the hull
/// tilts the view out of them (by `0.99 (1 - yaw input^2)`), and the
/// reset that lines the view up again (+0x640).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TankLook {
    pub yaw: Inertia3,
    pub pitch: Inertia3,
}

impl TankLook {
    /// A step of `dt` seconds with the stick at `stick` (after
    /// [`stick`]): the yaw turned and the pitch moved, radians.
    pub fn step(&mut self, stick: [f32; 2], t: &TankLookTuning, dt: f32) -> (f32, f32) {
        self.yaw.step(stick[0], &t.keep, dt);
        self.pitch.step(stick[1], &t.keep, dt);
        (
            t.yaw_speed * self.yaw.value * dt * TANK_RATE_SCALE,
            t.pitch_speed * self.pitch.value * dt * TANK_RATE_SCALE,
        )
    }
}

/// A turret seat's camera (an emplaced gun's, a gunner's), as the 360's
/// `CameraTurret` camera (class 0x820F0390, built by 0x824C8E80 from the
/// seat's record, 0x8243EAC0): `YawSpeed` and `PitchSpeed` (radians a
/// second), `YawMin`/`YawMax` and `PitchMin`/`PitchMax` from its base,
/// and each axis's input inertias (`...InputInertia` for both input
/// keeps, `...NoInputInertia`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TurretLookTuning {
    pub yaw_speed: f32,
    pub pitch_speed: f32,
    pub yaw: (f32, f32),
    pub pitch: (f32, f32),
    pub yaw_keep: [f32; 3],
    pub pitch_keep: [f32; 3],
}

impl TurretLookTuning {
    /// The view's pitch limits from the base, radians up positive: the
    /// record's are the camera's, positive down (as the car's and the
    /// helicopter's turn), so `-PitchMax` to `-PitchMin` (inferred).
    pub fn pitch_up(&self) -> (f32, f32) {
        (-self.pitch.1, -self.pitch.0)
    }
}

/// A turret seat's camera's stick look (0x824C9198).
///
/// From the 360 code: its handler (0x824C94F8) keeps the look-yaw input
/// (the stick's x after [`stick`]) at +0x604 and the look-pitch (its y)
/// at +0x600. Each update (0x824C9058) eases them, negated, through
/// [`Inertia3`]: the yaw on +0x628 with the record's yaw keeps (+0x10,
/// +0x14: with input for both, without), the pitch on +0x610 (+0x24,
/// +0x28). It turns the yaw by `YawSpeed` (+0x658) and the pitch by
/// `PitchSpeed` (+0x65C) times the eased input times the time, radians,
/// with no curve (unlike the tank's, no 30). It then holds each between its
/// limits from the base (0x824C95F0: `YawMin`/`YawMax` at +0x668/+0x66C,
/// `PitchMin`/`PitchMax` at +0x660/+0x664). At a limit and pushing on,
/// its eased value is dropped (0x824C9538, [`TurretLook::stop`]).
///
/// **Not ported**: its aim assist (0x824C9768).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TurretLook {
    pub yaw: Inertia3,
    pub pitch: Inertia3,
}

impl TurretLook {
    /// A step of `dt` seconds with the stick at `stick` (after
    /// [`stick`]): the yaw turned (radians, towards the stick's +x) and
    /// the pitch moved (radians, towards its +y).
    pub fn step(&mut self, stick: [f32; 2], t: &TurretLookTuning, dt: f32) -> (f32, f32) {
        self.yaw.step(stick[0], &t.yaw_keep, dt);
        self.pitch.step(stick[1], &t.pitch_keep, dt);
        (
            t.yaw_speed * self.yaw.value * dt,
            t.pitch_speed * self.pitch.value * dt,
        )
    }

    /// At a limit, pushing on: that axis's eased value dropped
    /// (0x824C9538), yaw or pitch, by which way it pushes (+ the stick's
    /// +x or +y).
    pub fn stop(&mut self, yaw: Option<f32>, pitch: Option<f32>) {
        for (axis, way) in [(&mut self.yaw, yaw), (&mut self.pitch, pitch)] {
            if way.is_some_and(|w| axis.value * w > 0.0) {
                axis.value = 0.0;
            }
        }
    }
}

/// `angle` held to `limits` (radians) about `base` (0x824C95F0), wrapped.
pub fn turret_clamp(angle: f32, base: f32, limits: (f32, f32)) -> (f32, Option<f32>) {
    use std::f32::consts::{PI, TAU};
    let wrap = |a: f32| (a + PI).rem_euclid(TAU) - PI;
    if limits.1 - limits.0 >= TAU {
        return (angle, None);
    }
    let off = wrap(angle - base);
    if off < limits.0 {
        (wrap(base + limits.0), Some(-1.0))
    } else if off > limits.1 {
        (wrap(base + limits.1), Some(1.0))
    } else {
        (angle, None)
    }
}

/// A helicopter pilot's camera's stick look, as the 360's
/// `CameraHelicopter` camera (class 0x820F0240, built by 0x824C1D30 from
/// the seat's record, 0x8243EAC0; `PitchSpeed` +0x24 to +0x648,
/// `MinPitch`/`MaxPitch` +0x38/+0x3C to +0x638/+0x63C,
/// `PitchLimitCushion` +0x40 to +0x640, `PitchInputInertia` +0x48 for
/// both input keeps and `PitchNoInputInertia` +0x4C, the block +0x620).
/// Its pitch is the camera's: positive down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeliLookTuning {
    pub pitch_speed: f32,
    pub pitch: (f32, f32),
    pub cushion: f32,
    pub keep: [f32; 3],
    /// How the view follows the helicopter ([`HeliFollow`]): `YawLag`
    /// (+0x20 to the yaw block +0x608's three keeps), `RollAmp` and
    /// `RollLag` (+0x28/+0x2C to +0x64C/+0x650), `PitchAmp` and
    /// `PitchLag` (+0x30/+0x34 to +0x674/+0x678).
    pub yaw_lag: f32,
    pub roll_amp: f32,
    pub roll_lag: f32,
    pub pitch_amp: f32,
    pub pitch_lag: f32,
}

impl HeliLookTuning {
    /// The view's pitch limits, radians up positive: `-MaxPitch` to
    /// `-MinPitch`.
    pub fn pitch_up(&self) -> (f32, f32) {
        (-self.pitch.1, -self.pitch.0)
    }
}

/// A helicopter pilot's camera's stick look (0x824C1EB0).
///
/// From the 360 code: its handler (0x824C2480) takes the look-pitch
/// input only (the stick's y after [`stick`], +0x600); the stick's x
/// turns nothing, the camera's yaw following the helicopter's (`YawLag`).
/// Within `PitchLimitCushion` of a limit, an input pushing toward it is
/// scaled down by the distance left over the cushion; the input is then
/// eased through [`Inertia3`] and the pitch (+0x604, positive down) moves
/// by `-PitchSpeed` times it times the time, held to `MinPitch` ..
/// `MaxPitch`: no curve.
///
/// **Not ported**: the yaw's lag behind the helicopter (`YawLag`) and the
/// roll and pitch amplitudes and lags (`RollAmp`, `PitchAmp`...).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HeliLook {
    pub pitch: Inertia3,
}

impl HeliLook {
    /// A step of `dt` seconds with the stick at `stick` (after [`stick`])
    /// and the view's pitch now `pitch` (radians up positive): the pitch
    /// moved, radians up.
    pub fn step(&mut self, stick: [f32; 2], pitch: f32, t: &HeliLookTuning, dt: f32) -> f32 {
        let (min, max) = t.pitch;
        let down = -pitch;
        let mut input = stick[1];
        let cushion = t.cushion.max(1e-4);
        if down < min + cushion && input > 0.0 {
            input *= ((down - min) / cushion).clamp(0.0, 1.0);
        }
        if max - cushion < down && input < 0.0 {
            input *= ((max - down) / cushion).clamp(0.0, 1.0);
        }
        self.pitch.step(input, &t.keep, dt);
        t.pitch_speed * self.pitch.value * dt
    }
}

/// `x` moved toward the angle `target` the short way round for `dt`
/// seconds, keeping `keep` of the gap each sixtieth (0x824B9CE0's
/// wrapping path), wrapped.
pub fn approach_angle(x: f32, target: f32, keep: f32, dt: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let wrap = |a: f32| (a + PI).rem_euclid(TAU) - PI;
    let near = x + wrap(target - x);
    wrap(approach(x, near, keep, dt))
}

/// How a helicopter pilot's camera follows the helicopter (0x824C1EB0).
///
/// From the 360 code:
/// - **Yaw**: the helicopter's heading, eased through the input inertia
///   with `YawLag` as all three keeps, the short way round (the block
///   +0x608, its value +0x618).
/// - **Pitch**: the view's pitch (positive down) is its own (+0x604, the
///   stick's, [`HeliLook`]) less `PitchAmp` times the helicopter's nose
///   pitch eased once an update keeping `PitchLag` (+0x67C).
/// - **Roll**: `RollAmp` times the helicopter's roll, eased keeping
///   `RollLag` (0x825AF718's turn).
/// - **Re-centring** (0x824C23A0, [`HeliFollow::recentre`]): a reset
///   (the handler's `0xAA535A9F` and `0x5D4C76BF`) gives a quarter of a
///   second (+0x6A0) in which the yaw eases toward the heading and the
///   pitch toward `DefaultPitch` keeping 0.85 (0x820D2F64); a pitch input
///   over 0.1 squared ends it.
///
/// **Inferred**: the per-update eases taken a sixtieth of a second at a
/// time; the signs of the helicopter's pitch and roll (nose up and
/// right wing down positive) and of the view's roll. **Not ported**: the
/// flight-mode offsets (the record at store 0x83652020 blended by the
/// helicopter's mode: focus height, distance, yaw and pitch shifts).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HeliFollow {
    pub yaw: f32,
    pub nose: f32,
    pub roll: f32,
    primed: bool,
    recentre: f32,
}

impl HeliFollow {
    /// A step of `dt` seconds: the helicopter heading `heading`, its
    /// nose pitched `nose` (up positive) and rolled `roll`. Returns the
    /// view's yaw, the pitch it adds to the view's own (radians up) and
    /// its roll.
    pub fn step(
        &mut self,
        t: &HeliLookTuning,
        heading: f32,
        nose: f32,
        roll: f32,
        dt: f32,
    ) -> (f32, f32, f32) {
        if !self.primed {
            *self = Self {
                yaw: heading,
                nose,
                roll: t.roll_amp * roll,
                primed: true,
                recentre: 0.0,
            };
        }
        self.yaw = approach_angle(self.yaw, heading, t.yaw_lag, dt);
        self.nose = approach(self.nose, nose, t.pitch_lag, dt);
        self.roll = approach(self.roll, t.roll_amp * roll, t.roll_lag, dt);
        (self.yaw, t.pitch_amp * self.nose, self.roll)
    }

    /// A reset of the view (0x824C23A0): for a quarter of a second it
    /// eases back.
    pub fn recentre(&mut self) {
        self.recentre = 0.25;
    }

    /// While re-centring: the view's pitch (radians up) eased toward the
    /// default one, `default_down` (`DefaultPitch`, positive down), and
    /// the yaw toward `heading`; ends early on a pitch input over 0.1
    /// squared (`eased` the stick's eased pitch).
    pub fn recentring(
        &mut self,
        pitch: f32,
        default_down: f32,
        heading: f32,
        eased: f32,
        dt: f32,
    ) -> f32 {
        self.recentre = (self.recentre - dt).max(0.0);
        let out = if self.recentre > 0.0 {
            self.yaw = approach_angle(self.yaw, heading, 0.85, dt);
            approach(pitch, -default_down, 0.85, dt)
        } else {
            pitch
        };
        if eased * eased > 0.1 {
            self.recentre = 0.0;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_helicopter_camera_lags_its_yaw_and_leans_with_it() {
        // An MD-500's: YawLag 0.9, RollAmp 0.65 lag 0.95, PitchAmp 0.75 lag
        // 0.95.
        let md = HeliLookTuning {
            pitch_speed: 2.8,
            pitch: (-0.5, 1.0),
            cushion: 1.7,
            keep: [0.7, 0.7, 0.5],
            yaw_lag: 0.9,
            roll_amp: 0.65,
            roll_lag: 0.95,
            pitch_amp: 0.75,
            pitch_lag: 0.95,
        };
        let dt = 1.0 / 60.0;
        let mut f = HeliFollow::default();
        f.step(&md, 0.0, 0.0, 0.0, dt);
        // Turned to 1 rad: a tenth of the gap a sixtieth, so 0.65 after
        // ten.
        let mut yaw = 0.0;
        for _ in 0..10 {
            yaw = f.step(&md, 1.0, 0.0, 0.0, dt).0;
        }
        assert!((yaw - (1.0 - 0.9f32.powi(10))).abs() < 1e-3, "{yaw}");
        // The short way round, across the back.
        let mut g = HeliFollow::default();
        g.step(&md, 3.0, 0.0, 0.0, dt);
        let y = g.step(&md, -3.0, 0.0, 0.0, dt).0;
        assert!(!(-3.0..=3.0).contains(&y), "{y}");
        // Nose up 0.2 held: the view lifts 0.75 of it in time; rolled
        // 0.4, the view rolls 0.65 of it.
        let (mut lift, mut roll) = (0.0, 0.0);
        for _ in 0..600 {
            let (_, l, r) = f.step(&md, 1.0, 0.2, 0.4, dt);
            lift = l;
            roll = r;
        }
        assert!((lift - 0.15).abs() < 1e-3 && (roll - 0.26).abs() < 1e-3);
        // A reset eases the pitch back to its default for a quarter second.
        f.recentre();
        let mut p = 0.5;
        for _ in 0..30 {
            p = f.recentring(p, 0.25, 1.0, 0.0, dt);
        }
        assert!(p < 0.5 && p > -0.25, "{p}");
    }

    #[test]
    fn a_helicopter_camera_pitches_only_slowing_into_its_limits() {
        // An MD-500's pilot: 2.8 rad/s, -0.5 to 1.0 (positive down),
        // cushion 1.7, keeps 0.7 and 0.5.
        let md = HeliLookTuning {
            pitch_speed: 2.8,
            pitch: (-0.5, 1.0),
            cushion: 1.7,
            keep: [0.7, 0.7, 0.5],
            yaw_lag: 0.9,
            roll_amp: 0.65,
            roll_lag: 0.95,
            pitch_amp: 0.75,
            pitch_lag: 0.95,
        };
        let dt = 1.0 / 60.0;
        assert_eq!(md.pitch_up(), (-1.0, 0.5));
        // From its default, 0.25 down: up toward its 0.5 up limit, the
        // input scaled by 0.75 / 1.7 (0.44): 2.8 * 0.44 = 1.24 rad/s.
        let mut c = HeliLook::default();
        let mut last = 0.0;
        for _ in 0..60 {
            last = c.step(stick(0.0, 1.0), -0.25, &md, dt);
        }
        assert!((last / dt - 1.235).abs() < 0.01, "{}", last / dt);
        // At the limit, nothing more.
        let mut c = HeliLook::default();
        for _ in 0..60 {
            last = c.step(stick(0.0, 1.0), 0.5, &md, dt);
        }
        assert_eq!(last, 0.0);
        // Down from level, the other limit 1.0 away: scaled by 1/1.7.
        let mut c = HeliLook::default();
        for _ in 0..60 {
            last = c.step(stick(0.0, -1.0), 0.0, &md, dt);
        }
        assert!((last / dt + 2.8 / 1.7).abs() < 0.01, "{}", last / dt);
        // The stick's x turns nothing.
        let mut c = HeliLook::default();
        assert_eq!(c.step(stick(1.0, 0.0), 0.0, &md, dt), 0.0);
    }

    #[test]
    fn a_turret_camera_turns_at_its_speed_and_stops_at_its_limits() {
        // An emplaced Quad 50's: 2 and 2 rad/s, yaw all round, pitch -0.5
        // to 0.4 (positive down: 0.4 down to 0.5 up); keeps as the seat's.
        let quad = TurretLookTuning {
            yaw_speed: 2.0,
            pitch_speed: 2.0,
            yaw: (-4.0, 4.0),
            pitch: (-0.5, 0.4),
            yaw_keep: [0.9, 0.9, 0.5],
            pitch_keep: [0.9, 0.9, 0.5],
        };
        let dt = 1.0 / 60.0;
        let mut c = TurretLook::default();
        let mut last = 0.0;
        for _ in 0..120 {
            last = c.step(stick(1.0, 0.0), &quad, dt).0;
        }
        // Full: 2 rad/s, 114.6 deg/s; linear past the dead zone.
        assert!((last.to_degrees() / dt - 114.6).abs() < 0.5);
        let mut c = TurretLook::default();
        for _ in 0..120 {
            last = c.step(stick(0.75, 0.0), &quad, dt).0;
        }
        assert!((last.to_degrees() / dt - 63.4).abs() < 0.5);
        assert_eq!(quad.pitch_up(), (-0.4, 0.5));
        // Its yaw all round (8 rad) is never held; its pitch is.
        assert_eq!(turret_clamp(3.0, 0.0, quad.yaw), (3.0, None));
        let (p, way) = turret_clamp(0.7, 0.0, quad.pitch_up());
        assert_eq!((p, way), (0.5, Some(1.0)));
        // Held there, pushing on drops its eased value; easing off doesn't.
        c.pitch.value = 0.8;
        c.stop(None, way);
        assert_eq!(c.pitch.value, 0.0);
        c.pitch.value = -0.8;
        c.stop(None, way);
        assert_eq!(c.pitch.value, -0.8);
    }

    #[test]
    fn a_tank_camera_turns_at_its_seats_speed_eased_by_its_inertia() {
        // The LAV III's seat: 0.1 and 0.1, keeps 0.975, 0.9 and 0.8.
        let lav = TankLookTuning {
            yaw_speed: 0.1,
            pitch_speed: 0.1,
            keep: [0.975, 0.9, 0.8],
        };
        let dt = 1.0 / 60.0;
        let rate = |tilt: f32, secs: f32| {
            let mut c = TankLook::default();
            let mut last = 0.0;
            for _ in 0..(secs / dt).round() as usize {
                last = c.step(stick(tilt, 0.0), &lav, dt).0;
            }
            last.to_degrees() / dt
        };
        // Full: 3 rad/s (172 deg/s) once eased; slow to ease, a starting
        // stick taking the other-sign keep 0.9 then the same-sign 0.975.
        assert!((rate(1.0, 5.0) - 171.9).abs() < 1.0, "{}", rate(1.0, 5.0));
        assert!(
            rate(1.0, 0.5) < 160.0 && rate(1.0, 0.5) > 60.0,
            "{}",
            rate(1.0, 0.5)
        );
        // Linear past the dead zone: 0.75 tilt 0.554 of it.
        assert!((rate(0.75, 5.0) - 95.2).abs() < 1.0, "{}", rate(0.75, 5.0));
        assert_eq!(rate(0.4, 2.0), 0.0);
        // An AMX-30's 0.04: 69 deg/s.
        let amx = TankLookTuning {
            yaw_speed: 0.04,
            ..lav
        };
        let mut c = TankLook::default();
        let mut a = 0.0;
        for _ in 0..600 {
            a = c.step(stick(1.0, 0.0), &amx, dt).0;
        }
        assert!((a.to_degrees() / dt - 68.75).abs() < 0.5);
    }

    /// The car camera held at `tilt` along x (or y) for `secs`: degrees
    /// a second over the last half second.
    fn car_rate(tilt: [f32; 2], secs: f32) -> (f32, f32) {
        let mut c = CarLook::default();
        let dt = 1.0 / 60.0;
        let n = (secs / dt).round() as usize;
        let window = n.min(30);
        let (mut yaw, mut pitch) = (0.0, 0.0);
        for i in 0..n {
            let (a, b) = c.step(stick(tilt[0], tilt[1]), dt);
            if i >= n - window {
                yaw += a;
                pitch += b;
            }
        }
        let span = window as f32 * dt;
        (yaw.to_degrees() / span, pitch.to_degrees() / span)
    }

    #[test]
    fn the_car_camera_turns_linearly_after_the_dead_zone() {
        // The yaw takes nothing under 0.316 of input (0.62 of tilt);
        // past it straight in proportion: 0.75 63.4 deg/s, full 114.6.
        // The pitch has the stick's own dead zone only: 0.5 tilt 6.1.
        assert_eq!(car_rate([0.4, 0.0], 1.0), (0.0, 0.0));
        assert_eq!(car_rate([0.5, 0.0], 1.0), (0.0, 0.0));
        assert_eq!(car_rate([0.6, 0.0], 1.0), (0.0, 0.0));
        let (yaw, _) = car_rate([0.65, 0.0], 1.0);
        assert!(yaw > 30.0, "{yaw}");
        let (_, pitch) = car_rate([0.0, 0.5], 1.0);
        assert!((pitch - 6.1).abs() < 0.3, "{pitch}");
        let (yaw, _) = car_rate([0.75, 0.0], 1.0);
        assert!((yaw - 63.4).abs() < 0.5, "{yaw}");
        let (yaw, _) = car_rate([1.0, 0.0], 1.0);
        assert!((yaw - 114.6).abs() < 0.5, "{yaw}");
        let (_, pitch) = car_rate([0.0, 1.0], 1.0);
        assert!((pitch - 57.3).abs() < 0.5, "{pitch}");
        // It eases in: the first sixth of a second well under full.
        let (yaw, _) = car_rate([1.0, 0.0], 0.1);
        assert!(yaw < 100.0, "{yaw}");
    }

    #[test]
    fn the_car_camera_drifts_back_after_its_reset_time() {
        // The ImpactSUT's preset: 2 s, spring 1.5 and 0.4.
        let p = CarPreset {
            yaw_reset_time: 2.0,
            default_pitch: -0.2,
            spring_strength: 1.5,
            spring_dampening: 0.4,
        };
        let dt = 1.0 / 60.0;
        let mut f = CarFollow::default();
        // Looked a right angle off, the car driving on at 10 m/s.
        let mut yaw = f.step(&p, 1.5, 0.0, 10.0, true, dt);
        let at = |secs: f32, yaw: &mut f32, f: &mut CarFollow| {
            for _ in 0..(secs / dt).round() as usize {
                *yaw = f.step(&p, *yaw, 0.0, 10.0, false, dt);
            }
            *yaw
        };
        // Free for the 2 s, then the fade: applied to the view as it is
        // each update, it compounds, so the view swings back within a
        // tenth or two of a second (the 360's boat: +89 to +14 degrees in
        // 0.25 s after its 6 s and the half second's wait).
        assert!((at(2.0, &mut yaw, &mut f) - 1.5).abs() < 1e-3, "{yaw}");
        let mid = at(0.1, &mut yaw, &mut f);
        assert!(mid < 1.2 && mid > 0.3, "{mid}");
        let end = at(0.4, &mut yaw, &mut f);
        assert!(end.abs() < 1e-3, "{end}");
        // The swing starts as the timer enters the fade, 2 s after the
        // stick (the 360's log: 2.10 s).
        let mut f = CarFollow::default();
        let mut yaw = f.step(&p, 1.5, 0.0, 10.0, true, dt);
        for _ in 0..119 {
            yaw = f.step(&p, yaw, 0.0, 10.0, false, dt);
        }
        assert!((yaw - 1.5).abs() < 1e-4, "{yaw} {}", f.timer);
        for _ in 0..3 {
            yaw = f.step(&p, yaw, 0.0, 10.0, false, dt);
        }
        assert!(yaw < 1.45, "{yaw}");
        // Parked: the timer stalls a step inside the fade (the 360's 0.47)
        // and, its follow heading the view's own, the look stays 12 s on.
        let mut f = CarFollow::default();
        let mut yaw = f.step(&p, 1.5, 0.0, 0.0, true, dt);
        for _ in 0..720 {
            yaw = f.step(&p, yaw, 0.0, 0.0, false, dt);
        }
        assert!(f.timer < CAR_LOOK_FADE && f.timer > 0.45, "{}", f.timer);
        assert!((yaw - 1.5).abs() < 1e-4, "{yaw}");
        // The car turns: the follow heading's spring catches up.
        let mut f = CarFollow::default();
        let mut yaw = f.step(&p, 0.0, 0.0, 10.0, false, dt);
        for _ in 0..30 {
            yaw = f.step(&p, yaw, 1.0, 10.0, false, dt);
        }
        assert!(yaw > 0.2 && yaw < 1.0, "{yaw}");
        for _ in 0..600 {
            yaw = f.step(&p, yaw, 1.0, 10.0, false, dt);
        }
        assert!((yaw - 1.0).abs() < 1e-2, "{yaw}");
    }

    /// Holding the stick at `tilt` along x (or y) for `secs` at 60 a
    /// second: the rate over the last half second, degrees a second (the
    /// pitch's 0..1 over the footage's 97 degrees).
    fn rate(tilt: [f32; 2], secs: f32) -> (f32, f32) {
        let mut c = LookCamera::default();
        let dt = 1.0 / 60.0;
        let n = (secs / dt).round() as usize;
        let window = n.min(30);
        let (mut yaw, mut t) = (0.0, t_at(0.0, &STANDING));
        let mut from = 0.0;
        for i in 0..n {
            if i == n - window {
                from = pitch_at(t, &STANDING);
            }
            let (a, b) = c.step(stick(tilt[0], tilt[1]), 0.5, 0, false, false, dt);
            t = (t + b).clamp(0.0, 1.0);
            if i >= n - window {
                yaw += a;
            }
        }
        let span = window as f32 * dt;
        let pitch = pitch_at(t, &STANDING) - from;
        (yaw.to_degrees() / span, pitch.to_degrees() / span)
    }

    #[test]
    fn the_stick_cuts_a_wide_dead_zone() {
        // Along one axis nothing up to 0.44.
        assert_eq!(stick(0.43, 0.0), [0.0, 0.0]);
        assert!(stick(0.45, 0.0)[0] > 0.0);
        let [x, _] = stick(0.75, 0.0);
        assert!((x - 0.553_6).abs() < 0.004, "{x}");
        assert_eq!(stick(1.0, 0.0), [1.0, 0.0]);
        assert_eq!(stick(-1.0, 0.0), [-1.0, 0.0]);
    }

    #[test]
    fn the_rates_match_the_360_at_the_middle_sensitivity() {
        // Xenia, round 13: half tilt about 0.6 deg/s yaw, 0.4 to 0.9
        // pitch; three quarters 16 to 18 yaw, 14 to 20 pitch.
        let (yaw, _) = rate([0.5, 0.0], 3.0);
        assert!((0.4..0.9).contains(&yaw), "{yaw}");
        let (_, pitch) = rate([0.0, 0.5], 3.0);
        assert!((0.3..0.9).contains(&pitch), "{pitch}");
        let (yaw, _) = rate([0.75, 0.0], 3.0);
        assert!((16.0..19.0).contains(&yaw), "{yaw}");
        let (_, pitch) = rate([0.0, 0.75], 1.0);
        assert!((14.0..20.0).contains(&pitch), "{pitch}");
        // Full tilt: the base rate at once (57 deg/s yaw), the extra
        // building on it after an eighth of a second held over: Xenia's
        // 170 to 190 yaw is the second half second's, 282 by 3 s. Up from
        // level the pitch goes 56.6 then 43.8 deg/s, slowing into its
        // +50 degrees (Xenia's 44 to 58), and stops there.
        let (yaw, _) = rate([1.0, 0.0], 1.0);
        assert!((170.0..200.0).contains(&yaw), "{yaw}");
        let (_, pitch) = rate([0.0, 1.0], 0.5);
        assert!((44.0..58.0).contains(&pitch), "{pitch}");
        let (_, pitch) = rate([0.0, 1.0], 1.0);
        assert!((40.0..58.0).contains(&pitch), "{pitch}");
        let (yaw, pitch) = (rate([1.0, 0.0], 3.0).0, rate([0.0, 1.0], 3.0).1);
        assert!(
            (270.0..290.0).contains(&yaw) && pitch == 0.0,
            "{yaw} {pitch}"
        );
    }

    #[test]
    fn the_pitch_limits_are_the_offsets_angles() {
        assert!((pitch_at(0.0, &STANDING).to_degrees() + 46.975).abs() < 0.01);
        assert!((pitch_at(1.0, &STANDING).to_degrees() - 50.194).abs() < 0.01);
        assert_eq!(
            PITCH_RANGE,
            [pitch_at(0.0, &STANDING), pitch_at(1.0, &STANDING)]
        );
        assert!((pitch_at(0.0, &AIMING).to_degrees() + 33.23).abs() < 0.01);
        assert!((pitch_at(1.0, &AIMING).to_degrees() - 50.71).abs() < 0.01);
        // Rising all the way, and t_at is its inverse.
        let mut last = f32::MIN;
        for i in 0..=100 {
            let p = pitch_at(i as f32 / 100.0, &STANDING);
            assert!(p > last);
            last = p;
        }
        assert!((pitch_at(t_at(0.3, &STANDING), &STANDING) - 0.3).abs() < 1e-4);
    }

    #[test]
    fn the_items_yaw_speed_picks_its_row() {
        // Every weapon's RotationSpeed is 3: the pistols' 1.5 row 1, the
        // RPG's 0.35 and the light MG's 0.6 row 2, the rifles' 0.8 row 0.
        assert_eq!(camera_row(3, 1.5), 1);
        assert_eq!(camera_row(3, 0.35), 2);
        assert_eq!(camera_row(3, 0.6), 2);
        assert_eq!(camera_row(3, 0.8), 0);
        assert_eq!(camera_row(3, 1.25), 0);
        assert_eq!(camera_row(1, 0.1), 1);
        // Row 2 turns at a quarter of the sensitivity, eased in: an RPG
        // held turns 0.75 tilt slower than a rifle (the low presets share
        // the middle's base rate, so by their inertia: about 11 percent).
        let turn = |row: usize| {
            let mut c = LookCamera::default();
            let mut sum = 0.0;
            for _ in 0..180 {
                sum += c
                    .step(stick(0.75, 0.0), 0.5, row, false, false, 1.0 / 60.0)
                    .0;
            }
            sum
        };
        assert!(turn(2) < turn(0) * 0.95, "{} {}", turn(2), turn(0));
        assert_eq!(turn(1), turn(0));
    }

    #[test]
    fn a_target_slows_the_base_rate_below_full_tilt() {
        let mut c = LookCamera::default();
        for _ in 0..120 {
            c.step(stick(0.75, 0.0), 0.5, 0, false, true, 1.0 / 60.0);
        }
        assert!((c.ramp - 0.8).abs() < 1e-3, "{}", c.ramp);
        // Held over a quarter of a second, full at once.
        for _ in 0..16 {
            c.step(stick(1.0, 0.0), 0.5, 0, false, true, 1.0 / 60.0);
        }
        assert_eq!(c.ramp, 1.0);
        // A blend under way holds the extra back.
        let mut blended = LookCamera {
            blend: 0.5,
            ..LookCamera::default()
        };
        let mut settled = LookCamera::default();
        let (mut a, mut b) = (0.0, 0.0);
        for _ in 0..60 {
            a += blended
                .step(stick(1.0, 0.0), 0.5, 0, false, false, 1.0 / 60.0)
                .0;
            b += settled
                .step(stick(1.0, 0.0), 0.5, 0, false, false, 1.0 / 60.0)
                .0;
        }
        assert!(a < b, "{a} {b}");
    }

    #[test]
    fn the_presets_blend_by_sensitivity() {
        assert_eq!(preset(0.5, YAW), PRESETS[1][YAW]);
        assert_eq!(preset(0.0, PITCH), PRESETS[0][PITCH]);
        assert_eq!(preset(1.0, YAW), PRESETS[2][YAW]);
        assert!((preset(0.75, YAW).base - 1.5).abs() < 1e-6);
    }

    #[test]
    fn approach_keeps_its_share_each_sixtieth() {
        let x = approach(0.0, 1.0, 0.9, 1.0 / 60.0);
        assert!((x - 0.1).abs() < 1e-4, "{x}");
        assert!((approach(0.0, 1.0, 0.975, 1.0) - (1.0 - 0.975f32.powi(60))).abs() < 0.02);
    }
}
