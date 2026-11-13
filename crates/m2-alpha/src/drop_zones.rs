//! The scripts' drop zone tests (`Ai.TestDropZone`: `pursuitcopter`
//! looking for where to land a pursuit's soldiers, supply drops, vehicle
//! deliveries) searched in the game's world.
//!
//! From the 360 code: the test (0x82312540) queues a landing-zone search
//! (0x822CE968) with the sizes its table gives, the same search a
//! helicopter's `HeliLand` makes ([`m2_engine::heli_land`]); the manager
//! (0x822CEB80) gives each one think a frame and, once it ends, calls the
//! test's callback with whether a spot was found and where.
//!
//! Not modelled: a test for a spot on the water (`Water`: the search's
//! other branch, not traced); it is answered found at its point, as
//! before. Its world is the alpha's physics with nothing left out (the
//! 360's test passes no object of its own to ignore, +0x24 left 0).

use std::collections::HashMap;

use m2_engine::Physics;
use m2_engine::heli_land::{DropZone, LandingSearch, Search};
use m2_engine::heli_pilot::PilotWorld;
use m2_math::glam::{Quat, Vec3};

/// The alpha's physics as a drop zone search sees it.
pub struct Ground<'a> {
    pub physics: &'a Physics,
}

impl PilotWorld for Ground<'_> {
    fn ground(&mut self, from: Vec3, to: Vec3) -> Option<Vec3> {
        let d = to - from;
        self.physics
            .cast_ray(from, d.normalize_or_zero(), d.length())
            .map(|(t, _)| from + d.normalize_or_zero() * t)
    }

    fn sweep(&mut self, origin: Vec3, turn: Quat, half: Vec3, way: Vec3, max: f32) -> f32 {
        self.physics
            .cast_box(origin, turn, half, way, max, None)
            .map_or(1.0, |t| (t / max).clamp(0.0, 1.0))
    }

    fn column(&mut self, top: Vec3, down: f32, radius: f32) -> bool {
        self.physics.sphere_meets_below(top, down, radius, None)
    }

    fn water(&mut self, _: Vec3) -> Option<f32> {
        self.physics.water()
    }
}

/// The searches under way, by test.
#[derive(Debug, Default)]
pub struct DropZones {
    searches: HashMap<u32, LandingSearch>,
    /// Tests answered, found and not, for the log.
    pub found: usize,
    pub failed: usize,
}

impl DropZones {
    /// One frame: starts a search for each test new in `tests` (number,
    /// point, test), forgets those gone, gives each one think in `world`,
    /// and returns the tests ended: number, found, the spot.
    pub fn step(
        &mut self,
        tests: &[(u32, [f32; 3], DropZone)],
        world: &mut dyn PilotWorld,
    ) -> Vec<(u32, bool, [f32; 3])> {
        self.searches
            .retain(|id, _| tests.iter().any(|(t, _, _)| t == id));
        let mut out = Vec::new();
        for (id, at, zone) in tests {
            if zone.water {
                out.push((*id, true, *at));
                continue;
            }
            let search = self
                .searches
                .entry(*id)
                .or_insert_with(|| LandingSearch::drop_zone(Vec3::from_array(*at), zone));
            match search.step(world) {
                Search::Looking => {}
                Search::Found(p) => {
                    self.found += 1;
                    out.push((*id, true, p.to_array()));
                }
                Search::Failed => {
                    self.failed += 1;
                    out.push((*id, false, search.centre().to_array()));
                }
            }
        }
        for (id, _, _) in &out {
            self.searches.remove(id);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Level ground at 0 for x >= 0; open water over a seabed 40 m down
    /// for x < 0.
    struct Coast;

    impl PilotWorld for Coast {
        fn ground(&mut self, from: Vec3, to: Vec3) -> Option<Vec3> {
            let h = if from.x < 0.0 { -40.0 } else { 0.0 };
            (from.y >= h && to.y <= h).then(|| from.with_y(h))
        }
        fn sweep(&mut self, _: Vec3, _: Quat, _: Vec3, _: Vec3, _: f32) -> f32 {
            1.0
        }
        fn water(&mut self, at: Vec3) -> Option<f32> {
            (at.x < 0.0).then_some(0.0)
        }
    }

    /// `pursuitcopter`'s test (`FindLZ`).
    const PURSUIT: DropZone = DropZone {
        search_radius: 40.0,
        inner_radius: 6.0,
        outer_radius: 19.0,
        inner_height: 1.0,
        outer_height: 2.5,
        height_max: 20.0,
        water: false,
        no_height_restriction: false,
    };

    fn run(zones: &mut DropZones, tests: &[(u32, [f32; 3], DropZone)]) -> (u32, bool, [f32; 3]) {
        for _ in 0..5000 {
            if let Some(a) = zones.step(tests, &mut Coast).pop() {
                return a;
            }
        }
        panic!("never answered");
    }

    // On land the test is found where she stands, one think a frame (its
    // window fills in three); far out at sea it fails, so the script's
    // retry and its going home run; a water test is answered at once.
    #[test]
    fn a_test_is_searched_a_think_a_frame_and_fails_at_sea() {
        let mut zones = DropZones::default();
        let land = [(1, [100.0, 0.0, 0.0], PURSUIT)];
        assert!(zones.step(&land, &mut Coast).is_empty());
        assert!(zones.step(&land, &mut Coast).is_empty());
        assert_eq!(
            zones.step(&land, &mut Coast),
            [(1, true, [100.0, 0.0, 0.0])]
        );
        let sea = [(2, [-200.0, 0.0, 0.0], PURSUIT)];
        let (id, found, _) = run(&mut zones, &sea);
        assert_eq!((id, found), (2, false));
        assert_eq!((zones.found, zones.failed), (1, 1));
        let boat = DropZone {
            water: true,
            ..PURSUIT
        };
        assert_eq!(
            zones.step(&[(3, [-200.0, 0.0, 0.0], boat)], &mut Coast),
            [(3, true, [-200.0, 0.0, 0.0])]
        );
    }

    // A test that is gone (its mission over) is forgotten.
    #[test]
    fn a_test_gone_is_forgotten() {
        let mut zones = DropZones::default();
        zones.step(&[(1, [100.0, 0.0, 0.0], PURSUIT)], &mut Coast);
        assert_eq!(zones.searches.len(), 1);
        zones.step(&[], &mut Coast);
        assert!(zones.searches.is_empty());
    }
}
