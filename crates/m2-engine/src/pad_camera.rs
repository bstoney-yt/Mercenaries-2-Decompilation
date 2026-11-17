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

#[cfg(test)]
mod tests {
    use super::*;

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
