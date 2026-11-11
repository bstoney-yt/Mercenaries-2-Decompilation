//! The helicopter wave spawner: `Pg.StartHeliWaveSpawner(waves)` and
//! `Pg.StopHeliWaveSpawner()`. PmcCon003 sends six waves of gunships
//! (Alouette 3s and an Mi35) after her once she has the bunker buster.
//!
//! From the 360 code (natives 0x8233b6c0 and 0x8233b990; parsing
//! 0x8233b4d0 and 0x8233b1c0; the spawner 0x8223a958, 0x8223b030,
//! 0x8223ab38, 0x8223add0, 0x8223b170 and 0x8223b390):
//!
//! - Each wave is a table `{ NumToSpawn = n, WaveDelay = s, SpawnDist =
//!   m, units = { { template, weight }, ... } }`; `NumToSpawn` defaults to
//!   1, `WaveDelay` to 15 s, `SpawnDist` to 150 m, a unit's weight to 1
//!   (at least 1). A wave with no unit is left out; at most 16 waves.
//!   Starting stops the spawner first; it then waits the first wave's
//!   delay.
//! - When a wave's delay has run out it spawns `NumToSpawn` helicopters
//!   (at most 8), each a unit picked at random by weight, `SpawnDist`
//!   ahead of the player within 0.4 rad either side of where she faces,
//!   at least 70 m up, side by side 12 m apart; each faces her. No room
//!   there (the 360 raises the point up to ten times by 5 m before giving
//!   up): try again in 2 s.
//! - It follows each helicopter until it is destroyed or gone, or more
//!   than 500 m from her (the 360 then posts it a message, 0x8357f9b8, and
//!   stops following it). When none is left it starts the next wave's
//!   delay; after the last wave it stops.
//!
//! Here (inferred, not from the 360 code): the ground isn't known, so
//! "70 m up" is 70 m over her own height, and nothing is in the way of
//! the point; the 500 m message isn't sent. The random numbers come from
//! the spawner's own generator, not the 360's.

use m2_math::glam::Vec3;

/// The most waves the 360 reads.
pub const MAX_WAVES: usize = 16;
/// The most helicopters one wave spawns.
pub const MAX_PER_WAVE: u32 = 8;
/// Defaults for a wave's missing fields (0x820b7b44, 0x820ebf4c).
pub const DEFAULT_DELAY: f32 = 15.0;
pub const DEFAULT_DISTANCE: f32 = 150.0;
/// How far either side of where she faces a wave may come from, radians:
/// (0.5 - r) * 0.8 for r in [0, 1) (0x820d9edc, 0x820b8034).
pub const SPREAD: f32 = 0.4;
/// The least height over the ground a wave comes in at (0x820b7b54).
pub const MIN_HEIGHT: f32 = 70.0;
/// How far apart a wave's helicopters come, side by side (0x82de895c).
pub const SPACING: f32 = 12.0;
/// How long to wait to try again when there was no room (0x820c77a4).
pub const RETRY: f32 = 2.0;
/// Beyond this from her a helicopter no longer counts (squared,
/// 0x820f8bec: 500 m).
pub const GONE_SQUARED: f32 = 250_000.0;

/// One wave, as the scripts give it.
#[derive(Clone, Debug, PartialEq)]
pub struct Wave {
    pub count: u32,
    pub delay: f32,
    pub distance: f32,
    /// Templates (their guids) and their weights.
    pub units: Vec<(u32, u32)>,
}

/// What the spawner asks of the world.
pub trait WaveWorld {
    /// The player's place and facing (radians about the vertical axis,
    /// forward `(sin, 0, cos)`).
    fn player(&self) -> Option<([f32; 3], f32)>;
    /// Spawns template `template` at `at`, facing `yaw`; its guid.
    fn spawn(&mut self, template: u32, at: [f32; 3], yaw: f32) -> Option<u32>;
    /// Where `guid` is while it is still there and not destroyed.
    fn alive_at(&self, guid: u32) -> Option<[f32; 3]>;
}

#[derive(Clone, Debug, Default, PartialEq)]
enum Phase {
    #[default]
    Off,
    /// Waiting this much longer before spawning wave `index`.
    Waiting(f32),
    /// Following the helicopters wave `index` spawned.
    Following,
}

#[derive(Clone, Debug, Default)]
pub struct HeliWaves {
    waves: Vec<Wave>,
    index: usize,
    phase: Phase,
    following: Vec<u32>,
    seed: u32,
}

impl HeliWaves {
    /// `Pg.StartHeliWaveSpawner`: stops any waves under way, then waits
    /// for the first of these.
    pub fn start(&mut self, waves: Vec<Wave>) {
        self.stop();
        self.waves = waves
            .into_iter()
            .filter(|w| !w.units.is_empty())
            .take(MAX_WAVES)
            .collect();
        if let Some(first) = self.waves.first() {
            self.phase = Phase::Waiting(first.delay);
        }
    }

    /// `Pg.StopHeliWaveSpawner`: no more waves (those already out stay).
    pub fn stop(&mut self) {
        let seed = self.seed;
        *self = Self {
            seed,
            ..Self::default()
        };
    }

    pub fn running(&self) -> bool {
        self.phase != Phase::Off
    }

    /// The helicopters of the wave under way that still count.
    pub fn following(&self) -> &[u32] {
        &self.following
    }

    /// A number in [0, 1).
    fn random(&mut self) -> f32 {
        // xorshift32 (a seed of 0 would stay 0).
        let mut x = if self.seed == 0 {
            0x9e37_79b9
        } else {
            self.seed
        };
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.seed = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Moves the spawner on by `dt` seconds.
    pub fn update(&mut self, dt: f32, world: &mut impl WaveWorld) {
        match self.phase {
            Phase::Off => {}
            Phase::Waiting(left) if left > dt => self.phase = Phase::Waiting(left - dt),
            Phase::Waiting(_) => self.spawn_wave(world),
            Phase::Following => {
                let near = world.player().map(|(p, _)| Vec3::from(p));
                self.following.retain(|&g| match (world.alive_at(g), near) {
                    (Some(at), Some(her)) => Vec3::from(at).distance_squared(her) <= GONE_SQUARED,
                    (Some(_), None) => true,
                    (None, _) => false,
                });
                if self.following.is_empty() {
                    self.index += 1;
                    match self.waves.get(self.index) {
                        Some(next) => self.phase = Phase::Waiting(next.delay),
                        None => self.stop(),
                    }
                }
            }
        }
    }

    fn spawn_wave(&mut self, world: &mut impl WaveWorld) {
        let Some(wave) = self.waves.get(self.index).cloned() else {
            self.stop();
            return;
        };
        let Some((her, facing)) = world.player() else {
            self.phase = Phase::Waiting(RETRY);
            return;
        };
        let heading = facing + (0.5 - self.random()) * 2.0 * SPREAD;
        let (s, c) = heading.sin_cos();
        let mut at = [
            her[0] + s * wave.distance,
            her[1] + MIN_HEIGHT,
            her[2] + c * wave.distance,
        ];
        // Side by side: along the heading turned a quarter.
        let (ss, sc) = (heading + std::f32::consts::FRAC_PI_2).sin_cos();
        // Facing her.
        let yaw = (her[0] - at[0]).atan2(her[2] - at[2]);
        let total: u32 = wave.units.iter().map(|(_, w)| (*w).max(1)).sum();
        for _ in 0..wave.count.clamp(1, MAX_PER_WAVE) {
            let pick = (self.random() * total as f32) as u32;
            let mut sum = 0;
            let template = wave
                .units
                .iter()
                .find(|(_, w)| {
                    sum += (*w).max(1);
                    pick < sum
                })
                .or(wave.units.last())
                .map(|(t, _)| *t);
            if let Some(g) = template.and_then(|t| world.spawn(t, at, yaw)) {
                self.following.push(g);
                at = [at[0] + ss * SPACING, at[1], at[2] + sc * SPACING];
            }
        }
        self.phase = Phase::Following;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Fake {
        her: [f32; 3],
        objects: HashMap<u32, (u32, [f32; 3])>,
        next: u32,
    }

    impl WaveWorld for Fake {
        fn player(&self) -> Option<([f32; 3], f32)> {
            Some((self.her, 0.0))
        }
        fn spawn(&mut self, template: u32, at: [f32; 3], _: f32) -> Option<u32> {
            self.next += 1;
            self.objects.insert(self.next, (template, at));
            Some(self.next)
        }
        fn alive_at(&self, guid: u32) -> Option<[f32; 3]> {
            self.objects.get(&guid).map(|(_, at)| *at)
        }
    }

    fn wave(count: u32, delay: f32, units: Vec<(u32, u32)>) -> Wave {
        Wave {
            count,
            delay,
            distance: 300.0,
            units,
        }
    }

    #[test]
    fn waves_come_after_their_delay_and_the_next_once_the_last_is_down() {
        let mut s = HeliWaves::default();
        let mut w = Fake::default();
        s.start(vec![
            wave(2, 8.0, vec![(7, 1)]),
            wave(1, 15.0, vec![(9, 1)]),
        ]);
        s.update(7.9, &mut w);
        assert!(w.objects.is_empty());
        s.update(0.2, &mut w);
        assert_eq!(w.objects.len(), 2);
        // Ahead of her (she faces +z), 300 m off within the spread, 70 m
        // up, 12 m apart.
        let (a, b) = (w.objects[&1].1, w.objects[&2].1);
        assert!((a[1] - MIN_HEIGHT).abs() < 1e-3);
        let flat = Vec3::new(a[0], 0.0, a[2]).length();
        assert!((flat - 300.0).abs() < 1e-2 && a[2] > 0.0);
        assert!((Vec3::from(a).distance(Vec3::from(b)) - SPACING).abs() < 1e-3);
        // One down: the wave goes on.
        w.objects.remove(&1);
        s.update(1.0, &mut w);
        assert_eq!(s.following(), [2]);
        // The other flies off past 500 m: done, and the next wave waits.
        w.objects.get_mut(&2).unwrap().1 = [0.0, 0.0, 501.0];
        s.update(1.0, &mut w);
        s.update(14.9, &mut w);
        assert_eq!(w.objects.len(), 1);
        s.update(0.2, &mut w);
        assert_eq!(w.objects[&3].0, 9);
        // The last wave down: the spawner stops.
        w.objects.clear();
        s.update(1.0, &mut w);
        assert!(!s.running());
    }

    #[test]
    fn a_wave_spawns_at_most_eight_and_picks_units_by_weight() {
        let mut s = HeliWaves::default();
        let mut w = Fake::default();
        s.start(vec![wave(20, 0.0, vec![(7, 1), (9, 3)])]);
        s.update(0.1, &mut w);
        assert_eq!(w.objects.len(), MAX_PER_WAVE as usize);
        assert!(w.objects.values().all(|(t, _)| *t == 7 || *t == 9));
    }

    #[test]
    fn starting_again_or_stopping_drops_the_waves_under_way() {
        let mut s = HeliWaves::default();
        let mut w = Fake::default();
        s.start(vec![wave(1, 5.0, vec![(7, 1)]), wave(1, 5.0, vec![])]);
        // The wave without units is left out.
        s.update(5.1, &mut w);
        w.objects.clear();
        s.update(0.1, &mut w);
        assert!(!s.running());
        s.start(vec![wave(1, 5.0, vec![(7, 1)])]);
        s.stop();
        s.update(10.0, &mut w);
        assert!(w.objects.is_empty());
    }
}
