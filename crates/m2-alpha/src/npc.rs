//! People on the island: soldiers at the posts and on the patrol routes the
//! entity layers give, and civilians wandering around the player.
//!
//! Who a spawner really spawns is not decoded, so a spawn's model is picked
//! from its entity name (`VZ Soldier ..`, `VZ Heavy ..`, `VZ Deathsquad ..`),
//! as in the browser viewer. The behaviour (standing at posts, walking
//! routes there and back with pauses, civilians strolling about) is
//! invented: the game's AI is not decoded. So is how they fight (see
//! [`combat`](crate::combat)): a soldier who hears gunfire close by or is
//! hit turns on the player and fires, civilians run.

use std::collections::HashSet;

use m2_engine::world::SpawnPoint;
use m2_engine::{Character, Gait, GpuModel, Physics, RagdollId};
use m2_math::glam::{Mat4, Vec2, Vec3};

use crate::scene::{Carry, Rig};

/// Soldiers come and go within these distances of the player, metres.
const ACTIVE: f32 = 450.0;
const INACTIVE: f32 = 500.0;
/// Most soldiers about at once.
const MAX_SOLDIERS: usize = 48;
/// Civilians about the player, and how far from her they stroll.
const CIVILIANS: usize = 24;
const CIVILIAN_RANGE: (f32, f32) = (15.0, 120.0);
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
/// Seconds between a soldier's shots: at least this, plus up to as much
/// again at random.
const FIRE_GAP: (f32, f32) = (0.9, 0.8);
const PAUSE: (f32, f32) = (2.0, 6.0);
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
    /// `vz_hum_soldier`, `civ_hum_female`, ...; a story character's
    /// template (`Carmona`).
    pub role: String,
    pub soldier: bool,
    /// A story character, in their template's own model: stands only
    /// where the layers place them, never dealt out as a passer-by.
    pub named: bool,
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

    /// Every clip, for rebinding.
    pub fn all(&self) -> impl Iterator<Item = usize> {
        [self.aim, self.ready, self.jump, self.die]
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
    /// The state layer that placed them, if not a static one: they go
    /// when it does.
    pub layer: Option<u32>,
    pub hp: f32,
    pub alive: bool,
    /// A soldier turned on the player.
    pub alert: bool,
    /// Set by a tick when a soldier fires; the fight takes the shot.
    pub fires: bool,
    cooldown: f32,
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
            hp: 100.0,
            alive: true,
            alert: false,
            fires: false,
            cooldown: 0.0,
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

    /// Runs from `danger` (civilians). Someone already running, and not
    /// towards it, keeps going the way they are.
    pub fn panic(&mut self, danger: Vec3) {
        if !self.alive || self.soldier {
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

    /// A soldier's tick in a fight with the player at `threat`: stand, turn
    /// to them, aim and fire. False when out of range (the fight is off).
    fn fight(&mut self, physics: &Physics, threat: Vec3, rng: &mut Rng, dt: f32) -> bool {
        let clips = self.clips;
        let to = threat - self.character.position;
        let distance = to.with_y(0.0).length();
        if distance > FIGHT_RANGE {
            return false;
        }
        physics.move_character(&mut self.character, Vec3::ZERO, None, dt);
        let off = self.turn_to(to.x.atan2(to.z), dt);
        self.rig.hold_rifle(Gait::Idle, &clips, Carry::Aim);
        self.rig.aim_along(self.facing, Some(threat - self.chest()));
        self.cooldown -= dt;
        if self.cooldown <= 0.0 && off < FIRE_ARC && distance < FIRE_RANGE {
            self.fires = true;
            self.cooldown = FIRE_GAP.0 + rng.range(0.0, FIRE_GAP.1);
        }
        self.rig.advance(dt);
        true
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
        if self.alert {
            match threat {
                Some(t) if self.fight(physics, t, rng, dt) => return,
                _ => self.alert = false,
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
    /// so (`vza_beachguard_1`): plain VZ soldiers.
    pub mission_soldiers: HashSet<u32>,
    /// The state layers in, by name hash: a state layer's people stand
    /// only while it is. `None` when that isn't known (all of them stand).
    pub state_layers: Option<HashSet<u32>>,
    /// Story characters' models to load, by their template: model and
    /// template name (taken by the scene).
    pub wanted: Vec<(u32, String)>,
    /// Models asked for so far, each by the template it is for: several
    /// templates share a model (the guerrillas' men and the GR
    /// Defenders), and each becomes a kind of its own.
    asked: HashSet<(u32, String)>,
    active: HashSet<u32>,
    rng: Rng,
}

impl People {
    /// The kind a spawn's entity name asks for: VZ soldiers by rank;
    /// spawns of vehicles, targets, buildings and other armies get none.
    pub fn kind_for(&self, name: &str) -> Option<usize> {
        let n = name.to_ascii_lowercase();
        let other = [
            "amx30", "scorpion", "hind", "m151", "boat", "target", "_bld_", "heli", "tank", "truck",
        ];
        if other.iter().any(|w| n.contains(w)) || !n.starts_with("vz ") {
            return None;
        }
        let role = if n.contains("deathsquad") || n.contains("mook") {
            "vz_hum_deathsquad"
        } else if n.contains("heavy") || n.contains("elite") {
            "vz_hum_soldierelite"
        } else {
            "vz_hum_soldier"
        };
        self.kinds.iter().position(|k| k.role == role)
    }

    /// Whether `npcs[i]` is a woman (her model's role says so,
    /// `civ_hum_*female`).
    pub fn is_woman(&self, i: usize) -> bool {
        self.npcs
            .get(i)
            .and_then(|n| self.kinds.get(n.kind))
            .is_some_and(|k| k.role.contains("female"))
    }

    /// The kind of a story character's spawn, once their model is in: a
    /// person template named in one word (`Carmona`, not `Allied
    /// Soldier`) with a model of its own. Asks for the model the first
    /// time.
    fn character(&mut self, s: &SpawnPoint) -> Option<usize> {
        let (template, model) = (s.template.as_deref()?, s.model?);
        if template.contains(' ') {
            return None;
        }
        if let Some(k) = self
            .kinds
            .iter()
            .position(|k| k.named && k.role == template)
        {
            return Some(k);
        }
        if self.asked.insert((model, template.to_owned())) {
            self.wanted.push((model, template.to_owned()));
        }
        None
    }

    /// Who a spawner of people sends out: one of its list by weight (a
    /// Venezuelan soldier as the island's own, anyone else in their
    /// template's model, asked for once). The factions' attitudes aren't
    /// in yet, so only the Venezuelans fight.
    fn listed(&mut self, s: &SpawnPoint) -> Option<usize> {
        let weights: Vec<(u32, f32)> = s
            .list
            .iter()
            .enumerate()
            .map(|(i, (_, _, w))| (i as u32, *w))
            .collect();
        // The whole list's models are asked for at once, so a spawner
        // isn't held up model by model.
        for (template, model, _) in &s.list {
            if self.list_kind(template).is_none() && self.asked.insert((*model, template.clone())) {
                self.wanted.push((*model, template.clone()));
            }
        }
        let roll = self.rng.range(0.0, 1.0);
        let (template, _, _) = &s.list[m2_engine::population::pick(&weights, roll)? as usize];
        self.list_kind(template)
    }

    /// The kind a spawner's list entry is, once its model is in.
    fn list_kind(&self, template: &str) -> Option<usize> {
        self.kind_for(template).or_else(|| {
            self.kinds
                .iter()
                .position(|k| k.named && k.role == template)
        })
    }

    pub fn add_kind(&mut self, kind: PersonKind) {
        if !kind.soldier && !kind.named {
            // Civilians are dealt out across the kinds in: start them
            // again so the new one gets its share.
            self.npcs.retain(|n| n.spawn.is_some());
        }
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
        for npc in &mut self.npcs {
            npc.update(physics, &mut self.rng, threat, dt);
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
    }

    /// Vehicles going through people: `cars` are (model to world,
    /// velocity, model-space hull box) of each one moving. Anyone a car's
    /// box meets is knocked flying, dead; civilians in its path (as wide
    /// as it is, and a little more) dive out of the way. The number of
    /// soldiers killed.
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

    /// Whether a spawn of state layer `layer` (`None`: a static one) can
    /// stand now: it is in, or which are in isn't known.
    pub fn layer_in(&self, layer: Option<u32>) -> bool {
        layer.is_none_or(|l| {
            self.state_layers
                .as_ref()
                .is_none_or(|set| set.contains(&l))
        })
    }

    fn populate(&mut self, physics: &mut Physics, spawns: &[SpawnPoint], focus: Vec3) {
        let far = |p: Vec3, d: f32| Vec2::new(p.x - focus.x, p.z - focus.z).length() > d;
        // Soldiers out of range, or whose layer went, go; civilians left
        // behind come back round the player.
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
        let civilian_kinds: Vec<usize> = (0..self.kinds.len())
            .filter(|&k| !self.kinds[k].soldier && !self.kinds[k].named)
            .collect();
        for n in self.npcs.iter_mut().filter(|n| n.spawn.is_none()) {
            if far(n.character.position, INACTIVE) {
                let home = focus + self.rng.ring(CIVILIAN_RANGE.0, CIVILIAN_RANGE.1);
                n.character.position = Vec3::new(home.x, focus.y, home.z);
                n.character.velocity = Vec3::ZERO;
                n.landed = false;
                n.alive = true;
                n.hp = 100.0;
                n.dead_for = 0.0;
                if let Some(doll) = n.ragdoll.take() {
                    physics.remove_ragdoll(doll);
                }
                n.fling = None;
                n.flee = None;
                n.rig.set_gait(Gait::Idle);
                n.brain = Brain::Wander {
                    home,
                    target: home,
                    wait: 0.0,
                };
            }
        }
        // Soldiers who come into range: a mission's targets first, then
        // the nearest.
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
            // Who: the template's name says (a renamed entity's own name
            // doesn't), else the entity's, else a mission's target is a
            // plain soldier; or a story character.
            let kind = s
                .template
                .as_deref()
                .and_then(|t| self.kind_for(t))
                .or_else(|| self.kind_for(&s.name))
                .or_else(|| target.then(|| self.kind_for("vz soldier")).flatten())
                .or_else(|| self.character(s))
                .or_else(|| self.listed(s));
            let Some(kind) = kind else {
                // A spawner whose list is still loading keeps its place,
                // so spawns further off don't take it meanwhile.
                if !s.list.is_empty() {
                    room = room.saturating_sub(1);
                }
                continue;
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
            // A story character doesn't fight.
            npc.soldier = self.kinds[kind].soldier;
            self.npcs.push(npc);
        }
        // Civilians, once their models are in.
        let civilians = self.npcs.len() - self.npcs.iter().filter(|n| n.spawn.is_some()).count();
        if !civilian_kinds.is_empty() {
            for i in civilians..CIVILIANS {
                let kind = civilian_kinds[i % civilian_kinds.len()];
                let home = focus + self.rng.ring(CIVILIAN_RANGE.0, CIVILIAN_RANGE.1);
                let at = Vec3::new(home.x, focus.y, home.z);
                let facing = self.rng.range(0.0, std::f32::consts::TAU);
                let brain = Brain::Wander {
                    home,
                    target: home,
                    wait: self.rng.range(0.0, PAUSE.1),
                };
                self.npcs.push(Npc::new(
                    kind,
                    &self.kinds[kind].rig,
                    at,
                    facing,
                    brain,
                    None,
                ));
            }
        }
    }
}

/// A small xorshift generator: where civilians go needs no more.
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
        people.kinds.push(PersonKind {
            role: "vz_hum_soldier".into(),
            soldier: true,
            named: false,
            model: empty_model(),
            rig: rig(),
        });
        let spawn = |entity, name: &str, x| SpawnPoint {
            entity,
            name: name.into(),
            template: None,
            position: Vec3::new(x, 0.0, 0.0),
            rotation: Quat::IDENTITY,
            route: Vec::new(),
            layer: None,
            model: None,
            list: Vec::new(),
        };
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
        assert!(people.npcs.iter().all(|n| n.spawn.is_none()));
    }

    #[test]
    fn spawners_send_out_whom_their_list_names() {
        let mut physics = ground();
        let mut people = People::default();
        let spawner = SpawnPoint {
            entity: 9,
            name: "PathSpawner_GurHQ_Peds".into(),
            template: Some("PathSpawner_GurHQ_Peds".into()),
            position: Vec3::new(10.0, 0.0, 0.0),
            rotation: Quat::IDENTITY,
            route: Vec::new(),
            layer: None,
            model: None,
            list: vec![
                ("Guerilla Soldier".into(), 0x6a11, 1.0),
                ("Guerilla Officer".into(), 0x6a12, 1.0),
                // Another template in the same model is a kind of its own.
                ("GR Defender (rifle)".into(), 0x6a11, 1.0),
            ],
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
            people.add_kind(PersonKind {
                role: role.into(),
                soldier: false,
                named: true,
                model: empty_model(),
                rig: rig(),
            });
        }
        people.update(&mut physics, &spawns, Vec3::ZERO, None, 1.0 / 60.0);
        assert_eq!(people.npcs.iter().filter(|n| n.spawn == Some(9)).count(), 1);
    }

    #[test]
    fn story_characters_stand_in_their_own_model() {
        let mut physics = ground();
        let mut people = People::default();
        let spawn = |entity, template: &str, model| SpawnPoint {
            entity,
            name: format!("{template}_VzaCon001"),
            template: Some(template.into()),
            position: Vec3::new(10.0, 0.0, 0.0),
            rotation: Quat::IDENTITY,
            route: Vec::new(),
            layer: None,
            model: Some(model),
            list: Vec::new(),
        };
        let spawns = [
            spawn(7, "Carmona", 0xbf83),
            spawn(8, "Allied Soldier", 0x45cc),
        ];
        people.update(&mut physics, &spawns, Vec3::ZERO, None, 1.0 / 60.0);
        // Nobody yet: his model is asked for (once), a soldier of another
        // army's isn't.
        assert!(people.npcs.is_empty());
        assert_eq!(people.wanted, [(0xbf83, "Carmona".to_owned())]);
        people.wanted.clear();
        people.update(&mut physics, &spawns, Vec3::ZERO, None, 1.0 / 60.0);
        assert!(people.wanted.is_empty());
        people.add_kind(PersonKind {
            role: "Carmona".into(),
            soldier: false,
            named: true,
            model: empty_model(),
            rig: rig(),
        });
        people.update(&mut physics, &spawns, Vec3::ZERO, None, 1.0 / 60.0);
        let him: Vec<_> = people.npcs.iter().filter(|n| n.spawn == Some(7)).collect();
        assert_eq!(him.len(), 1);
        assert!(!him[0].soldier, "he doesn't fight");
        // Never dealt out as a passer-by.
        assert!(people.npcs.iter().all(|n| n.spawn.is_some()));
    }

    #[test]
    fn a_state_layers_soldiers_stand_while_it_is_in() {
        let mut physics = ground();
        let mut people = People::default();
        people.kinds.push(PersonKind {
            role: "vz_hum_soldier".into(),
            soldier: true,
            named: false,
            model: empty_model(),
            rig: rig(),
        });
        // A renamed guard: its template says who it is.
        let guard = SpawnPoint {
            entity: 5,
            name: "vza_villageguard_2".into(),
            template: Some("VZ Soldier".into()),
            position: Vec3::new(10.0, 0.0, 0.0),
            rotation: Quat::IDENTITY,
            route: Vec::new(),
            layer: Some(0xc0),
            model: None,
            list: Vec::new(),
        };
        let spawns = [guard];
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
        people.state_layers = Some(HashSet::new());
        assert!(tick(&mut people, &mut physics).is_empty());
        // Not knowing which layers are in, they stand.
        people.state_layers = None;
        assert_eq!(tick(&mut people, &mut physics), [5]);
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
        assert!((2..=6).contains(&shots), "{shots}");
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
