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
//! fight. A story character (placed by a layer, not fighting: `Carmona`)
//! is the scripts': they stay where they are put. The behaviour (standing
//! at posts, walking routes there and back with pauses, running from
//! gunfire) is invented: the game's AI code is not decoded.
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

use std::collections::HashSet;

use m2_engine::tuning::HandGun;
use m2_engine::world::SpawnPoint;
use m2_engine::{AiProfile, BoundClip, Character, Gait, GpuModel, Physics, RagdollId};
use m2_math::glam::{Mat4, Quat, Vec2, Vec3};
use serde::{Deserialize, Serialize};

use crate::scene::{Carry, Rig};

/// Soldiers come and go within these distances of the player, metres.
const ACTIVE: f32 = 450.0;
const INACTIVE: f32 = 500.0;
/// Most soldiers about at once.
const MAX_SOLDIERS: usize = 48;
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
/// `FIRE_RANGE` once facing within `FIRE_ARC` radians of them.
const FIGHT_RANGE: f32 = 80.0;
const FIRE_RANGE: f32 = 70.0;
const FIRE_ARC: f32 = 0.3;
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
/// Between steps about in a fight: seconds (at least, plus up to), how
/// far a step goes, metres, and how close he pushes in to her (inferred).
const STEP_GAP: (f32, f32) = (2.0, 3.0);
const STEP: (f32, f32) = (3.0, 6.0);
const PUSH_TO: f32 = 15.0;
/// Seconds between a soldier's shots: at least this, plus up to as much
/// again at random.
const FIRE_GAP: (f32, f32) = (0.9, 0.8);
/// They fire in bursts: at least this many rounds, plus up to this many
/// more at random (invented, as no field names them), at their gun's
/// `RateOfFire`; this many seconds apart for those whose template names
/// no gun (the "Assault Rifle"'s 450 a minute, as the AK47 stands in).
const BURST: (u32, u32) = (3, 2);
const BURST_GAP: f32 = 60.0 / 450.0;
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
const RUN_OVER: f32 = 2.5;
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

    pub fn find(rig: &Rig) -> Self {
        let find = |hash| rig.clips.iter().position(|c| c.hash == hash);
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
        }
    }

    /// The clips carrying `event`: each one's index, when in it the event
    /// comes, and how far its first pose is from the rifle held ready
    /// (clip `ready`), nearest first.
    fn carrying(rig: &Rig, event: &str, ready: Option<usize>) -> Vec<Carrying> {
        let mut list: Vec<Carrying> = rig
            .clips
            .iter()
            .enumerate()
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

    /// A hero's: where a clip picked by name is not of the hero's own
    /// group ([`Rig::own`]) and the group has clips of that kind, the
    /// group's (heroes round 02: each group holds its hero's runs,
    /// jumps, landings, throws and low bashes, told apart by their events
    /// and root speeds); of several, the one starting nearest the rifle
    /// held ready. The high bash stays the butt smash picked from
    /// pictures (the groups' `meleebash` clips are not known to be one).
    pub fn own_group(mut self, rig: &Rig) -> Self {
        if rig.own.is_empty() {
            return self;
        }
        let own = |i: Option<usize>| i.is_some_and(|i| rig.own.contains(&rig.clips[i].hash));
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
        swap(&mut self.run_low, &|c| {
            has(c, "footstep_run") && (4.0..6.0).contains(&c.info.speed)
        });
        swap(&mut self.takeoff, &|c| {
            has(c, "jumptakeoff") && c.info.speed >= 1.0
        });
        swap(&mut self.takeoff_still, &|c| {
            has(c, "jumptakeoff") && c.info.speed < 0.5
        });
        swap(&mut self.land, &|c| {
            has(c, "jumpland") && !air(c) && c.info.speed >= 3.0 && c.info.duration >= 0.3
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
    /// A story character (placed by a layer, not fighting: `Carmona`):
    /// the scripts', they stay where they are put and never run.
    pub held: bool,
    /// The gun their template hands them (`None`: the AK47's "Assault
    /// Rifle" stands in): what they draw, and how fast and hard it fires.
    pub gun: Option<HandGun>,
    pub hp: f32,
    pub alive: bool,
    /// A soldier turned on the player.
    pub alert: bool,
    /// Set by a tick when a soldier fires; the fight takes the shot.
    pub fires: bool,
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
    /// Rounds left in the burst he is firing.
    burst: u32,
    /// Running from gunfire: where to, and for how much longer.
    flee: Option<(Vec3, f32)>,
    brain: Brain,
    /// The aim and fall clips in this person's rig.
    clips: FightClips,
    /// The right hand's node, where the rifle is held, and its attach
    /// point.
    pub hand: Option<usize>,
    pub attach: Option<usize>,
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
}

/// What a person shouts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shout {
    /// A soldier turning on the player.
    Alert,
    /// A civilian running from gunfire.
    Flee,
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
            gun: None,
            hp: 100.0,
            alive: true,
            alert: false,
            fires: false,
            cooldown: 0.0,
            ai: AiProfile::default(),
            last_seen: None,
            unseen: 0.0,
            look: 0.0,
            step: None,
            step_wait: 0.0,
            throws: None,
            grenade_wait: 0.0,
            smart: SmarterAi::default(),
            spotted: false,
            cover: None,
            hit: false,
            burst: 0,
            flee: None,
            brain,
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

    /// Runs from `danger` (those who don't fight, but story characters).
    /// Someone already running, and not towards it, keeps going the way
    /// they are.
    pub fn panic(&mut self, danger: Vec3) {
        if !self.alive || self.soldier || self.held {
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
    /// sees her within `FIRE_RANGE` he turns to her, aims and fires, and
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
        } else if !seen || distance > FIRE_RANGE {
            self.step = None;
            let flat = (goal - at).with_y(0.0);
            if flat.length() > CLOSE_ENOUGH {
                wanted = flat.normalize() * SPRINT * self.ai.pursue_speed;
            }
        } else {
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
        physics.move_character(&mut self.character, wanted, None, dt);
        let moved = Vec2::new(self.character.velocity.x, self.character.velocity.z).length();
        if wanted != Vec3::ZERO && moved < 0.2 {
            // Walked into something: stop stepping; a chase gives up on
            // that spot and stands looking.
            self.step = None;
            if !seen {
                self.last_seen = Some(at);
            }
        }
        let seen = seen && !hiding;
        let face = if seen || wanted == Vec3::ZERO {
            to.x.atan2(to.z)
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
        let carry = if seen { Carry::Aim } else { Carry::Ready };
        self.rig.hold_rifle(gait, &clips, carry);
        if seen {
            self.rig.aim_along(self.facing, Some(threat - self.chest()));
        }
        self.cooldown -= dt;
        if seen && self.cooldown <= 0.0 && off < FIRE_ARC && distance < FIRE_RANGE {
            self.fires = true;
            if self.burst == 0 {
                let more = rng.range(0.0, BURST.1 as f32 + 1.0) as u32;
                self.burst = BURST.0 + more.min(BURST.1);
            }
            self.burst -= 1;
            self.cooldown = if self.burst > 0 {
                self.burst_gap()
            } else {
                FIRE_GAP.0 + rng.range(0.0, FIRE_GAP.1)
            };
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
            if let Some(y) = hit {
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
        if self.alert {
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
        }
        physics.move_character(&mut self.character, wanted, None, dt);
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
                (None, Brain::Wander { home, target, .. }) => *target = *home + rng.disc(WANDER),
                (None, Brain::Patrol { wait, .. }) => *wait = 1.0,
                (None, Brain::Post { .. }) => {}
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
    /// The player's aim and fall clips.
    pub clips: FightClips,
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
    active: HashSet<u32>,
    rng: Rng,
    /// The enhanced, smarter soldiers' settings (off: the original's).
    pub smarter: SmarterAi,
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
    /// Nobody: a vehicle's spawn, a person of no model.
    Nobody,
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
    /// (by name) and a mission's targets fight.
    fn who(&mut self, s: &SpawnPoint, target: bool) -> Who {
        let names = [s.template.as_deref(), Some(s.name.as_str())];
        let fights = target || names.iter().flatten().any(|n| role_for(n).is_some());
        if let (Some(template), Some(model)) = (s.template.as_deref(), s.model) {
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
        let fights = role_for(template).is_some();
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
        let target = self.mission_soldiers.contains(&s.entity);
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
        self.populate(physics, spawns, focus);
        let mut callouts = Vec::new();
        for npc in &mut self.npcs {
            npc.smart = self.smarter;
            npc.update(physics, &mut self.rng, threat, dt);
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
    /// velocity, model-space hull box) of each one moving. Anyone a car's
    /// box meets is knocked flying, dead; those who don't fight in its
    /// path (as wide as it is, and a little more) dive out of the way, but
    /// story characters. The number of soldiers killed.
    pub fn traffic(&mut self, cars: &[(Mat4, Vec3, (Vec3, Vec3))]) -> usize {
        let mut killed = 0;
        for &(at, velocity, (lo, hi)) in cars {
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
                let pad = Vec3::new(0.35, 0.9, 0.35);
                if local.cmpge(lo - pad).all() && local.cmple(hi + pad).all() {
                    let push = velocity * 1.1 + Vec3::Y * (2.0 + speed * 0.15);
                    if n.hurt(1000.0, push) && n.soldier {
                        killed += 1;
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
        // People who come into range: a mission's targets first, then the
        // nearest.
        let soldiers = self.npcs.iter().filter(|n| n.spawn.is_some()).count();
        let mut near: Vec<(f32, &SpawnPoint)> = spawns
            .iter()
            .filter(|s| {
                !self.active.contains(&s.entity)
                    && !self.killed.contains(&s.entity)
                    && !far(s.position, ACTIVE)
                    && self.layer_in(s.layer)
            })
            .map(|s| (s.position.distance(focus), s))
            .collect();
        let targets = &self.mission_soldiers;
        near.sort_by(|a, b| {
            (!targets.contains(&a.1.entity))
                .cmp(&!targets.contains(&b.1.entity))
                .then(a.0.total_cmp(&b.0))
        });
        // A mission's targets come even when the island's soldiers fill
        // the place up.
        let mut room = MAX_SOLDIERS.saturating_sub(soldiers);
        for (_, s) in near {
            let target = self.mission_soldiers.contains(&s.entity);
            if room == 0 && !target {
                break;
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
                    room = room.saturating_sub(1);
                    continue;
                }
                Who::Nobody => continue,
            };
            room = room.saturating_sub(1);
            let forward = s.rotation * Vec3::Z;
            let facing = forward.x.atan2(forward.z);
            let brain = if s.route.len() >= 2 {
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
                s.position,
                facing,
                brain,
                Some(s.entity),
            );
            npc.layer = s.layer;
            if let Some(ai) = s.ai {
                npc.ai = ai;
            }
            // Their template's `Health` (a plain VZ soldier's 75 when none
            // speaks for them).
            npc.hp = npc.ai.health;
            npc.soldier = fights;
            // A story character the layers place is the scripts': where
            // they are put, they stay.
            npc.held = !fights && !listed;
            npc.gun = self.kinds[kind].gun;
            self.tell(s, &npc);
            self.npcs.push(npc);
        }
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
    use m2_engine::{Animator, CollisionShape, Skeleton, StaticKey};
    use m2_math::glam::Quat;
    use std::sync::Arc;

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
        assert!(people.npcs[0].soldier && people.npcs[0].gun.is_none());
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
        people.traffic(&[(Mat4::IDENTITY, Vec3::new(0.0, 0.0, 10.0), bounds)]);
        assert!(people.npcs[0].alive && people.npcs[0].flee.is_none());
        assert!(people.take_shouts().is_empty(), "nor shouts");
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
        let car = (Mat4::IDENTITY, Vec3::new(0.0, 0.0, 10.0), bounds);
        people.traffic(&[car]);
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
        people.traffic(&[(Mat4::IDENTITY, Vec3::ZERO, bounds)]);
        assert!(people.npcs[0].alive);
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
}
