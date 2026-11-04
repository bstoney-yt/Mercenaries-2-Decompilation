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
//! **Inferred**: how the group's blend shares its inputs. Standing, the
//! sweep and its firing clip are a crossfade (1 - w and w). Moving, the
//! moving sweep's input is set to the node's +0x158C path (taken as 1)
//! and its firing clip's to w, taken as shares of their sum.
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

/// The firing clip's share of a group for firing weight `w`: standing a
/// crossfade, moving its share of 1 and `w` (inferred).
pub fn fire_share(w: f32, moving: bool) -> f32 {
    let w = w.clamp(0.0, 1.0);
    if moving { w / (1.0 + w) } else { w }
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
        assert_eq!(fire_share(0.0, false), 0.0);
        assert_eq!(fire_share(0.5, false), 0.5);
        assert!((fire_share(1.0, true) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn pitch_shares_the_groups() {
        assert_eq!(weights(0.0), [0.0, 1.0, 0.0]);
        let down = weights((-22.5f32).to_radians());
        assert!((down[0] - 0.5).abs() < 1e-6 && (down[1] - 0.5).abs() < 1e-6);
        assert_eq!(weights((60f32).to_radians()), [0.0, 0.0, 1.0]);
    }
}
