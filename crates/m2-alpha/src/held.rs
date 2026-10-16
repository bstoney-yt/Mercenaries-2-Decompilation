//! What a hero plays for the weapon in her hands: the on-foot animation
//! table's clips for each equipment class she can hold, the stow and
//! take-up clips of a weapon switch, and the weapon pickup.
//!
//! The game keys a character's clips by the class of its primary weapon
//! (the on-foot table's `PrimaryEquipmentClass`: `rifle`, `pistol`,
//! `rocket`, ...), so a machine pistol is held, aimed and run with as a
//! pistol, not a rifle. The stance and action table (`6802c321`) names
//! the moves a switch is made of, each an animation handle the on-foot
//! table gives clips for by class:
//!
//! - `StowRightIdle`, `StowRightRun`, `StowLeftIdle`, `StowLeftRun`:
//!   putting away what she holds, standing or on the move;
//! - `EquipRightIdle`, ... `EquipLeftRun`: taking up the next;
//! - `PickupWeapon`: picking a weapon up off the ground (whole body; the
//!   heroes each have their own).
//!
//! Their rows lay the switch's moves over the upper body (`ActionMask`
//! `UpperBody`) and the pickup over the whole (`FullBody`). Right and
//! left are the sides of her back the weapon goes to or comes from: in
//! the rifle's `StowRightIdle` her left hand rises to her right shoulder,
//! in `StowLeftIdle` her right hand to her left (round 13's traces), as
//! the rifle hangs on her right and what she carries in the launcher's
//! place on her left ([`crate::combat::Combat::slung`]).
//!
//! Provenance: the action names are from samples (round 13: joined from
//! the scripts' words, each matching a table hash); which side is which
//! is inferred from the clips' hands; that a switch is a stow then a
//! take-up, and that the class's clips change between the two, is
//! inferred from the names (the 360 code was not read).

use std::collections::HashMap;

use m2_assets::animation_table::{self as table, AnimationTable};
use m2_engine::Gait;

use crate::npc::FightClips;
use crate::scene::Rig;

/// The equipment classes a hero's clips are worked out for: what the
/// heroes' guns' `Equipment` classes hold (the carbine and rifles
/// `Rifle`, the pistols and machine pistols `Pistol`, the RPG and
/// grenade launcher `Rocket`).
pub const CLASSES: [&str; 3] = ["rifle", "pistol", "rocket"];

/// Which side of her back a weapon hangs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    /// The rifle's place.
    Right,
    /// The launcher's place.
    Left,
}

impl Side {
    fn word(self) -> &'static str {
        match self {
            Side::Right => "Right",
            Side::Left => "Left",
        }
    }
}

/// What a hero plays holding one class of weapon.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Held {
    /// The class (name hash: `rifle`, `pistol`, ...).
    pub class: u32,
    pub clips: FightClips,
    /// The gaits' clips (indices into her rig's clips).
    pub gaits: Vec<(Gait, usize)>,
    /// Putting it away and taking it up, by side and standing or running.
    pub stow: HashMap<(Side, bool), usize>,
    pub equip: HashMap<(Side, bool), usize>,
}

/// Every class's [`Held`], and her pickup.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Moves {
    pub held: Vec<Held>,
    /// `PickupWeapon`: the whole body, once.
    pub pickup: Option<usize>,
}

impl Moves {
    /// The clips for class `class`; the rifle's for a class not worked
    /// out (inferred: the rifle is what every hero starts with).
    pub fn of(&self, class: u32) -> Option<&Held> {
        self.held
            .iter()
            .find(|h| h.class == class)
            .or_else(|| self.held.iter().find(|h| h.class == table::RIFLE))
    }
}

/// A weapon in her hands: its class and the side of her back it hangs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InHand {
    pub class: u32,
    pub side: Side,
}

impl Default for InHand {
    fn default() -> Self {
        Self {
            class: table::RIFLE,
            side: Side::Right,
        }
    }
}

/// A weapon switch under way: `from` is being put away, then `to` taken
/// up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Switch {
    pub from: InHand,
    pub to: InHand,
    /// Seconds in.
    pub time: f32,
    /// The stow's clip and length (none: nothing to play, the take-up at
    /// once), and the take-up's.
    pub stow: Option<(usize, f32)>,
    pub equip: Option<(usize, f32)>,
}

/// Where in a switch she is ([`Switch::at`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// Putting away: clip and seconds in.
    Stow(usize, f32),
    /// Taking up: clip and seconds in.
    Equip(usize, f32),
    Done,
}

impl Switch {
    /// A switch from `from` to `to` with the clips `moves` gives, running
    /// or standing as `running` says.
    pub fn new(moves: &Moves, from: InHand, to: InHand, running: bool) -> Self {
        let clip = |held: Option<&Held>, stow: bool, side: Side| {
            let held = held?;
            let map = if stow { &held.stow } else { &held.equip };
            map.get(&(side, running))
                .or_else(|| map.get(&(side, !running)))
                .copied()
        };
        Self {
            from,
            to,
            time: 0.0,
            stow: clip(moves.of(from.class), true, from.side).map(|c| (c, 0.0)),
            equip: clip(moves.of(to.class), false, to.side).map(|c| (c, 0.0)),
        }
    }

    /// With each clip's length from `length`.
    pub fn timed(mut self, length: impl Fn(usize) -> f32) -> Self {
        self.stow = self.stow.map(|(c, _)| (c, length(c)));
        self.equip = self.equip.map(|(c, _)| (c, length(c)));
        self
    }

    /// Where `time` seconds in falls.
    pub fn at(&self, time: f32) -> Phase {
        let stow = self.stow.map_or(0.0, |s| s.1);
        match (self.stow, self.equip) {
            (Some((c, _)), _) if time < stow => Phase::Stow(c, time),
            (_, Some((c, len))) if time - stow < len => Phase::Equip(c, time - stow),
            _ => Phase::Done,
        }
    }

    /// Whether the class taken up holds by now (the stow is over).
    pub fn swapped(&self) -> bool {
        !matches!(self.at(self.time), Phase::Stow(..))
    }
}

/// Works out [`Moves`] for a hero (`who`: the template's gender and
/// character name) on `rig`: for each of [`CLASSES`], the fight clips and
/// gaits the on-foot table `t` gives the class by `keys` (each slot's or
/// gait's key clip, [`FightClips::TABLE_KEYS`] and the gait keys), over
/// `base` (the rifle's as already picked: kept for a slot the table
/// gives the class nothing) and `base_gaits`; the switch's stow and
/// take-up for each class and side; and the pickup. The rifle's are
/// `base` and `base_gaits` themselves (picked against the footage).
/// `actions` is the stance and action table.
pub fn resolve(
    t: &AnimationTable,
    actions: &AnimationTable,
    who: (u32, u32),
    rig: &Rig,
    base: FightClips,
    base_gaits: &[(Gait, usize)],
    gait_keys: &[(Gait, u32)],
) -> Moves {
    let index = |hash: u32| {
        rig.clips
            .iter()
            .position(|c| c.hash == hash && !rig.foreign.contains(&hash))
    };
    let fits = |c: u32| index(c).is_some();
    let by_action = |action: &str, class: u32| {
        let h = table::action_handle(actions, "Upright", action)?;
        let p = table::handle_clip(t, h, who, class, fits)?;
        index(p.clip)
    };
    let mut held = Vec::new();
    for name in CLASSES {
        let class = m2_assets::name_hash(name);
        let (clips, gaits) = if class == table::RIFLE {
            (base, base_gaits.to_vec())
        } else {
            let theirs = |key: u32| Some(table::character_clip(t, key, who, class, fits)?.clip);
            let mut clips = base;
            clips.by_table(rig, &theirs);
            let gaits = gait_keys
                .iter()
                .filter_map(|&(g, key)| Some((g, index(theirs(key)?)?)))
                .collect();
            (clips, gaits)
        };
        let mut stow = HashMap::new();
        let mut equip = HashMap::new();
        for side in [Side::Right, Side::Left] {
            for running in [false, true] {
                let gait = if running { "Run" } else { "Idle" };
                let w = side.word();
                if let Some(c) = by_action(&format!("Stow{w}{gait}"), class) {
                    stow.insert((side, running), c);
                }
                if let Some(c) = by_action(&format!("Equip{w}{gait}"), class) {
                    equip.insert((side, running), c);
                }
            }
        }
        held.push(Held {
            class,
            clips,
            gaits,
            stow,
            equip,
        });
    }
    Moves {
        held,
        pickup: by_action("PickupWeapon", table::RIFLE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moves() -> Moves {
        let pistol = m2_assets::name_hash("pistol");
        let rifle = Held {
            class: table::RIFLE,
            stow: [((Side::Right, false), 1), ((Side::Right, true), 2)].into(),
            equip: [((Side::Right, false), 3)].into(),
            ..Held::default()
        };
        let pistol = Held {
            class: pistol,
            stow: [((Side::Right, false), 4)].into(),
            equip: [((Side::Right, false), 5), ((Side::Right, true), 6)].into(),
            ..Held::default()
        };
        let rocket = Held {
            class: m2_assets::name_hash("rocket"),
            equip: [((Side::Left, false), 7)].into(),
            ..Held::default()
        };
        Moves {
            held: vec![rifle, pistol, rocket],
            pickup: Some(9),
        }
    }

    #[test]
    fn the_classes_are_the_tables_words() {
        assert_eq!(m2_assets::name_hash(CLASSES[0]), table::RIFLE);
        assert_eq!(m2_assets::name_hash("Rifle"), table::RIFLE);
    }

    #[test]
    fn a_switch_stows_the_old_then_takes_up_the_new() {
        let m = moves();
        let pistol = m2_assets::name_hash("pistol");
        let from = InHand::default();
        let to = InHand {
            class: pistol,
            side: Side::Right,
        };
        let s = Switch::new(&m, from, to, true).timed(|_| 0.5);
        // Running: the rifle's running stow, the pistol's running take-up.
        assert_eq!((s.stow, s.equip), (Some((2, 0.5)), Some((6, 0.5))));
        assert_eq!(s.at(0.25), Phase::Stow(2, 0.25));
        assert_eq!(s.at(0.75), Phase::Equip(6, 0.25));
        assert_eq!(s.at(1.25), Phase::Done);
        assert!(!s.swapped());
        // Standing back to the rifle: the pistol's standing stow, and the
        // rifle's standing take-up.
        let back = Switch::new(&m, to, from, false).timed(|_| 0.5);
        assert_eq!((back.stow, back.equip), (Some((4, 0.5)), Some((3, 0.5))));
        // Running back: the rifle has no running take-up, so its standing
        // one.
        let back = Switch::new(&m, to, from, true);
        assert_eq!(back.equip.map(|e| e.0), Some(3));
        let to_rocket = InHand {
            class: m2_assets::name_hash("rocket"),
            side: Side::Left,
        };
        let s = Switch::new(&m, from, to_rocket, false).timed(|_| 0.5);
        assert_eq!((s.stow, s.equip), (Some((1, 0.5)), Some((7, 0.5))));
        assert!(matches!(s.at(0.0), Phase::Stow(1, _)));
        // Nothing for the side: no clip, it is done at once.
        let none = Switch::new(&m, to_rocket, to_rocket, false);
        assert_eq!(none.at(0.0), Phase::Done);
        assert!(none.swapped());
    }

    #[test]
    fn a_class_not_worked_out_holds_as_the_rifle() {
        let m = moves();
        assert_eq!(m.of(0x1234).map(|h| h.class), Some(table::RIFLE));
        assert_eq!(
            m.of(m2_assets::name_hash("pistol")).map(|h| h.class),
            Some(m2_assets::name_hash("pistol"))
        );
    }
}
