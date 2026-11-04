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
//! **Not ported**: the standing groups' third clip, weighted by a 0 to 1
//! value at the aiming component's +0x30 (its meaning not traced; taken
//! as 0, so the second clip alone), the moving groups' fourth (weighted
//! by the same value), and the node's easing of the aim yaw (rate at
//! 0x82DE8828).

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
    fn pitch_shares_the_groups() {
        assert_eq!(weights(0.0), [0.0, 1.0, 0.0]);
        let down = weights((-22.5f32).to_radians());
        assert!((down[0] - 0.5).abs() < 1e-6 && (down[1] - 0.5).abs() < 1e-6);
        assert_eq!(weights((60f32).to_radians()), [0.0, 0.0, 1.0]);
    }
}
