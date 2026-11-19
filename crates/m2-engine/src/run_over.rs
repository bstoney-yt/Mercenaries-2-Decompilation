//! A body running into a person: the collision event their character
//! proxy makes on each contact update, and what the crash step does with
//! it when one side is a person.
//!
//! From the 360 code:
//!
//! - **The event.** The human's proxy contact callbacks (0x82176830, and
//!   0x82176520 for a body its push impulse exceeds 1e-4 on) call
//!   0x82178AD8 on each contact update. That builds a collision event
//!   (0x82163B80) and posts it to the ordinary collision queue 0x8320C6F0.
//!   - `closing` (the event's +0x38) is the contact normal dotted with the
//!     body's velocity at the contact point less the person's velocity
//!     (human vfunc +0x4C).
//!   - The mass is that of whichever side has the more `0.5 m v^2`: the
//!     person's (proxy +0x70, human vfunc +0x60, 0x82175C60) or the body's
//!     (its +0xD0, or 15000 kg when 0, 0x820C77FC).
//!   - `size` (the event's +0) is `0.5 * sqrt(mass) * closing^2` times each
//!     side's factor for the other (vfunc +0xBC, 1.0 for both, 0x822FB8F0).
//!     It is posted only when over 1 (0x820BAEE8), so it can come on every
//!     update the two stay in contact.
//!   - The person is side 0 and the body's object side 1. A body that is a
//!     pushable prop (0x82155620) is flagged (+0x4D) to take no damage.
//! - **The crash step** (0x8228B5F0, from 0x82282530):
//!   - Under [`crate::durability::CRASH_FLOOR`] (5) nothing happens.
//!   - With no `Crusher` standing and exactly one side a person (object
//!     kind 1, vfunc +0xE0; the human's class, 0x82176F38), there is no
//!     damage unless the other side moves at [`OTHER_SPEED`] or more (its
//!     speed squared against 1.0) and `closing` is [`PERSON_CLOSING`] or
//!     more (0x820B7B50).
//!   - Otherwise each side takes a hundredth of the size times its damage
//!     table cell, counted only past 1 (0x82287CF8,
//!     [`crate::durability::crash_damage`]).
//!
//! **Inferred**: the contact normal points from the body toward the
//! person (Havok's character interaction normal), so a body coming at
//! them closes positively. With the opposite sense the gate would only
//! pass a person running into a body faster than 6 m/s.

use m2_math::glam::Vec3;

use crate::durability::{FIXED_MASS, crash_damage};

/// The least `closing` (m/s) at which the crash step lets a body hurt a
/// person (0x820B7B50).
pub const PERSON_CLOSING: f32 = 6.0;

/// The least speed (m/s) of the other side for it to hurt a person (its
/// speed squared against 1.0, 0x8228B5F0).
pub const OTHER_SPEED: f32 = 1.0;

/// An event is posted only above this size (0x820BAEE8).
pub const POSTED: f32 = 1.0;

/// One contact update between a person's proxy and a body.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Touch {
    /// The person's mass (kg) and velocity (m/s).
    pub person_mass: f32,
    pub person_velocity: Vec3,
    /// The body's mass (kg; 0 counts as 15000) and its velocity at the
    /// contact point (m/s).
    pub body_mass: f32,
    pub body_velocity: Vec3,
    /// The contact normal, from the body toward the person.
    pub normal: Vec3,
}

/// The collision event a contact update makes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    pub size: f32,
    pub closing: f32,
}

impl Touch {
    /// The event this contact update posts, if any (0x82178AD8).
    pub fn event(&self) -> Option<Event> {
        let n = self.normal.normalize_or_zero();
        let closing = n.dot(self.body_velocity - self.person_velocity);
        let body_mass = if self.body_mass > 0.0 {
            self.body_mass
        } else {
            FIXED_MASS
        };
        let person = 0.5 * self.person_mass * self.person_velocity.length_squared();
        let body = 0.5 * body_mass * self.body_velocity.length_squared();
        let mass = if person > body {
            self.person_mass
        } else {
            body_mass
        };
        let size = 0.5 * mass.sqrt() * closing * closing;
        (size > POSTED).then_some(Event { size, closing })
    }
}

/// What the crash step (0x8228B5F0, 0x82287CF8) does to the person for an
/// event, the other side moving at `other_speed` (m/s), with the person's
/// damage table `cell` for the other side's kind: 0 when the person gate
/// holds it back or it is not over 1.
pub fn person_damage(event: &Event, other_speed: f32, cell: f32) -> f32 {
    if other_speed * other_speed < OTHER_SPEED * OTHER_SPEED || event.closing < PERSON_CLOSING {
        return 0.0;
    }
    crash_damage(event.size, cell)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(closing: f32, her: Vec3) -> Touch {
        Touch {
            person_mass: 80.0,
            person_velocity: her,
            body_mass: 2000.0,
            body_velocity: Vec3::new(0.0, 0.0, closing),
            normal: Vec3::Z,
        }
    }

    /// A 2 t car closing at 8 m/s: size 0.5 x sqrt(2000) x 64, about
    /// 1431; a `Hero` (cell 0.2) takes a hundredth of it times 0.2.
    #[test]
    fn a_car_closing_fast_hurts_by_the_square_of_its_closing() {
        let e = touch(8.0, Vec3::ZERO).event().expect("posted");
        assert!((e.size - 0.5 * 2000f32.sqrt() * 64.0).abs() < 1e-2);
        assert_eq!(e.closing, 8.0);
        let d = person_damage(&e, 8.0, 0.2);
        assert!((d - e.size * 0.01 * 0.2).abs() < 1e-4, "{d}");
        // Twice the closing, four times the damage.
        let f = touch(16.0, Vec3::ZERO).event().expect("posted");
        assert!((person_damage(&f, 16.0, 0.2) / d - 4.0).abs() < 1e-3);
    }

    /// Under 6 m/s closing, or the other side under 1 m/s, nothing.
    #[test]
    fn the_person_gate_holds_back_slow_closing_and_still_bodies() {
        let slow = touch(5.0, Vec3::ZERO).event().expect("posted");
        assert_eq!(person_damage(&slow, 5.0, 0.2), 0.0);
        // Walking into it at 2 m/s while it comes at 5: 7 m/s closing.
        let walking = touch(5.0, Vec3::new(0.0, 0.0, -2.0))
            .event()
            .expect("posted");
        assert_eq!(walking.closing, 7.0);
        assert!(person_damage(&walking, 5.0, 0.2) > 0.0);
        // She runs into a body lying still at 7 m/s: it does not move.
        let still = touch(0.0, Vec3::new(0.0, 0.0, -7.0))
            .event()
            .expect("posted");
        assert_eq!(person_damage(&still, 0.0, 0.2), 0.0);
    }

    /// A body moving away from her closes negatively and is held back.
    #[test]
    fn a_body_moving_off_does_no_harm() {
        let e = touch(-8.0, Vec3::ZERO).event().expect("posted");
        assert_eq!(person_damage(&e, 8.0, 0.2), 0.0);
    }

    /// The heavier-moving side's mass: a still body counts at 15000 kg
    /// when it has none.
    #[test]
    fn the_mass_is_the_side_with_the_more_energy() {
        let mut t = touch(0.0, Vec3::new(0.0, 0.0, -3.0));
        t.body_mass = 0.0;
        let e = t.event().expect("posted");
        assert!((e.size - 0.5 * 80f32.sqrt() * 9.0).abs() < 1e-3);
        t.body_velocity = Vec3::new(0.3, 0.0, 0.0);
        let e = t.event().expect("posted");
        assert!((e.size - 0.5 * FIXED_MASS.sqrt() * 9.0).abs() < 1e-2);
        // Nothing posted at or under a size of 1.
        assert!(touch(0.1, Vec3::ZERO).event().is_none());
    }
}
