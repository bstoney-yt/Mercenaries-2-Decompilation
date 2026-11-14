//! Homing weapons: the lock a launcher builds on a vehicle, the missile's
//! steering after it, and the warnings the vehicle's driver hears.
//!
//! **From the 360 code:**
//! - The lock (0x82294318, run per homing weapon each frame by
//!   0x82294E00): the weapon asks a line-of-sight query for the target its
//!   cone test (0x822938C0) picks, and reads the answer a frame later
//!   (0x82295638). A new answer sets the time left to the weapon's
//!   `LockOnTime` (`HomingWeapon` +0x10) and sends `HomingLockStart`; the
//!   same answer again takes `dt` off it and sends `HomingLockUpdate` with
//!   the share done, `1 - left / LockOnTime` (1 once it runs out or when
//!   `LockOnTime` is 0). An empty answer resets the lock without a word,
//!   unless the cone finds nothing that frame too: then `HomingLockClear`
//!   goes for the target it had. A reset lock's time left is `f32::MAX` (0x820F8BFC).
//!   Of one holder's events a frame only the strongest goes (update over
//!   start over clear).
//! - Locked means a target with no time left (0x82293660 and the AI's gate
//!   in 0x822C95D8 / 0x822AE598 both read `+0x24 != 0 && +0x20 <= 0`).
//!   Firing (0x82293660) sends `HomingLockClear` to the target's player,
//!   then `HomingLaunched` if it was locked, and resets the lock: each
//!   missile is locked afresh.
//! - The cone (0x822938C0): a target counts when its signature of the
//!   weapon's kind (`HomingWeapon` +0: 1 ground, 2 air; `HomingTarget`
//!   +0 / +4) is at least the weapon's least (+4, none when 0), it is ahead
//!   (cosine >= 0), the point of the aim ray abreast of it (`d / cos`
//!   along) is within the weapon's range (+0xc), and the target lies
//!   within `tan(cone) * d / cos` of that point (+8, degrees). Its score is
//!   `45 cos + 5 (1 - d / range) + 50 * 0.1 * signature`; the best above 0
//!   wins.
//! - A holder with no player (an AI) widens the cone to at least 60 degrees
//!   (0x82294318: `max(cone, 60)` when the holder's player slot +0x1c is
//!   empty).
//! - An AI names its target to its homing weapon before firing (message 5
//!   to 0x82280D80, from 0x822C95D8 / 0x822AE598; 0x822AE598's lasts 3 s,
//!   0x820C7A20); while named, the lock runs against that target alone
//!   (0x82294B70), and the AI holds its fire until locked.
//! - The missile (0x82293280): it steers once 0.1 s old (0x820BB468): it
//!   turns its nose toward the target by at most its turn rate times `dt`
//!   (`HomingProjectile` +0, degrees a second), straight at it when nearer
//!   than that; a target more than its widest angle (+4, degrees) off the
//!   nose is lost and the missile flies on straight. The turn rate and
//!   widest angle are read as degrees and turned to radians when the
//!   missile is made (0x82391DA0, 0x82DE4968).
//!
//! **From samples** (the `AntiAir` object script's listing): the warnings.
//! Each locker coming on adds to its target's player's "targeting" count,
//! each one done (share 1) to "targeted"; the first of either starts its
//! cue (`ui_hud_radar_targeting_alert`, `ui_hud_sam_targeted`), the last
//! gone stops it. A new lock plays `ui_hud_radar_targeting_new_alert` once,
//! at most once a second (`knCueAlertCooldown`). A lock with no word for
//! 1 s is cleared (its `Event.Timer`). The shipped script's `_SetSound`
//! keeps only the alert's cooldown and plays nothing, and the cue names
//! are not in the executable: that the cues sound when its counts say is
//! **invented** from the script's names and counts.
//!
//! The script itself now runs too ([`ScriptSide`]): the lock events go to
//! `AntiAir`'s handlers with the guids the scripts know, and each missile
//! fired on a lock is an object of theirs while it flies, so
//! `HomingMissile` blips it (m2-game `mission::homing`).
//!
//! **Invented / approximated:** the target is a sphere of its vehicle's
//! bounds (0x822938C0 measures to its box); the line of sight is the port's
//! ray to it; soldiers aim straight at the vehicle (so their cone always
//! holds while they see it); a soldier's named target lasts the 3 s of
//! 0x822AE598 (0x822C95D8's own time is not read).

use m2_math::glam::{Quat, Vec3};

/// A reset lock's time left (`FLT_MAX`, 0x820F8BFC).
pub const CLEARED: f32 = f32::MAX;
/// The narrowest cone an AI holder's weapon has, degrees (0x82B7D884).
pub const AI_CONE: f32 = 60.0;
/// How long an AI's naming of its target lasts, s (0x820C7A20).
pub const NAMED: f32 = 3.0;
/// How old a missile is before it steers, s (0x820BB468).
pub const ARMING: f32 = 0.1;
/// The cone test's score weights: cosine, nearness, signature
/// (0x82B7C5EC, 0x820BAF84, 0x820BB468 x 0x820B7FB8).
const SCORE: [f32; 3] = [45.0, 5.0, 0.1 * 50.0];

/// The `AntiAir` script's cues and the alert's cooldown, s.
pub const CUE_TARGETING: &str = "ui_hud_radar_targeting_alert";
pub const CUE_TARGETED: &str = "ui_hud_sam_targeted";
pub const CUE_ALERT: &str = "ui_hud_radar_targeting_new_alert";
pub const ALERT_COOLDOWN: f32 = 1.0;
/// How long a lock goes with no word before the script drops it, s.
pub const HANGING: f32 = 1.0;

/// What a lock tells its target's player (the HUD events' names).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LockEvent {
    /// `HomingLockStart`.
    Start,
    /// `HomingLockUpdate`, with `nPercent`: the share done, 0 to 1.
    Update(f32),
    /// `HomingLockClear`.
    Clear,
    /// `HomingLaunched`.
    Launched,
}

impl LockEvent {
    /// Its HUD event's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Start => "HomingLockStart",
            Self::Update(_) => "HomingLockUpdate",
            Self::Clear => "HomingLockClear",
            Self::Launched => "HomingLaunched",
        }
    }
}

/// One homing weapon's lock (its record's +0x20 time left, +0x24 target,
/// +0x38 query).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lock<T> {
    left: f32,
    target: Option<T>,
    /// The query asked last frame, if one was: its answer.
    asked: Option<Option<T>>,
}

impl<T> Default for Lock<T> {
    fn default() -> Self {
        Self {
            left: CLEARED,
            target: None,
            asked: None,
        }
    }
}

impl<T: Copy + PartialEq> Lock<T> {
    /// One frame of 0x82294318. `seen` is this frame's cone pick and
    /// whether the line to it is clear (answered next frame). Returns the
    /// event for the target's player, and that target.
    pub fn step(
        &mut self,
        dt: f32,
        lock_time: f32,
        seen: Option<(T, bool)>,
    ) -> Option<(LockEvent, T)> {
        let mut event = None;
        // The target an empty answer took away: the cone finding nothing
        // too clears it aloud (its player's, 0x82294FE8 of the old one).
        let mut lost = None;
        if let Some(answer) = self.asked {
            match answer {
                None => {
                    lost = self.target;
                    self.reset();
                }
                Some(c) if self.target == Some(c) => {
                    self.left -= dt;
                    let share = if self.left > 0.0 && lock_time > 0.0 {
                        1.0 - self.left / lock_time
                    } else {
                        1.0
                    };
                    event = Some((LockEvent::Update(share), c));
                }
                Some(c) => {
                    self.left = lock_time;
                    self.target = Some(c);
                    event = Some((LockEvent::Start, c));
                }
            }
        }
        if dt > 0.0 {
            let had = self.asked.is_some();
            self.asked = seen.map(|(c, clear)| clear.then_some(c));
            if had && self.asked.is_none() {
                if event.is_none()
                    && let Some(t) = lost
                {
                    event = Some((LockEvent::Clear, t));
                }
                self.reset();
            }
        } else {
            self.asked = None;
        }
        event
    }

    fn reset(&mut self) {
        self.left = CLEARED;
        self.target = None;
    }

    /// The target it is locking or locked on.
    pub fn target(&self) -> Option<T> {
        self.target
    }

    /// The target it is locked on: one with no time left.
    pub fn locked(&self) -> Option<T> {
        self.target.filter(|_| self.left <= 0.0)
    }

    /// Firing (0x82293660): the events for the target's player (a clear,
    /// then the launch if it was locked) and the target the missile
    /// homes on; the lock starts again.
    pub fn launched(&mut self) -> (Vec<(LockEvent, T)>, Option<T>) {
        let locked = self.locked();
        let mut events = Vec::new();
        if let Some(t) = self.target {
            events.push((LockEvent::Clear, t));
        }
        if let Some(t) = locked {
            events.push((LockEvent::Launched, t));
        }
        self.reset();
        (events, locked)
    }
}

/// A `HomingWeapon` record as the cone test reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cone {
    /// 1 ground, 2 air.
    pub kind: u32,
    pub min_signature: f32,
    /// Half-angle, radians.
    pub cone: f32,
    pub range: f32,
}

impl Cone {
    /// Weapon `s`'s, held by an AI when `ai` (its cone at least 60
    /// degrees).
    pub fn of(s: &m2_engine::weapons::Seeker, ai: bool) -> Self {
        let cone = if ai { s.cone.max(AI_CONE) } else { s.cone };
        Self {
            kind: s.kind,
            min_signature: s.min_signature,
            cone: cone.to_radians(),
            range: s.range,
        }
    }

    /// Its score for a target at `at` of `radius` and `signature`
    /// (ground, air), from `origin` aiming along unit `dir`, if the target
    /// is in its cone (0x822938C0).
    pub fn score(
        &self,
        origin: Vec3,
        dir: Vec3,
        at: Vec3,
        radius: f32,
        signature: [f32; 2],
    ) -> Option<f32> {
        let sig = match self.kind {
            1 => signature[0],
            2 => signature[1],
            _ => 0.0,
        };
        if self.min_signature > 0.0 && self.min_signature > sig {
            return None;
        }
        let to = at - origin;
        let d = to.length();
        if d <= 0.0 {
            return None;
        }
        let cos = dir.dot(to / d);
        if cos < 0.0 || d > self.range * cos {
            return None;
        }
        let along = d / cos.max(1e-6);
        let abreast = origin + dir * along;
        let reach = self.cone.tan() * along;
        let gap = (abreast.distance(at) - radius).max(0.0);
        if gap > reach {
            return None;
        }
        let score = SCORE[0] * cos + SCORE[1] * (1.0 - d / self.range) + SCORE[2] * sig;
        (score > 0.0).then_some(score)
    }
}

/// A missile's homing (0x82293280's record).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Homing<T> {
    pub target: T,
    /// Radians a second.
    pub turn: f32,
    /// Radians.
    pub widest: f32,
    pub age: f32,
}

impl<T: Copy> Homing<T> {
    /// On `target`, from its round's `HomingProjectile` (turn rate,
    /// widest angle; degrees).
    pub fn new(target: T, (turn, widest): (f32, f32)) -> Self {
        Self {
            target,
            turn: turn.to_radians(),
            widest: widest.to_radians(),
            age: 0.0,
        }
    }

    /// One frame's steering of a missile at `at` flying `velocity` after
    /// its target at `goal`: its new velocity, or `None` once it has lost
    /// the target (it flies on as it was).
    pub fn steer(&mut self, at: Vec3, velocity: Vec3, goal: Vec3, dt: f32) -> Option<Vec3> {
        self.age += dt;
        if self.age < ARMING {
            return Some(velocity);
        }
        let to = goal - at;
        let speed = velocity.length();
        if to.length_squared() < 1e-4 || speed <= 0.0 {
            return Some(velocity);
        }
        let want = to.normalize();
        let nose = velocity / speed;
        let dot = nose.dot(want);
        if dot >= 1.0 {
            return Some(velocity);
        }
        let angle = dot.clamp(-1.0, 1.0).acos();
        if angle > self.widest {
            return None;
        }
        let most = self.turn * dt;
        let dir = if most < angle {
            let axis = nose.cross(want);
            if axis.length_squared() > 1e-8 {
                Quat::from_axis_angle(axis.normalize(), most) * nose
            } else {
                nose
            }
        } else {
            want
        };
        Some(dir * speed)
    }
}

/// A wire-guided weapon's guidance (`HomingWeapon` kind 3, the TOW:
/// 0x82294C98, called each frame by 0x82293BE8 for a kind-3 weapon in
/// place of the lock). Its missiles fly at [`Wire::point`]: where its
/// holder's aim line meets something (record +0x28), answered a frame
/// late (+0x38). 0x82293660 launches a kind-3 missile with no target and
/// flag 0x40, and 0x82293280 then steers it at its weapon's point.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Wire {
    /// Where its missiles fly to; none before its first answer.
    pub point: Option<Vec3>,
    /// The line asked last frame: where it met something, or its end.
    asked: Option<Vec3>,
}

impl Wire {
    /// One frame: last frame's answer becomes the point, and a new line
    /// is asked along unit `dir` from 1 m past `origin` (0x82294C98 adds
    /// the direction to the aim's origin), `range` long (the record's
    /// +0xc). `cast` gives the distance to the first thing along it.
    pub fn aim(
        &mut self,
        origin: Vec3,
        dir: Vec3,
        range: f32,
        cast: impl FnOnce(Vec3, Vec3, f32) -> Option<f32>,
    ) {
        if let Some(p) = self.asked.take() {
            self.point = Some(p);
        }
        let start = origin + dir;
        self.asked = Some(start + dir * cast(start, dir, range).unwrap_or(range));
    }
}

/// The warning sounds one player hears of the locks on her (the
/// `AntiAir` script's `_UpdateHomingState` and `_SetSound`).
#[derive(Debug, Default)]
pub struct Warnings {
    /// By locker: done (share 1), and the time since its last word.
    locks: Vec<(u32, bool, f32)>,
    /// `nTargeting`, `nTargeted`.
    counts: [usize; 2],
    cooling: f32,
    cues: Vec<(&'static str, bool)>,
}

impl Warnings {
    /// Locker `owner`'s event.
    pub fn hear(&mut self, owner: u32, event: LockEvent) {
        if !matches!(event, LockEvent::Update(_)) {
            eprintln!("homing: {} (locker {owner})", event.name());
        }
        let found = self.locks.iter().position(|l| l.0 == owner);
        match (event, found) {
            (LockEvent::Start, None) => {
                self.add(false, false);
                self.locks.push((owner, false, 0.0));
            }
            (LockEvent::Update(share), None) => {
                // An update with no start is taken as one (`_HomingLockUpdate`).
                self.hear(owner, LockEvent::Start);
                self.hear(owner, LockEvent::Update(share));
            }
            (LockEvent::Update(share), Some(i)) => {
                let done = share >= 1.0;
                self.locks[i].2 = 0.0;
                let was = self.locks[i].1;
                if was != done {
                    self.remove(was);
                    self.add(done, true);
                    self.locks[i].1 = done;
                }
            }
            (LockEvent::Clear, Some(i)) => {
                let (_, done, _) = self.locks.remove(i);
                self.remove(done);
            }
            // `_HomingLaunched` blips the missile; the clear before it
            // ended the lock.
            (LockEvent::Start, Some(_)) | (LockEvent::Clear | LockEvent::Launched, _) => {}
        }
    }

    fn cue(done: bool) -> &'static str {
        if done { CUE_TARGETED } else { CUE_TARGETING }
    }

    fn add(&mut self, done: bool, quiet: bool) {
        let n = &mut self.counts[usize::from(done)];
        if *n == 0 {
            self.cues.push((Self::cue(done), false));
        }
        *n += 1;
        if !quiet && self.cooling <= 0.0 {
            self.cooling = ALERT_COOLDOWN;
            self.cues.push((CUE_ALERT, false));
        }
    }

    fn remove(&mut self, done: bool) {
        let n = &mut self.counts[usize::from(done)];
        *n = n.saturating_sub(1);
        if *n == 0 {
            self.cues.push((Self::cue(done), true));
        }
    }

    /// A frame: locks with no word for 1 s are dropped.
    pub fn tick(&mut self, dt: f32) {
        self.cooling -= dt;
        for l in &mut self.locks {
            l.2 += dt;
        }
        while let Some(i) = self.locks.iter().position(|l| l.2 > HANGING) {
            let owner = self.locks[i].0;
            self.hear(owner, LockEvent::Clear);
        }
    }

    /// The cues to play (`false`) or stop (`true`) since last asked.
    pub fn take_cues(&mut self) -> Vec<(&'static str, bool)> {
        std::mem::take(&mut self.cues)
    }
}

impl Warnings {
    /// Every lock dropped: she left the vehicle they were on (the script's
    /// `ObjectInSeat` / `ObjectDeath` events clear them).
    pub fn drop_all(&mut self) {
        while let Some(&(owner, ..)) = self.locks.first() {
            self.hear(owner, LockEvent::Clear);
        }
    }
}

/// Who holds a lock, as the scripts know them: `uOwnerGuid` and
/// `uVehicleGuid` (0x822956C0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Locker {
    pub owner: u32,
    pub vehicle: Option<u32>,
}

/// The guids made up for lockers the scripts have no object for (the
/// port's own patrols and crews), by their number as lockers. Invented:
/// above the scripts' spawned guids (`0x4000_0000` on).
pub const UNKNOWN_LOCKER: u32 = 0x7f00_0000;

impl Locker {
    /// A soldier, `n`th locker, placed by layer entity `spawn` if one.
    pub fn soldier(spawn: Option<u32>, n: u32) -> Self {
        Self {
            owner: spawn.unwrap_or(UNKNOWN_LOCKER + n),
            vehicle: None,
        }
    }

    /// A vehicle's gun, `n`th locker, the vehicle the scripts' `vehicle`
    /// if theirs. Its crewman is not an object of theirs here, so the
    /// vehicle stands for him as the owner too (invented).
    pub fn crew(vehicle: Option<u32>, n: u32) -> Self {
        Self {
            owner: vehicle.unwrap_or(UNKNOWN_LOCKER + n),
            vehicle,
        }
    }
}

/// A lock event for the scripts: what, by whom, and for a launch the
/// round fired ([`ScriptSide::fired`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScriptPost {
    pub event: LockEvent,
    pub locker: Locker,
    pub round: Option<u32>,
}

/// What the scripts hear of the locks on her vehicle: the events, and the
/// homing missiles fired on them (numbered rounds, made objects of theirs
/// by the mission).
#[derive(Debug, Default)]
pub struct ScriptSide {
    posts: Vec<ScriptPost>,
    /// Round, its ammo template, where it left.
    fired: Vec<(u32, u32, Vec3)>,
    rounds: u32,
    /// The round a launch was posted for, until its missile flies.
    armed: Option<u32>,
}

impl ScriptSide {
    /// Locker `locker`'s event. A launch numbers the round the missile
    /// about to fly will be.
    pub fn post(&mut self, event: LockEvent, locker: Locker) {
        let round = (event == LockEvent::Launched).then(|| {
            self.rounds += 1;
            self.armed = Some(self.rounds);
            self.rounds
        });
        self.posts.push(ScriptPost {
            event,
            locker,
            round,
        });
    }

    /// A homing missile of ammo `template` fired at `at`: the round its
    /// launch was posted for, if one was.
    pub fn fired(&mut self, template: u32, at: Vec3) -> Option<u32> {
        let r = self.armed.take()?;
        self.fired.push((r, template, at));
        Some(r)
    }

    /// No missile followed the launch.
    pub fn disarm(&mut self) {
        self.armed = None;
    }

    /// The rounds fired and the events since last asked, rounds first (a
    /// launch's missile must be theirs before they hear of it).
    pub fn take(&mut self) -> (Vec<(u32, u32, Vec3)>, Vec<ScriptPost>) {
        (
            std::mem::take(&mut self.fired),
            std::mem::take(&mut self.posts),
        )
    }
}

/// The vehicle she drives, as homing weapons see it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mark<T> {
    pub id: T,
    pub at: Vec3,
    /// Half its bounds' diagonal (the sphere taken for its box).
    pub radius: f32,
    /// Ground and air signatures (`HomingTarget`).
    pub signature: [f32; 2],
}

/// An AI's homing weapon: its lock, how long its named target lasts, a
/// shot held for the lock, and its number as a locker for the warnings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seeking<T> {
    pub lock: Lock<T>,
    pub named: f32,
    pub held: bool,
    pub owner: u32,
}

impl<T> Default for Seeking<T> {
    fn default() -> Self {
        Self {
            lock: Lock::default(),
            named: 0.0,
            held: false,
            owner: 0,
        }
    }
}

impl<T: Copy + PartialEq> Seeking<T> {
    /// One frame of an AI's weapon `seeker` at `from`: while named on her
    /// vehicle `mark` it runs the lock against it alone (0x82294B70),
    /// aimed straight at it; `clear` says whether the line to it is.
    /// Returns the event, if one is for her.
    pub fn tick(
        &mut self,
        dt: f32,
        seeker: &m2_engine::weapons::Seeker,
        from: Vec3,
        mark: Option<Mark<T>>,
        clear: impl FnOnce(Vec3, Vec3, f32) -> Option<T>,
    ) -> Option<LockEvent> {
        let named = self.named > 0.0;
        self.named -= dt;
        if !named {
            self.held = false;
        }
        let seen = mark.filter(|_| named).and_then(|m| {
            let dir = (m.at - from).normalize_or(Vec3::Z);
            let cone = Cone::of(seeker, true);
            cone.score(from, dir, m.at, m.radius, m.signature)?;
            Some((m.id, clear(from, dir, cone.range) == Some(m.id)))
        });
        let (event, target) = self.lock.step(dt, seeker.lock_time, seen)?;
        (mark.map(|m| m.id) == Some(target)).then_some(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_engine::weapons::Seeker;

    const DT: f32 = 1.0 / 30.0;

    // A launch numbers the round; the missile fired after it takes that
    // number, once; a fire with no launch posted has none.
    #[test]
    fn launches_number_their_rounds() {
        let mut s = ScriptSide::default();
        let who = Locker::crew(Some(0x50), 1);
        s.post(LockEvent::Clear, who);
        s.post(LockEvent::Launched, who);
        assert_eq!(s.fired(7, Vec3::ZERO), Some(1));
        assert_eq!(s.fired(7, Vec3::ZERO), None);
        let (fired, posts) = s.take();
        assert_eq!(fired, [(1, 7, Vec3::ZERO)]);
        assert_eq!(
            posts.iter().map(|p| (p.event, p.round)).collect::<Vec<_>>(),
            [(LockEvent::Clear, None), (LockEvent::Launched, Some(1))]
        );
        s.post(LockEvent::Launched, who);
        s.disarm();
        assert_eq!(s.fired(7, Vec3::ZERO), None);
    }

    // The scripts' guids: a layer's soldier by his entity, a vehicle by its
    // own; the port's own get made-up ones by locker number.
    #[test]
    fn lockers_take_the_scripts_guids() {
        assert_eq!(
            Locker::soldier(Some(0x1234), 3),
            Locker {
                owner: 0x1234,
                vehicle: None
            }
        );
        assert_eq!(Locker::soldier(None, 3).owner, UNKNOWN_LOCKER + 3);
        assert_eq!(
            Locker::crew(Some(0x50), 2),
            Locker {
                owner: 0x50,
                vehicle: Some(0x50)
            }
        );
        assert_eq!(
            Locker::crew(None, 2),
            Locker {
                owner: UNKNOWN_LOCKER + 2,
                vehicle: None
            }
        );
    }

    // The template data's mounted SAM: air, least signature 1, 15
    // degrees, 400 m, 1.5 s.
    const SAM: Seeker = Seeker {
        kind: 2,
        min_signature: 1.0,
        cone: 15.0,
        range: 400.0,
        lock_time: 1.5,
    };

    fn heli(at: Vec3) -> Mark<u32> {
        Mark {
            id: 3,
            at,
            radius: 4.0,
            signature: [0.0, 1.0],
        }
    }

    // The lock reads its query a frame late: a start the frame after it
    // is first seen, then LockOnTime counted down by dt; locked once
    // none is left, its share 1 from then.
    #[test]
    fn a_lock_starts_a_frame_late_and_takes_lock_on_time() {
        let mut l = Lock::<u32>::default();
        assert_eq!(l.step(DT, 1.5, Some((7, true))), None);
        assert_eq!(
            l.step(DT, 1.5, Some((7, true))),
            Some((LockEvent::Start, 7))
        );
        let mut frames = 2;
        let mut last = 0.0;
        while l.locked().is_none() {
            let Some((LockEvent::Update(share), 7)) = l.step(DT, 1.5, Some((7, true))) else {
                panic!("an update each frame");
            };
            assert!(share >= last);
            last = share;
            frames += 1;
            assert!(frames < 100);
        }
        assert_eq!(last, 1.0);
        let at = frames as f32 * DT;
        assert!((at - (1.5 + 2.0 * DT)).abs() < 1.5 * DT, "locked at {at}");
    }

    // A blocked line resets it without a word; the cone losing it
    // clears it and says so; firing clears it, then says launched if it
    // was locked, and starts again.
    #[test]
    fn a_lock_resets_clears_and_launches_as_the_360_does() {
        let mut l = Lock::<u32>::default();
        l.step(DT, 1.0, Some((7, true)));
        l.step(DT, 1.0, Some((7, false)));
        assert_eq!(
            l.step(DT, 1.0, Some((7, true))),
            None,
            "blocked: silent reset"
        );
        assert_eq!(
            l.step(DT, 1.0, Some((7, false))),
            Some((LockEvent::Start, 7))
        );
        assert_eq!(l.step(DT, 1.0, None), Some((LockEvent::Clear, 7)), "gone");
        assert_eq!(l.locked(), None);
        // Gone the frame it answered: the answer's event, no clear.
        l.step(DT, 1.0, Some((7, true)));
        assert_eq!(l.step(DT, 1.0, None), Some((LockEvent::Start, 7)));
        assert_eq!(l.step(DT, 1.0, None), None);

        let mut l = Lock::<u32>::default();
        for _ in 0..40 {
            l.step(DT, 1.0, Some((7, true)));
        }
        assert_eq!(l.locked(), Some(7));
        let (events, target) = l.launched();
        assert_eq!(target, Some(7));
        assert_eq!(
            events,
            vec![(LockEvent::Clear, 7), (LockEvent::Launched, 7)]
        );
        assert_eq!(
            l.step(DT, 1.0, Some((7, true))),
            Some((LockEvent::Start, 7))
        );
    }

    // The cone: signature of its kind at least its least, ahead, within
    // range along the ray, within tan(cone) of the ray; an AI's at
    // least 60 degrees.
    #[test]
    fn the_cone_reads_kind_signature_range_and_angle() {
        let player = Cone::of(&SAM, false);
        let ai = Cone::of(&SAM, true);
        assert!((ai.cone - 60f32.to_radians()).abs() < 1e-6);
        let o = Vec3::ZERO;
        let ahead = Vec3::Z;
        let air = [0.0, 1.0];
        let car = [1.0, 0.0];
        let z = |d: f32| Vec3::new(0.0, 0.0, d);
        assert!(player.score(o, ahead, z(200.0), 0.0, air).is_some());
        assert!(player.score(o, ahead, z(200.0), 0.0, car).is_none());
        assert!(player.score(o, ahead, z(401.0), 0.0, air).is_none());
        assert!(player.score(o, ahead, z(-50.0), 0.0, air).is_none());
        // 30 degrees off: out of the SAM's 15, in an AI's 60.
        let off = Vec3::new(100.0, 0.0, 100.0 * 3f32.sqrt());
        assert!(player.score(o, ahead, off, 0.0, air).is_none());
        assert!(ai.score(o, ahead, off, 0.0, air).is_some());
        // Nearer and straighter scores higher.
        let near = player.score(o, ahead, z(50.0), 0.0, air).unwrap();
        let far = player.score(o, ahead, z(300.0), 0.0, air).unwrap();
        assert!(near > far);
        assert!((near - (45.0 + 5.0 * (1.0 - 50.0 / 400.0) + 5.0)).abs() < 1e-4);
    }

    // A missile flies straight its first 0.1 s, then turns by at most
    // its turn rate a frame, and loses a target past its widest angle.
    #[test]
    fn a_missile_arms_turns_at_its_rate_and_loses_wide_targets() {
        // The AA missile's 90 degrees a second and 180 widest.
        let mut h = Homing::new(1u32, (90.0, 180.0));
        let v = Vec3::Z * 50.0;
        let goal = Vec3::new(100.0, 0.0, 0.0);
        assert_eq!(h.steer(Vec3::ZERO, v, goal, 0.05), Some(v));
        let turned = h.steer(Vec3::ZERO, v, goal, 0.1).unwrap();
        assert!((turned.length() - 50.0).abs() < 1e-3);
        let angle = turned.angle_between(v).to_degrees();
        assert!((angle - 9.0).abs() < 1e-3, "{angle}");
        // Nearly on: straight at it.
        let near = Vec3::new(1.0, 0.0, 100.0);
        let on = h.steer(Vec3::ZERO, v, near, 0.1).unwrap();
        assert!(on.normalize().dot(near.normalize()) > 0.9999);
        // The AT missile's 60 widest: a target behind is lost.
        let mut at = Homing::new(1u32, (30.0, 60.0));
        at.age = 1.0;
        assert_eq!(
            at.steer(Vec3::ZERO, v, Vec3::new(0.0, 0.0, -10.0), DT),
            None
        );
    }

    // A TOW's point follows its holder's aim a frame late: nothing
    // before the first answer, then where the line met something, else
    // the line's end (300 m, from 1 m past the aim's origin).
    #[test]
    fn a_wire_follows_the_aim_a_frame_late() {
        let mut w = Wire::default();
        let o = Vec3::new(0.0, 2.0, 0.0);
        w.aim(o, Vec3::Z, 300.0, |start, _, _| {
            assert_eq!(start, Vec3::new(0.0, 2.0, 1.0));
            Some(49.0)
        });
        assert_eq!(w.point, None);
        w.aim(o, Vec3::X, 300.0, |_, _, _| None);
        assert_eq!(w.point, Some(Vec3::new(0.0, 2.0, 50.0)));
        w.aim(o, Vec3::X, 300.0, |_, _, _| None);
        assert_eq!(w.point, Some(Vec3::new(301.0, 2.0, 0.0)));
    }

    // The warnings: a lock coming on starts the targeting cue and the
    // new-lock alert; done, the targeting cue stops and the targeted one
    // starts; cleared, it stops. A second new lock within 1 s gets no
    // alert; a lock with no word for 1 s is dropped.
    #[test]
    fn warnings_follow_the_antiair_script() {
        let mut w = Warnings::default();
        w.hear(1, LockEvent::Start);
        assert_eq!(
            w.take_cues(),
            vec![(CUE_TARGETING, false), (CUE_ALERT, false)]
        );
        w.hear(1, LockEvent::Update(0.5));
        assert!(w.take_cues().is_empty());
        w.hear(2, LockEvent::Start);
        assert!(w.take_cues().is_empty(), "cooling down, already targeting");
        w.hear(1, LockEvent::Update(1.0));
        assert_eq!(w.take_cues(), vec![(CUE_TARGETED, false)]);
        w.hear(2, LockEvent::Clear);
        assert_eq!(w.take_cues(), vec![(CUE_TARGETING, true)]);
        w.hear(1, LockEvent::Clear);
        w.hear(1, LockEvent::Launched);
        assert_eq!(w.take_cues(), vec![(CUE_TARGETED, true)]);
        w.tick(2.0);
        w.hear(3, LockEvent::Start);
        assert_eq!(
            w.take_cues(),
            vec![(CUE_TARGETING, false), (CUE_ALERT, false)]
        );
        w.tick(0.5);
        w.tick(0.6);
        assert_eq!(w.take_cues(), vec![(CUE_TARGETING, true)]);
    }

    // The scene: an AI's SAM named on a helicopter 200 m off, line
    // clear, locks after its 1.5 s (and the query's frame), telling her
    // start, updates up to 1, and on firing clear and launched.
    #[test]
    fn an_ai_sam_locks_on_her_helicopter_in_its_lock_on_time() {
        let mark = heli(Vec3::new(50.0, 120.0, 150.0));
        let mut s = Seeking::<u32>::default();
        let mut events = Vec::new();
        let mut t = 0.0;
        while s.lock.locked().is_none() {
            s.named = NAMED;
            if let Some(e) = s.tick(DT, &SAM, Vec3::ZERO, Some(mark), |_, _, _| Some(mark.id)) {
                events.push(e);
            }
            t += DT;
            assert!(t < 5.0);
        }
        assert!((t - (1.5 + 2.0 * DT)).abs() < 1.5 * DT, "locked after {t}");
        assert_eq!(events[0], LockEvent::Start);
        assert_eq!(events.last(), Some(&LockEvent::Update(1.0)));
        let (fired, target) = s.lock.launched();
        assert_eq!(target, Some(mark.id));
        let kinds: Vec<LockEvent> = fired.iter().map(|e| e.0).collect();
        assert_eq!(kinds, [LockEvent::Clear, LockEvent::Launched]);
        // Not named, or hidden, or a car to a SAM: no lock.
        let mut s = Seeking::<u32>::default();
        for _ in 0..100 {
            s.tick(DT, &SAM, Vec3::ZERO, Some(mark), |_, _, _| Some(mark.id));
        }
        assert_eq!(s.lock.locked(), None);
        let mut s = Seeking::<u32>::default();
        for _ in 0..100 {
            s.named = NAMED;
            s.tick(DT, &SAM, Vec3::ZERO, Some(mark), |_, _, _| None);
        }
        assert_eq!(s.lock.locked(), None);
        let car = Mark {
            signature: [1.0, 0.0],
            ..mark
        };
        let mut s = Seeking::<u32>::default();
        for _ in 0..100 {
            s.named = NAMED;
            s.tick(DT, &SAM, Vec3::ZERO, Some(car), |_, _, _| Some(car.id));
        }
        assert_eq!(s.lock.locked(), None);
    }
}
