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
//!   heroes each have their own);
//! - `ReloadIdle`, `ReloadRun`: changing the clip, standing or on the
//!   move (upper body; the rifle, the pistol and the rocket each have
//!   their own; playtest 8).
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
use m2_engine::hero_action::{self, ThrowWay};

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
    /// Changing its clip, running (true) or standing.
    pub reload: HashMap<bool, usize>,
    /// Throwing a grenade with it held, on the move (true) or standing,
    /// each way from her facing: the clip and when in it the grenade
    /// leaves her hand ([`throw_clip`]).
    pub throws: HashMap<(bool, ThrowWay), (usize, f32)>,
    /// Crouched and aiming ([`m2_engine::on_foot::CROUCH_HOLD`]): each
    /// gait's clip, walking forwards (false) or backwards (true), from
    /// the on-foot table's `Crouch_Aim_*` handles ([`CROUCH_HANDLES`]).
    pub crouch: Vec<(Gait, bool, usize)>,
    /// Crouched and aiming, standing: the shuffles round to the left and
    /// to the right (`Crouch_Aim_Shuffle_Left90`, `..._Right90`).
    pub shuffle: [Option<usize>; 2],
    /// Her own aim sweeps for the class, standing and moving, and their
    /// firing sweeps ([`m2_engine::aim_sweep`]): the on-foot table's rows
    /// for her and the class under the rifle sweeps' handles.
    pub sweeps: Option<crate::scene::Sweeps>,
    /// Standing still with it: the second idle she settles into and the
    /// fidget ([`m2_engine::stand_idle`]), the upright locomotion node's
    /// entries 12 ([`SETTLED_HANDLE`]) and 13 (`Upright_Fidget`).
    pub settled: Option<usize>,
    pub fidget: Option<usize>,
}

/// The upright locomotion node's second idle: entry 12 of its handle
/// list (0x82B4DA98, from the 360 code). Its name is not known (no name
/// guessed hashes to it); the on-foot table gives it rows by class and
/// character, and to anyone holding anything its stand-in for no clip.
pub const SETTLED_HANDLE: u32 = 0x2403_f715;

/// The upright locomotion node's fidget, entry 13 of the same list, by
/// name (its hash is the entry, 700d4de0).
pub const FIDGET_HANDLE: &str = "Upright_Fidget";

/// The on-foot table's handles for the crouched, aiming gaits, by name
/// (their hashes are in the table; the names are the ones that hash to
/// them, as `Upright_Idle` and `Upright_Jog_Forward` do to the upright
/// gaits'): the 360 aims in by going `Crouched`, and its aim movement
/// (`AimMovementNodes`, 0x82802B10) is these seven and two 90 degree
/// shuffles. The alpha's run and sprint are the 360's jog and run.
pub const CROUCH_HANDLES: [(Gait, bool, &str); 7] = [
    (Gait::Idle, false, "Crouch_Aim_Idle"),
    (Gait::Walk, false, "Crouch_Aim_WalkForward"),
    (Gait::Walk, true, "Crouch_Aim_WalkBackward"),
    (Gait::Run, false, "Crouch_Aim_JogForward"),
    (Gait::Run, true, "Crouch_Aim_JogBackward"),
    (Gait::Sprint, false, "Crouch_Aim_RunForward"),
    (Gait::Sprint, true, "Crouch_Aim_RunBackward"),
];

impl Held {
    /// The reload to play, running or standing as `running` says (the
    /// other where the table gives only one).
    pub fn reload_clip(&self, running: bool) -> Option<usize> {
        self.reload
            .get(&running)
            .or_else(|| self.reload.get(&!running))
            .copied()
    }
}

/// Every class's [`Held`], and her pickup.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Moves {
    pub held: Vec<Held>,
    /// Coming down from a fall that hurt, light and heavy (`Upright`
    /// `FallImpactLight` and `FallImpactHeavy`): the whole body, once
    /// ([`m2_engine::on_foot::fall_impact`]).
    pub fall_impacts: [Option<usize>; 2],
    /// `PickupWeapon`: the whole body, once.
    pub pickup: Option<usize>,
    /// `RunStop`: the slide to a stop after a run, the whole body
    /// ([`m2_engine::on_foot::RunStop`]).
    pub run_stop: Option<usize>,
    /// Carrying someone: standing, walking and running, the `Carrying`
    /// locomotion node's gaits ([`hero_action::CARRY_GAITS`]).
    pub carrying: [Option<usize>; 3],
}

impl Moves {
    /// The clip she carries someone with at `gait`: the 360's `Carrying`
    /// node has one clip for the jog, the run and the sprint.
    pub fn carry_clip(&self, gait: Gait) -> Option<usize> {
        self.carrying[match gait {
            Gait::Idle => 0,
            Gait::Walk => 1,
            _ => 2,
        }]
    }

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

/// The animation handle of each fight clip slot whose action the 360
/// code names ([`hero_action::SLOT_ACTIONS`]), by the slot's key clip
/// ([`FightClips::TABLE_KEYS`]): the stance and action table's, looked up
/// forward. A slot the table has no row for is left out (it stays keyed
/// by its clip).
pub fn slot_handles(actions: &AnimationTable) -> HashMap<u32, u32> {
    FightClips::TABLE_KEYS
        .iter()
        .filter_map(|&(slot, key)| {
            let a = hero_action::slot_action(slot)?;
            Some((key, table::action_handle(actions, a.stance, a.action)?))
        })
        .collect()
}

/// The clip the on-foot table `t` gives `who` holding `class` for the
/// slot keyed by `key`: by the slot's action's handle where `forward`
/// ([`slot_handles`]) has one, else by the first row listing the key
/// clip ([`table::character_clip`]).
pub fn slot_clip(
    t: &AnimationTable,
    forward: &HashMap<u32, u32>,
    key: u32,
    who: (u32, u32),
    class: u32,
    fits: impl Fn(u32) -> bool,
) -> Option<table::CharacterClip> {
    match forward.get(&key) {
        Some(&handle) => table::handle_clip(t, handle, who, class, fits),
        None => table::character_clip(t, key, who, class, fits),
    }
}

/// The clip `who` throws a grenade with, holding `class`: the throw's
/// action and way ([`hero_action`]) through both tables, with a grenade
/// in use.
pub fn throw_clip(
    t: &AnimationTable,
    actions: &AnimationTable,
    who: (u32, u32),
    class: u32,
    (moving, way): (bool, ThrowWay),
    fits: impl Fn(u32) -> bool,
) -> Option<table::CharacterClip> {
    let action = hero_action::throw_action(moving);
    let handle = table::action_handle_toward(actions, "Upright", action, way.hash())?;
    table::handle_clip_using(t, handle, who, class, table::GRENADE, fits)
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
    let forward = slot_handles(actions);
    let mut held = Vec::new();
    for name in CLASSES {
        let class = m2_assets::name_hash(name);
        let (clips, gaits) = if class == table::RIFLE {
            (base, base_gaits.to_vec())
        } else {
            let theirs = |key: u32| Some(slot_clip(t, &forward, key, who, class, fits)?.clip);
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
        let mut reload = HashMap::new();
        for (running, action) in [(false, "ReloadIdle"), (true, "ReloadRun")] {
            if let Some(c) = by_action(action, class) {
                reload.insert(running, c);
            }
        }
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
        let mut throws = HashMap::new();
        for moving in [false, true] {
            for way in ThrowWay::ALL {
                let found = throw_clip(t, actions, who, class, (moving, way), fits)
                    .and_then(|p| index(p.clip))
                    .and_then(|i| Some((i, FightClips::release(&rig.clips[i])?)));
                if let Some(found) = found {
                    throws.insert((moving, way), found);
                }
            }
        }
        let crouch = CROUCH_HANDLES
            .iter()
            .filter_map(|&(g, back, name)| {
                let p = table::handle_clip(t, m2_assets::name_hash(name), who, class, fits)?;
                Some((g, back, index(p.clip)?))
            })
            .collect();
        let shuffle = ["Crouch_Aim_Shuffle_Left90", "Crouch_Aim_Shuffle_Right90"].map(|name| {
            let p = table::handle_clip(t, m2_assets::name_hash(name), who, class, fits)?;
            index(p.clip)
        });
        let sweep_of = |keys: [u32; 3]| -> Option<[usize; 3]> {
            let mut out = [0; 3];
            for (o, key) in out.iter_mut().zip(keys) {
                *o = index(table::character_clip(t, key, who, class, fits)?.clip)?;
            }
            Some(out)
        };
        use m2_engine::aim_sweep::{
            CROUCHED_MOVING, CROUCHED_MOVING_FIRE, CROUCHED_STANDING, CROUCHED_STANDING_FIRE,
            RIFLE_MOVING, RIFLE_MOVING_FIRE, RIFLE_STANDING, RIFLE_STANDING_FIRE,
        };
        // She aims in crouched, so the crouched node's sweeps (by handle);
        // the upright ones where the table gives her none.
        let crouched_of = |handles: [u32; 3]| -> Option<[usize; 3]> {
            let mut out = [0; 3];
            for (o, h) in out.iter_mut().zip(handles) {
                *o = index(table::handle_clip(t, h, who, class, fits)?.clip)?;
            }
            Some(out)
        };
        let crouched = crouched_of(CROUCHED_STANDING)
            .zip(crouched_of(CROUCHED_MOVING))
            .map(|(s, m)| {
                let fire = crouched_of(CROUCHED_STANDING_FIRE)
                    .zip(crouched_of(CROUCHED_MOVING_FIRE))
                    .map(|(s, m)| [s, m]);
                crate::scene::Sweeps { aim: [s, m], fire }
            });
        let fire = sweep_of(RIFLE_STANDING_FIRE)
            .zip(sweep_of(RIFLE_MOVING_FIRE))
            .map(|(s, m)| [s, m]);
        let sweeps = crouched.or_else(|| {
            sweep_of(RIFLE_STANDING)
                .zip(sweep_of(RIFLE_MOVING))
                .map(|(s, m)| crate::scene::Sweeps { aim: [s, m], fire })
        });
        let by_handle = |handle: u32| {
            let p = table::handle_clip(t, handle, who, class, fits)?;
            (p.clip != table::NO_ANIMATION)
                .then(|| index(p.clip))
                .flatten()
        };
        held.push(Held {
            settled: by_handle(SETTLED_HANDLE),
            fidget: by_handle(m2_assets::name_hash(FIDGET_HANDLE)),
            sweeps,
            crouch,
            shuffle,
            class,
            clips,
            gaits,
            stow,
            equip,
            reload,
            throws,
        });
    }
    use m2_engine::on_foot;
    Moves {
        held,
        fall_impacts: [on_foot::FallImpact::Light, on_foot::FallImpact::Heavy]
            .map(|i| by_action(i.name(), table::RIFLE)),
        pickup: by_action("PickupWeapon", table::RIFLE),
        run_stop: by_action(m2_engine::on_foot::RUN_STOP_ACTION, table::RIFLE),
        // The carry's rows name no class and no one.
        carrying: hero_action::CARRY_GAITS.map(|h| {
            let p = table::handle_clip(t, h, who, table::RIFLE, fits)?;
            index(p.clip)
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every slot the 360 code names an action for is one of the fight
    /// clips' slots, so [`slot_handles`] leaves none of them out by a
    /// misspelt name.
    #[test]
    fn every_named_action_is_a_fight_clip_slot() {
        for a in hero_action::SLOT_ACTIONS {
            assert!(
                FightClips::TABLE_KEYS.iter().any(|k| k.0 == a.slot),
                "{}",
                a.slot
            );
        }
    }

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
            fall_impacts: [None, None],
            pickup: Some(9),
            run_stop: None,
            carrying: [Some(20), Some(21), Some(22)],
        }
    }

    /// The `Carrying` node has one clip for the jog, run and sprint.
    #[test]
    fn carrying_runs_on_one_clip_above_a_walk() {
        let m = moves();
        assert_eq!(m.carry_clip(Gait::Idle), Some(20));
        assert_eq!(m.carry_clip(Gait::Walk), Some(21));
        for g in [Gait::Run, Gait::Sprint, Gait::FullSprint] {
            assert_eq!(m.carry_clip(g), Some(22));
        }
        assert_eq!(Moves::default().carry_clip(Gait::Run), None);
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
    fn a_reload_plays_running_or_standing_as_the_table_has_it() {
        let both = Held {
            reload: [(false, 11), (true, 12)].into(),
            ..Held::default()
        };
        assert_eq!(both.reload_clip(false), Some(11));
        assert_eq!(both.reload_clip(true), Some(12));
        // Only standing: running plays that too.
        let standing = Held {
            reload: [(false, 11)].into(),
            ..Held::default()
        };
        assert_eq!(standing.reload_clip(true), Some(11));
        assert_eq!(Held::default().reload_clip(false), None);
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

    /// The fidget's name hashes to entry 13 of the upright node's list.
    #[test]
    fn the_fidgets_name_is_the_lists_entry() {
        assert_eq!(m2_assets::name_hash(FIDGET_HANDLE), 0x700d_4de0);
    }
}
