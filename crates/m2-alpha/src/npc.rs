//! People on the island, as the entity layers place them: soldiers at the
//! posts and on the patrol routes the layers give, story characters, and
//! whom the layers' spawners send out, each while their layer is in (a
//! mission's state layer, or a static sub-layer the game loaded). Nobody
//! else: the layers' `PopulationDensity` and `PopulationFlow` (likely the
//! game's passers-by) are not decoded.
//!
//! Each stands in their template's own model (its `ModelName`) with the
//! gun its `EquipmentLink`s hand them. A soldier whose template has no
//! model of its own (or one that won't load) stands in the catalogue's
//! model for the role their name asks for (`VZ Soldier ..`, `VZ Heavy ..`,
//! `VZ Deathsquad ..`), as in the browser viewer. Which side anyone is on
//! is not decoded: the Venezuelans (by name) and a mission's targets
//! fight. Whom spawners send out stand whoever they are, but of the
//! rest the layers place only story characters do (a one-word template:
//! `Carmona`), not another army's. A story character is the scripts':
//! they stay where they are put. The
//! behaviour (standing at posts, walking routes there and back with
//! pauses, running from gunfire) is invented: the game's AI code is not
//! decoded.
//!
//! How a soldier fights comes from their template's AI data
//! ([`AiProfile`]): their health, the cone they see in (`VisionFOV`, 180
//! degrees for people) and how far (`Vision`, `Hearing` multiples), how
//! fast they chase and move about in a fight (`PursueSpeed`,
//! `CombatSpeed`, fractions of a sprint), how well they shoot
//! (`Accuracy`, see [`combat`](crate::combat)), how often they push in
//! rather than step aside (`Offensive`), and `Pacifist`. The distances
//! those multiply and the timings are the alpha's own (inferred, marked
//! so). A soldier who sees the player, hears gunfire close by or is hit
//! turns on her: fires while he sees her and she is in range, chases to
//! where he last saw her when not, steps about between shots, and gives
//! up after a while without sight of her, and lob a grenade (if their
//! template gives them one and `NoGrenades` doesn't stop them) at where
//! she hides. Spawners' people who don't fight run.
//!
//! [`SmarterAi`] (the settings' `[enhanced.smarter_ai]`, off unless
//! switched on) is not the original game: soldiers call out where she is
//! to those near, spread around her rather than stepping at random, run
//! for cover when hit, throw grenades sooner, and aim better.

use std::collections::{HashMap, HashSet};

use m2_engine::tuning::HandGun;
use m2_engine::weapons::FireHint;
use m2_engine::world::{SpawnPoint, is_walker};
use m2_engine::{AiProfile, BoundClip, Character, Gait, GpuModel, Physics, RagdollId};
use m2_math::glam::{Mat4, Quat, Vec2, Vec3};
use serde::{Deserialize, Serialize};

use crate::scene::{Carry, Rig};

/// Soldiers come and go within these distances of the player, metres.
const ACTIVE: f32 = 450.0;
const INACTIVE: f32 = 500.0;
/// Most soldiers about at once, and most passers-by on the roads
/// (invented: the frame rate's limits; the traffic zones ask for 20 at
/// most, Maracaibo's centre).
const MAX_SOLDIERS: usize = 48;
const MAX_WALKERS: usize = 24;
/// A spawner whose one fell sends the next its `SecondsPerCycle` later,
/// but not while she is this near it, metres (invented: the data's
/// fields for how many it sends in all aren't decoded).
const REFILL_CLEAR: f32 = 20.0;
/// How far from where they settled after running people stroll, metres.
const WANDER: f32 = 30.0;
/// Walking and fleeing speeds, m/s, and the pause at each end of a route
/// or stroll.
const WALK: f32 = 1.4;
const FLEE: f32 = 5.0;
const FLEE_TIME: f32 = 6.0;
/// How far a runner who meets a wall looks along it each way, metres.
const CLEAR: f32 = 6.0;
/// A soldier fights within this range of the player, and fires within
/// his gun's `FireRange` once facing within `FIRE_ARC` radians of them
/// (`FIRE_RANGE` for one with no gun: the "Assault Rifle"'s).
const FIGHT_RANGE: f32 = 80.0;
const FIRE_RANGE: f32 = 70.0;
const FIRE_ARC: f32 = 0.3;
/// He aims (the aim pose, turned onto her) only while she is within this
/// of the way he faces, radians (60 degrees; invented, playtest 8).
const AIM_ARC: f32 = 1.05;
/// How far a soldier with `Vision` 1 sees, metres (inferred: the range
/// they fight in), from how high (his head), and how often he looks,
/// seconds.
const SIGHT: f32 = FIGHT_RANGE;
const EYE: f32 = 1.6;
const LOOK_EVERY: f32 = 0.2;
/// A soldier's full sprint, m/s, that `PursueSpeed` and `CombatSpeed`
/// are fractions of (inferred: the player's sprint).
const SPRINT: f32 = 7.2;
/// Seconds a soldier hunts without sight of her before calming down, and
/// how close to where he last saw her he goes (inferred).
const GIVE_UP: f32 = 20.0;
const CLOSE_ENOUGH: f32 = 2.0;
/// A chase stopped by a wall goes along it until the way on is open this
/// far, metres, or for at most this long, seconds (invented).
const DETOUR_OPEN: f32 = 2.5;
const DETOUR_TIME: f32 = 4.0;
/// The least he goes round before looking again, seconds (invented).
const DETOUR_MIN: f32 = 1.0;
/// Between steps about in a fight: seconds (at least, plus up to), how
/// far a step goes, metres, and how close he pushes in to her (inferred).
const STEP_GAP: (f32, f32) = (2.0, 3.0);
const STEP: (f32, f32) = (3.0, 6.0);
const PUSH_TO: f32 = 15.0;
/// They fire in bursts as their gun's `WeaponHint` has it
/// ([`FireHint`]): rounds at its `RateOfFire` for `BurstOnLength`, then
/// `BurstOffLength`'s pause (the close pair within [`FireHint::CLOSE`]),
/// between its `MinFireRange` and `FireRange`. Those whose template names
/// no gun fire as the "Assault Rifle" does (the AK47 stands in): 450 a
/// minute, its `WeaponHint` below (gap round 13's dump).
const BURST_GAP: f32 = 60.0 / 450.0;
const STAND_IN: FireHint = FireHint {
    min_range: 1.0,
    range: FIRE_RANGE,
    angle: 2,
    on: 0.8,
    off: 9.0,
    on_close: 0.8,
    off_close: 1.5,
};
/// Each pause is this much longer or shorter at random, so soldiers who
/// see her together don't fire together (invented).
const PAUSE_JITTER: f32 = 0.25;
const PAUSE: (f32, f32) = (2.0, 6.0);
/// A soldier lobs a grenade at where she hides once she has been out of
/// his sight this long, seconds, within this reach, metres, and no
/// sooner than this after his last (inferred).
const GRENADE_AFTER: f32 = 4.0;
const GRENADE_REACH: (f32, f32) = (8.0, 25.0);
const GRENADE_GAP: f32 = 15.0;
/// She counts as hiding where he last saw her while within this of it,
/// metres: no grenades after her once she has driven or run off.
const HIDING: f32 = 6.0;

/// The enhanced, smarter soldiers: not the original game's. Off by
/// default; every number is the alpha's own.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SmarterAi {
    pub enabled: bool,
    /// A soldier who sees her tells those within this, metres, where she
    /// is: they join the fight knowing.
    pub callout_range: f32,
    /// Soldiers spread around her, each to his own side, this far from
    /// her (metres), rather than stepping aside at random.
    pub flank: bool,
    pub flank_distance: f32,
    /// A soldier hit runs to the nearest spot within `cover_search`
    /// metres that she can't see, and stays down there this long,
    /// seconds, before coming out again.
    pub cover: bool,
    pub cover_search: f32,
    pub cover_time: f32,
    /// Grenades at her hiding place after this long out of sight, and
    /// again after this, seconds.
    pub grenade_after: f32,
    pub grenade_gap: f32,
    /// How much less they miss by: 1 is the original's spread.
    pub accuracy_scale: f32,
}

impl Default for SmarterAi {
    fn default() -> Self {
        Self {
            enabled: false,
            callout_range: 40.0,
            flank: true,
            flank_distance: 25.0,
            cover: true,
            cover_search: 10.0,
            cover_time: 2.5,
            grenade_after: 2.0,
            grenade_gap: 8.0,
            accuracy_scale: 1.5,
        }
    }
}

/// People are drawn this close to the camera, metres.
pub const DRAW_DISTANCE: f32 = 200.0;
const TURN_RATE: f32 = 6.0;
/// Seconds the dead take to topple when no fall clip fits their model.
const TOPPLE: f32 = 0.6;
/// Slowest a vehicle hurts people it meets, m/s.
pub const RUN_OVER: f32 = 2.5;
/// Seconds after a vehicle hurts someone it doesn't kill before one can
/// hurt them again, and how hard it shoves them aside (m/s). Invented.
pub const RAM_GAP: f32 = 1.0;
const SHOVE: f32 = 4.0;
/// How far ahead (seconds) people see a vehicle coming at them.
const DODGE_TIME: f32 = 2.0;

/// A person model and the rig that animates it.
#[derive(Debug)]
pub struct PersonKind {
    /// The template whose own model it is (`VZ Soldier`, `Carmona`); or
    /// the catalogue's role (`vz_hum_soldier`), for soldiers whose
    /// template has no model of its own.
    pub role: String,
    /// In a template's own model: `role` is the template.
    pub named: bool,
    /// The gun the template's `EquipmentLink`s hand them; `None` for a
    /// role's, or a template naming none (the AK47 stands in).
    pub gun: Option<HandGun>,
    pub model: GpuModel,
    pub rig: Rig,
}

/// What a person is doing.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Brain {
    /// Standing at a post, facing `facing`.
    Post { facing: f32 },
    /// Walking a route: heading to point `next`, `step` +1 or -1 along it
    /// (open routes are walked there and back).
    Patrol {
        route: Vec<Vec3>,
        next: usize,
        step: isize,
        wait: f32,
    },
    /// Strolling to `target`, near `home`.
    Wander { home: Vec3, target: Vec3, wait: f32 },
    /// Sent along a path by the scripts (`Ai.Goal` `PathMove`, `MoveTo`):
    /// running to point `next`, then at a post at its end.
    Route { route: Vec<Vec3>, next: usize },
}

/// A clip carrying an event: its index, when in it the event comes, and
/// how far its first pose is from the rifle held ready.
pub type Carrying = (usize, f32, f32);

/// The clips a fight plays, indices into every human rig's clips (they
/// share one list). Clips are named only by hash; these were picked by
/// looking at each pose, or by the events they carry (a jump's
/// `footstep_jumptakeoff`, a throw's `releasegrenade`, a melee bash's
/// `meleebash`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FightClips {
    /// Standing, rifle shouldered and aimed (resident 0163f2aa).
    pub aim: Option<usize>,
    /// Standing, rifle held across the chest (resident 178a5642).
    pub ready: Option<usize>,
    /// Running with the rifle held low across the body in both hands.
    pub run_low: Option<usize>,
    /// Sprinting with the rifle in both hands: her sprint gait's clip when
    /// it holds the rifle so ([`two_handed_sprint`]), else the run above
    /// (the player's alone: set by [`crate::player::Player`]).
    pub sprint: Option<usize>,
    /// A jump: take-off and the air, ending off the ground (resident
    /// c846e6fd); the stand-in when the take-offs below aren't there.
    pub jump: Option<usize>,
    /// Taking off on the run, and from standing (Jennifer's own, with
    /// `footstep_jumptakeoff` 0.1 s in).
    pub takeoff: Option<usize>,
    pub takeoff_still: Option<usize>,
    /// Landing on the run, and standing.
    pub land: Option<usize>,
    pub land_still: Option<usize>,
    /// Throwing a grenade, and how far into it (seconds) the grenade
    /// leaves the hand: its `releasegrenade` event.
    pub throw: Option<(usize, f32)>,
    /// Falling down dead (resident 479c4a51).
    pub die: Option<usize>,
    /// A melee bash with what she holds, and one down low (at a crate on
    /// the ground), with how far into each (seconds) the blow lands: its
    /// `meleebash` or `MeleeBashLow` event. Twelve clips carry each (round
    /// 89): the high one is [`BASH_CLIPS`](Self::BASH_CLIPS), the low one
    /// the one starting nearest the rifle held ready.
    pub bash: Option<(usize, f32)>,
    pub bash_low: Option<(usize, f32)>,
    /// Swimming: the looped stroke, and standing up out of the water. From
    /// samples (the on-foot tables): `Swim` `EnterSwim` (handle cca02d6b)
    /// gives c846e6fd, `Upright` `ExitSwim` (e5653a8b) 97c840ed. The
    /// [`jump`](Self::jump) stand-in is that stroke too, as named before
    /// the tables were read; Jennifer's own take-offs come first.
    pub swim: Option<usize>,
    pub swim_out: Option<usize>,
}

/// How far apart a clip's hands get over the whole clip, for its hip
/// height: a rifle held in both hands keeps them close.
pub fn hand_spread(rig: &Rig, c: &BoundClip) -> Option<f32> {
    let (right, left) = crate::combat::hands(&rig.skeleton)?;
    let last = c.clip.frame_count.saturating_sub(1).max(1) as f32;
    let widest = (0..c.clip.frame_count)
        .map(|f| {
            let pose = c.sample(&rig.skeleton, c.info.duration * f as f32 / last);
            let nodes = rig.skeleton.model_matrices(&pose);
            (nodes[right].w_axis - nodes[left].w_axis)
                .truncate()
                .length()
        })
        .fold(0.0, f32::max);
    (c.info.hips > 0.0).then(|| widest / c.info.hips)
}

/// What she sprints with a rifle in: her sprint gait's clip if its hands
/// stay as close as a two-handed run's (nearer the spread of `run_low`,
/// the rifle low across her in both hands, than of the run gait, arms
/// free), else `run_low` itself. Playtest 8: the sprint clip's arms are
/// free, and the rifle, hung from a hand held as she stands, dangled by
/// her leg. From samples (the clips' hands); that the game sprints with
/// it in both hands is inferred from its rifle run.
pub fn two_handed_sprint(rig: &Rig, run_low: Option<usize>) -> Option<usize> {
    let run_low = run_low?;
    let sprint = rig.gait_clip(Gait::Sprint);
    let spread = |i: usize| hand_spread(rig, &rig.clips[i]);
    let both = spread(run_low);
    let free = rig.gait_clip(Gait::Run).and_then(spread);
    let s = sprint.and_then(spread);
    let holds =
        matches!((both, free, s), (Some(b), Some(f), Some(s)) if b < f && s < (b + f) * 0.5);
    eprintln!(
        "hero clips: sprint {}: hands {} apart for the hips (two-handed run {}, run gait {}): {}",
        sprint.map_or("none".to_owned(), |i| format!("{:08x}", rig.clips[i].hash)),
        s.map_or("?".to_owned(), |s| format!("{s:.2}")),
        both.map_or("?".to_owned(), |s| format!("{s:.2}")),
        free.map_or("?".to_owned(), |s| format!("{s:.2}")),
        if holds {
            "it holds the rifle"
        } else {
            "the two-handed run plays"
        },
    );
    if holds { sprint } else { Some(run_low) }
}

impl FightClips {
    pub const AIM: u32 = 0x0163_f2aa;
    pub const READY: u32 = 0x178a_5642;
    /// Running with the rifle low across the body, two-handed, likeliest
    /// first (round 152's measures: Jennifer's 4.65 m/s run holds it 34
    /// degrees across, muzzle a little down; the resident's 5.21 m/s
    /// ones 11 degrees across, or 42 with the muzzle up and the rifle
    /// rolled); the run she had, 10912665, is Mattias's.
    pub const RUN_LOW: [u32; 4] = [0xe340_14c3, 0x2105_c540, 0x94b1_e4ac, 0x745e_b940];
    pub const JUMP: u32 = 0xc846_e6fd;
    pub const DIE: u32 = 0x479c_4a51;
    /// The swim stroke and the climb-out (see [`FightClips::swim`]).
    pub const SWIM: u32 = 0xc846_e6fd;
    pub const SWIM_OUT: u32 = 0x97c8_40ed;
    /// Jennifer's running take-offs (0.93 s, 3.6 m/s), then the
    /// resident's; standing (0.37 s).
    pub const TAKEOFF: [u32; 4] = [0xdaf7_966f, 0x4fdb_3497, 0x0e18_b683, 0x47af_6321];
    pub const TAKEOFF_STILL: [u32; 1] = [0x3aee_9ae8];
    /// Jennifer's landings on the run (0.37 s, 6.4 m/s) and standing
    /// (0.47 s).
    pub const LAND: [u32; 3] = [0xfe10_b04e, 0xd325_9f36, 0xf57a_8dea];
    pub const LAND_STILL: [u32; 4] = [0xbe97_1f8f, 0xe4dc_de37, 0x3c17_66b3, 0x4de5_ef60];
    /// Grenade throws, likeliest first: a whole throw of 0.77 s (release
    /// 0.30 s in), one of 1.74 s (0.80 s), a short one (0.47 s).
    pub const THROW: [u32; 3] = [0xec6f_fa31, 0xb2e6_acf6, 0x91eb_aa5c];
    /// The high bash, likeliest first: of the twelve clips with a
    /// `meleebash`, these alone bring the rifle's butt down (round 91's
    /// pictures: a one-handed overhead smash, fe47371e leaning into it;
    /// the rest jab with the muzzle), and with the launcher they club with
    /// its back end.
    pub const BASH_CLIPS: [u32; 4] = [0xfe47_371e, 0xdb08_bd63, 0x0efe_2b64, 0xf997_6b4d];
    /// The event a throw lets go of the grenade at.
    const RELEASE: &str = "releasegrenade";
    /// The events a melee bash lands at, high and low.
    const BASH: &str = "meleebash";
    const BASH_LOW: &str = "MeleeBashLow";

    /// The clips by name and by their events; a hero's not of another
    /// hero's own group ([`Rig::foreign`]: Jennifer's run and jumps are
    /// hers), so that the next listed, everyone's, is taken.
    pub fn find(rig: &Rig) -> Self {
        let find = |hash| {
            rig.clips
                .iter()
                .position(|c| c.hash == hash && !rig.foreign.contains(&hash))
        };
        let first = |hashes: &[u32]| hashes.iter().find_map(|&h| find(h));
        let throw = Self::THROW.iter().find_map(|&h| {
            let i = find(h)?;
            let release = rig.clips[i]
                .clip
                .events
                .iter()
                .find(|e| e.name.eq_ignore_ascii_case(Self::RELEASE))
                .map(|e| e.time)?;
            Some((i, release))
        });
        let ready = find(Self::READY);
        // Of the clips carrying `event`, the one starting nearest the rifle
        // held ready, and when in it the event comes.
        let by_event = |event: &str| {
            Self::carrying(rig, event, ready)
                .first()
                .map(|&(i, at, _)| (i, at))
        };
        Self {
            aim: find(Self::AIM),
            ready,
            run_low: first(&Self::RUN_LOW),
            sprint: None,
            jump: find(Self::JUMP),
            takeoff: first(&Self::TAKEOFF),
            takeoff_still: first(&Self::TAKEOFF_STILL),
            land: first(&Self::LAND),
            land_still: first(&Self::LAND_STILL),
            throw,
            die: find(Self::DIE),
            bash: Self::BASH_CLIPS
                .iter()
                .find_map(|&h| {
                    let i = find(h)?;
                    let blow = rig.clips[i]
                        .clip
                        .events
                        .iter()
                        .find(|e| e.name.eq_ignore_ascii_case(Self::BASH))?;
                    Some((i, blow.time))
                })
                .or_else(|| by_event(Self::BASH)),
            bash_low: by_event(Self::BASH_LOW),
            swim: find(Self::SWIM),
            swim_out: find(Self::SWIM_OUT),
        }
    }

    /// The clips carrying `event` (not another hero's): each one's index,
    /// when in it the event comes, and how far its first pose is from the
    /// rifle held ready (clip `ready`), nearest first.
    fn carrying(rig: &Rig, event: &str, ready: Option<usize>) -> Vec<Carrying> {
        let mut list: Vec<Carrying> = rig
            .clips
            .iter()
            .enumerate()
            .filter(|(_, c)| !rig.foreign.contains(&c.hash))
            .filter_map(|(i, c)| {
                let at = c
                    .clip
                    .events
                    .iter()
                    .find(|e| e.name.eq_ignore_ascii_case(event))?
                    .time;
                let off = ready.map_or(0.0, |r| c.posture_distance_to(&rig.clips[r]));
                Some((i, at, off))
            })
            .collect();
        list.sort_by(|a, b| a.2.total_cmp(&b.2));
        list
    }

    /// The clips carrying a bash's events, high and low ([`carrying`](Self::carrying)).
    pub fn by_posture(rig: &Rig) -> [(&'static str, Vec<Carrying>); 2] {
        let ready = rig.clips.iter().position(|c| c.hash == Self::READY);
        [Self::BASH, Self::BASH_LOW].map(|e| (e, Self::carrying(rig, e, ready)))
    }

    /// Each slot's name ([`slots`](Self::slots)'s, in its order) and the
    /// clip of Jennifer's that keys it in the on-foot animation table:
    /// the first row listing it gives the slot's animation handle
    /// ([`m2_assets::animation_table::character_clip`]). Her clip of each
    /// slot as picked by name (the first listed), and for the low bash the
    /// one picked on her rig by its event (`0ae19446`, PC log of
    /// 2026-10-08). Measured on the PC (2026-10-09), the handles are:
    /// aim `9f4eb947`, ready `f2322e08` (her row there is a pistol one),
    /// run `b99ad3b3`, jump `cca02d6b` (anyone's, anything held),
    /// take-offs `2ae219a8` and `24af79c8`, landings `42f2ab41` and
    /// `d1f2ee32` (her rows there are rocket ones), throw `399f45f4`
    /// (with a grenade in use), bashes `fbc71879` and `7a9110c8`, die
    /// `f9d7a61c`.
    pub const TABLE_KEYS: [(&'static str, u32); 12] = [
        ("aim", Self::AIM),
        ("ready", Self::READY),
        ("run", Self::RUN_LOW[0]),
        ("jump", Self::JUMP),
        ("take-off", Self::TAKEOFF[0]),
        ("standing take-off", Self::TAKEOFF_STILL[0]),
        ("land", Self::LAND[0]),
        ("standing land", Self::LAND_STILL[0]),
        ("throw", Self::THROW[0]),
        ("bash", Self::BASH_CLIPS[0]),
        ("low bash", 0x0ae1_9446),
        ("die", Self::DIE),
    ];

    /// A hero's, by the on-foot animation table: each slot's clip as
    /// `theirs` gives the hero one for the slot's key
    /// ([`TABLE_KEYS`](Self::TABLE_KEYS)), where that clip is on the rig
    /// and not another hero's; throws and bashes only with their event.
    /// Returns the clips the table settled, which
    /// [`own_group`](Self::own_group) leaves be. A slot `theirs` gives
    /// nothing keeps what it was picked by.
    pub fn by_table(&mut self, rig: &Rig, theirs: &dyn Fn(u32) -> Option<u32>) -> HashSet<usize> {
        let index = |hash: u32| {
            rig.clips
                .iter()
                .position(|c| c.hash == hash && !rig.foreign.contains(&hash))
        };
        let event_at = |i: usize, name: &str| {
            rig.clips[i]
                .clip
                .events
                .iter()
                .find(|e| e.name.eq_ignore_ascii_case(name))
                .map(|e| e.time)
        };
        let key = |slot: &str| {
            Self::TABLE_KEYS
                .iter()
                .find(|k| k.0 == slot)
                .map_or(0, |k| k.1)
        };
        let mut settled = HashSet::new();
        let mut table = |slot: &mut Option<usize>, name: &str| {
            if let Some(i) = theirs(key(name)).and_then(index) {
                *slot = Some(i);
                settled.insert(i);
            }
        };
        table(&mut self.aim, "aim");
        table(&mut self.ready, "ready");
        table(&mut self.run_low, "run");
        table(&mut self.jump, "jump");
        table(&mut self.takeoff, "take-off");
        table(&mut self.takeoff_still, "standing take-off");
        table(&mut self.land, "land");
        table(&mut self.land_still, "standing land");
        table(&mut self.die, "die");
        let mut timed = |slot: &mut Option<(usize, f32)>, name: &str, event: &str| {
            if let Some(i) = theirs(key(name)).and_then(index)
                && let Some(at) = event_at(i, event)
            {
                *slot = Some((i, at));
                settled.insert(i);
            }
        };
        timed(&mut self.throw, "throw", Self::RELEASE);
        timed(&mut self.bash, "bash", Self::BASH);
        timed(&mut self.bash_low, "low bash", Self::BASH_LOW);
        settled
    }

    /// Each slot's clip, by the slot's name (for the logs).
    pub fn slots(&self) -> [(&'static str, Option<usize>); 12] {
        [
            ("aim", self.aim),
            ("ready", self.ready),
            ("run", self.run_low),
            ("jump", self.jump),
            ("take-off", self.takeoff),
            ("standing take-off", self.takeoff_still),
            ("land", self.land),
            ("standing land", self.land_still),
            ("throw", self.throw.map(|t| t.0)),
            ("bash", self.bash.map(|t| t.0)),
            ("low bash", self.bash_low.map(|t| t.0)),
            ("die", self.die),
        ]
    }

    /// A hero's: where a clip picked by name is not of the hero's own
    /// group ([`Rig::own`]) nor `settled` by the on-foot table
    /// ([`by_table`](Self::by_table)) and the group has clips of that
    /// kind, the group's (heroes round 02: each group holds its hero's
    /// runs, jumps, landings, throws and low bashes, told apart by their
    /// events and root speeds); of several, the one starting nearest the
    /// rifle held ready. The group holds runs and jumps that are not the
    /// rifle's too, so a run must go forward standing (as the gaits are
    /// chosen, [`m2_engine::Locomotion`]) with the hands kept together as
    /// the two-handed run picked by name keeps them, and a jump on the run
    /// must go forward. The high bash stays the butt smash picked from
    /// pictures (the groups' `meleebash` clips are not known to be one).
    pub fn own_group(mut self, rig: &Rig, settled: &HashSet<usize>) -> Self {
        if rig.own.is_empty() {
            return self;
        }
        let own = |i: Option<usize>| {
            i.is_some_and(|i| settled.contains(&i) || rig.own.contains(&rig.clips[i].hash))
        };
        let has = |c: &BoundClip, word: &str| {
            c.clip
                .events
                .iter()
                .any(|e| e.name.to_ascii_lowercase().contains(word))
        };
        // A short clip of the air alone carries the bare `jump` event.
        let air = |c: &BoundClip| {
            c.clip
                .events
                .iter()
                .any(|e| e.name.eq_ignore_ascii_case("jump"))
        };
        let candidates = |keep: &dyn Fn(&BoundClip) -> bool| -> Vec<usize> {
            rig.clips
                .iter()
                .enumerate()
                .filter(|(_, c)| rig.own.contains(&c.hash) && keep(c))
                .map(|(i, _)| i)
                .collect()
        };
        // Of several, the one starting nearest the rifle held ready (as
        // the low bash is picked).
        let ready = self.ready;
        let nearest = |list: Vec<usize>| -> Option<usize> {
            list.into_iter().min_by(|&a, &b| {
                let d = |i: usize| {
                    ready.map_or(0.0, |r| rig.clips[i].posture_distance_to(&rig.clips[r]))
                };
                d(a).total_cmp(&d(b))
            })
        };
        let swap = |slot: &mut Option<usize>, keep: &dyn Fn(&BoundClip) -> bool| {
            if !own(*slot)
                && let Some(i) = nearest(candidates(keep))
            {
                *slot = Some(i);
            }
        };
        // Forward and standing, as `Locomotion` takes its gaits.
        let forward = rig.locomotion.as_ref().map(|l| l.forward);
        let ahead = |c: &BoundClip| forward.is_some_and(|f| c.info.direction.dot(f) > 0.95);
        let standing = |c: &BoundClip| c.info.hips > 0.85;
        let spread = |c: &BoundClip| hand_spread(rig, c);
        // Two-handed: nearer the spread of the run picked by name (the
        // rifle low across the body in both hands) than that of the run
        // gait (the arms free). Without both, or with the gait's hands no
        // further apart, no run is swapped.
        let runs = (
            self.run_low.and_then(|i| spread(&rig.clips[i])),
            rig.gait_clip(Gait::Run).and_then(|i| spread(&rig.clips[i])),
        );
        let two_handed = |c: &BoundClip| match (runs, spread(c)) {
            ((Some(both), Some(free)), Some(s)) if both < free => s < (both + free) * 0.5,
            _ => false,
        };
        let running = |c: &BoundClip| has(c, "footstep_run") && (4.0..6.0).contains(&c.info.speed);
        let listed = if own(self.run_low) {
            Vec::new()
        } else {
            candidates(&running)
        };
        for i in listed {
            let c = &rig.clips[i];
            eprintln!(
                "hero clips: own run {:08x}: {:.2} m/s, {}, {}, hands {} apart for the hips (two-handed run {}, run gait {}): {}",
                c.hash,
                c.info.speed,
                if ahead(c) { "forward" } else { "not forward" },
                if standing(c) {
                    "standing"
                } else {
                    "not standing"
                },
                spread(c).map_or("?".to_owned(), |s| format!("{s:.2}")),
                runs.0.map_or("?".to_owned(), |s| format!("{s:.2}")),
                runs.1.map_or("?".to_owned(), |s| format!("{s:.2}")),
                if ahead(c) && standing(c) && two_handed(c) {
                    "the rifle's"
                } else {
                    "not the rifle's"
                },
            );
        }
        swap(&mut self.run_low, &|c| {
            running(c) && ahead(c) && standing(c) && two_handed(c)
        });
        swap(&mut self.takeoff, &|c| {
            has(c, "jumptakeoff") && c.info.speed >= 1.0 && ahead(c)
        });
        swap(&mut self.takeoff_still, &|c| {
            has(c, "jumptakeoff") && c.info.speed < 0.5
        });
        swap(&mut self.land, &|c| {
            has(c, "jumpland")
                && !air(c)
                && c.info.speed >= 3.0
                && c.info.duration >= 0.3
                && ahead(c)
        });
        swap(&mut self.land_still, &|c| {
            has(c, "jumpland") && !air(c) && c.info.speed < 0.5 && c.info.duration <= 1.0
        });
        // Throws and bashes keep when in them the grenade leaves or the
        // blow lands.
        let event_at = |i: usize, name: &str| {
            rig.clips[i]
                .clip
                .events
                .iter()
                .find(|e| e.name.eq_ignore_ascii_case(name))
                .map(|e| e.time)
        };
        for (slot, name) in [
            (&mut self.throw, Self::RELEASE),
            (&mut self.bash_low, Self::BASH_LOW),
        ] {
            if own(slot.map(|(i, _)| i)) {
                continue;
            }
            if let Some(i) = nearest(candidates(&|c| {
                c.clip
                    .events
                    .iter()
                    .any(|e| e.name.eq_ignore_ascii_case(name))
            })) && let Some(at) = event_at(i, name)
            {
                *slot = Some((i, at));
            }
        }
        self
    }

    /// Every clip, for rebinding.
    pub fn all(&self) -> impl Iterator<Item = usize> {
        [self.aim, self.ready, self.run_low, self.jump, self.die]
            .into_iter()
            .flatten()
    }
}

#[derive(Debug)]
pub struct Npc {
    pub kind: usize,
    pub soldier: bool,
    pub character: Character,
    pub facing: f32,
    pub rig: Rig,
    /// False until there is ground under them to stand on.
    pub landed: bool,
    /// The layer entity that placed them (soldiers).
    pub spawn: Option<u32>,
    /// The layer that placed them (a state layer or a static sub-layer):
    /// they go when it does.
    pub layer: Option<u32>,
    /// The faction they are of ([`faction_of`]), if an army's.
    pub faction: Option<&'static str>,
    /// She hurt them: they fight her while her pursuit lasts.
    pub provoked: bool,
    /// Their death was taken as an infraction already.
    pub reported_dead: bool,
    /// Hurt since the last look ([`People::pursue`]).
    pub hurt_by_her: bool,
    /// A story character (placed by a layer, not fighting: `Carmona`):
    /// the scripts', they stay where they are put and never run.
    pub held: bool,
    /// The gun their template hands them (`None`: the AK47's "Assault
    /// Rifle" stands in): what they draw, and how fast and hard it fires.
    pub gun: Option<HandGun>,
    pub hp: f32,
    /// The health they started with (their template's for a soldier, 100
    /// for a civilian): the HUD's target arc shows a share of it.
    pub most_hp: f32,
    pub alive: bool,
    /// A soldier turned on the player.
    pub alert: bool,
    /// Set by a tick when a soldier fires; the fight takes the shot.
    pub fires: bool,
    /// Seconds before a vehicle can hurt them again ([`RAM_GAP`]).
    pub rammed: f32,
    cooldown: f32,
    /// How their template fights (a plain VZ soldier's when unknown).
    pub ai: AiProfile,
    /// In a fight: where they last saw her, and for how long they haven't.
    last_seen: Option<Vec3>,
    unseen: f32,
    /// Seconds to the next look about.
    look: f32,
    /// A step about in a fight: where to and for how much longer; and the
    /// seconds to the next.
    step: Option<(Vec3, f32)>,
    step_wait: f32,
    /// Set by a tick when a soldier throws a grenade, at where; the
    /// fight lobs it.
    pub throws: Option<Vec3>,
    /// Their look: per node of their model, hidden (the heads, bodies
    /// and kit their mixer didn't pick, [`People::dress`]); empty for the
    /// model's own.
    pub outfit: Vec<bool>,
    grenade_wait: f32,
    /// The smarter soldiers' settings (off: the original's).
    pub smart: SmarterAi,
    /// Saw her this tick for the first time in the fight (smarter: he
    /// calls it out).
    spotted: bool,
    /// Smarter: running to cover, or down in it, for how much longer;
    /// and the seconds since he was last hit.
    cover: Option<(Vec3, f32)>,
    hit: bool,
    /// In a chase, a way round what stopped him: which way and for how
    /// much longer; and the side (1 or -1, 0 for none yet) he went round.
    detour: Option<(Vec3, f32)>,
    detour_side: f32,
    /// Rounds left in the burst he is firing, and seconds since his last.
    burst: u32,
    since_shot: f32,
    /// Running from gunfire: where to, and for how much longer.
    flee: Option<(Vec3, f32)>,
    /// Put out of a vehicle's seat ([`People::put_out`]): no spawn keeps
    /// them, so they go once out of range.
    put_out: bool,
    brain: Brain,
    /// The aim and fall clips in this person's rig.
    clips: FightClips,
    /// The right hand's node, where the rifle is held, and its attach
    /// point.
    pub hand: Option<usize>,
    pub attach: Option<usize>,
    /// A number of this person's, fixed where they came in: picks their
    /// variant of a gun's shot ([`m2_engine::gun_sounds::GunCues::shot_by`];
    /// invented).
    pub shooter: u32,
    /// Seconds since they died.
    dead_for: f32,
    /// Their body gone limp, once dead.
    pub ragdoll: Option<RagdollId>,
    /// A shove not yet given to their body (m/s): they went limp, or
    /// were hit again after.
    fling: Option<Vec3>,
    /// Has shouted since they were last calm.
    shouted: bool,
    /// Killed since [`People::take_deaths`] last looked.
    died: bool,
    /// Turning done standing still lately, radians, fading by half every
    /// [`SPIN_FADE`] seconds (for checking: someone spinning on the spot).
    spun: f32,
    /// On a route the scripts gave: how near its next point they have
    /// been, metres, and the seconds since they last got nearer.
    route_near: f32,
    route_stall: f32,
    /// Got to the end of a route the scripts gave (for checking).
    reached: bool,
    /// Got there, not yet told the scripts ([`People::take_route_ends`]).
    ended: bool,
    /// On such a route, going over or through what stopped them to its
    /// next point ([`ROUTE_STALL`]).
    through: bool,
}

/// What a person shouts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shout {
    /// A soldier turning on the player.
    Alert,
    /// A civilian running from gunfire.
    Flee,
}

/// A person's number from where they came in and which way they faced:
/// the bits mixed (FNV-1a over the three words), so people a step apart
/// differ.
fn shooter_number(at: Vec3, facing: f32) -> u32 {
    [at.x, at.z, facing]
        .iter()
        .flat_map(|v| v.to_bits().to_le_bytes())
        .fold(0x811c_9dc5_u32, |h, b| {
            (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
        })
}

impl Npc {
    pub(crate) fn new(
        kind: usize,
        rig: &Rig,
        at: Vec3,
        facing: f32,
        brain: Brain,
        spawn: Option<u32>,
    ) -> Self {
        let mut rig = rig.clone();
        rig.set_gait(Gait::Idle);
        let clips = FightClips::find(&rig);
        let hand = rig.rifle_hand();
        Self {
            clips,
            hand,
            attach: crate::combat::right_attach(&rig.skeleton),
            shooter: shooter_number(at, facing),
            dead_for: 0.0,
            ragdoll: None,
            fling: None,
            shouted: false,
            died: false,
            kind,
            soldier: spawn.is_some(),
            character: Character::new(at),
            facing,
            rig,
            landed: false,
            spawn,
            layer: None,
            held: false,
            faction: None,
            provoked: false,
            hurt_by_her: false,
            reported_dead: false,
            gun: None,
            hp: 100.0,
            most_hp: 100.0,
            alive: true,
            alert: false,
            fires: false,
            rammed: 0.0,
            cooldown: 0.0,
            ai: AiProfile::default(),
            last_seen: None,
            unseen: 0.0,
            look: 0.0,
            step: None,
            step_wait: 0.0,
            throws: None,
            outfit: Vec::new(),
            grenade_wait: 0.0,
            smart: SmarterAi::default(),
            spotted: false,
            cover: None,
            hit: false,
            detour: None,
            detour_side: 0.0,
            burst: 0,
            since_shot: f32::INFINITY,
            flee: None,
            put_out: false,
            brain,
            spun: 0.0,
            route_near: f32::INFINITY,
            route_stall: 0.0,
            reached: false,
            ended: false,
            through: false,
        }
    }

    /// Takes `damage`; at no health left, falls: true if it killed.
    /// `push` (m/s) shoves the body if it kills or is dead already.
    pub fn hurt(&mut self, damage: f32, push: Vec3) -> bool {
        let clips = self.clips;
        if !self.alive {
            self.fling = Some(self.fling.unwrap_or_default() + push);
            return false;
        }
        self.hp -= damage;
        self.hurt_by_her = true;
        self.alert = self.soldier;
        self.hit = true;
        if self.hp <= 0.0 {
            self.alive = false;
            self.died = true;
            self.alert = false;
            self.flee = None;
            self.rig.animator.set_upper(None);
            self.rig.two_handed = false;
            self.fling = Some(push);
            if let Some(die) = clips.die {
                self.rig.animator.play_once(die);
            }
        }
        !self.alive
    }

    /// Runs from `danger` (those who don't fight, but story characters
    /// and armies' soldiers standing down).
    /// Someone already running, and not towards it, keeps going the way
    /// they are.
    pub fn panic(&mut self, danger: Vec3) {
        if !self.alive || self.soldier || self.held || self.faction.is_some() {
            return;
        }
        let at = self.character.position;
        let away = (at - danger).with_y(0.0).normalize_or(Vec3::X);
        if let Some((target, left)) = &mut self.flee {
            let going = (*target - at).with_y(0.0).normalize_or_zero();
            if going.dot(away) > -0.2 {
                *left = left.max(FLEE_TIME * 0.5);
                return;
            }
        }
        self.flee = Some((at + away * FLEE * FLEE_TIME, FLEE_TIME));
    }

    /// Moves them at `wanted` (m/s) for `dt`, but not out into water (the
    /// sea, or the game's water heights: the villa's pool, rivers) deeper
    /// than [`WET`]: they stop at the water's edge, as at a wall (invented:
    /// nobody in the alpha swims; playtest 8).
    fn move_dry(&mut self, physics: &Physics, wanted: Vec3, dt: f32) {
        let before = self.character.position;
        physics.move_character(&mut self.character, wanted, None, dt);
        let now = self.character.position;
        // In it deeper than a wade, or over ground that is (stepping off a
        // dock or a bank): back where they were, unless they were in it
        // already (then they may walk out). Well above the water (no ground
        // that deep below), nothing to stop.
        let deep = |at: Vec3| {
            let Some(water) = physics.water_at(at.x, at.z) else {
                return false;
            };
            at.y < water - WET
                || (at.y < water + SEA_LOOK
                    && physics
                        .cast_ray(at + Vec3::Y, -Vec3::Y, SEA_LOOK + WET + 1.0)
                        .is_some_and(|(d, _)| at.y + 1.0 - d < water - WET))
        };
        if deep(now) && !deep(before) {
            self.character.position = before;
            self.character.velocity = Vec3::ZERO;
        }
    }

    /// Chest height, where shots aim.
    pub fn chest(&self) -> Vec3 {
        self.character.position + Vec3::Y * 1.3
    }

    fn turn_to(&mut self, target: f32, dt: f32) -> f32 {
        let mut turn = (target - self.facing).rem_euclid(std::f32::consts::TAU);
        if turn > std::f32::consts::PI {
            turn -= std::f32::consts::TAU;
        }
        self.facing += turn.clamp(-TURN_RATE * dt, TURN_RATE * dt);
        turn.abs()
    }

    /// Whether he sees `target` from his head: within `SIGHT` times his
    /// `Vision`, inside his `VisionFOV` (unless `any_way`, as in a fight,
    /// where he keeps turning to her), and nothing static between (a
    /// little short of her, so the car she sits in doesn't hide her).
    pub fn sees(&self, physics: &Physics, target: Vec3, any_way: bool) -> bool {
        let eye = self.character.position + Vec3::Y * EYE;
        let to = target - eye;
        let distance = to.length();
        if distance > SIGHT * self.ai.vision || distance < 1e-3 {
            return false;
        }
        if !any_way {
            let ahead = Vec2::new(self.facing.sin(), self.facing.cos());
            let flat = Vec2::new(to.x, to.z).normalize_or_zero();
            let half = (self.ai.vision_fov * 0.5).to_radians();
            if ahead.dot(flat) < half.cos() {
                return false;
            }
        }
        let clear = (distance - 2.5).max(0.5);
        physics.cast_ray(eye, to / distance, clear).is_none()
    }

    /// A soldier's tick in a fight with the player at `threat`. While he
    /// sees her within his gun's `FireRange` he turns to her, aims and
    /// fires in its bursts (no nearer than its `MinFireRange`), and
    /// every few seconds steps aside or (as `Offensive` has it) closer;
    /// otherwise he runs to where he last saw her (`PursueSpeed`). False
    /// when the fight is off: she is out of his sight's reach, or he has
    /// hunted too long without seeing her, or he is a `Pacifist`.
    fn fight(&mut self, physics: &Physics, threat: Vec3, rng: &mut Rng, dt: f32) -> bool {
        let clips = self.clips;
        let at = self.character.position;
        let to = threat - at;
        let distance = to.with_y(0.0).length();
        if self.ai.flags.pacifist || distance > SIGHT * self.ai.vision * 1.25 {
            return false;
        }
        self.look -= dt;
        if self.look <= 0.0 || self.last_seen.is_none() {
            self.look = LOOK_EVERY;
            if self.sees(physics, threat, true) {
                self.spotted |= self.unseen > LOOK_EVERY * 1.5 || self.last_seen.is_none();
                self.last_seen = Some(threat);
                self.unseen = 0.0;
            }
        }
        let seen = self.unseen < LOOK_EVERY * 1.5 && self.last_seen.is_some();
        self.unseen += dt;
        // Heard or hit without seeing her: he goes to the noise.
        let goal = self.last_seen.unwrap_or(threat);
        if self.unseen > GIVE_UP {
            return false;
        }
        let smart = self.smart;
        let mut wanted = Vec3::ZERO;
        // Grenades at her hiding place: she has been out of his sight a
        // while, near enough.
        self.grenade_wait -= dt;
        let (after, gap) = if smart.enabled {
            (smart.grenade_after, smart.grenade_gap)
        } else {
            (GRENADE_AFTER, GRENADE_GAP)
        };
        let reach = (goal - at).with_y(0.0).length();
        if self.ai.grenades
            && !self.ai.flags.no_grenades
            && self.last_seen.is_some()
            && self.unseen > after
            && self.grenade_wait <= 0.0
            // Hiding there, not driving off.
            && threat.distance(goal) < HIDING
            && (GRENADE_REACH.0..GRENADE_REACH.1).contains(&reach)
        {
            self.throws = Some(goal);
            self.grenade_wait = gap;
        }
        // Smarter: hit, he runs for cover and stays down a while.
        if smart.enabled && smart.cover && std::mem::take(&mut self.hit) && self.cover.is_none() {
            self.cover = self
                .find_cover(physics, threat)
                .map(|spot| (spot, smart.cover_time));
        }
        let mut hiding = false;
        if let Some((spot, left)) = &mut self.cover {
            let flat = (*spot - at).with_y(0.0);
            if flat.length() > 0.6 {
                wanted = flat.normalize() * SPRINT * self.ai.sprint_speed;
            } else {
                *left -= dt;
            }
            hiding = true;
            if *left <= 0.0 {
                self.cover = None;
            }
        }
        if hiding {
            self.step = None;
        } else if !seen || distance > self.fire_hint().range {
            self.step = None;
            let mut toward = (goal - at).with_y(0.0);
            // Where he last saw her, and she isn't there: on towards the
            // fight, where she is (invented: he hears it).
            if toward.length() <= CLOSE_ENOUGH {
                toward = (threat - at).with_y(0.0);
            }
            // Going round what stopped him, until the way on is open.
            if let Some((way, left)) = &mut self.detour {
                *left -= dt;
                let waist = at + Vec3::Y * 0.9;
                let open = physics
                    .cast_ray(waist, toward.normalize_or(Vec3::Z), DETOUR_OPEN)
                    .is_none();
                // Not given up the moment the way looks open: the thin
                // look misses what is low, and he would turn back and forth
                // between the wall and the way round (playtest 8).
                if *left <= 0.0 || (open && *left < DETOUR_TIME - DETOUR_MIN) {
                    self.detour = None;
                } else {
                    wanted = *way * SPRINT * self.ai.pursue_speed;
                }
            }
            if self.detour.is_none() && toward.length() > CLOSE_ENOUGH {
                wanted = toward.normalize() * SPRINT * self.ai.pursue_speed;
            }
        } else {
            self.detour = None;
            self.step_wait -= dt;
            if self.step.is_none() && self.step_wait <= 0.0 {
                self.step_wait = STEP_GAP.0 + rng.range(0.0, STEP_GAP.1);
                let toward = to.with_y(0.0).normalize_or(Vec3::Z);
                let side = Vec3::new(toward.z, 0.0, -toward.x) * rng.range(-1.0, 1.0).signum();
                let push = distance > PUSH_TO && rng.range(0.0, 1.0) < self.ai.offensive;
                let way = if push { toward } else { side };
                let mut length = rng.range(STEP.0, STEP.1);
                let mut target = at + way * length;
                if smart.enabled && smart.flank {
                    // His own side of her, by his spawn: five ways round
                    // from where he stands, 0 and +-40, +-80 degrees.
                    let lane = self.spawn.unwrap_or(0).wrapping_mul(2_654_435_761) % 5;
                    let angle = (lane as f32 - 2.0) * 40f32.to_radians();
                    let from_her = Quat::from_rotation_y(angle) * -toward;
                    let post = threat.with_y(at.y) + from_her * smart.flank_distance;
                    let flat = (post - at).with_y(0.0);
                    if flat.length() > 1.0 {
                        length = flat.length().min(STEP.1 * 1.5);
                        target = at + flat.normalize() * length;
                    }
                }
                self.step = Some((target, length / (SPRINT * self.ai.combat_speed)));
            }
            if let Some((target, left)) = &mut self.step {
                *left -= dt;
                let flat = (*target - at).with_y(0.0);
                if *left <= 0.0 || flat.length() < 0.5 {
                    self.step = None;
                } else {
                    wanted = flat.normalize() * SPRINT * self.ai.combat_speed;
                }
            }
        }
        self.move_dry(physics, wanted, dt);
        let moved = Vec2::new(self.character.velocity.x, self.character.velocity.z).length();
        if wanted != Vec3::ZERO && moved < 0.2 {
            // Walked into something: stop stepping; a chase goes round it
            // (along it the clearer way, or back if it is a corner) for a
            // while, then on.
            self.step = None;
            let chasing = !hiding && (!seen || distance > self.fire_hint().range);
            if chasing {
                let waist = self.character.position + Vec3::Y * 0.9;
                let clear = |d: Vec3| physics.cast_ray(waist, d, CLEAR).map_or(CLEAR, |(t, _)| t);
                let way = match self.detour.take() {
                    // The way round is blocked too: the other way.
                    Some(_) => {
                        self.detour_side = if self.detour_side > 0.0 { -1.0 } else { 1.0 };
                        let ahead = (goal - at).with_y(0.0).normalize_or(Vec3::Z);
                        Vec3::new(ahead.z, 0.0, -ahead.x) * self.detour_side
                    }
                    None => {
                        let ahead = wanted.with_y(0.0).normalize_or(Vec3::Z);
                        let side = Vec3::new(ahead.z, 0.0, -ahead.x);
                        let (l, r) = (clear(side), clear(-side));
                        // The side he went round last, while it is open;
                        // else the clearer; back if it is a corner.
                        if l.max(r) < 1.0 {
                            -ahead
                        } else {
                            if self.detour_side == 0.0 || clear(side * self.detour_side) < 1.0 {
                                self.detour_side = if l >= r { 1.0 } else { -1.0 };
                            }
                            side * self.detour_side
                        }
                    }
                };
                self.detour = Some((way, DETOUR_TIME));
            }
        }
        let seen = seen && !hiding;
        let face = if seen || wanted == Vec3::ZERO {
            // Straight above or below him (a floor up), which way she is
            // is noise: he keeps facing as he was.
            if distance < 0.5 {
                self.facing
            } else {
                to.x.atan2(to.z)
            }
        } else {
            wanted.x.atan2(wanted.z)
        };
        let off = self.turn_to(face, dt);
        let gait = if moved < 0.3 {
            Gait::Idle
        } else if moved > 2.5 {
            Gait::Run
        } else {
            Gait::Walk
        };
        // He aims only at her before him: the aim turns his whole drawn body
        // onto her, and with her behind him that turn wrapped between
        // +180 and -180 degrees tick by tick, so soldiers she drove round
        // stood flipping about (playtest 8, his clip at 14:00). Behind him,
        // he turns round with the rifle ready first.
        let ahead = seen && off < AIM_ARC;
        let carry = if ahead { Carry::Aim } else { Carry::Ready };
        self.rig.hold_rifle(gait, &clips, carry);
        if ahead {
            self.rig.aim_along(self.facing, Some(threat - self.chest()));
        }
        self.cooldown -= dt;
        let hint = self.fire_hint();
        if seen
            && self.cooldown <= 0.0
            && off < FIRE_ARC
            && distance < hint.range
            && distance >= hint.min_range
        {
            self.fires = true;
            self.since_shot = 0.0;
            let close = distance < FireHint::CLOSE;
            if self.burst == 0 {
                self.burst = hint.rounds(self.burst_gap(), close);
            }
            self.burst -= 1;
            self.cooldown = self.burst_gap();
            if self.burst == 0 {
                let pause = hint.burst(close).1;
                self.cooldown += pause * rng.range(1.0 - PAUSE_JITTER, 1.0 + PAUSE_JITTER);
            }
        }
        let rate = match self.rig.gait_speed(gait) {
            Some(clip) if gait != Gait::Idle && clip > 0.1 => (moved / clip).clamp(0.6, 1.4),
            _ => 1.0,
        };
        self.rig.advance(dt * rate);
        true
    }

    /// Seconds between the rounds of a burst: his gun's `RateOfFire`,
    /// else the "Assault Rifle"'s ([`BURST_GAP`]).
    fn burst_gap(&self) -> f32 {
        self.gun
            .filter(|g| g.rate_of_fire > 0.0)
            .map_or(BURST_GAP, |g| 60.0 / g.rate_of_fire)
    }

    /// How he fires his gun: its `WeaponHint`, else the "Assault
    /// Rifle"'s ([`STAND_IN`]).
    fn fire_hint(&self) -> FireHint {
        self.gun.and_then(|g| g.hint).unwrap_or(STAND_IN)
    }

    /// His `Accuracy`, as the smarter soldiers have it when on.
    pub fn accuracy(&self) -> f32 {
        if self.smart.enabled {
            1.0 - (1.0 - self.ai.accuracy) / self.smart.accuracy_scale.max(0.1)
        } else {
            self.ai.accuracy
        }
    }

    /// In a fight, what he is at (for checking): shooting, chasing,
    /// stepping about, taking cover.
    pub fn fight_state(&self) -> Option<&'static str> {
        if !self.alive || !self.alert {
            return None;
        }
        Some(if self.cover.is_some() {
            "in cover"
        } else if self.unseen > LOOK_EVERY * 1.5 {
            "chasing"
        } else if self.step.is_some() {
            "stepping"
        } else {
            "shooting"
        })
    }

    /// Smarter: the nearest spot within `cover_search` that `threat`
    /// can't see (from her chest to his head), tried round him.
    fn find_cover(&self, physics: &Physics, threat: Vec3) -> Option<Vec3> {
        let at = self.character.position;
        let away = (at - threat).with_y(0.0).normalize_or(Vec3::Z);
        let mut best: Option<(f32, Vec3)> = None;
        for ring in [0.4, 0.7, 1.0] {
            for k in 0..12 {
                let turn = Quat::from_rotation_y(k as f32 * std::f32::consts::TAU / 12.0);
                let spot = at + turn * away * self.smart.cover_search * ring;
                // Onto the ground there.
                let top = Vec3::new(spot.x, at.y + 4.0, spot.z);
                let Some((down, _)) = physics.cast_ray(top, -Vec3::Y, 10.0) else {
                    continue;
                };
                let ground = top - Vec3::Y * down;
                let head = ground + Vec3::Y * EYE;
                let to = head - threat;
                let d = to.length();
                if d < 3.0 || physics.cast_ray(threat, to / d, d - 0.3).is_none() {
                    continue;
                }
                let cost = ground.distance(at);
                if best.is_none_or(|(c, _)| cost < c) {
                    best = Some((cost, ground));
                }
            }
            if best.is_some() {
                break;
            }
        }
        best.map(|(_, p)| p)
    }

    /// Calm again: forgets the fight.
    fn calm(&mut self) {
        self.alert = false;
        self.last_seen = None;
        self.unseen = 0.0;
        self.step = None;
        self.detour = None;
        self.detour_side = 0.0;
        self.cover = None;
        self.hit = false;
    }

    /// Told (smarter) where she is by a soldier who sees her.
    fn told(&mut self, threat: Vec3) {
        if !self.alive || !self.soldier || self.ai.flags.pacifist {
            return;
        }
        self.alert = true;
        if self.last_seen.is_none() || self.unseen > LOOK_EVERY * 1.5 {
            self.last_seen = Some(threat);
            self.unseen = LOOK_EVERY * 2.0;
        }
    }

    pub fn patrolling(&self) -> bool {
        matches!(self.brain, Brain::Patrol { .. })
    }

    /// What they are doing, in a word or two (for checking).
    pub fn doing(&self) -> &'static str {
        if !self.alive {
            return "dead";
        }
        if !self.landed {
            return "landing";
        }
        if self.alert {
            return "fighting";
        }
        if self.flee.is_some() {
            return "fleeing";
        }
        match &self.brain {
            Brain::Post { .. } => "at a post",
            Brain::Patrol { wait, .. } if *wait > 0.0 => "pausing on a route",
            Brain::Patrol { .. } => "patrolling",
            Brain::Wander { wait, .. } if *wait > 0.0 => "pausing",
            Brain::Wander { .. } => "strolling",
            Brain::Route { .. } => "running where the scripts sent them",
        }
    }

    /// Model to world, turned so the model faces `facing`. The dead
    /// whose model has no fall clip that fits topple over backwards about
    /// their feet instead.
    pub fn model_matrix(&self) -> Mat4 {
        let forward = self.rig.locomotion.as_ref().map_or(Vec2::Y, |l| l.forward);
        let turned = Mat4::from_translation(self.character.position)
            * Mat4::from_rotation_y(self.facing - forward.x.atan2(forward.y));
        if self.alive || self.clips.die.is_some() {
            return turned;
        }
        let t = (self.dead_for / TOPPLE).min(1.0);
        // Eased: slow to start, fast at the end, as a fall is.
        turned * Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2 * t * t)
    }

    fn update(&mut self, physics: &Physics, rng: &mut Rng, threat: Option<Vec3>, dt: f32) {
        if !self.landed {
            // Onto the ground (or the roof or floor of their post) under
            // where they were placed.
            let p = self.character.position;
            let from = Vec3::new(p.x, p.y + 2.0, p.z);
            let hit = physics
                .cast_ray(from, -Vec3::Y, 60.0)
                .map(|(d, _)| from.y - d)
                .or_else(|| {
                    physics
                        .cast_ray(Vec3::new(p.x, 3000.0, p.z), -Vec3::Y, 6000.0)
                        .map(|(d, _)| 3000.0 - d)
                });
            // Never onto the sea bed: placed on a dock or deck whose
            // collision isn't in yet, they wait for it (not drawn while
            // they do), and nobody stands under the water (playtest 8).
            let dry = |y: f32| physics.water_at(p.x, p.z).is_none_or(|w| y >= w - WET);
            if let Some(y) = hit.filter(|&y| dry(y)) {
                self.character.position.y = y + 0.05;
                self.landed = true;
            }
            return;
        }
        if !self.alive {
            self.dead_for += dt;
            self.rig.advance(dt);
            return;
        }
        self.since_shot += dt;
        // A soldier on watch who catches sight of her turns on her.
        if self.soldier && !self.alert && !self.ai.flags.pacifist {
            self.look -= dt;
            if self.look <= 0.0 {
                self.look = LOOK_EVERY;
                if let Some(t) = threat
                    && self.sees(physics, t, false)
                {
                    self.alert = true;
                    self.spotted = true;
                    self.last_seen = Some(t);
                }
            }
        }
        // Sent along a route by the scripts (their goals are `hiPri`): they
        // run it to its end first, and fight from there (inferred: the
        // villa's waves come in to the hall).
        let sent = matches!(self.brain, Brain::Route { .. });
        if self.alert && !sent {
            match threat {
                Some(t) if self.fight(physics, t, rng, dt) => return,
                _ => self.calm(),
            }
        }
        let at = self.character.position;
        let mut wanted = Vec3::ZERO;
        let mut face = None;
        let mut speed = WALK;
        if let Some((target, left)) = &mut self.flee {
            *left -= dt;
            let to = (*target - at).with_y(0.0);
            if *left <= 0.0 || to.length() < 1.0 {
                // Settle down where they ran to.
                self.brain = Brain::Wander {
                    home: at,
                    target: at,
                    wait: rng.range(PAUSE.0, PAUSE.1),
                };
                self.flee = None;
            } else {
                wanted = to.normalize() * FLEE;
                speed = FLEE;
            }
        }
        let mut arrived = false;
        let mut glide = None;
        match &mut self.brain {
            _ if self.flee.is_some() => {}
            Brain::Post { facing } => face = Some(*facing),
            Brain::Patrol {
                route,
                next,
                step,
                wait,
            } => {
                if *wait > 0.0 {
                    *wait -= dt;
                } else {
                    let to = route[*next] - at;
                    let flat = Vec3::new(to.x, 0.0, to.z);
                    if flat.length() < 0.6 {
                        let last = route.len() - 1;
                        let closed = route.len() > 2 && route[0].distance(route[last]) < 2.0;
                        let n = *next as isize + *step;
                        if n < 0 || n > last as isize {
                            if closed {
                                *next = if *step > 0 { 0 } else { last };
                            } else {
                                *step = -*step;
                                *next = (*next as isize + *step) as usize;
                                *wait = rng.range(PAUSE.0, PAUSE.1);
                            }
                        } else {
                            *next = n as usize;
                        }
                    } else {
                        wanted = flat.normalize() * speed;
                    }
                }
            }
            Brain::Wander { home, target, wait } => {
                if *wait > 0.0 {
                    *wait -= dt;
                } else {
                    let to = *target - at;
                    let flat = Vec3::new(to.x, 0.0, to.z);
                    if flat.length() < 0.8 {
                        *wait = rng.range(PAUSE.0, PAUSE.1);
                        *target = *home + rng.disc(WANDER);
                    } else {
                        wanted = flat.normalize() * speed;
                    }
                }
            }
            Brain::Route { route, next } => {
                let point = route[*next];
                let flat = (point - at).with_y(0.0);
                if flat.length() < 0.8 {
                    self.route_near = f32::INFINITY;
                    self.route_stall = 0.0;
                    // Over what stopped them: onto the floor under the
                    // point they made for.
                    if std::mem::take(&mut self.through) {
                        self.landed = false;
                    }
                    if *next + 1 < route.len() {
                        *next += 1;
                    } else {
                        arrived = true;
                    }
                } else {
                    if flat.length() < self.route_near - 0.1 {
                        self.route_near = flat.length();
                        self.route_stall = 0.0;
                    } else {
                        self.route_stall += dt;
                    }
                    // Getting no nearer the next point (a wall, a door, a
                    // ledge their path climbs, which the game's own
                    // vaults and climbs would take): they go on over or
                    // through it to the point (invented: the alpha has no
                    // climbing, nor the game's navigation).
                    if self.route_stall > ROUTE_STALL {
                        self.through = true;
                    }
                    if self.through {
                        glide = Some(point);
                        face = Some(flat.x.atan2(flat.z));
                    } else {
                        // At a run, as the scripts' `Haste` 1 asks.
                        wanted = flat.normalize() * FLEE;
                    }
                }
            }
        }
        if arrived {
            self.brain = Brain::Post {
                facing: self.facing,
            };
            self.reached = true;
            self.ended = true;
        }
        match glide {
            Some(point) => {
                let to = point - self.character.position;
                let d = to.length();
                let step = to / d.max(1e-3) * (FLEE * dt).min(d);
                self.character.position += step;
                self.character.velocity = step / dt;
            }
            None => self.move_dry(physics, wanted, dt),
        }
        // Walked into something: give up on that point (strolls) or wait
        // and try again (routes).
        let moved = Vec2::new(self.character.velocity.x, self.character.velocity.z).length();
        if wanted != Vec3::ZERO && moved < 0.2 {
            match (&mut self.flee, &mut self.brain) {
                // Ran into a wall: along it, the clearer way, or back the
                // way they came if it is a corner (not stood against it).
                (Some((target, left)), _) => {
                    let ahead = (*target - at).with_y(0.0).normalize_or(Vec3::X);
                    let side = Vec3::new(ahead.z, 0.0, -ahead.x);
                    let waist = self.character.position + Vec3::Y * 0.9;
                    let clear =
                        |d: Vec3| physics.cast_ray(waist, d, CLEAR).map_or(CLEAR, |(t, _)| t);
                    let (l, r) = (clear(side), clear(-side));
                    let way = match () {
                        _ if l.max(r) < 1.0 => -ahead,
                        _ if l >= r => side,
                        _ => -side,
                    };
                    *target = self.character.position + way * FLEE * *left;
                }
                // A new spot after a pause, not one every tick (which turned
                // them on the spot while blocked; playtest 8).
                (None, Brain::Wander { home, target, wait }) => {
                    *target = *home + rng.disc(WANDER);
                    *wait = 1.0;
                }
                (None, Brain::Patrol { wait, .. }) => *wait = 1.0,
                (None, Brain::Post { .. } | Brain::Route { .. }) => {}
            }
        }
        if wanted.length_squared() > 0.01 {
            face = Some(wanted.x.atan2(wanted.z));
        }
        if let Some(target) = face {
            self.turn_to(target, dt);
        }
        let gait = if moved < 0.3 {
            Gait::Idle
        } else if moved > 2.5 {
            Gait::Run
        } else {
            Gait::Walk
        };
        if self.soldier {
            let clips = self.clips;
            self.rig.hold_rifle(gait, &clips, Carry::Ready);
        } else {
            self.rig.set_gait(gait);
        }
        let rate = match self.rig.gait_speed(gait) {
            Some(clip) if gait != Gait::Idle && clip > 0.1 => (moved / clip).clamp(0.6, 1.4),
            _ => 1.0,
        };
        self.rig.advance(dt * rate);
    }
}

/// Everyone on the island but the player.
#[derive(Debug, Default)]
pub struct People {
    pub kinds: Vec<PersonKind>,
    pub npcs: Vec<Npc>,
    /// Soldiers (by spawn entity) killed: they stay dead.
    pub killed: HashSet<u32>,
    /// Spawns a mission wants soldiers at though their names don't say
    /// so (`vza_beachguard_1`): they fight, in their template's model (a
    /// plain VZ soldier's when it has none).
    pub mission_soldiers: HashSet<u32>,
    /// The layers in, by name hash (state layers and static sub-layers): a
    /// layer's people stand only while it is. `None` when that isn't
    /// known (all of them stand).
    pub state_layers: Option<HashSet<u32>>,
    /// Templates' own models to load: model and template name (taken by
    /// the scene).
    pub wanted: Vec<(u32, String)>,
    /// Models asked for so far, each by the template it is for: several
    /// templates share a model (the guerrillas' men and the GR
    /// Defenders), and each becomes a kind of its own.
    asked: HashSet<(u32, String)>,
    /// Templates' own models that won't do (they didn't load, or the
    /// clips don't fit their skeleton), by model and template.
    refused: HashSet<(u32, String)>,
    /// Whom spawns stand as, said once per kind and template (for
    /// checking against the data).
    told: HashSet<(usize, String)>,
    /// Static sub-layers said (in the log) not to be loaded where their
    /// people would stand.
    unloaded: HashSet<u32>,
    /// Where the scripts last put people, by spawn entity: there they
    /// stand, not where their layer has them.
    moved: HashMap<u32, Vec3>,
    /// Where the scripts last sent people ([`People::walk`]), by spawn
    /// entity.
    routes: HashMap<u32, Vec<Vec3>>,
    active: HashSet<u32>,
    /// Spawners sending from a list, by entity: their `SecondsPerCycle`
    /// (filled on first use).
    cycles: Option<HashMap<u32, f32>>,
    /// Spawners whose one fell, by entity: seconds before the next.
    pub refills: HashMap<u32, f32>,
    /// How many passers-by the traffic zone where she is asks for (its
    /// people count, [`m2_engine::population::DensityZone`]).
    pub walker_cap: usize,
    rng: Rng,
    /// The enhanced, smarter soldiers' settings (off: the original's).
    pub smarter: SmarterAi,
    /// What soldiers near her have been at (for checking).
    pub idle: IdleTally,
    /// What shouldn't happen to anyone (for checking).
    pub checks: Checks,
    /// Each person template's mixer profile, by name, lower case
    /// ([`m2_engine::world::Templates::mixer_profiles`]).
    pub mixer_profiles: HashMap<String, u32>,
    /// The looks spawned so far, each by kind and parts picked (for
    /// checking: each new one is said once).
    pub looks_seen: HashSet<(usize, Vec<usize>)>,
    /// The factions (the faction manager's abbreviations, `Vza`, `All`) the
    /// scripts have hostile to her, as they last said
    /// ([`People::set_hostile`]); `None` before they have: then only the
    /// Venezuelans fight.
    pub hostile: Option<HashSet<&'static str>>,
    /// Factions she provoked (hurt one of theirs): their soldiers fight her
    /// for this many seconds more ([`PURSUIT_SECONDS`]).
    pub pursuit: HashMap<&'static str, f32>,
    /// What she did to the factions' people since the mission last took
    /// them ([`People::take_infractions`]), for their reporters.
    pub infractions: Vec<Infraction>,
}

/// Soldier-seconds near her, by what the soldiers were at (for checking
/// soldiers who stand about in a fight): those on alert within
/// `FIRE_RANGE` of her shooting (a round within [`STILL_AFTER`]),
/// moving, or neither, with her in sight or not; and those not on alert
/// within [`UNAWARE_RANGE`] with her in sight.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct IdleTally {
    pub fighting: f32,
    pub shooting: f32,
    pub moving: f32,
    pub still_seen: f32,
    pub still_unseen: f32,
    pub unaware: f32,
}

/// A soldier who hasn't fired for this long, seconds, isn't shooting (his
/// longest pause between bursts is 1.7 s).
const STILL_AFTER: f32 = 3.0;
/// How near her a calm soldier who could see her counts, metres.
const UNAWARE_RANGE: f32 = 30.0;

impl IdleTally {
    /// The tally in a line.
    pub fn line(&self) -> String {
        format!(
            "soldier-seconds near her: {:.0} fighting ({:.0} shooting, {:.0} moving, {:.0} still with her in sight, {:.0} still without); {:.0} calm with her in sight",
            self.fighting,
            self.shooting,
            self.moving,
            self.still_seen,
            self.still_unseen,
            self.unaware
        )
    }

    fn count(&mut self, physics: &Physics, n: &Npc, threat: Vec3, dt: f32) {
        if !n.alive || !n.soldier || !n.landed {
            return;
        }
        let d = n.character.position.distance(threat);
        if n.alert && d < FIRE_RANGE {
            self.fighting += dt;
            let speed = Vec2::new(n.character.velocity.x, n.character.velocity.z).length();
            if n.since_shot < STILL_AFTER {
                self.shooting += dt;
            } else if speed > 0.3 {
                self.moving += dt;
            } else if n.sees(physics, threat, true) {
                self.still_seen += dt;
            } else {
                self.still_unseen += dt;
            }
        } else if !n.alert && d < UNAWARE_RANGE && n.sees(physics, threat, true) {
            self.unaware += dt;
        }
    }
}

/// Seconds over which turning on the spot fades by half, for
/// [`Checks`]: someone who turns a whole circle and more standing still
/// within a few seconds is spinning (one turn to face somewhere is half
/// a circle at most).
const SPIN_FADE: f32 = 2.0;
/// How deep in the sea someone's feet may be before they count as stood
/// in it, metres (a wade at the water's edge is fine).
const WET: f32 = 1.0;
/// How high above the sea someone looks down for deep water under them
/// before a step, metres.
const SEA_LOOK: f32 = 8.0;
/// Seconds without getting nearer the next point of a route the scripts
/// gave before a runner counts as stuck.
const STALL: f32 = 3.0;
/// Seconds a runner on such a route gets no nearer its next point before
/// going over or through what stops them (invented).
const ROUTE_STALL: f32 = 1.5;

/// What shouldn't happen to people, tallied (for checking, playtest 8):
/// spinning on the spot, standing in the sea, and runners the scripts
/// sent along a route not getting to its end. Each by the spawn entities
/// it happened to, and person-seconds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Checks {
    pub spinning: f32,
    pub spinners: HashSet<u32>,
    /// Aiming at her behind him, the drawn body flipping (person-seconds,
    /// and who).
    pub aim_behind: f32,
    pub aimed_behind: HashSet<u32>,
    pub wet: f32,
    pub wetted: HashSet<u32>,
    /// Runners sent along a route, those who got to its end, and those
    /// who stuck on the way (once each).
    pub sent: HashSet<u32>,
    pub arrived: HashSet<u32>,
    pub stuck: HashSet<u32>,
}

impl Checks {
    /// The tally in a line.
    pub fn line(&self) -> String {
        format!(
            "people checks: spinning {:.1} s ({} people), aiming behind {:.1} s ({} people), in water {:.1} s ({} people); runners {} sent, {} at their route's end, {} stuck on the way",
            self.spinning,
            self.spinners.len(),
            self.aim_behind,
            self.aimed_behind.len(),
            self.wet,
            self.wetted.len(),
            self.sent.len(),
            self.arrived.len(),
            self.stuck.len()
        )
    }

    /// Counts `n`'s tick: they faced `facing` before it, and the sea is at
    /// `sea`.
    fn count(&mut self, n: &mut Npc, facing: f32, sea: Option<f32>, dt: f32) {
        let fade = 0.5f32.powf(dt / SPIN_FADE);
        n.spun *= fade;
        if !n.alive || !n.landed {
            n.spun = 0.0;
            return;
        }
        let id = n.spawn.unwrap_or(u32::MAX);
        let speed = Vec2::new(n.character.velocity.x, n.character.velocity.z).length();
        if speed < 0.3 {
            let turn = (n.facing - facing + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
                - std::f32::consts::PI;
            n.spun += turn.abs();
        }
        // Aiming at her behind him (more than a right angle off the way he
        // faces): the aim pose turns his drawn body onto her, which flips
        // there (his clip at 14:00).
        if let Some(aim) = n.rig.aim {
            let ahead = n.rig.locomotion.as_ref().map_or(Vec2::Y, |l| l.forward);
            let off = (aim.x.atan2(aim.z) - ahead.x.atan2(ahead.y) + std::f32::consts::PI)
                .rem_euclid(std::f32::consts::TAU)
                - std::f32::consts::PI;
            if off.abs() > std::f32::consts::FRAC_PI_2 {
                self.aim_behind += dt;
                if self.aimed_behind.insert(id) {
                    eprintln!(
                        "people check: {id:08x} aims at her behind him at {:.1}",
                        n.character.position
                    );
                }
            }
        }
        if n.spun > std::f32::consts::TAU {
            self.spinning += dt;
            if self.spinners.insert(id) {
                eprintln!(
                    "people check: {id:08x} spins on the spot at {:.1} ({})",
                    n.character.position,
                    n.doing()
                );
            }
        }
        if sea.is_some_and(|s| n.character.position.y < s - WET) {
            self.wet += dt;
            if self.wetted.insert(id) {
                eprintln!(
                    "people check: {id:08x} stands in the sea at {:.1} ({})",
                    n.character.position,
                    n.doing()
                );
            }
        }
        if std::mem::take(&mut n.reached) && self.arrived.insert(id) {
            eprintln!(
                "people check: {id:08x} at its route's end, {:.1}",
                n.character.position
            );
        }
        if let Brain::Route { route, next } = &n.brain {
            self.sent.insert(id);
            let d = (route[*next] - n.character.position).with_y(0.0).length();
            if n.route_stall > STALL && self.stuck.insert(id) {
                eprintln!(
                    "people check: {id:08x} stuck on its route at {:.1}, {:.1} m short of point {} of {}",
                    n.character.position,
                    d,
                    next + 1,
                    route.len()
                );
            }
        }
    }
}

/// Who a spawn stands as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Who {
    /// A kind; whether they fight; sent out by a spawner's list.
    Kind {
        kind: usize,
        fights: bool,
        listed: bool,
    },
    /// Their model (or a spawner's pick's) is loading.
    Waiting,
    /// Nobody: a vehicle's spawn, a person of no model, another army's.
    Nobody,
}

/// Whether a person of template or entity name `name` is a Venezuelan
/// (`VZ Soldier`, `VZ Tank Crew`): they fight. For people in a
/// template's own model, where [`role_for`]'s vehicle words don't apply.
fn venezuelan(name: &str) -> bool {
    name.to_ascii_lowercase().starts_with("vz ")
}

/// The faction a person of template or entity name `name` is of, as the
/// faction manager (`MrxFactionManager`) abbreviates it: `Vza` (`VZ
/// Soldier`), `All` (`Allied Soldier`), `Chi` (`Chinese Soldier`), `Gur`
/// (`Guerilla Soldier`), `Oil` (`OC Soldier`), `Pir` (`Pirate ..`).
/// From samples: the faction manager's `sFactionTemplate`s (`Allied`,
/// `China`, `Guerilla`, `OC`, `Pirate`, `VZ`) and the layers' person
/// templates, which begin with them (inferred: a template's own faction
/// link isn't read).
pub fn faction_of(name: &str) -> Option<&'static str> {
    let n = name.to_ascii_lowercase();
    [
        ("vz ", "Vza"),
        ("allied ", "All"),
        ("chinese ", "Chi"),
        ("china ", "Chi"),
        ("guerilla ", "Gur"),
        ("guerrilla ", "Gur"),
        ("oc ", "Oil"),
        ("pirate ", "Pir"),
    ]
    .into_iter()
    .find_map(|(prefix, f)| n.starts_with(prefix).then_some(f))
}

/// Something she did to one of a faction's people, for its witnesses to
/// report ([`m2_game::report`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Infraction {
    /// The faction, as [`faction_of`] has it.
    pub faction: &'static str,
    /// `DamagePerson` or `DestroyPerson` (a [`m2_game::report::KINDS`] key).
    pub kind: &'static str,
    /// Where the victim was.
    pub at: Vec3,
    /// The victim's spawn entity, if a layer's.
    pub victim: Option<u32>,
}

/// The faction's label as the faction manager names it (`Object.HasLabel`
/// with its `sFactionTemplate`: `OC`, `Allied`) from [`faction_of`]'s
/// abbreviation.
pub fn faction_label(f: &str) -> &'static str {
    match f {
        "All" => "Allied",
        "Chi" => "China",
        "Gur" => "Guerilla",
        "Oil" => "OC",
        "Pir" => "Pirate",
        _ => "VZ",
    }
}

/// Seconds the soldiers of a faction she provoked fight her: the first of
/// the pursuit level times the faction manager sets
/// (`Pg.SetPursuitLevelTimes(120, 300)`, from the scripts; which level is
/// which is inferred).
pub const PURSUIT_SECONDS: f32 = 120.0;

/// Whether spawn `s` (a `target` of a mission's or not) is in its
/// template's own model but left out: of those who don't fight, a story
/// character (a one-word template, `Carmona`) and an army's soldier (one
/// with an `Ai` of a faction, [`faction_of`]) stand, not a passer-by
/// (`GR Defender` without one).
fn left_out(s: &SpawnPoint, target: bool) -> bool {
    let names = [s.template.as_deref(), Some(s.name.as_str())];
    let fights = target || names.iter().flatten().any(|n| venezuelan(n));
    let army = s.ai.is_some() && names.iter().flatten().any(|n| faction_of(n).is_some());
    s.model.is_some() && !fights && !army && s.template.as_deref().is_some_and(|t| t.contains(' '))
}

/// The role a name asks for when it is a Venezuelan's (`VZ Heavy ..`):
/// the catalogue's model for soldiers whose template has none of its own.
/// Spawns of vehicles, targets, buildings and other armies ask for none.
fn role_for(name: &str) -> Option<&'static str> {
    let n = name.to_ascii_lowercase();
    let other = [
        "amx30", "scorpion", "hind", "m151", "boat", "target", "_bld_", "heli", "tank", "truck",
    ];
    if other.iter().any(|w| n.contains(w)) || !n.starts_with("vz ") {
        return None;
    }
    // A "mook" is a plain VZ soldier (their `Health` of 75), not a death
    // squad's.
    Some(if n.contains("deathsquad") {
        "vz_hum_deathsquad"
    } else if n.contains("heavy") || n.contains("elite") {
        "vz_hum_soldierelite"
    } else {
        "vz_hum_soldier"
    })
}

/// Whether someone whose middle (0.9 m above their feet) is at `local`, in
/// a vehicle's space, is in the way of its hull `(lo, hi)`: the hull
/// padded by a person's half-width and half-height (invented).
pub fn in_ram(local: Vec3, (lo, hi): (Vec3, Vec3)) -> bool {
    let pad = Vec3::new(0.35, 0.9, 0.35);
    local.cmpge(lo - pad).all() && local.cmple(hi + pad).all()
}

impl People {
    /// The kind of the catalogue's model a name's role asks for
    /// ([`role_for`]), for soldiers whose template has no model of its own.
    pub fn kind_for(&self, name: &str) -> Option<usize> {
        let role = role_for(name)?;
        self.kinds.iter().position(|k| !k.named && k.role == role)
    }

    /// Whether `npcs[i]` is a woman (her model's role or template says
    /// so: `civ_hum_*female`).
    pub fn is_woman(&self, i: usize) -> bool {
        self.npcs
            .get(i)
            .and_then(|n| self.kinds.get(n.kind))
            .is_some_and(|k| k.role.to_ascii_lowercase().contains("female"))
    }

    /// The kind a vehicle's rider of template `template` is drawn as: its
    /// own model `model` when it has one (asked for the first time; `None`
    /// until it is in), else its name's role ([`kind_for`](Self::kind_for)).
    pub(crate) fn rider_kind(&mut self, template: &str, model: Option<u32>) -> Option<usize> {
        match model {
            Some(m) if !self.refused.contains(&(m, template.to_owned())) => self.own(template, m),
            _ => self.kind_for(template),
        }
    }

    /// The kind of template `template` in its own model `model`, once
    /// that is in; asks for the model the first time.
    fn own(&mut self, template: &str, model: u32) -> Option<usize> {
        if let Some(k) = self
            .kinds
            .iter()
            .position(|k| k.named && k.role == template)
        {
            return Some(k);
        }
        let key = (model, template.to_owned());
        if !self.refused.contains(&key) && self.asked.insert(key.clone()) {
            self.wanted.push(key);
        }
        None
    }

    /// Someone of the scripts' own, `guid` (template `template`, its own
    /// model `model`), stands at `at` facing `facing` as they have them:
    /// the HQ's starter out of her seat for her briefing. Brought in
    /// (in the looks `outfit`, if given) the first time; moved and turned
    /// after. They don't fight or run (a story character). False when
    /// their kind isn't in yet.
    pub(crate) fn stand_scripted(
        &mut self,
        guid: u32,
        template: &str,
        model: Option<u32>,
        outfit: Option<Vec<bool>>,
        at: Vec3,
        facing: f32,
    ) -> bool {
        if let Some(n) = self.npcs.iter_mut().find(|n| n.spawn == Some(guid)) {
            if n.character.position.distance(at) > 0.25 || (n.facing - facing).abs() > 0.01 {
                n.character.position = at;
                n.character.velocity = Vec3::ZERO;
                n.facing = facing;
                n.brain = Brain::Post { facing };
            }
            return true;
        }
        let Some(kind) = self.rider_kind(template, model) else {
            return false;
        };
        let mut n = Npc::new(
            kind,
            &self.kinds[kind].rig,
            at,
            facing,
            Brain::Post { facing },
            Some(guid),
        );
        n.outfit = outfit.unwrap_or_else(|| self.dress_as(kind, template));
        n.held = true;
        // Unarmed and not fighting (inferred: the scripts give the HQ's
        // starters no weapon; a soldier is drawn with the stand-in AK47).
        n.soldier = false;
        n.gun = None;
        eprintln!("people: {template} ({guid:#x}) stands at {at:.1}, as the scripts have them");
        self.npcs.push(n);
        true
    }

    /// The scripts' own `guid` ([`People::stand_scripted`]) is no longer
    /// on foot (back in a seat, or gone).
    pub(crate) fn unstand_scripted(&mut self, physics: &mut Physics, guid: u32) {
        self.npcs.retain(|n| {
            if n.spawn != Some(guid) {
                return true;
            }
            if let Some(doll) = n.ragdoll {
                physics.remove_ragdoll(doll);
            }
            false
        });
    }

    /// Whether soldiers of faction `f` fight her now: their faction is
    /// hostile to her (the scripts' attitudes; before they say, only the
    /// Venezuelans), or she provoked them a while ago ([`PURSUIT_SECONDS`]).
    pub fn fights_for(&self, f: &str) -> bool {
        let hostile = match &self.hostile {
            Some(set) => set.contains(f),
            None => f == "Vza",
        };
        hostile || self.pursuit.contains_key(f)
    }

    /// The factions hostile to her, as the scripts' attitudes now have
    /// them (asked of the faction manager once a second, `crate::mission`): their
    /// soldiers about fight her, the rest's stand down.
    pub fn set_hostile(&mut self, hostile: HashSet<&'static str>) {
        if self.hostile.as_ref() != Some(&hostile) {
            eprintln!("people: factions hostile to her now {hostile:?}");
            self.hostile = Some(hostile);
            self.refresh_sides();
        }
    }

    /// Faction soldiers fight her or stand down as their faction now
    /// does ([`fights_for`](Self::fights_for)), one she hurt fights on;
    /// the scripts' targets always fight.
    fn refresh_sides(&mut self) {
        let fights: Vec<bool> = self
            .npcs
            .iter()
            .map(|n| match n.faction {
                Some(f) => {
                    n.provoked
                        || self.fights_for(f)
                        || n.spawn.is_some_and(|s| self.mission_soldiers.contains(&s))
                }
                None => n.soldier,
            })
            .collect();
        for (n, fights) in self.npcs.iter_mut().zip(fights) {
            if n.soldier != fights {
                n.soldier = fights;
                if !fights {
                    n.alert = false;
                }
            }
        }
    }

    /// What she did to the factions' people since the last call.
    pub fn take_infractions(&mut self) -> Vec<Infraction> {
        std::mem::take(&mut self.infractions)
    }

    /// The pursuits: a faction member she hurt starts one for their
    /// faction ([`PURSUIT_SECONDS`]), and each runs down by `dt`.
    fn pursue(&mut self, dt: f32) {
        let mut started = Vec::new();
        for n in &mut self.npcs {
            let Some(f) = n.faction else { continue };
            let hurt = std::mem::take(&mut n.hurt_by_her);
            if hurt {
                // What a witness of theirs reports: hurt, or killed.
                let kind = if n.alive {
                    "DamagePerson"
                } else {
                    "DestroyPerson"
                };
                if n.alive || !n.reported_dead {
                    n.reported_dead |= !n.alive;
                    self.infractions.push(Infraction {
                        faction: f,
                        kind,
                        at: n.character.position,
                        victim: n.spawn,
                    });
                }
            }
            if hurt && !n.provoked {
                n.provoked = true;
                started.push(f);
            }
        }
        let before = self.pursuit.len();
        for f in started {
            if self.pursuit.insert(f, PURSUIT_SECONDS).is_none() {
                eprintln!("people: she provoked {f}: they fight her for {PURSUIT_SECONDS:.0} s");
            }
        }
        let mut ended = Vec::new();
        self.pursuit.retain(|f, left| {
            *left -= dt;
            if *left <= 0.0 {
                eprintln!("people: {f} give up the pursuit");
                ended.push(*f);
            }
            *left > 0.0
        });
        for n in &mut self.npcs {
            if n.faction.is_some_and(|f| ended.contains(&f)) {
                n.provoked = false;
            }
        }
        if self.pursuit.len() != before || self.npcs.iter().any(|n| n.provoked && !n.soldier) {
            self.refresh_sides();
        }
    }

    /// A rider of template `template` (its own model `model`) put out of
    /// their seat at `at`: they stand there, in the looks their seat had
    /// (`outfit`), and run from `from` (invented: no car hijack in the
    /// data says what a driver does). False when their kind isn't in.
    pub(crate) fn put_out(
        &mut self,
        template: &str,
        model: Option<u32>,
        outfit: Option<Vec<bool>>,
        at: Vec3,
        from: Vec3,
    ) -> bool {
        let Some(kind) = self.rider_kind(template, model) else {
            return false;
        };
        let away = (at - from).with_y(0.0);
        let facing = away.x.atan2(away.z);
        let mut n = Npc::new(
            kind,
            &self.kinds[kind].rig,
            at,
            facing,
            Brain::Post { facing },
            None,
        );
        n.outfit = outfit.unwrap_or_else(|| self.dress_as(kind, template));
        n.put_out = true;
        n.panic(from);
        self.npcs.push(n);
        true
    }

    /// Template `template`'s own model `model` won't do (it didn't load,
    /// or the clips don't fit its skeleton): its soldiers stand as their
    /// name's role instead, the rest not at all.
    pub fn refuse(&mut self, model: u32, template: String) {
        self.refused.insert((model, template));
    }

    /// Who spawn `s` stands as: their template's own model, once it is in;
    /// a spawner's pick from its list; else (no model of their own, or one
    /// that won't do) the catalogue's model for the role their name asks
    /// for, a plain soldier's for a mission's `target`. The Venezuelans
    /// (by name) and a mission's targets fight; of the rest in a model of
    /// their own, only story characters stand ([`left_out`]).
    fn who(&mut self, s: &SpawnPoint, target: bool) -> Who {
        let names = [s.template.as_deref(), Some(s.name.as_str())];
        if let (Some(template), Some(model)) = (s.template.as_deref(), s.model) {
            if left_out(s, target) {
                return Who::Nobody;
            }
            let fights = target
                || names
                    .iter()
                    .flatten()
                    .any(|n| faction_of(n).is_some_and(|f| self.fights_for(f)));
            if let Some(kind) = self.own(template, model) {
                return Who::Kind {
                    kind,
                    fights,
                    listed: false,
                };
            }
            if !self.refused.contains(&(model, template.to_owned())) {
                return Who::Waiting;
            }
        } else if !s.list.is_empty() {
            return self.listed(s);
        }
        let role = names
            .iter()
            .flatten()
            .find_map(|n| self.kind_for(n))
            .or_else(|| target.then(|| self.kind_for("vz soldier")).flatten());
        match role {
            Some(kind) => Who::Kind {
                kind,
                fights: true,
                listed: false,
            },
            None => Who::Nobody,
        }
    }

    /// Who a spawner of people sends out: one of its list by weight, in
    /// their template's own model (the whole list's are asked for at
    /// once, so a spawner isn't held up model by model). The factions'
    /// attitudes aren't in yet, so only the Venezuelans fight.
    fn listed(&mut self, s: &SpawnPoint) -> Who {
        let weights: Vec<(u32, f32)> = s
            .list
            .iter()
            .enumerate()
            .map(|(i, (_, _, w))| (i as u32, *w))
            .collect();
        for (template, model, _) in &s.list {
            self.own(template, *model);
        }
        let roll = self.rng.range(0.0, 1.0);
        let Some(i) = m2_engine::population::pick(&weights, roll) else {
            return Who::Nobody;
        };
        let (template, model, _) = &s.list[i as usize];
        let fights = faction_of(template).is_some_and(|f| self.fights_for(f));
        if let Some(kind) = self.own(template, *model) {
            return Who::Kind {
                kind,
                fights,
                listed: true,
            };
        }
        if !self.refused.contains(&(*model, template.clone())) {
            return Who::Waiting;
        }
        match self.kind_for(template) {
            Some(kind) => Who::Kind {
                kind,
                fights: true,
                listed: true,
            },
            None => Who::Nobody,
        }
    }

    /// Who spawn `s` stands as, in words (for checking): their template's
    /// own model, the role their name asks for, a spawner's list, or
    /// nobody.
    pub fn stands_as(&self, s: &SpawnPoint) -> String {
        if let Some(n) = self.npcs.iter().find(|n| n.spawn == Some(s.entity)) {
            let k = &self.kinds[n.kind];
            let how = if k.named {
                "own model"
            } else {
                "role, by name"
            };
            let what = match (n.soldier, n.held) {
                (true, _) => "fights",
                (false, true) => "a story character",
                (false, false) => "doesn't fight",
            };
            return format!("{:?} ({how} {:08x}), {what}", k.role, k.model.hash);
        }
        let target = self.mission_soldiers.contains(&s.entity);
        if left_out(s, target) {
            return "nobody (in a model of its own, not fighting, not a story character)"
                .to_owned();
        }
        if let (Some(template), Some(model)) = (s.template.as_deref(), s.model) {
            let key = (model, template.to_owned());
            let state = if self.refused.contains(&key) {
                "won't do"
            } else if self.asked.contains(&key) {
                "asked for"
            } else {
                "not asked for yet"
            };
            return format!("its own model {model:08x} ({state})");
        }
        if !s.list.is_empty() {
            return format!("a spawner of {} templates", s.list.len());
        }
        let role = [s.template.as_deref(), Some(s.name.as_str())]
            .iter()
            .flatten()
            .find_map(|n| role_for(n))
            .or(target.then_some("vz_hum_soldier"));
        match role {
            Some(role) => format!("the catalogue's {role}, by name"),
            None => "nobody".to_owned(),
        }
    }

    pub fn add_kind(&mut self, kind: PersonKind) {
        eprintln!("people: kind {:?}: {}", kind.role, kind.rig.idle_line());
        self.kinds.push(kind);
    }

    /// One fixed tick: people come and go around `focus`, then move;
    /// soldiers on alert fight the player at `threat` (her chest; `None`
    /// while she is down or not playing).
    pub fn update(
        &mut self,
        physics: &mut Physics,
        spawns: &[SpawnPoint],
        focus: Vec3,
        threat: Option<Vec3>,
        dt: f32,
    ) {
        self.pursue(dt);
        self.refill(spawns, focus, dt);
        self.populate(physics, spawns, focus);
        let mut callouts = Vec::new();
        for npc in &mut self.npcs {
            npc.smart = self.smarter;
            npc.rammed = (npc.rammed - dt).max(0.0);
            let facing = npc.facing;
            npc.update(physics, &mut self.rng, threat, dt);
            let p = npc.character.position;
            self.checks
                .count(npc, facing, physics.water_at(p.x, p.z), dt);
            if let Some(t) = threat {
                self.idle.count(physics, npc, t, dt);
            }
            if std::mem::take(&mut npc.spotted) && npc.alive && self.smarter.enabled {
                callouts.push(npc.character.position);
            }
            // The dead go limp from where they stood, shoved as they were
            // hit.
            let Some(push) = npc.fling.take() else {
                continue;
            };
            match npc.ragdoll {
                Some(id) => physics.push_ragdoll(id, push),
                None => {
                    let (nodes, _) = npc.rig.posed();
                    let body = npc.model_matrix();
                    let world: Vec<Mat4> = nodes.iter().map(|m| body * *m).collect();
                    let id = physics.add_ragdoll(&npc.rig.skeleton.parents, &world, push);
                    npc.ragdoll = Some(id);
                }
            }
        }
        // Smarter: those near a soldier who spots her are told where she is.
        if let Some(t) = threat {
            let range = self.smarter.callout_range;
            for from in callouts {
                for npc in &mut self.npcs {
                    if npc.character.position.distance(from) < range {
                        npc.told(t);
                    }
                }
            }
        }
    }

    /// The soldiers fighting by what they are at, in a line (for checking).
    pub fn fight_summary(&self) -> String {
        let mut counts: Vec<(&str, usize)> = Vec::new();
        for state in self.npcs.iter().filter_map(Npc::fight_state) {
            match counts.iter_mut().find(|c| c.0 == state) {
                Some(c) => c.1 += 1,
                None => counts.push((state, 1)),
            }
        }
        if counts.is_empty() {
            return "no soldier fighting".to_owned();
        }
        let parts: Vec<String> = counts.iter().map(|(s, n)| format!("{n} {s}")).collect();
        format!("soldiers {}", parts.join(", "))
    }

    /// Vehicles going through people: `cars` are (model to world,
    /// velocity, model-space hull box, mass in kg) of each one moving.
    /// Anyone a car's box meets takes its ram by the damage table `table`
    /// ([`m2_engine::durability::run_over`]: a `Human` three times a
    /// `Car`'s, a `HumanArmor` once, the bosses and targets once) and is
    /// knocked flying if it kills, shoved aside if not (and not hurt by a
    /// vehicle again for [`RAM_GAP`]); without the table, as before, it
    /// kills. Those who don't fight in its path (as wide as it is, and a
    /// little more) dive out of the way, but story characters. The number
    /// of soldiers killed.
    pub fn traffic(
        &mut self,
        cars: &[(Mat4, Vec3, (Vec3, Vec3), f32)],
        table: Option<&m2_engine::tuning::DamageTable>,
    ) -> usize {
        let human = m2_assets::name_hash("Human");
        let kind = m2_assets::name_hash(m2_engine::durability::RAM_KIND);
        let mut killed = 0;
        for &(at, velocity, (lo, hi), mass) in cars {
            let speed = velocity.length();
            if speed < RUN_OVER {
                continue;
            }
            let to_car = at.inverse();
            let centre = at.transform_point3((lo + hi) * 0.5);
            let heading = velocity.with_y(0.0).normalize_or_zero();
            let width = (hi.x - lo.x) * 0.5;
            for n in self.npcs.iter_mut().filter(|n| n.alive && n.landed) {
                let p = n.character.position;
                let local = to_car.transform_point3(p + Vec3::Y * 0.9);
                if in_ram(local, (lo, hi)) {
                    if n.rammed > 0.0 {
                        continue;
                    }
                    let push = velocity * 1.1 + Vec3::Y * (2.0 + speed * 0.15);
                    let damage = table.map_or(1000.0, |t| {
                        let row = n.ai.modifier.unwrap_or(human);
                        m2_engine::durability::run_over(t, row, kind, mass, speed)
                    });
                    if n.hurt(damage, push) {
                        killed += usize::from(n.soldier);
                    } else {
                        // Knocked aside, off the way it drives.
                        let rel = (p - centre).with_y(0.0);
                        let side = (rel - heading * rel.dot(heading))
                            .normalize_or(Vec3::new(heading.z, 0.0, -heading.x));
                        n.character.velocity = heading * speed * 0.5 + side * SHOVE;
                        n.rammed = RAM_GAP;
                        n.panic(centre);
                    }
                    continue;
                }
                // In its path in the next couple of seconds: off the line
                // it drives along.
                let rel = (p - centre).with_y(0.0);
                let ahead = rel.dot(heading);
                let side = (rel - heading * ahead).length();
                if ahead > 0.0 && ahead < speed * DODGE_TIME && side < width + 1.5 {
                    n.panic(centre + heading * ahead);
                }
            }
        }
        killed
    }

    /// Who shouts this tick: a soldier the first tick they are on alert,
    /// a civilian the first tick they run (once until they calm down).
    /// Index into `npcs`, what, and where (their head).
    pub fn take_shouts(&mut self) -> Vec<(usize, Shout, Vec3)> {
        let mut out = Vec::new();
        for (i, n) in self.npcs.iter_mut().enumerate() {
            let calm = !n.alive || (!n.alert && n.flee.is_none());
            if calm {
                n.shouted = false;
                continue;
            }
            if !n.shouted {
                n.shouted = true;
                let what = if n.soldier { Shout::Alert } else { Shout::Flee };
                out.push((i, what, n.character.position + Vec3::Y * 1.7));
            }
        }
        out
    }

    /// Every soldier killed so far, about or not.
    pub fn all_killed(&self) -> Vec<u32> {
        let mut out: Vec<u32> = self
            .killed
            .iter()
            .copied()
            .chain(
                self.npcs
                    .iter()
                    .filter(|n| !n.alive)
                    .filter_map(|n| n.spawn),
            )
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The scripts put the person of spawn `entity` at `at`: they stand
    /// there, at a post facing the way they do, and there they stand
    /// when they next come.
    pub fn put(&mut self, entity: u32, at: Vec3) {
        self.moved.insert(entity, at);
        for n in self
            .npcs
            .iter_mut()
            .filter(|n| n.spawn == Some(entity) && n.alive)
        {
            n.character.position = at;
            n.character.velocity = Vec3::ZERO;
            n.landed = false;
            n.flee = None;
            n.through = false;
            n.brain = Brain::Post { facing: n.facing };
        }
        eprintln!("people: the scripts put {entity:08x} at {at:.1}");
    }

    /// The people who got to the end of a route the scripts sent them
    /// along since the last call: spawn entity, and where they stand. The
    /// scripts hear their goal is done (an outpost's rushers at its flag).
    pub fn take_route_ends(&mut self) -> Vec<(u32, Vec3)> {
        self.npcs
            .iter_mut()
            .filter_map(|n| {
                std::mem::take(&mut n.ended).then_some((n.spawn?, n.character.position))
            })
            .collect()
    }

    /// The scripts send the person of spawn `entity` along `route`
    /// (`Ai.Goal` `PathMove`, or `MoveTo` a point): they run it, then stand
    /// at its end. One not yet in play runs it when they come.
    pub fn walk(&mut self, entity: u32, route: Vec<Vec3>) {
        if route.is_empty() {
            return;
        }
        self.routes.insert(entity, route.clone());
        let mut here = false;
        for n in self
            .npcs
            .iter_mut()
            .filter(|n| n.spawn == Some(entity) && n.alive)
        {
            here = true;
            eprintln!(
                "people: {entity:08x} sent from {:.1} along {} points, first {:.1}",
                n.character.position,
                route.len(),
                route[0]
            );
            n.flee = None;
            n.route_near = f32::INFINITY;
            n.route_stall = 0.0;
            n.through = false;
            n.brain = Brain::Route {
                route: route.clone(),
                next: 0,
            };
        }
        if !here {
            eprintln!(
                "people: {entity:08x} sent along {} points, first {:.1}, not in play yet",
                route.len(),
                route[0]
            );
        }
    }

    /// The scripts killed the soldier of spawn `entity` (or took it away,
    /// `gone`): dead where they stand, or not there at all. Either way they
    /// don't come back.
    pub fn script_kill(&mut self, physics: &mut Physics, entity: u32, gone: bool) {
        self.killed.insert(entity);
        if gone {
            let (npcs, active) = (&mut self.npcs, &mut self.active);
            npcs.retain(|n| {
                if n.spawn != Some(entity) {
                    return true;
                }
                if let Some(doll) = n.ragdoll {
                    physics.remove_ragdoll(doll);
                }
                active.remove(&entity);
                false
            });
        } else if let Some(n) = self.npcs.iter_mut().find(|n| n.spawn == Some(entity)) {
            n.hurt(f32::INFINITY, Vec3::ZERO);
        }
    }

    /// Starts over with these soldiers dead and nobody about (they come
    /// back round the player on the next tick).
    pub fn reset(&mut self, killed: impl IntoIterator<Item = u32>) {
        self.npcs.clear();
        self.active.clear();
        self.killed = killed.into_iter().collect();
    }

    /// Soldiers killed since the last call: where each fell and the
    /// layer entity that placed them.
    pub fn take_deaths(&mut self) -> Vec<(Vec3, Option<u32>)> {
        self.npcs
            .iter_mut()
            .filter_map(|n| {
                (std::mem::take(&mut n.died) && n.soldier)
                    .then_some((n.character.position, n.spawn))
            })
            .collect()
    }

    /// Calls off every fight (the player went down).
    pub fn stand_down(&mut self) {
        for n in &mut self.npcs {
            n.alert = false;
        }
    }

    /// Whether a spawn of layer `layer` (a state layer or a static
    /// sub-layer; `None`: none known) can stand now: it is in, or which
    /// are in isn't known.
    pub fn layer_in(&self, layer: Option<u32>) -> bool {
        layer.is_none_or(|l| {
            self.state_layers
                .as_ref()
                .is_none_or(|set| set.contains(&l))
        })
    }

    /// A spawner sending from its list (not a mission's target) sends
    /// another its `SecondsPerCycle` after its one fell, while it is in
    /// reach and she isn't on top of it: the fallen one lies where he fell,
    /// no longer the spawner's. From the data: the cycle; the rest is
    /// invented (see [`REFILL_CLEAR`]).
    fn refill(&mut self, spawns: &[SpawnPoint], focus: Vec3, dt: f32) {
        let cycles = self.cycles.get_or_insert_with(|| {
            spawns
                .iter()
                .filter(|s| !s.list.is_empty())
                .filter_map(|s| Some((s.entity, s.cycle?)))
                .collect()
        });
        for n in &mut self.npcs {
            let Some(id) = n.spawn.filter(|_| !n.alive) else {
                continue;
            };
            let Some(&cycle) = cycles.get(&id) else {
                continue;
            };
            if self.mission_soldiers.contains(&id) {
                continue;
            }
            let left = self.refills.entry(id).or_insert(cycle);
            *left -= dt;
            let at = n.character.position;
            if *left <= 0.0 && Vec2::new(at.x - focus.x, at.z - focus.z).length() > REFILL_CLEAR {
                self.refills.remove(&id);
                self.active.remove(&id);
                n.spawn = None;
            }
        }
    }

    fn populate(&mut self, physics: &mut Physics, spawns: &[SpawnPoint], focus: Vec3) {
        let far = |p: Vec3, d: f32| Vec2::new(p.x - focus.x, p.z - focus.z).length() > d;
        // People out of range, or whose layer went, go.
        let layers = self.state_layers.as_ref();
        let (active, killed) = (&mut self.active, &mut self.killed);
        self.npcs.retain(|n| match n.spawn {
            Some(id)
                if far(n.character.position, INACTIVE)
                    || n.layer
                        .zip(layers)
                        .is_some_and(|(l, set)| !set.contains(&l)) =>
            {
                if let Some(doll) = n.ragdoll {
                    physics.remove_ragdoll(doll);
                }
                active.remove(&id);
                if !n.alive {
                    killed.insert(id);
                }
                false
            }
            // Put out of a seat, they go once out of range too.
            None if n.put_out && far(n.character.position, INACTIVE) => {
                if let Some(doll) = n.ragdoll {
                    physics.remove_ragdoll(doll);
                }
                false
            }
            _ => true,
        });
        // Said once each, for checking against the layers the game loads:
        // static sub-layers whose people near here don't stand (not while
        // which are in is still to come).
        if self
            .state_layers
            .as_ref()
            .is_some_and(|set| !set.is_empty())
        {
            for s in spawns
                .iter()
                .filter(|s| s.static_layer && !far(s.position, ACTIVE))
            {
                if let Some(l) = s.layer
                    && !self.layer_in(s.layer)
                    && self.unloaded.insert(l)
                {
                    eprintln!(
                        "people: static layer {l:08x} isn't loaded: {:?} and its other people don't stand",
                        s.name
                    );
                }
            }
        }
        // One the scripts made a mission's target after they stood (a
        // guard whose objective comes later) fights from then on.
        for n in &mut self.npcs {
            if !n.soldier && n.spawn.is_some_and(|e| self.mission_soldiers.contains(&e)) {
                n.soldier = true;
                n.held = false;
                eprintln!(
                    "people: {:?} ({:08x}) is a mission's target now: fights",
                    self.kinds[n.kind].role,
                    n.spawn.unwrap_or(0)
                );
            }
        }
        // People who come into range: a mission's targets first, then the
        // nearest. Where the scripts put someone, they stand.
        let soldiers = self
            .npcs
            .iter()
            .filter(|n| n.spawn.is_some_and(|e| !is_walker(e)))
            .count();
        let walkers = self
            .npcs
            .iter()
            .filter(|n| n.spawn.is_some_and(is_walker))
            .count();
        let mut near: Vec<(f32, &SpawnPoint, Vec3)> = spawns
            .iter()
            .map(|s| (s, self.moved.get(&s.entity).copied()))
            .filter(|&(s, moved)| {
                !self.active.contains(&s.entity)
                    && !self.killed.contains(&s.entity)
                    && !far(moved.unwrap_or(s.position), ACTIVE)
                    && self.layer_in(s.layer)
            })
            .map(|(s, moved)| {
                let at = moved.unwrap_or(s.position);
                (at.distance(focus), s, at)
            })
            .collect();
        // A mission's targets, and those the scripts sent somewhere (the
        // villa's waves, `Ai.Goal` `PathMove`), come first.
        let (targets, routes) = (&self.mission_soldiers, &self.routes);
        let wanted = |e: &u32| targets.contains(e) || routes.contains_key(e);
        near.sort_by(|a, b| {
            (!wanted(&a.1.entity))
                .cmp(&!wanted(&b.1.entity))
                .then(a.0.total_cmp(&b.0))
        });
        // And they come even when the island's soldiers fill the place up
        // (playtest 8: with 48 about, the villa's waves never stood).
        let mut room = MAX_SOLDIERS.saturating_sub(soldiers);
        let mut walker_room = self.walker_cap.min(MAX_WALKERS).saturating_sub(walkers);
        for (_, s, at) in near {
            let target = self.mission_soldiers.contains(&s.entity);
            let sent = self.routes.contains_key(&s.entity);
            let room = if is_walker(s.entity) {
                &mut walker_room
            } else {
                &mut room
            };
            if *room == 0 && !target && !sent {
                continue;
            }
            if *room == 0 && sent {
                eprintln!(
                    "people: {:08x} comes though {MAX_SOLDIERS} soldiers are about: the scripts sent them",
                    s.entity
                );
            }
            let (kind, fights, listed) = match self.who(s, target) {
                Who::Kind {
                    kind,
                    fights,
                    listed,
                } => (kind, fights, listed),
                // One whose model is still loading keeps their place, so
                // spawns further off don't take it meanwhile.
                Who::Waiting => {
                    *room = room.saturating_sub(1);
                    continue;
                }
                Who::Nobody => continue,
            };
            *room = room.saturating_sub(1);
            let forward = s.rotation * Vec3::Z;
            let facing = forward.x.atan2(forward.z);
            let brain = if let Some(route) = self.routes.get(&s.entity) {
                eprintln!(
                    "people: {:08x} comes in at {at:.1} to run the route the scripts gave",
                    s.entity
                );
                Brain::Route {
                    route: route.clone(),
                    next: 0,
                }
            } else if s.route.len() >= 2 && !self.moved.contains_key(&s.entity) {
                Brain::Patrol {
                    route: s.route.clone(),
                    next: 0,
                    step: 1,
                    wait: 0.0,
                }
            } else {
                Brain::Post { facing }
            };
            self.active.insert(s.entity);
            let mut npc = Npc::new(
                kind,
                &self.kinds[kind].rig,
                at,
                facing,
                brain,
                Some(s.entity),
            );
            npc.layer = s.layer;
            if let Some(ai) = s.ai {
                npc.ai = ai;
            }
            // Their template's `Health` (a plain VZ soldier's 100 when none
            // speaks for them).
            npc.hp = npc.ai.health;
            npc.most_hp = npc.ai.health;
            npc.soldier = fights;
            // An army's soldier (by their template's name, else their
            // spawner's list's first): they fight as their faction does.
            npc.faction = [s.template.as_deref(), Some(s.name.as_str())]
                .into_iter()
                .flatten()
                .chain(s.list.first().map(|(t, _, _)| t.as_str()))
                .find_map(faction_of);
            // A story character the layers place is the scripts': where
            // they are put, they stay.
            npc.held = !fights && !listed && npc.faction.is_none();
            // Their own template's gun (in the catalogue's model too); a
            // spawner's pick's, by the kind of their template.
            npc.gun = if listed {
                self.kinds[kind].gun
            } else {
                s.gun.or(self.kinds[kind].gun)
            };
            npc.outfit = self.dress(kind, s);
            self.tell(s, &npc);
            self.npcs.push(npc);
        }
    }

    /// A look for one of kind `kind` spawned at `s`: of the parts their
    /// template's `ModelMixerProfile` lets them wear in their model (every
    /// part, for a template naming none or one the model lacks), a random
    /// pick ([`m2_engine::Looks::pick`]); empty (the model's own) for a
    /// model with no mixer. Each new look is said once.
    fn dress(&mut self, kind: usize, s: &SpawnPoint) -> Vec<bool> {
        let template = s.template.clone().unwrap_or_else(|| s.name.clone());
        self.dress_as(kind, &template)
    }

    /// A look for someone of kind `kind` spawned as template `template`
    /// (a template's own model is dressed as that template), as
    /// [`dress`](Self::dress) picks one.
    pub(crate) fn dress_as(&mut self, kind: usize, template: &str) -> Vec<bool> {
        let k = &self.kinds[kind];
        let Some(looks) = k.model.looks.clone() else {
            return Vec::new();
        };
        let template = if k.named {
            k.role.clone()
        } else {
            template.to_owned()
        };
        let model = k.model.hash;
        let profile = self
            .mixer_profiles
            .get(&template.to_ascii_lowercase())
            .copied();
        let rng = &mut self.rng;
        let (picked, hidden) = looks.pick(profile, |n| {
            ((rng.next() * n as f32) as usize).min(n.saturating_sub(1))
        });
        let has = profile.is_some_and(|p| looks.mixer.profiles.iter().any(|x| x.0 == p));
        if self.looks_seen.insert((kind, picked.clone())) {
            eprintln!(
                "look: {template:?} in {model:08x}, profile {} ({}): parts {picked:?}; {} looks so far",
                profile.map_or_else(|| "none".to_owned(), |p| format!("{p:08x}")),
                if has { "its own" } else { "every part" },
                self.looks_seen.len()
            );
        }
        hidden
    }

    /// Says whom spawn `s` first stands as, once per kind and template:
    /// for checking the models and guns against the data.
    fn tell(&mut self, s: &SpawnPoint, npc: &Npc) {
        let template = s.template.clone().unwrap_or_default();
        if !self.told.insert((npc.kind, template.clone())) {
            return;
        }
        let k = &self.kinds[npc.kind];
        let model = if k.named {
            format!("its template's own model {:08x}", k.model.hash)
        } else {
            format!(
                "the catalogue's {} model {:08x} (by its name: no model of its own that will do)",
                k.role, k.model.hash
            )
        };
        let what = match (npc.soldier, npc.held) {
            (true, _) => "fights",
            (false, true) => "a story character: stays where put",
            (false, false) => "doesn't fight",
        };
        let gun = npc
            .gun
            .map_or("none of its own (the AK47 stands in)".to_owned(), |g| {
                format!(
                    "{:#x} ({:.0} a minute, {} a round)",
                    g.template, g.rate_of_fire, g.damage
                )
            });
        eprintln!(
            "people: {:?} (template {template:?}, as {:?}) stands in {model}, {what}, gun {gun}",
            s.name, k.role
        );
    }
}

/// A small xorshift generator: where people stroll and whom spawners
/// send out need no more.
#[derive(Debug)]
struct Rng(u64);

impl Default for Rng {
    fn default() -> Self {
        Self(0x9e37_79b9_7f4a_7c15)
    }
}

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next()
    }

    /// A point on the ground plane between `lo` and `hi` metres away.
    fn ring(&mut self, lo: f32, hi: f32) -> Vec3 {
        let a = self.range(0.0, std::f32::consts::TAU);
        let r = self.range(lo, hi);
        Vec3::new(a.sin() * r, 0.0, a.cos() * r)
    }

    /// A point on the ground plane within `r` metres.
    fn disc(&mut self, r: f32) -> Vec3 {
        self.ring(0.0, r)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn a_hull_padded_by_a_person_rams_them() {
        let hull = (Vec3::new(-1.0, 0.0, -2.0), Vec3::new(1.0, 1.5, 2.0));
        // Beside the bumper, within a person's half-width.
        assert!(in_ram(Vec3::new(1.3, 0.9, 0.0), hull));
        assert!(!in_ram(Vec3::new(1.4, 0.9, 0.0), hull));
        // Standing well above the roof.
        assert!(!in_ram(Vec3::new(0.0, 2.5, 0.0), hull));
    }
    use m2_engine::{Animator, CollisionShape, Skeleton, StaticKey};
    use m2_math::glam::Quat;

    #[test]
    fn the_table_keys_follow_the_slots() {
        let names: Vec<&str> = FightClips::default().slots().iter().map(|s| s.0).collect();
        let keyed: Vec<&str> = FightClips::TABLE_KEYS.iter().map(|k| k.0).collect();
        assert_eq!(names, keyed);
    }
    use std::sync::Arc;

    // A squad standing a step apart takes all three of the assault
    // rifles' shot cues (six variants), and a person keeps their number.
    #[test]
    fn a_squad_a_step_apart_spreads_over_the_variants() {
        let at = |i: usize| Vec3::new(10.0 + i as f32, 0.0, -4.0);
        let mut cues = std::collections::HashSet::new();
        for i in 0..8 {
            cues.insert(shooter_number(at(i), 0.5) % 3);
        }
        assert_eq!(cues.len(), 3);
        assert_eq!(shooter_number(at(2), 0.5), shooter_number(at(2), 0.5));
        assert_ne!(shooter_number(at(2), 0.5), shooter_number(at(2), 0.6));
    }

    pub(crate) fn ground() -> Physics {
        let mut p = Physics::new();
        let h = 200.0;
        let v = [
            Vec3::new(-h, 0.0, -h),
            Vec3::new(h, 0.0, -h),
            Vec3::new(h, 0.0, h),
            Vec3::new(-h, 0.0, h),
        ];
        let shape = CollisionShape::mesh(&v, &[[0, 1, 2], [0, 2, 3]]).unwrap();
        p.add_static(StaticKey::Cell(1), &shape, Vec3::ZERO, Quat::IDENTITY);
        p.step(1.0 / 60.0);
        p
    }

    pub(crate) fn empty_model() -> GpuModel {
        let cpu = m2_engine::CpuModel {
            hash: 0,
            pieces: Vec::new(),
            materials: Vec::new(),
            bounds: (Vec3::ZERO, Vec3::ZERO),
            skipped_groups: 0,
            collision: None,
            members: Vec::new(),
            hidden: Default::default(),
            textures: Vec::new(),
            tiny_members: Vec::new(),
            sinks: Vec::new(),
            breakup: None,
            parts: None,
            looks: None,
        };
        let mut backend = m2_render::NullBackend::default();
        let mut materials = m2_engine::MaterialCache::new();
        GpuModel::upload(&mut backend, &mut materials, &cpu).unwrap()
    }

    pub(crate) fn rig() -> Rig {
        Rig {
            skeleton: Arc::new(Skeleton::from_nodes(&[])),
            clips: Arc::new(Vec::new()),
            locomotion: None,
            animator: Animator::default(),
            gait: Gait::Idle,
            upper: None,
            two_handed: false,
            aim: None,
            hands: Default::default(),
            arm: Default::default(),
            cues: Vec::new(),
            own: Default::default(),
            foreign: Default::default(),
            aim_clip: None,
            barrel: true,
        }
    }

    #[test]
    fn patrols_walk_there_and_back() {
        let physics = ground();
        let route = vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 10.0)];
        let brain = Brain::Patrol {
            route,
            next: 0,
            step: 1,
            wait: 0.0,
        };
        let mut npc = Npc::new(0, &rig(), Vec3::new(0.0, 1.0, 0.0), 0.0, brain, Some(1));
        let mut rng = Rng::default();
        let dt = 1.0 / 60.0;
        let mut furthest: f32 = 0.0;
        for _ in 0..60 * 12 {
            npc.update(&physics, &mut rng, None, dt);
            furthest = furthest.max(npc.character.position.z);
        }
        assert!(npc.landed);
        assert!(furthest > 9.0, "{furthest}");
        // Back at the start (or pausing near it) by now: 10 m at 1.4 m/s
        // each way plus a pause.
        for _ in 0..60 * 14 {
            npc.update(&physics, &mut rng, None, dt);
        }
        assert!(npc.character.position.z < 2.0, "{}", npc.character.position);
    }

    #[test]
    fn soldiers_come_and_go_with_the_player() {
        let mut physics = ground();
        let mut people = People::default();
        people.kinds.push(kind("vz_hum_soldier", false));
        let spawns = [
            spawn(1, "VZ Soldier 01", 10.0),
            spawn(2, "VZ Soldier 02", 1000.0),
            spawn(3, "VZ Hind 01", 20.0),
        ];
        people.update(&mut physics, &spawns, Vec3::ZERO, None, 1.0 / 60.0);
        assert_eq!(
            people.npcs.iter().map(|n| n.spawn).collect::<Vec<_>>(),
            [Some(1)]
        );
        people.update(
            &mut physics,
            &spawns,
            Vec3::new(1000.0, 0.0, 0.0),
            None,
            1.0 / 60.0,
        );
        assert_eq!(
            people.npcs.iter().map(|n| n.spawn).collect::<Vec<_>>(),
            [Some(2)]
        );
        // The dead stay dead through a save and load.
        people.npcs[0].alive = false;
        let killed = people.all_killed();
        assert_eq!(killed, [2]);
        people.reset(killed);
        people.update(
            &mut physics,
            &spawns,
            Vec3::new(1000.0, 0.0, 0.0),
            None,
            1.0 / 60.0,
        );
        // Nobody the layers don't place.
        assert!(people.npcs.is_empty());
    }

    #[test]
    fn spawners_send_out_whom_their_list_names() {
        let mut physics = ground();
        let mut people = People::default();
        let spawner = SpawnPoint {
            template: Some("PathSpawner_GurHQ_Peds".into()),
            list: vec![
                ("Guerilla Soldier".into(), 0x6a11, 1.0),
                ("Guerilla Officer".into(), 0x6a12, 1.0),
                // Another template in the same model is a kind of its own.
                ("GR Defender (rifle)".into(), 0x6a11, 1.0),
            ],
            ..spawn(9, "PathSpawner_GurHQ_Peds", 10.0)
        };
        let spawns = [spawner];
        people.update(&mut physics, &spawns, Vec3::ZERO, None, 1.0 / 60.0);
        // Every model on the list is asked for at once.
        assert_eq!(
            people.wanted,
            [
                (0x6a11, "Guerilla Soldier".to_owned()),
                (0x6a12, "Guerilla Officer".to_owned()),
                (0x6a11, "GR Defender (rifle)".to_owned())
            ]
        );
        for role in [
            "Guerilla Soldier",
            "Guerilla Officer",
            "GR Defender (rifle)",
        ] {
            people.add_kind(kind(role, true));
        }
        people.update(&mut physics, &spawns, Vec3::ZERO, None, 1.0 / 60.0);
        let sent: Vec<&Npc> = people.npcs.iter().filter(|n| n.spawn == Some(9)).collect();
        assert_eq!(sent.len(), 1);
        // Not a Venezuelan: doesn't fight, nor is he a story character.
        assert!(!sent[0].soldier && !sent[0].held);
    }

    #[test]
    fn an_armys_soldiers_fight_her_when_their_faction_is_hostile_or_she_hurt_one() {
        assert_eq!(faction_of("Allied Soldier"), Some("All"));
        assert_eq!(faction_of("OC Soldier (Saunter)"), Some("Oil"));
        assert_eq!(faction_of("VZ Heavy (RPG)"), Some("Vza"));
        assert_eq!(faction_of("Carmona"), None);
        let mut physics = ground();
        let mut people = People::default();
        let post = SpawnPoint {
            template: Some("OC Soldier".into()),
            ai: Some(AiProfile::default()),
            model: Some(0x0d0b),
            ..spawn(5, "OC Soldier 0x0011055e", 10.0)
        };
        let spawns = [post];
        people.update(&mut physics, &spawns, Vec3::ZERO, None, 1.0 / 60.0);
        // Not left out: his template's own model is asked for.
        assert_eq!(people.wanted, [(0x0d0b, "OC Soldier".to_owned())]);
        people.add_kind(kind("OC Soldier", true));
        people.update(&mut physics, &spawns, Vec3::ZERO, None, 1.0 / 60.0);
        let him = |p: &People| {
            p.npcs
                .iter()
                .find(|n| n.spawn == Some(5))
                .map(|n| (n.soldier, n.held))
        };
        // He stands (not left out), armed but not fighting, nor the scripts'.
        assert_eq!(him(&people), Some((false, false)));
        // The scripts' attitudes make OC hostile: he fights; back: he doesn't.
        people.set_hostile(["Vza", "Oil"].into_iter().collect());
        assert_eq!(him(&people), Some((true, false)));
        people.set_hostile(["Vza"].into_iter().collect());
        assert_eq!(him(&people), Some((false, false)));
        // She hurts him: OC pursue her for the pursuit's time, then stop.
        if let Some(n) = people.npcs.iter_mut().find(|n| n.spawn == Some(5)) {
            n.hurt(1.0, Vec3::ZERO);
        }
        people.update(&mut physics, &spawns, Vec3::ZERO, None, 1.0 / 60.0);
        assert!(people.pursuit.contains_key("Oil"));
        assert_eq!(him(&people), Some((true, false)));
        people.update(
            &mut physics,
            &spawns,
            Vec3::ZERO,
            None,
            PURSUIT_SECONDS + 1.0,
        );
        assert!(people.pursuit.is_empty());
        assert_eq!(him(&people), Some((false, false)));
    }

    #[test]
    fn a_spawner_sends_another_a_cycle_after_its_one_fell() {
        let mut physics = ground();
        let mut people = People::default();
        people.add_kind(kind("VZ Soldier", true));
        let spawner = SpawnPoint {
            list: vec![("VZ Soldier".into(), 0x45cc, 1.0)],
            cycle: Some(10.0),
            ..spawn(9, "PopulationSpawner", 40.0)
        };
        let spawns = [spawner];
        let dt = 1.0 / 60.0;
        people.update(&mut physics, &spawns, Vec3::ZERO, None, dt);
        let first = people.npcs.iter().position(|n| n.spawn == Some(9));
        let first = first.expect("sent one");
        assert!(people.npcs[first].hurt(1000.0, Vec3::ZERO));
        // Not before the cycle is out.
        for _ in 0..(9.5 / dt) as usize {
            people.update(&mut physics, &spawns, Vec3::ZERO, None, dt);
        }
        assert_eq!(people.npcs.iter().filter(|n| n.alive).count(), 0);
        for _ in 0..(1.0 / dt) as usize {
            people.update(&mut physics, &spawns, Vec3::ZERO, None, dt);
        }
        let alive: Vec<&Npc> = people.npcs.iter().filter(|n| n.alive).collect();
        assert_eq!(alive.len(), 1);
        assert_eq!(alive[0].spawn, Some(9));
        // The fallen one lies where he fell, the spawner's no more.
        assert!(people.npcs.iter().any(|n| !n.alive && n.spawn.is_none()));
        // Nor while she stands on top of it.
        let i = people.npcs.iter().position(|n| n.alive).expect("one");
        assert!(people.npcs[i].hurt(1000.0, Vec3::ZERO));
        let near = Vec3::new(40.0, 0.0, 5.0);
        for _ in 0..(12.0 / dt) as usize {
            people.update(&mut physics, &spawns, near, None, dt);
        }
        assert_eq!(people.npcs.iter().filter(|n| n.alive).count(), 0);
    }

    #[test]
    fn as_many_pass_by_as_the_zone_asks() {
        let mut physics = ground();
        let mut people = People::default();
        people.add_kind(kind("Civ Casual (male)", true));
        let walker = |i: u32, x: f32| SpawnPoint {
            list: vec![("Civ Casual (male)".into(), 0x6e44, 1.0)],
            ..spawn(m2_engine::world::WALKER_BASE + i, "walker", x)
        };
        let spawns = [walker(0, 5.0), walker(1, 10.0), walker(2, 15.0)];
        let dt = 1.0 / 60.0;
        people.update(&mut physics, &spawns, Vec3::ZERO, None, dt);
        assert!(people.npcs.is_empty(), "a zone of none");
        people.walker_cap = 2;
        people.update(&mut physics, &spawns, Vec3::ZERO, None, dt);
        let out: Vec<Option<u32>> = people.npcs.iter().map(|n| n.spawn).collect();
        // The nearest two.
        let base = m2_engine::world::WALKER_BASE;
        assert_eq!(out, [Some(base), Some(base + 1)]);
        assert!(people.npcs.iter().all(|n| !n.soldier));
    }

    #[test]
    fn everyone_stands_in_their_templates_own_model() {
        let mut physics = ground();
        let mut people = People::default();
        // The catalogue's, for soldiers with no model of their own.
        people.kinds.push(kind("vz_hum_soldier", false));
        let own = |entity, template: &str, model| SpawnPoint {
            template: Some(template.into()),
            model: Some(model),
            ..spawn(entity, &format!("{template}_VzaCon001"), 10.0)
        };
        let spawns = [own(7, "Carmona", 0xbf83), own(8, "VZ Soldier", 0x45cc)];
        people.update(&mut physics, &spawns, Vec3::ZERO, None, 1.0 / 60.0);
        // Nobody yet: each template's model is asked for, once.
        assert!(people.npcs.is_empty());
        assert_eq!(
            people.wanted,
            [
                (0xbf83, "Carmona".to_owned()),
                (0x45cc, "VZ Soldier".to_owned())
            ]
        );
        people.wanted.clear();
        people.update(&mut physics, &spawns, Vec3::ZERO, None, 1.0 / 60.0);
        assert!(people.wanted.is_empty());
        let gun = HandGun {
            template: 0x8000_0123,
            model: Some(0x99),
            rate_of_fire: 600.0,
            clip: 30,
            reserve_clips: 7,
            damage: 9.0,
            damage_key: None,
            hint: None,
        };
        people.add_kind(kind("Carmona", true));
        people.add_kind(PersonKind {
            gun: Some(gun),
            ..kind("VZ Soldier", true)
        });
        people.update(&mut physics, &spawns, Vec3::ZERO, None, 1.0 / 60.0);
        let by = |e| people.npcs.iter().find(|n| n.spawn == Some(e)).unwrap();
        let (him, guard) = (by(7), by(8));
        assert_eq!(people.kinds[him.kind].role, "Carmona");
        assert!(!him.soldier, "he doesn't fight");
        assert!(him.held, "the scripts' to place");
        // The guard in his template's model, not the catalogue's, with his
        // template's gun, firing at its rate.
        assert_eq!(people.kinds[guard.kind].role, "VZ Soldier");
        assert!(guard.soldier && !guard.held);
        assert_eq!(guard.gun, Some(gun));
        assert!((guard.burst_gap() - 0.1).abs() < 1e-6);
    }

    #[test]
    fn a_model_that_wont_do_leaves_soldiers_their_names_role() {
        let mut physics = ground();
        let mut people = People::default();
        people.kinds.push(kind("vz_hum_soldier", false));
        let spawns = [
            SpawnPoint {
                template: Some("VZ Soldier".into()),
                model: Some(0x45cc),
                gun: Some(rifle()),
                ..spawn(1, "vza_beachguard_1", 10.0)
            },
            SpawnPoint {
                template: Some("Carmona".into()),
                model: Some(0xbf83),
                ..spawn(2, "Carmona_VzaCon001", 12.0)
            },
        ];
        people.update(&mut physics, &spawns, Vec3::ZERO, None, 1.0 / 60.0);
        assert!(people.npcs.is_empty(), "their models are loading");
        people.refuse(0x45cc, "VZ Soldier".into());
        people.refuse(0xbf83, "Carmona".into());
        people.wanted.clear();
        people.update(&mut physics, &spawns, Vec3::ZERO, None, 1.0 / 60.0);
        let stood: Vec<(Option<u32>, &str)> = people
            .npcs
            .iter()
            .map(|n| (n.spawn, people.kinds[n.kind].role.as_str()))
            .collect();
        assert_eq!(stood, [(Some(1), "vz_hum_soldier")]);
        // In the catalogue's model, still with his template's gun.
        assert!(people.npcs[0].soldier);
        assert_eq!(people.npcs[0].gun, Some(rifle()));
        assert!(people.wanted.is_empty(), "not asked for again");
    }

    #[test]
    fn names_ask_for_roles_mooks_are_plain_soldiers() {
        assert_eq!(role_for("VZ Soldier 03"), Some("vz_hum_soldier"));
        assert_eq!(role_for("VZ Mook 03"), Some("vz_hum_soldier"));
        assert_eq!(role_for("VZ Heavy MG"), Some("vz_hum_soldierelite"));
        assert_eq!(role_for("VZ Deathsquad 01"), Some("vz_hum_deathsquad"));
        assert_eq!(role_for("VZ Hind 01"), None);
        assert_eq!(role_for("Carmona"), None);
    }

    #[test]
    fn a_layers_people_stand_while_it_is_in() {
        let mut physics = ground();
        let mut people = People::default();
        people.kinds.push(kind("vz_hum_soldier", false));
        // A renamed guard: its template says who it is.
        let guard = SpawnPoint {
            template: Some("VZ Soldier".into()),
            layer: Some(0xc0),
            ..spawn(5, "vza_villageguard_2", 10.0)
        };
        // A garrison of a static sub-layer the game may not load.
        let garrison = SpawnPoint {
            layer: Some(0x51),
            static_layer: true,
            ..spawn(6, "VZ Soldier 04", 20.0)
        };
        let spawns = [guard, garrison];
        let tick = |people: &mut People, physics: &mut Physics| {
            people.update(physics, &spawns, Vec3::ZERO, None, 1.0 / 60.0);
            people
                .npcs
                .iter()
                .filter_map(|n| n.spawn)
                .collect::<Vec<_>>()
        };
        people.state_layers = Some(HashSet::new());
        assert!(tick(&mut people, &mut physics).is_empty());
        people.state_layers = Some(HashSet::from([0xc0]));
        assert_eq!(tick(&mut people, &mut physics), [5]);
        people.state_layers = Some(HashSet::from([0xc0, 0x51]));
        assert_eq!(tick(&mut people, &mut physics), [5, 6]);
        people.state_layers = Some(HashSet::new());
        assert!(tick(&mut people, &mut physics).is_empty());
        // Not knowing which layers are in, they stand.
        people.state_layers = None;
        assert_eq!(tick(&mut people, &mut physics), [5, 6]);
    }

    #[test]
    fn story_characters_never_run() {
        let post = Brain::Post { facing: 0.0 };
        let mut him = Npc::new(0, &rig(), Vec3::new(0.3, 0.0, 12.0), 0.0, post, Some(7));
        him.soldier = false;
        him.held = true;
        him.landed = true;
        him.panic(Vec3::new(-5.0, 0.0, 12.0));
        assert!(him.flee.is_none(), "not from gunfire");
        // Nor from a car coming at him, 12 m ahead of it.
        let mut people = People::default();
        people.npcs.push(him);
        let bounds = (Vec3::new(-1.0, 0.0, -2.0), Vec3::new(1.0, 1.5, 2.0));
        people.traffic(
            &[(Mat4::IDENTITY, Vec3::new(0.0, 0.0, 10.0), bounds, 1000.0)],
            None,
        );
        assert!(people.npcs[0].alive && people.npcs[0].flee.is_none());
        assert!(people.take_shouts().is_empty(), "nor shouts");
    }

    #[test]
    fn other_armies_and_passers_by_in_their_own_models_dont_stand() {
        let mut physics = ground();
        let mut people = People::default();
        let own = |entity, template: &str, model| SpawnPoint {
            template: Some(template.into()),
            model: Some(model),
            ..spawn(entity, &format!("{template} 01"), 10.0)
        };
        let spawns = [
            own(1, "Allied Soldier", 0x11),
            own(2, "GR Defender", 0x12),
            own(3, "VZ Tank Crew", 0x13),
        ];
        people.update(&mut physics, &spawns, Vec3::ZERO, None, 1.0 / 60.0);
        // Only the Venezuelan's model is asked for: a crewman fights.
        assert_eq!(people.wanted, [(0x13, "VZ Tank Crew".to_owned())]);
        people.wanted.clear();
        people.add_kind(kind("VZ Tank Crew", true));
        people.update(&mut physics, &spawns, Vec3::ZERO, None, 1.0 / 60.0);
        let stood: Vec<(Option<u32>, bool, bool)> = people
            .npcs
            .iter()
            .map(|n| (n.spawn, n.soldier, n.held))
            .collect();
        assert_eq!(stood, [(Some(3), true, false)]);
        // A mission's target stands whatever army he is.
        people.mission_soldiers.insert(1);
        people.update(&mut physics, &spawns, Vec3::ZERO, None, 1.0 / 60.0);
        assert_eq!(people.wanted, [(0x11, "Allied Soldier".to_owned())]);
    }

    #[test]
    fn a_story_character_made_a_target_fights() {
        let mut physics = ground();
        let mut people = People::default();
        people.kinds.push(kind("Carmona", true));
        let spawns = [SpawnPoint {
            template: Some("Carmona".into()),
            model: Some(0xbf83),
            ..spawn(7, "Carmona_VzaCon001", 10.0)
        }];
        people.update(&mut physics, &spawns, Vec3::ZERO, None, 1.0 / 60.0);
        assert!(people.npcs[0].held && !people.npcs[0].soldier);
        people.mission_soldiers.insert(7);
        people.update(&mut physics, &spawns, Vec3::ZERO, None, 1.0 / 60.0);
        assert_eq!(people.npcs.len(), 1);
        assert!(people.npcs[0].soldier && !people.npcs[0].held);
    }

    #[test]
    fn story_characters_stand_where_the_scripts_put_them() {
        let mut physics = ground();
        let carmona = || {
            let mut people = People::default();
            people.kinds.push(kind("Carmona", true));
            people
        };
        let spawns = [SpawnPoint {
            template: Some("Carmona".into()),
            model: Some(0xbf83),
            route: vec![Vec3::new(10.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 20.0)],
            ..spawn(7, "Carmona_VzaCon001", 10.0)
        }];
        let to = Vec3::new(30.0, 0.0, 5.0);
        let flat = |p: Vec3| Vec2::new(p.x, p.z);
        let mut people = carmona();
        people.update(&mut physics, &spawns, Vec3::ZERO, None, 1.0 / 60.0);
        people.put(7, to);
        for _ in 0..120 {
            people.update(&mut physics, &spawns, Vec3::ZERO, None, 1.0 / 60.0);
        }
        let him = &people.npcs[0];
        assert!(flat(him.character.position).distance(flat(to)) < 0.1);
        assert!(matches!(him.brain, Brain::Post { .. }), "he stays put");
        // Put before he stands: he comes there, not to his layer's place.
        let mut people = carmona();
        people.put(7, to);
        people.update(&mut physics, &spawns, Vec3::ZERO, None, 1.0 / 60.0);
        let him = &people.npcs[0];
        assert!(flat(him.character.position).distance(flat(to)) < 0.1);
        assert!(matches!(him.brain, Brain::Post { .. }));
    }

    /// A rifle as a template's `EquipmentLink`s hand it.
    fn rifle() -> HandGun {
        HandGun {
            template: 0x8000_0456,
            model: Some(0x98),
            rate_of_fire: 487.0,
            clip: 30,
            reserve_clips: 7,
            damage: 8.0,
            damage_key: None,
            hint: None,
        }
    }

    /// A kind of `role`: in a template's own model (`named`), or the
    /// catalogue's.
    fn kind(role: &str, named: bool) -> PersonKind {
        PersonKind {
            role: role.into(),
            named,
            gun: None,
            model: empty_model(),
            rig: rig(),
        }
    }

    /// A spawn named `name` at (x, 0, 0), of no template, model or layer.
    fn spawn(entity: u32, name: &str, x: f32) -> SpawnPoint {
        SpawnPoint {
            entity,
            name: name.into(),
            template: None,
            position: Vec3::new(x, 0.0, 0.0),
            rotation: Quat::IDENTITY,
            route: Vec::new(),
            layer: None,
            static_layer: false,
            model: None,
            list: Vec::new(),
            ai: None,
            gun: None,
            cycle: None,
        }
    }

    #[test]
    fn alert_soldiers_turn_and_fire_civilians_run() {
        let physics = ground();
        let mut rng = Rng::default();
        let dt = 1.0 / 60.0;
        let post = Brain::Post { facing: 0.0 };
        let mut soldier = Npc::new(0, &rig(), Vec3::new(0.0, 1.0, 0.0), 0.0, post, Some(1));
        soldier.update(&physics, &mut rng, None, dt);
        soldier.alert = true;
        // The player off to +x: he turns that way and fires.
        let threat = Some(Vec3::new(30.0, 1.2, 0.0));
        let mut shots = 0;
        for _ in 0..60 * 5 {
            soldier.update(&physics, &mut rng, threat, dt);
            shots += usize::from(std::mem::take(&mut soldier.fires));
        }
        assert!((soldier.facing - std::f32::consts::FRAC_PI_2).abs() < 0.05);
        // In bursts of three to five.
        assert!((6..=30).contains(&shots), "{shots}");
        // Out of range: he stands down.
        soldier.update(&physics, &mut rng, Some(Vec3::new(500.0, 0.0, 0.0)), dt);
        assert!(!soldier.alert);

        let stroll = Brain::Wander {
            home: Vec3::ZERO,
            target: Vec3::ZERO,
            wait: 10.0,
        };
        let mut civilian = Npc::new(0, &rig(), Vec3::new(0.0, 1.0, 0.0), 0.0, stroll, None);
        civilian.update(&physics, &mut rng, None, dt);
        civilian.panic(Vec3::new(-5.0, 0.0, 0.0));
        for _ in 0..60 * 2 {
            civilian.update(&physics, &mut rng, None, dt);
        }
        assert!(
            civilian.character.position.x > 7.0,
            "{}",
            civilian.character.position
        );
        // Hurt to death.
        assert!(!civilian.hurt(60.0, Vec3::ZERO));
        assert!(civilian.alive);
        assert!(civilian.hurt(60.0, Vec3::ZERO));
        assert!(!civilian.alive);
        assert!(!civilian.hurt(60.0, Vec3::ZERO), "only dies once");
    }

    #[test]
    fn soldiers_see_her_in_front_and_chase_where_she_was() {
        let physics = ground();
        let mut rng = Rng::default();
        let dt = 1.0 / 60.0;
        let post = || Brain::Post { facing: 0.0 };
        let mut soldier = Npc::new(0, &rig(), Vec3::new(0.0, 1.0, 0.0), 0.0, post(), Some(1));
        soldier.update(&physics, &mut rng, None, dt);
        // Behind him (he faces +z, 180 degrees of view): unseen.
        for _ in 0..60 {
            soldier.update(&physics, &mut rng, Some(Vec3::new(0.0, 1.2, -20.0)), dt);
        }
        assert!(!soldier.alert, "out of his view");
        // In front, 40 m off: seen at his next look, and he turns on her.
        for _ in 0..15 {
            soldier.update(&physics, &mut rng, Some(Vec3::new(5.0, 1.2, 40.0)), dt);
        }
        assert!(soldier.alert, "seen in front");
        // A pacifist never does.
        let mut calm = Npc::new(0, &rig(), Vec3::new(0.0, 1.0, 0.0), 0.0, post(), Some(2));
        calm.ai.flags.pacifist = true;
        calm.update(&physics, &mut rng, None, dt);
        for _ in 0..30 {
            calm.update(&physics, &mut rng, Some(Vec3::new(0.0, 1.2, 10.0)), dt);
        }
        assert!(!calm.alert);
        // Gone past his sight: he runs (at his PursueSpeed) to where he
        // last saw her, and doesn't fire.
        let far = Some(Vec3::new(0.0, 1.2, 90.0));
        soldier.fires = false;
        let start = soldier.character.position;
        let mut shots = 0;
        for _ in 0..60 {
            soldier.update(&physics, &mut rng, far, dt);
            shots += usize::from(std::mem::take(&mut soldier.fires));
        }
        let gone = soldier.character.position.z - start.z;
        let pursue = SPRINT * soldier.ai.pursue_speed;
        assert!(
            gone > pursue * 0.6 && gone < pursue * 1.1,
            "{gone} vs {pursue}"
        );
        assert_eq!(shots, 0, "out of firing range");
    }

    #[test]
    fn a_soldier_lobs_a_grenade_where_she_hides() {
        let physics = ground();
        let mut rng = Rng::default();
        let dt = 1.0 / 60.0;
        let post = Brain::Post { facing: 0.0 };
        let mut soldier = Npc::new(0, &rig(), Vec3::new(0.0, 1.0, 0.0), 0.0, post, Some(1));
        soldier.ai.pursue_speed = 0.0;
        soldier.ai.combat_speed = 0.0;
        soldier.update(&physics, &mut rng, None, dt);
        // Seen 15 m off, then gone out of his sight.
        for _ in 0..30 {
            soldier.update(&physics, &mut rng, Some(Vec3::new(0.0, 1.2, 15.0)), dt);
        }
        assert!(soldier.alert);
        // Hidden where he saw her (he can't see through the wall there).
        let mut wall = ground();
        let shape = CollisionShape::mesh(
            &[
                Vec3::new(-5.0, 0.0, 12.0),
                Vec3::new(5.0, 0.0, 12.0),
                Vec3::new(5.0, 4.0, 12.0),
                Vec3::new(-5.0, 4.0, 12.0),
            ],
            &[[0, 1, 2], [0, 2, 3], [0, 2, 1], [0, 3, 2]],
        )
        .unwrap();
        wall.add_static(StaticKey::Cell(2), &shape, Vec3::ZERO, Quat::IDENTITY);
        wall.step(dt);
        let mut thrown = Vec::new();
        for _ in 0..60 * 6 {
            soldier.update(&wall, &mut rng, Some(Vec3::new(0.0, 1.2, 15.0)), dt);
            thrown.extend(soldier.throws.take());
        }
        assert_eq!(thrown.len(), 1, "one, then he waits");
        assert!(thrown[0].distance(Vec3::new(0.0, 1.2, 15.0)) < 0.1);
        // NoGrenades: none.
        let post = Brain::Post { facing: 0.0 };
        let mut held = Npc::new(0, &rig(), Vec3::new(0.0, 1.0, 0.0), 0.0, post, Some(2));
        held.ai.pursue_speed = 0.0;
        held.ai.flags.no_grenades = true;
        held.update(&physics, &mut rng, None, dt);
        for _ in 0..30 {
            held.update(&physics, &mut rng, Some(Vec3::new(0.0, 1.2, 15.0)), dt);
        }
        for _ in 0..60 * 6 {
            held.update(&wall, &mut rng, Some(Vec3::new(0.0, 1.2, 15.0)), dt);
            assert!(held.throws.take().is_none());
        }
        // Gone off (driven away): none either.
        let post = Brain::Post { facing: 0.0 };
        let mut left = Npc::new(0, &rig(), Vec3::new(0.0, 1.0, 0.0), 0.0, post, Some(3));
        left.ai.pursue_speed = 0.0;
        left.update(&physics, &mut rng, None, dt);
        for _ in 0..30 {
            left.update(&physics, &mut rng, Some(Vec3::new(0.0, 1.2, 15.0)), dt);
        }
        for _ in 0..60 * 6 {
            left.update(&physics, &mut rng, Some(Vec3::new(0.0, 1.2, 95.0)), dt);
            assert!(left.throws.take().is_none());
        }
    }

    #[test]
    fn smarter_soldiers_call_her_out_to_those_near() {
        let mut physics = ground();
        let dt = 1.0 / 60.0;
        let run = |smart: bool| {
            let mut people = People::default();
            people.smarter.enabled = smart;
            let mut add = |facing: f32, at: Vec3, id| {
                let post = Brain::Post { facing };
                let mut n = Npc::new(0, &rig(), at, facing, post, Some(id));
                n.landed = true;
                people.npcs.push(n);
            };
            // One facing her, one 20 m off facing away.
            add(0.0, Vec3::new(0.0, 0.0, 0.0), 1);
            add(std::f32::consts::PI, Vec3::new(20.0, 0.0, 0.0), 2);
            people
        };
        for smart in [false, true] {
            let mut people = run(smart);
            for _ in 0..20 {
                people.update(
                    &mut physics,
                    &[],
                    Vec3::ZERO,
                    Some(Vec3::new(0.0, 1.2, 30.0)),
                    dt,
                );
            }
            assert!(people.npcs[0].alert, "he sees her");
            assert_eq!(people.npcs[1].alert, smart, "told only when smarter");
        }
    }

    #[test]
    fn cars_knock_people_flying_and_the_rest_dive_aside() {
        let stroll = || Brain::Wander {
            home: Vec3::ZERO,
            target: Vec3::ZERO,
            wait: 10.0,
        };
        let mut people = People::default();
        for z in [1.5, 12.0, -6.0] {
            let mut n = Npc::new(0, &rig(), Vec3::new(0.3, 0.0, z), 0.0, stroll(), None);
            n.landed = true;
            people.npcs.push(n);
        }
        // A car at the origin doing 10 m/s along +z.
        let bounds = (Vec3::new(-1.0, 0.0, -2.0), Vec3::new(1.0, 1.5, 2.0));
        let car = (Mat4::IDENTITY, Vec3::new(0.0, 0.0, 10.0), bounds, 1000.0);
        people.traffic(&[car], None);
        let [hit, ahead, behind] = &people.npcs[..] else {
            unreachable!()
        };
        assert!(!hit.alive && hit.fling.is_some_and(|v| v.z > 10.0 && v.y > 2.0));
        assert!(ahead.alive && ahead.flee.is_some(), "dives aside");
        assert!(behind.alive && behind.flee.is_none());
        // Parked, nobody minds.
        let mut people = People::default();
        let mut n = Npc::new(0, &rig(), Vec3::new(0.3, 0.0, 1.5), 0.0, stroll(), None);
        n.landed = true;
        people.npcs.push(n);
        people.traffic(&[(Mat4::IDENTITY, Vec3::ZERO, bounds, 1000.0)], None);
        assert!(people.npcs[0].alive);
    }

    // With the damage table, a car hurts as its ram, times what people
    // take of it, times their row's `Car` cell: a tonne at 3 m/s does 45
    // to a `Human` (3x), who is shoved aside and not hurt again at once;
    // two tonnes at 8 m/s do 240 and kill; a `HumanArmor` (1x) takes 80
    // of that and lives.
    #[test]
    fn a_car_hurts_people_by_the_damage_table() {
        let (human, armor, car_key) = (
            m2_assets::name_hash("Human"),
            m2_assets::name_hash("HumanArmor"),
            m2_assets::name_hash("Car"),
        );
        let mut data = vec![0u8; 0x48];
        let mut put = |v: u32| data.extend_from_slice(&v.to_le_bytes());
        put(2);
        put(1);
        put(human);
        put(armor);
        put(car_key);
        put(3.0f32.to_bits());
        put(1.0f32.to_bits());
        let table = m2_engine::tuning::DamageTable::parse(&data).expect("table");
        let stroll = || Brain::Wander {
            home: Vec3::ZERO,
            target: Vec3::ZERO,
            wait: 10.0,
        };
        let bounds = (Vec3::new(-1.0, 0.0, -2.0), Vec3::new(1.0, 1.5, 2.0));
        let person = |modifier: Option<u32>| {
            let mut n = Npc::new(0, &rig(), Vec3::new(0.3, 0.0, 1.5), 0.0, stroll(), None);
            n.landed = true;
            n.ai.modifier = modifier;
            n
        };
        let mut people = People::default();
        people.npcs.push(person(None));
        let slow = (Mat4::IDENTITY, Vec3::new(0.0, 0.0, 3.0), bounds, 1000.0);
        assert_eq!(people.traffic(&[slow], Some(&table)), 0);
        let n = &people.npcs[0];
        assert!(n.alive && n.hp == 55.0 && n.rammed == RAM_GAP);
        assert!(n.character.velocity.x > 0.0, "shoved off its line");
        people.traffic(&[slow], Some(&table));
        assert_eq!(people.npcs[0].hp, 55.0, "not again at once");
        // Faster and heavier: a soldier dies, a heavy lives.
        let fast = (Mat4::IDENTITY, Vec3::new(0.0, 0.0, 8.0), bounds, 2000.0);
        let mut people = People::default();
        people.npcs.push(person(Some(human)));
        people.npcs.push(person(Some(armor)));
        for n in &mut people.npcs {
            n.soldier = true;
        }
        assert_eq!(people.traffic(&[fast], Some(&table)), 1);
        assert!(!people.npcs[0].alive);
        assert!(people.npcs[1].alive && people.npcs[1].hp == 20.0);
    }

    // Running from a car into a wall, they run along it, not stand
    // against it; told to run again, they keep going their way.
    #[test]
    fn runners_turn_along_a_wall() {
        let mut physics = ground();
        let wall = [
            Vec3::new(1.2, -1.0, -50.0),
            Vec3::new(1.2, -1.0, 50.0),
            Vec3::new(1.2, 6.0, 50.0),
            Vec3::new(1.2, 6.0, -50.0),
        ];
        let shape = CollisionShape::mesh(&wall, &[[0, 1, 2], [0, 2, 3]]).unwrap();
        physics.add_static(StaticKey::Cell(2), &shape, Vec3::ZERO, Quat::IDENTITY);
        physics.step(1.0 / 60.0);
        let mut rng = Rng::default();
        let dt = 1.0 / 60.0;
        let stroll = Brain::Wander {
            home: Vec3::ZERO,
            target: Vec3::ZERO,
            wait: 10.0,
        };
        let mut n = Npc::new(0, &rig(), Vec3::new(0.0, 1.0, 0.0), 0.0, stroll, None);
        n.update(&physics, &mut rng, None, dt);
        for _ in 0..60 * 2 {
            // A car along the road at x = -3, level with them.
            let danger = Vec3::new(-3.0, 0.0, n.character.position.z);
            n.panic(danger);
            n.update(&physics, &mut rng, None, dt);
        }
        let p = n.character.position;
        assert!(p.x < 1.0 && p.z.abs() > 6.0, "{p}");
    }

    // Hit by her from behind a wall, a soldier chasing her goes round it
    // and comes to see her, not stand against it.
    #[test]
    fn a_chase_goes_round_a_wall() {
        let mut physics = ground();
        let shape = CollisionShape::mesh(
            &[
                Vec3::new(-5.0, -1.0, 6.0),
                Vec3::new(5.0, -1.0, 6.0),
                Vec3::new(5.0, 4.0, 6.0),
                Vec3::new(-5.0, 4.0, 6.0),
            ],
            &[[0, 1, 2], [0, 2, 3], [0, 2, 1], [0, 3, 2]],
        )
        .unwrap();
        physics.add_static(StaticKey::Cell(2), &shape, Vec3::ZERO, Quat::IDENTITY);
        let dt = 1.0 / 60.0;
        physics.step(dt);
        let mut rng = Rng::default();
        let post = Brain::Post { facing: 0.0 };
        let mut n = Npc::new(0, &rig(), Vec3::new(0.0, 1.0, 0.0), 0.0, post, Some(1));
        n.update(&physics, &mut rng, None, dt);
        let her = Vec3::new(0.0, 1.2, 14.0);
        assert!(!n.sees(&physics, her, true));
        n.hurt(1.0, Vec3::ZERO);
        let mut saw = false;
        for _ in 0..60 * 12 {
            n.update(&physics, &mut rng, Some(her), dt);
            saw |= n.sees(&physics, her, true);
        }
        assert!(n.alert, "still fighting");
        assert!(saw, "came round: {}", n.character.position);
    }

    #[test]
    fn rng_stays_in_range() {
        let mut r = Rng::default();
        for _ in 0..1000 {
            let x = r.range(2.0, 6.0);
            assert!((2.0..6.0).contains(&x));
            let p = r.ring(15.0, 120.0);
            let d = p.length();
            assert!((14.99..120.01).contains(&d), "{d}");
        }
    }

    #[test]
    fn a_driver_put_out_stands_by_the_door_runs_and_goes_out_of_range() {
        let mut physics = ground();
        let mut people = People::default();
        people.add_kind(kind("Civ Driver", true));
        let door = Vec3::new(2.0, 1.0, 0.0);
        let her = Vec3::new(0.0, 1.0, 0.0);
        assert!(people.put_out("Civ Driver", Some(1), Some(vec![true]), door, her));
        // One whose kind isn't in can't be drawn.
        assert!(!people.put_out("VZ Nobody", None, None, door, her));
        assert_eq!(people.npcs.len(), 1);
        let n = &people.npcs[0];
        assert!(!n.soldier && n.put_out);
        assert_eq!(n.outfit, vec![true], "the looks they had seated");
        for _ in 0..120 {
            people.update(&mut physics, &[], her, None, 1.0 / 60.0);
        }
        let p = people.npcs[0].character.position;
        assert!(p.x > door.x + 1.0, "ran from her: {p}");
        // No spawn keeps them: out of range, they go.
        let far = her + Vec3::X * (INACTIVE + 50.0);
        people.update(&mut physics, &[], far, None, 1.0 / 60.0);
        assert!(people.npcs.is_empty());
    }

    /// A wall `half` metres either way along x, 3 m high, at z.
    fn wall(physics: &mut Physics, z: f32, half: f32) {
        let shape = CollisionShape::cuboid(Vec3::new(half, 1.5, 0.2));
        physics.add_static(
            StaticKey::Cell(2),
            &shape,
            Vec3::new(0.0, 1.5, z),
            Quat::IDENTITY,
        );
        physics.step(1.0 / 60.0);
    }

    // Playtest 8: the villa's waves stuck outside, on walls their paths
    // go over. A runner the scripts sent gets to the route's end through
    // a wall, and doesn't turn to fight on the way (she in sight).
    #[test]
    fn a_runner_the_scripts_sent_gets_past_a_wall_to_the_end() {
        let mut physics = ground();
        wall(&mut physics, 5.0, 20.0);
        let route = vec![Vec3::new(0.0, 0.0, 10.0), Vec3::new(3.0, 0.0, 12.0)];
        let mut people = People::default();
        people.kinds.push(kind("vz_hum_soldier", false));
        people.walk(1, route.clone());
        let spawns = [spawn(1, "VZ Soldier 01", 0.0)];
        let her = Vec3::new(0.0, 1.3, -6.0);
        for _ in 0..60 * 15 {
            people.update(&mut physics, &spawns, Vec3::ZERO, Some(her), 1.0 / 60.0);
        }
        let n = &people.npcs[0];
        let end = route[1];
        assert!(
            (n.character.position - end).with_y(0.0).length() < 1.0,
            "{}",
            n.character.position
        );
        assert!(people.checks.arrived.contains(&1));
        assert!(people.checks.stuck.is_empty(), "{:?}", people.checks);
    }

    // Playtest 8: with the island's soldiers at the most about, the
    // villa's waves never stood. Those the scripts sent come anyway.
    #[test]
    fn those_the_scripts_sent_come_though_the_soldiers_are_at_the_most() {
        let mut physics = ground();
        let mut people = People::default();
        people.kinds.push(kind("vz_hum_soldier", false));
        let mut spawns: Vec<SpawnPoint> = (0..MAX_SOLDIERS as u32)
            .map(|i| spawn(100 + i, "VZ Soldier", 5.0 + i as f32))
            .collect();
        people.update(&mut physics, &spawns, Vec3::ZERO, None, 1.0 / 60.0);
        assert_eq!(people.npcs.len(), MAX_SOLDIERS);
        spawns.push(spawn(1, "VZ Soldier wave", 60.0));
        spawns.push(spawn(2, "VZ Soldier other", 61.0));
        people.walk(1, vec![Vec3::new(0.0, 0.0, 10.0)]);
        people.update(&mut physics, &spawns, Vec3::ZERO, None, 1.0 / 60.0);
        let came: Vec<u32> = people
            .npcs
            .iter()
            .filter_map(|n| n.spawn)
            .filter(|&e| e < 100)
            .collect();
        assert_eq!(came, [1]);
    }

    // Playtest 8: soldiers stood under the sea. Placed over water whose
    // floor is the sea bed (a dock not in yet), they don't land; and a
    // patrol into the sea stops at its edge.
    #[test]
    fn nobody_lands_on_the_sea_bed_or_walks_out_into_the_sea() {
        let mut physics = ground();
        physics.set_water(2.0);
        let mut rng = Rng::default();
        let mut npc = Npc::new(
            0,
            &rig(),
            Vec3::new(0.0, 3.0, 0.0),
            0.0,
            Brain::Post { facing: 0.0 },
            Some(1),
        );
        for _ in 0..30 {
            npc.update(&physics, &mut rng, None, 1.0 / 60.0);
        }
        assert!(!npc.landed);
        // A shore: dry ground at 0 for z < 5, the sea bed 3 m down
        // beyond, the sea at -1.
        let mut physics = Physics::new();
        let floor = |p: &mut Physics, key: u32, z0: f32, z1: f32, y: f32| {
            let v = [
                Vec3::new(-50.0, y, z0),
                Vec3::new(50.0, y, z0),
                Vec3::new(50.0, y, z1),
                Vec3::new(-50.0, y, z1),
            ];
            let shape = CollisionShape::mesh(&v, &[[0, 2, 1], [0, 3, 2]]).unwrap();
            p.add_static(StaticKey::Cell(key), &shape, Vec3::ZERO, Quat::IDENTITY);
        };
        floor(&mut physics, 1, -50.0, 5.0, 0.0);
        floor(&mut physics, 2, 5.0, 50.0, -3.0);
        physics.step(1.0 / 60.0);
        physics.set_water(-1.0);
        let brain = Brain::Patrol {
            route: vec![Vec3::ZERO, Vec3::new(0.0, -3.0, 20.0)],
            next: 1,
            step: 1,
            wait: 0.0,
        };
        let mut npc = Npc::new(0, &rig(), Vec3::new(0.0, 0.5, 0.0), 0.0, brain, Some(2));
        for _ in 0..60 * 10 {
            npc.update(&physics, &mut rng, None, 1.0 / 60.0);
        }
        assert!(npc.landed);
        let p = npc.character.position;
        assert!(p.y >= -1.0 - WET && p.z < 6.0, "{p}");
    }

    // Playtest 8: soldiers turning on the spot. Facing a new way each tick
    // while standing still counts as spinning; one turn to face her
    // doesn't.
    #[test]
    fn turning_round_and_round_on_the_spot_counts_as_spinning() {
        let mut checks = Checks::default();
        let mut npc = Npc::new(
            0,
            &rig(),
            Vec3::ZERO,
            0.0,
            Brain::Post { facing: 0.0 },
            Some(7),
        );
        npc.landed = true;
        // Half a turn, once.
        let before = npc.facing;
        npc.facing = std::f32::consts::PI;
        checks.count(&mut npc, before, None, 1.0 / 60.0);
        for _ in 0..60 * 3 {
            let f = npc.facing;
            checks.count(&mut npc, f, None, 1.0 / 60.0);
        }
        assert!(checks.spinners.is_empty());
        // Round and round, 6 radians a second for 3 s.
        for _ in 0..60 * 3 {
            let f = npc.facing;
            npc.facing += 0.1;
            checks.count(&mut npc, f, None, 1.0 / 60.0);
        }
        assert!(checks.spinners.contains(&7));
        assert!(checks.spinning > 1.0, "{}", checks.spinning);
    }

    // Playtest 8 (his clip at 16:57): a soldier stood in the villa
    // garden's pool, water the game's heights give but the sea isn't.
    // Nobody lands in it nor walks into it.
    #[test]
    fn nobody_stands_in_a_pool_the_water_heights_give() {
        let mut physics = ground();
        // A pool 1.5 m deep over the ground's middle (x, z -16..16).
        let pool = m2_assets::water::WaterHeights {
            columns: 2,
            rows: 2,
            cell: 32.0,
            origin: [-16.0, -16.0],
            heights: vec![1.5; 4],
            wet: Vec::new(),
        };
        physics.set_inland(Arc::new(pool));
        let mut rng = Rng::default();
        let mut npc = Npc::new(
            0,
            &rig(),
            Vec3::new(0.0, 1.0, 0.0),
            0.0,
            Brain::Post { facing: 0.0 },
            Some(1),
        );
        for _ in 0..30 {
            npc.update(&physics, &mut rng, None, 1.0 / 60.0);
        }
        assert!(!npc.landed);
        // One beside it, sent across it, stops at its edge.
        let brain = Brain::Patrol {
            route: vec![Vec3::new(30.0, 0.0, 0.0), Vec3::ZERO],
            next: 1,
            step: 1,
            wait: 0.0,
        };
        let mut npc = Npc::new(0, &rig(), Vec3::new(30.0, 1.0, 0.0), 0.0, brain, Some(2));
        for _ in 0..60 * 15 {
            npc.update(&physics, &mut rng, None, 1.0 / 60.0);
        }
        assert!(npc.landed);
        assert!(
            npc.character.position.x > 15.0,
            "{}",
            npc.character.position
        );
    }

    // Playtest 8 (his clip at 14:00): soldiers stood flipping about as she
    // drove round them, the aim turning their whole drawn body onto her
    // behind them. With her behind him a soldier holds the rifle ready and
    // turns; he aims once she is before him.
    #[test]
    fn a_soldier_aims_only_at_her_before_him() {
        let physics = ground();
        let mut rng = Rng::default();
        let mut npc = Npc::new(
            0,
            &rig(),
            Vec3::new(0.0, 1.0, 0.0),
            0.0,
            Brain::Post { facing: 0.0 },
            Some(3),
        );
        npc.soldier = true;
        npc.update(&physics, &mut rng, None, 1.0 / 60.0);
        assert!(npc.landed);
        npc.alert = true;
        // Behind him (he faces +z).
        let behind = Vec3::new(0.0, 1.3, -20.0);
        npc.update(&physics, &mut rng, Some(behind), 1.0 / 60.0);
        assert!(npc.rig.aim.is_none());
        // Turned round to her (half a turn at his rate takes about 0.5 s).
        for _ in 0..60 {
            npc.update(&physics, &mut rng, Some(behind), 1.0 / 60.0);
        }
        assert!(npc.rig.aim.is_some());
    }
}
