//! Whether a living person staggers when hit, and how: the original's
//! pick of `StaggerShort`, `StaggerInPlace` or `StaggerLong` and of the
//! `DamageDirection` its clip is chosen by.
//!
//! # Provenance
//!
//! From the 360 code:
//!
//! - **The damage event** is built by 0x82157898 and posted to the queue
//!   at 0x8318C480. Its flags word (+0x3C) has the builder's last
//!   argument in bit 31 and a 3-bit kind in bits 26 to 28. Of the 16
//!   places that build one, bit 31 is set by the ordnance hit path
//!   (0x82287CF8: bullets), the melee hit (0x8222B2E8) and the falls,
//!   and clear in the area damage (0x8220CC30, which goes round the
//!   people it finds): [`Event::direct`]. The kind is 3 only in the
//!   melee hit, whose damage is scaled by the `MeleeBash` key:
//!   [`Event::melee`].
//! - **The animation driver's damage handler** (0x824203F0), for a person
//!   still alive, tries a knockdown first when the damage is at least a
//!   threshold; if there is none it calls the picker.
//! - **The picker** (0x824220B0):
//!   - nothing sooner than the longer of the person's `StaggerCooldown`
//!     and [`MIN_GAP`] (the float 0.25 at 0x820DABDC) after their last
//!     stagger;
//!   - a player-kind person never staggers when 0x8228CB80 says so
//!     ([`Person::immune`]), nor crouched unless the hit is a melee bash;
//!   - area damage staggers only when the ordnance's damage key is
//!     `Explosion` (0x16930AFE) or `ExplosionLarge` (0xDC0A895F), and
//!     then it is `StaggerLong` (0x9DFC6A70);
//!   - a direct hit staggers anyone not player-kind, and a player-kind
//!     person only by a melee bash: `StaggerInPlace` (0x03CAA640) when a
//!     field of their movement record is set ([`Person::moving`]) or
//!     their last stagger is at least `StaggerResetSeconds` ago, else
//!     `StaggerShort` (0x3EB62604);
//!   - the direction is by `a`, the person's heading less the heading of
//!     the event's direction, wrapped: over 135 degrees either way
//!     `Front`, under 45 `Back`, else `Right` for `a` under 0 and `Left`
//!     otherwise (the floats 135 at 0x820F9868 and 45 at 0x82B7C5EC);
//!   - the person is given a heading to turn to: the event direction's
//!     plus pi for `Front`, plus a quarter turn for `Left`, less one for
//!     `Right`, and itself for `Back`. The animation driver's
//!     state-change handler (0x8241FE38) sets their physics heading to
//!     it at once as a stagger action starts: a snap, not a turn.
//! - `StaggerCooldown` and `StaggerResetSeconds` are the human physics
//!   template's, registered with the defaults [`COOLDOWN`] and [`RESET`].
//!
//! From samples: the stance and action table has one `Upright` row for
//! each of the three staggers and four directions, twelve handles, each
//! with an on-foot row per class of weapon held, played at a rate
//! between 1.0 and 1.5. The state table leads the three messages to the
//! `Upright` actions of their names.
//!
//! **Inferred**:
//!
//! - The event's direction is the way the hit was travelling: a hit
//!   facing the person head on then reads `Front`, and the heading to
//!   turn to faces where it came from.
//! - [`Person::moving`]: the field is bits 0x3800 of a word of the
//!   movement record; that it means "on the move" is a reading.
//! - Which way `a` turns. Headings here are this crate's, clockwise from
//!   `+z` seen from above; if the original's turn the other way, `Left`
//!   and `Right` are mirrored.
//! - A person who has not staggered yet may: the original's timer was
//!   not read at its start.
//!
//! **Not traced, so not here**: the knockdown's threshold (a variable at
//! 0x82DE887C) and rule, what 0x8228CB80 reads, and the `Stagger` message
//! (0xE0436D42) posted from 0x8227ED30.

use m2_math::glam::Vec3;

/// The least time between two staggers, seconds.
pub const MIN_GAP: f32 = 0.25;

/// The registered defaults of the human physics template's
/// `StaggerCooldown` and `StaggerResetSeconds`, seconds.
pub const COOLDOWN: f32 = 0.0;
pub const RESET: f32 = 1.0;

/// The bounds of the direction's angle, degrees.
pub const FRONT_OVER: f32 = 135.0;
pub const BACK_UNDER: f32 = 45.0;

/// The ordnance damage keys whose area damage staggers.
pub const EXPLOSION: u32 = 0x1693_0afe;
pub const EXPLOSION_LARGE: u32 = 0xdc0a_895f;

/// Which stagger: the message posted, and the `Upright` action it leads
/// to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stagger {
    Short,
    InPlace,
    Long,
}

impl Stagger {
    /// The three, in the order of their discriminants.
    pub const ALL: [Self; 3] = [Self::Short, Self::InPlace, Self::Long];

    /// The action's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Short => "StaggerShort",
            Self::InPlace => "StaggerInPlace",
            Self::Long => "StaggerLong",
        }
    }

    /// The action's name hash.
    pub fn action(self) -> u32 {
        match self {
            Self::Short => crate::blend::action::STAGGER_SHORT,
            Self::InPlace => crate::blend::action::STAGGER_IN_PLACE,
            Self::Long => crate::blend::action::STAGGER_LONG,
        }
    }
}

/// The `DamageDirection` a stagger's clip is chosen by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Front,
    Back,
    Left,
    Right,
}

impl Direction {
    /// The four, in the order of their discriminants.
    pub const ALL: [Self; 4] = [Self::Front, Self::Back, Self::Left, Self::Right];

    /// The direction's name hash, as the stance and action table has it.
    pub fn hash(self) -> u32 {
        match self {
            Self::Front => 0xe4d5_3df6,
            Self::Back => 0xe490_c918,
            Self::Left => 0x25f2_11ae,
            Self::Right => 0x1ca1_1cdd,
        }
    }

    /// What is added to the hit's heading for the heading the person
    /// turns to, radians.
    pub fn turn(self) -> f32 {
        use std::f32::consts::{FRAC_PI_2, PI};
        match self {
            Self::Front => PI,
            Self::Back => 0.0,
            Self::Left => FRAC_PI_2,
            Self::Right => -FRAC_PI_2,
        }
    }
}

/// The person hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Person {
    /// Heading faced, radians clockwise from `+z` seen from above.
    pub facing: f32,
    /// A player-kind person.
    pub player: bool,
    /// What 0x8228CB80 says of a player-kind person (not traced).
    pub immune: bool,
    pub crouched: bool,
    /// The movement record's field (read as "on the move").
    pub moving: bool,
    /// Seconds since their last stagger; `None` before the first.
    pub since: Option<f32>,
    /// Their template's `StaggerCooldown` and `StaggerResetSeconds`.
    pub cooldown: f32,
    pub reset: f32,
}

/// The damage event.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    /// The way the hit was travelling.
    pub direction: Vec3,
    /// Flags bit 31: a direct hit, not area damage.
    pub direct: bool,
    /// The flags' kind is 3: a melee bash.
    pub melee: bool,
    /// The ordnance's damage key, if any.
    pub key: Option<u32>,
}

/// A stagger picked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pick {
    pub stagger: Stagger,
    pub direction: Direction,
    /// The heading the person turns to, radians in `(-pi, pi]`.
    pub turn_to: f32,
}

fn wrap(a: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let a = a.rem_euclid(TAU);
    if a > PI { a - TAU } else { a }
}

/// The direction of a hit travelling `direction` for a person facing
/// `facing`.
pub fn direction(facing: f32, direction: Vec3) -> Direction {
    let a = wrap(facing - direction.x.atan2(direction.z)).to_degrees();
    if a.abs() > FRONT_OVER {
        Direction::Front
    } else if a.abs() < BACK_UNDER {
        Direction::Back
    } else if a < 0.0 {
        Direction::Right
    } else {
        Direction::Left
    }
}

/// The stagger `event` gives the living `person`, if any (0x824220B0).
pub fn pick(person: &Person, event: &Event) -> Option<Pick> {
    let gap = person.cooldown.max(MIN_GAP);
    if person.since.is_some_and(|s| s < gap) {
        return None;
    }
    if person.player && (person.immune || (person.crouched && !event.melee)) {
        return None;
    }
    let stagger = if !event.direct {
        matches!(event.key, Some(EXPLOSION | EXPLOSION_LARGE)).then_some(Stagger::Long)?
    } else if !person.player || event.melee {
        if person.moving || person.since.is_none_or(|s| s >= person.reset) {
            Stagger::InPlace
        } else {
            Stagger::Short
        }
    } else {
        return None;
    };
    let way = direction(person.facing, event.direction);
    Some(Pick {
        stagger,
        direction: way,
        turn_to: wrap(event.direction.x.atan2(event.direction.z) + way.turn()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_assets::name_hash;

    fn soldier() -> Person {
        Person {
            facing: 0.0,
            player: false,
            immune: false,
            crouched: false,
            moving: false,
            since: None,
            cooldown: COOLDOWN,
            reset: RESET,
        }
    }

    fn bullet(direction: Vec3) -> Event {
        Event {
            direction,
            direct: true,
            melee: false,
            key: None,
        }
    }

    #[test]
    fn the_staggers_names_hash_to_their_actions() {
        for s in Stagger::ALL {
            assert_eq!(s.action(), name_hash(s.name()), "{}", s.name());
        }
        assert_eq!(Stagger::ALL.map(|s| s as usize), [0, 1, 2]);
        assert_eq!(Direction::ALL.map(|d| d as usize), [0, 1, 2, 3]);
    }

    #[test]
    fn the_directions_are_the_tables_names() {
        for (d, name) in [
            (Direction::Front, "Front"),
            (Direction::Back, "Back"),
            (Direction::Left, "Left"),
            (Direction::Right, "Right"),
        ] {
            assert_eq!(d.hash(), name_hash(name), "{name}");
        }
    }

    #[test]
    fn a_hit_head_on_is_front_and_one_from_behind_back() {
        // Facing +z: a hit travelling -z meets them head on.
        assert_eq!(direction(0.0, Vec3::NEG_Z), Direction::Front);
        assert_eq!(direction(0.0, Vec3::Z), Direction::Back);
        // facing - heading(+x) = -90 degrees: Right; the other way Left.
        assert_eq!(direction(0.0, Vec3::X), Direction::Right);
        assert_eq!(direction(0.0, Vec3::NEG_X), Direction::Left);
        // The bounds: 135 and 45 degrees themselves are the sides'.
        let at = |deg: f32| {
            let r = deg.to_radians();
            Vec3::new(r.sin(), 0.0, r.cos())
        };
        assert_eq!(direction(0.0, at(140.0)), Direction::Front);
        assert_eq!(direction(0.0, at(130.0)), Direction::Right);
        assert_eq!(direction(0.0, at(-50.0)), Direction::Left);
        assert_eq!(direction(0.0, at(-40.0)), Direction::Back);
    }

    #[test]
    fn a_soldier_shot_staggers_in_place_then_short_within_the_reset() {
        let mut p = soldier();
        let first = pick(&p, &bullet(Vec3::NEG_Z)).unwrap();
        assert_eq!(first.stagger, Stagger::InPlace);
        assert_eq!(first.direction, Direction::Front);
        // Turned to face where it came from: +z, heading 0.
        assert!(first.turn_to.abs() < 1e-5, "{}", first.turn_to);
        p.since = Some(0.2);
        assert_eq!(pick(&p, &bullet(Vec3::NEG_Z)), None);
        p.since = Some(0.5);
        assert_eq!(
            pick(&p, &bullet(Vec3::NEG_Z)).unwrap().stagger,
            Stagger::Short
        );
        p.moving = true;
        assert_eq!(
            pick(&p, &bullet(Vec3::NEG_Z)).unwrap().stagger,
            Stagger::InPlace
        );
        p.moving = false;
        p.since = Some(1.0);
        assert_eq!(
            pick(&p, &bullet(Vec3::NEG_Z)).unwrap().stagger,
            Stagger::InPlace
        );
        // A longer cooldown of their template's holds it off.
        p.cooldown = 2.0;
        assert_eq!(pick(&p, &bullet(Vec3::NEG_Z)), None);
    }

    #[test]
    fn a_player_staggers_only_by_a_melee_bash() {
        let mut p = Person {
            player: true,
            ..soldier()
        };
        assert_eq!(pick(&p, &bullet(Vec3::Z)), None);
        let bash = Event {
            melee: true,
            ..bullet(Vec3::Z)
        };
        let got = pick(&p, &bash).unwrap();
        assert_eq!(
            (got.stagger, got.direction),
            (Stagger::InPlace, Direction::Back)
        );
        // Crouched she still does by a bash; immune, not at all.
        p.crouched = true;
        assert!(pick(&p, &bash).is_some());
        p.immune = true;
        assert_eq!(pick(&p, &bash), None);
        // A crouched soldier staggers from a bullet all the same.
        let crouched = Person {
            crouched: true,
            ..soldier()
        };
        assert!(pick(&crouched, &bullet(Vec3::Z)).is_some());
    }

    #[test]
    fn area_damage_staggers_long_only_from_an_explosion() {
        let area = |key| Event {
            direction: Vec3::X,
            direct: false,
            melee: false,
            key,
        };
        let p = soldier();
        assert_eq!(pick(&p, &area(None)), None);
        assert_eq!(pick(&p, &area(Some(0x1234_5678))), None);
        for key in [EXPLOSION, EXPLOSION_LARGE] {
            let got = pick(&p, &area(Some(key))).unwrap();
            assert_eq!(got.stagger, Stagger::Long);
            assert_eq!(got.direction, Direction::Right);
        }
        assert_eq!(EXPLOSION, name_hash("Explosion"));
        assert_eq!(EXPLOSION_LARGE, name_hash("ExplosionLarge"));
        // The player, too: the player's bar is on direct hits only.
        let player = Person {
            player: true,
            ..soldier()
        };
        assert_eq!(
            pick(&player, &area(Some(EXPLOSION))).unwrap().stagger,
            Stagger::Long
        );
    }
}
