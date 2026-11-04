//! How a person's aim is posed, as the 360's locomotion node does it: not
//! by turning one aiming clip, but by holding a sweep clip at the moment
//! that points the way they aim.
//!
//! # Provenance
//!
//! From the 360 code (the locomotion node, class built at 0x82802B10):
//!
//! - The node has three aim sweep blends, `AimNodes_SweepLow`,
//!   `AimNodes_SweepMid` and `AimNodes_SweepHigh` (their names at
//!   0x8206F990, 0x8206F9A4 and 0x8206F9B8), each of four clips loaded
//!   from the node's handle list at 0x82B4DA98 (init 0x82804AE8): low from
//!   entries 14, 15, 17 and 16, mid from 18, 19, 21 and 20, high from 23,
//!   24, 26 and 25. Moving forward (0x82808018) fades in the first of each
//!   group; standing (0x82809B98) the second and third.
//! - Each step (0x8280AED8) sets every sweep clip to the same point,
//!   [`phase`] of the aim's yaw from the body (the node's +0x1580, plus an
//!   offset at +0x24 of the aiming component, taken as 0 here): the yaw
//!   window runs from -155 degrees (0x82070294, times the degree at
//!   0x82E75080) to 90 (0x820F8D04), 245 degrees across the clip.
//! - The three groups are weighted by the aim's pitch ([`weights`]):
//!   below level, low by `pitch / -45 degrees` (pi times 0.25, 0x8206F708
//!   and 0x82003BCC, stored at 0x82E7483C) and mid by the rest; above,
//!   high and mid likewise.
//!
//! The 360 measures the yaw the other way round from this crate's
//! `atan2(x, z)`: measured on the rifle sweep clips (round 10), their
//! rifle turns from about +170 degrees to -55 through the clip, so
//! [`phase`] takes this crate's yaw negated.
//!
//! - Each group's other two clips are its firing poses (standing: entries
//!   17, 21 and 26; moving: 16, 20 and 25). The step weights them by a
//!   value at +0x30 of the person's per-entity record (the pool at
//!   0x836572EC, records of 0x48 bytes), clamped to 0..1: the standing
//!   sweep takes one minus it, both firing clips take it. [`Kick`] is that
//!   value. Every shot a gun fires (0x82283A10) posts a message naming its
//!   wielder (type 0x2C49ADC9, the wielder at +0xC). Its handler
//!   (0x8242A6D0) sets the wielder's +0x34 to 30 (0x82DE88A8), and each
//!   frame (0x8241F580) eases the pair back to 0 as a critically damped
//!   spring, +0x30 the value and +0x34 its rate, of smooth time 0.1 s
//!   (2.0 at 0x820C77A4 over 0.1 at 0x82DE88AC; the series terms 0.48
//!   and 0.235 at 0x820F8C60 and 0x820F8C64).
//!
//! How the aim mixes with the crouched stance (round 11), from the same
//! node: its `MoveAim` blend has four inputs, the aim movement
//! (`AimMovementNodes`, the nine `Crouch_Aim_*` gaits and shuffles), the
//! loose movement, the aim (`AimNodes`) and `TTNoAimNodes`. Its init
//! (0x82804AE8) gives three of them a weight per bone (0x827FA2A8), from
//! fixed tables (0x82412CE0): the aim movement the `lowerbody` table
//! ([`LOWER_BODY`], at 0x82B76110), the aim and `TTNoAimNodes` the table
//! 0x2CA54698 ([`UPPER_BODY`], at 0x82B762F0, the same 48 bones as the
//! `upperbody` table at 0x82B76168), all at 1 and every other bone at 0.
//! Aiming (0x828071A8: on 0x8280B6E8 and 0x8280AE60) fades in the aim
//! movement and the aim and fades the loose movement out, so the legs
//! play the crouched clip and the upper body the aim. The blend's
//! weighted inputs (0x827F9C28, mode 0) are shared per bone, and a bone
//! neither table names falls to the skeleton's reference pose.
//!
//! The aim is itself a blend (`AimNodes`): the sweeps' pitch blend
//! (`AimNodes_StaticSweep`) and, laid over it as an additive input (mode
//! 1, set by 0x827FA550), the aim movement's motion from its start
//! (0x8280FD50's difference node: the movement less `AimReferenceNodes`,
//! the same clips bound again and held at their start). So the upper body
//! still bobs and sways with the crouched gait. That input's weight is
//! set each step (0x8280AED8, port +0x1048) to [`layer_weight`]: a smooth
//! value noise ([`value_noise`], 0x825736B0) of the node's clock (+0x1588)
//! times 2 (0x82003CF4), scaled by 1 and raised by 0.5, or a steady 0.5 for
//! a pistol (the equipment class `pistol`, 0xD48C7D34; init 0x82805E18).
//!
//! - Moving is the same crossfade as standing: the step also stores
//!   `1 - w` at the node's +0x158C (0x8280B444), and the moving steps
//!   (0x82808468 at 0x82808FB4, 0x82809468 at 0x828099C8) set each
//!   group's moving sweep (ports +0xB80, +0xD44, +0xF08) to it. Moving
//!   fades in each group's first and fourth inputs (0x82808018,
//!   0x82809188), standing its second and third (0x82809B98), and the
//!   blend (0x827F9C28, mode 0) shares its inputs by their sum, 1.
//!
//! **Not ported**: the node's easing of the aim yaw (rate at 0x82DE8828).

/// The aim yaw at the start and end of a sweep, degrees, in the 360's
/// sense (see the module documentation).
pub const FROM: f32 = -155.0;
pub const TO: f32 = 90.0;

/// The pitch at which an aim is all low or all high, degrees.
pub const FULL_PITCH: f32 = 45.0;

/// How far through its clip a sweep is held for an aim `yaw` radians
/// from the body (this crate's `atan2(x, z)` sense), 0 to 1.
pub fn phase(yaw: f32) -> f32 {
    let deg = -yaw.to_degrees();
    ((deg - FROM) / (TO - FROM)).clamp(0.0, 1.0)
}

/// The weights of the low, mid and high sweeps for an aim `pitch`
/// radians above level.
pub fn weights(pitch: f32) -> [f32; 3] {
    let t = (pitch.to_degrees().abs() / FULL_PITCH).clamp(0.0, 1.0);
    if pitch <= 0.0 {
        [t, 1.0 - t, 0.0]
    } else {
        [0.0, 1.0 - t, t]
    }
}

/// The rifle sweep clips (the on-foot table's generic `Rifle` rows), low,
/// mid and high: standing (entries 15, 19, 24) and moving (14, 18, 23).
pub const RIFLE_STANDING: [u32; 3] = [0xd86c_bb46, 0x0823_5d42, 0xf877_b44a];
pub const RIFLE_MOVING: [u32; 3] = [0x9e25_852b, 0x48f8_5c73, 0x034c_acb5];

/// The rifle's firing sweeps, low, mid and high: standing (entries 17,
/// 21 and 26: handles b33294eb, 8d9e02a7 and b011e74f) and moving (16,
/// 20 and 25: handles f341cf84, 0c11cea4 and 59ec84c6).
pub const RIFLE_STANDING_FIRE: [u32; 3] = [0x03ca_d528, 0xaf17_eb9c, 0x2330_6354];
pub const RIFLE_MOVING_FIRE: [u32; 3] = [0xc112_cc47, 0xb973_4a27, 0xbf3c_3419];

/// The rate a shot gives the firing weight, per second (0x82DE88A8).
pub const KICK_RATE: f32 = 30.0;

/// The spring's smooth time, seconds (0x82DE88AC).
pub const KICK_SMOOTH: f32 = 0.1;

/// The weight of the firing sweeps: kicked by each shot, eased back to 0
/// by a critically damped spring (see the module documentation).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Kick {
    /// The value, +0x30 of the record.
    pub value: f32,
    /// Its rate, +0x34.
    pub rate: f32,
}

impl Kick {
    /// A shot fired: the rate set (not added) to [`KICK_RATE`].
    pub fn fire(&mut self) {
        self.rate = KICK_RATE;
    }

    /// Eases on `dt` seconds toward 0.
    pub fn step(&mut self, dt: f32) {
        let omega = 2.0 / KICK_SMOOTH;
        let x = omega * dt;
        let e = 1.0 / (1.0 + x + 0.48 * x * x + 0.235 * x * x * x);
        let temp = (self.value * omega + self.rate) * dt;
        self.rate = (self.rate - temp * omega) * e;
        self.value = (self.value + temp) * e;
    }

    /// The firing clips' weight, 0 to 1.
    pub fn weight(&self) -> f32 {
        self.value.clamp(0.0, 1.0)
    }
}

/// The firing clip's share of a group for firing weight `w`, standing or
/// moving: a crossfade, the sweep taking `1 - w`.
pub fn fire_share(w: f32) -> f32 {
    w.clamp(0.0, 1.0)
}

/// The bones the crouched aim movement poses (the 360's `lowerbody`
/// table, 0x82B76110): the pelvis, hips, and each leg's thigh, calf, foot
/// and toe, by name hash.
pub const LOWER_BODY: [u32; 10] = [
    0xfaef_b386,
    0x24c5_009c,
    0x7685_3d12,
    0xa76c_9842,
    0x1226_f58d,
    0x901f_6d22,
    0xc229_9ac4,
    0x0163_705c,
    0x3120_671b,
    0x9723_4644,
];

/// The bones the aim poses (table 0x2CA54698, at 0x82B762F0): the spine,
/// neck, head, arms, hands and fingers, by name hash. Not the face.
pub const UPPER_BODY: [u32; 48] = [
    0x5ca4_9bd8,
    0x66a2_6cff,
    0x4c77_33ed,
    0xa6ef_dd4f,
    0xc187_105b,
    0x705c_4508,
    0xa3a5_094a,
    0xb2c9_ce63,
    0xbefc_09a2,
    0x0371_9eef,
    0xeb6f_1b2d,
    0xfa76_1573,
    0xca00_61a5,
    0x67f9_0b9a,
    0x49fb_1af7,
    0xa622_1fbb,
    0x8c24_3564,
    0xa626_9ce9,
    0x7f5d_343e,
    0x2164_ef09,
    0x0762_8784,
    0x1ce7_3684,
    0xb6e4_575b,
    0x94e1_e33e,
    0x0cc7_1f3f,
    0x02c9_4e18,
    0x2ccb_cecd,
    0xba8c_a3a8,
    0x20f6_35d9,
    0x23f6_f598,
    0x328c_1f25,
    0xd34f_7713,
    0xf450_ed0d,
    0x248e_1087,
    0xfa90_0d00,
    0x2492_8db5,
    0xe1bb_4731,
    0x5fb3_bec6,
    0x81b6_32e3,
    0x8def_bc44,
    0x27ec_dd1b,
    0x05ea_68fe,
    0x54c6_c05e,
    0x76cd_b1a9,
    0x5ccb_4a24,
    0x163f_93bd,
    0x9438_0b52,
    0xb63a_7f6f,
];

/// Which part of an aiming body a bone (by name hash) follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// The crouched gait ([`LOWER_BODY`]).
    Legs,
    /// The aim, the gait's motion laid over it ([`UPPER_BODY`]).
    Aim,
    /// Neither: the skeleton's reference pose.
    Rest,
}

/// The part of an aiming body the bone named `hash` follows.
pub fn part(hash: u32) -> Part {
    if LOWER_BODY.contains(&hash) {
        Part::Legs
    } else if UPPER_BODY.contains(&hash) {
        Part::Aim
    } else {
        Part::Rest
    }
}

/// How fast the layer's noise runs, per second of the node's clock
/// (0x82003CF4).
pub const LAYER_RATE: f32 = 2.0;

/// The weight of the crouched gait's motion over the aim, the node's
/// clock `clock` seconds in: `0.5 + noise(2 clock)`, or 0.5 for a pistol
/// (see the module documentation). About 0 to 1; not clamped, as the
/// 360 does not.
pub fn layer_weight(clock: f32, pistol: bool) -> f32 {
    let scale = if pistol { 0.0 } else { 1.0 };
    value_noise(clock * LAYER_RATE, 0.5) * scale + 0.5
}

/// The 360's smooth value noise at (`x`, `y`), -1 to 1 (0x825736B0): the
/// lattice's [`smoothed`] values round the point, blended in straight
/// lines across the cell.
pub fn value_noise(x: f32, y: f32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (xi, yi) = (x0 as i32, y0 as i32);
    let s = |dx: i32, dy: i32| smoothed(xi.wrapping_add(dx), yi.wrapping_add(dy));
    let near = s(0, 0) * (1.0 - fx) + s(1, 0) * fx;
    let far = s(0, 1) * (1.0 - fx) + s(1, 1) * fx;
    near * (1.0 - fy) + far * fy
}

/// A lattice point's value with its neighbours' (0x825733B8): a quarter of
/// its own, an eighth of each side's, a sixteenth of each corner's.
pub fn smoothed(x: i32, y: i32) -> f32 {
    let at = |dx: i32, dy: i32| {
        lattice(
            x.wrapping_add(dx)
                .wrapping_add(y.wrapping_add(dy).wrapping_mul(57)),
        )
    };
    let corners = at(-1, -1) + at(1, -1) + at(-1, 1) + at(1, 1);
    let sides = at(-1, 0) + at(1, 0) + at(0, -1) + at(0, 1);
    at(0, 0) * 0.25 + sides * 0.125 + corners * 0.0625
}

/// The lattice's value for key `n` (x + 57 y), -1 to 1 (0x825733B8): an
/// integer hash, `n` mixed by a shift of 13, cubed with odd constants,
/// its low 31 bits scaled by 2^-30 from 1.
pub fn lattice(n: i32) -> f32 {
    let n = (n << 13) ^ n;
    let v = n
        .wrapping_mul(n.wrapping_mul(n).wrapping_mul(15731).wrapping_add(789_221))
        .wrapping_add(1_376_312_589)
        & 0x7fff_ffff;
    1.0 - v as f32 / 1_073_741_824.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sweep_spans_245_degrees() {
        assert_eq!(phase(155f32.to_radians()), 0.0);
        assert_eq!(phase((-90f32).to_radians()), 1.0);
        // Straight ahead, five eighths in.
        assert!((phase(0.0) - 155.0 / 245.0).abs() < 1e-6);
        // Past either end it holds the end.
        assert_eq!(phase(3.0), 0.0);
        assert_eq!(phase(-3.0), 1.0);
    }

    #[test]
    fn a_shot_kicks_and_eases_back() {
        let mut k = Kick::default();
        k.fire();
        let dt = 1.0 / 60.0;
        let mut peak: (f32, usize) = (0.0, 0);
        for i in 1..=60 {
            k.step(dt);
            if k.value > peak.0 {
                peak = (k.value, i);
            }
        }
        // Critically damped: 30 t e^(-20 t), its peak 1.5 / e at 0.05 s.
        assert!((peak.0 - 0.552).abs() < 0.03, "{peak:?}");
        assert!((2..=4).contains(&peak.1), "{peak:?}");
        assert!(k.weight() < 1e-3);
        // Shots in quick succession hold it up, never past 1.
        for _ in 0..30 {
            k.fire();
            for _ in 0..6 {
                k.step(dt);
                assert!(k.weight() <= 1.0);
            }
        }
        assert!(k.weight() > 0.5);
    }

    #[test]
    fn firing_shares() {
        assert_eq!(fire_share(0.0), 0.0);
        assert_eq!(fire_share(0.5), 0.5);
        assert_eq!(fire_share(1.4), 1.0);
    }

    #[test]
    fn the_tables_split_the_body() {
        // The tables share no bone; both hands are the aim's, the pelvis
        // the legs'.
        assert!(LOWER_BODY.iter().all(|h| !UPPER_BODY.contains(h)));
        assert_eq!(part(0xd34f_7713), Part::Aim);
        assert_eq!(part(0xeb6f_1b2d), Part::Aim);
        assert_eq!(part(0xfaef_b386), Part::Legs);
        // The skeleton's root and a face bone rest.
        assert_eq!(part(0xcbc1_eb51), Part::Rest);
        assert_eq!(part(0xbdff_6658), Part::Rest);
    }

    #[test]
    fn the_layer_weight_wanders_round_a_half() {
        // The lattice's hash at 0: 1 - 1376312589 / 2^30.
        assert!((lattice(0) + 0.281_791).abs() < 1e-5, "{}", lattice(0));
        assert_eq!(layer_weight(3.7, true), 0.5);
        // On a lattice line the noise is the cell's two points' mean.
        let at = value_noise(4.0, 0.5);
        assert!((at - (smoothed(4, 0) + smoothed(4, 1)) * 0.5).abs() < 1e-6);
        // Smooth and within -1..1; it moves, and about a half on average.
        let (mut sum, mut last, mut low, mut high) = (0.0, layer_weight(0.0, false), 9f32, -9f32);
        for i in 1..=6000 {
            let w = layer_weight(i as f32 / 60.0, false);
            assert!((-0.5..=1.5).contains(&w));
            assert!((w - last).abs() < 0.05, "a jump at {i}: {last} to {w}");
            (sum, last, low, high) = (sum + w, w, low.min(w), high.max(w));
        }
        assert!(high - low > 0.2, "{low} {high}");
        assert!((sum / 6000.0 - 0.5).abs() < 0.15, "{}", sum / 6000.0);
    }

    #[test]
    fn pitch_shares_the_groups() {
        assert_eq!(weights(0.0), [0.0, 1.0, 0.0]);
        let down = weights((-22.5f32).to_radians());
        assert!((down[0] - 0.5).abs() < 1e-6 && (down[1] - 0.5).abs() < 1e-6);
        assert_eq!(weights((60f32).to_radians()), [0.0, 0.0, 1.0]);
    }
}
