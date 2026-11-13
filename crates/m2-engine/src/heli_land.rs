//! Where an AI helicopter lands, as the 360 code finds it: its `HeliLand`
//! task (0x822AF3D8) asks the landing-zone manager (0x822CE968, 32 at
//! once, stepped each frame by 0x822CEB80) for a spot near its target,
//! and the manager works outward from it in a square spiral of cells
//! (0x822CDB60), three rays a think, until a 3x3 patch is level enough,
//! dry, and clear above.
//!
//! The scripts' `Ai.TestDropZone` (0x82312540) queues the same search
//! with the sizes its table gives ([`DropZone`]) and hears the outcome
//! through its `Callback`: `(found, x, y, z)`, the spot (0x822CEB80: the
//! first value is whether the search's state is 5, found; then the
//! window's centre, 0x822CD988).
//!
//! Provenance: from the 360 code, but where noted. The objects of kind 6
//! whose records carry 0x71246D6D, which the 360 leaves out of its upper
//! clearance check, are not told apart here (none left out).

use m2_math::glam::Vec3;

use crate::heli_pilot::PilotWorld;

/// A cell with nothing under it within 10 m (the grid's -127).
const MISS: f32 = f32::MIN;

/// How a search stands (its record's +0x4C).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Search {
    /// Still looking.
    Looking,
    /// A spot: where the helicopter sets down.
    Found(Vec3),
    /// None within its radius (or the ground at the target too far off).
    Failed,
}

/// A drop zone test as the scripts' `Ai.TestDropZone` asks it
/// (0x82312540): its table's fields, by where the 360 keeps them in the
/// request (0x822CE968 copies them into the search's slot).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DropZone {
    /// `SearchRadius` (+0xC).
    pub search_radius: f32,
    /// `InnerRadius` (+0x10): the cell's size.
    pub inner_radius: f32,
    /// `OuterRadius` (+0x14): the clear radius over the spot.
    pub outer_radius: f32,
    /// `InnerHeightTolerance` (+0x18): the most the patch may rise.
    pub inner_height: f32,
    /// `OuterHeightTolerance` (+0x1C) and `HeightMax` (+0x20): the
    /// heights the clearance is checked between.
    pub outer_height: f32,
    pub height_max: f32,
    /// `Water` (+0x2C): a spot on the water (the search's other branch,
    /// not modelled here).
    pub water: bool,
    /// `NoHeightRestriction` (+0x2D): the point may be off the ground.
    pub no_height_restriction: bool,
}

/// A landing-zone request and its search, as the manager keeps it (0x50
/// bytes a slot).
#[derive(Clone, Debug, PartialEq)]
pub struct LandingSearch {
    /// The point asked for, its height the ground there once found
    /// (+0x0..+0x8).
    pub target: Vec3,
    /// How far out it looks (+0xC: 20), its cell's size (+0x10), the
    /// clear radius over the spot (+0x14), the most the patch may rise
    /// (+0x18, `LandingSlopeTolerance`), and the two heights the
    /// clearance is checked between over it (+0x1C, `OuterHeightTolerance`;
    /// +0x20, the helicopter's box height plus its `MinAltitude`).
    pub radius: f32,
    pub step: f32,
    pub clear: f32,
    pub slope: f32,
    pub low: f32,
    pub high: f32,
    /// Where the window is (+0x34, +0x3C) and the spot's height in it
    /// (+0x38).
    centre: Vec3,
    /// The window's heights (+0x40..+0x48: rows -z to +z, columns -x to
    /// +x), the steps in this side of the spiral (+0x49), the ring
    /// (+0x4A) and the side (+0x4C: 0 filling, 1..4 the sides).
    grid: [f32; 9],
    counter: i32,
    ring: i32,
    side: u8,
    done: Option<Search>,
    /// The request's `Water` or `NoHeightRestriction` (+0x2C, +0x2D):
    /// a point off the ground doesn't fail it (0x822CDB60).
    anywhere: bool,
}

impl LandingSearch {
    /// A request as `HeliLand` makes it (0x822AF3D8): for a helicopter
    /// whose box is `size` (lo, hi, model space) with its `MinAltitude`,
    /// `LandingSlopeTolerance` and `OuterHeightTolerance`, near `target`.
    pub fn new(
        target: Vec3,
        size: (Vec3, Vec3),
        min_altitude: f32,
        slope: f32,
        outer: f32,
    ) -> Self {
        let (lo, hi) = size;
        let clear = (hi - lo).with_y(0.0).length() * 0.375;
        let step = clear * 0.3;
        Self {
            target,
            radius: 20.0,
            step,
            clear,
            slope,
            low: outer,
            high: (hi.y - lo.y) + min_altitude,
            // It starts three cells to +z and fills its window coming
            // back (0x822CE968).
            centre: Vec3::new(target.x, target.y, target.z + 3.0 * step),
            grid: [MISS; 9],
            counter: 0,
            ring: 0,
            side: 0,
            done: None,
            anywhere: false,
        }
    }

    /// The search for a drop zone test (`Ai.TestDropZone`) near `target`.
    pub fn drop_zone(target: Vec3, zone: &DropZone) -> Self {
        let step = zone.inner_radius.max(0.1);
        Self {
            target,
            radius: zone.search_radius,
            step,
            clear: zone.outer_radius,
            slope: zone.inner_height,
            low: zone.outer_height,
            high: zone.height_max,
            centre: Vec3::new(target.x, target.y, target.z + 3.0 * step),
            grid: [MISS; 9],
            counter: 0,
            ring: 0,
            side: 0,
            done: None,
            anywhere: zone.water || zone.no_height_restriction,
        }
    }

    /// Where the window stands (+0x34..+0x3C): what the callback hears.
    pub fn centre(&self) -> Vec3 {
        self.centre
    }

    /// How it stands.
    pub fn result(&self) -> Search {
        self.done.unwrap_or(Search::Looking)
    }

    /// One think of the search (0x822CDB60).
    pub fn step(&mut self, world: &mut dyn PilotWorld) -> Search {
        if let Some(d) = self.done {
            return d;
        }
        let s = self.step;
        let rings = (self.radius / s) as i32;
        // First the ground at the target itself (0x822CD9F8): the first
        // thing down from 1000 m, or the water over it.
        if self.side == 0 && self.counter == 0 {
            let at = self.target;
            let hit = world
                .ground(
                    Vec3::new(at.x, 1000.0, at.z),
                    Vec3::new(at.x, -1000.0, at.z),
                )
                .map(|p| p.y);
            let ground = match (hit, world.water(at)) {
                (Some(h), Some(w)) => h.max(w),
                (Some(h), None) => h,
                (None, Some(w)) => w,
                (None, None) => -10000.0,
            };
            if !self.anywhere && (at.y - ground).abs() > 2.0 {
                self.done = Some(Search::Failed);
                return Search::Failed;
            }
            self.target.y = ground;
            self.centre.y = ground;
        }
        let g = &mut self.grid;
        let (cx, cz) = (self.centre.x, self.centre.z);
        let mut evaluate = true;
        // The window moves a cell and three new cells are cast.
        let cells: [(f32, f32, usize); 3] = match self.side {
            0 | 1 => {
                self.counter += 1;
                if self.side == 0 {
                    evaluate = self.counter == 3;
                    if evaluate {
                        self.counter = 0;
                        self.side = 1;
                    }
                } else if self.counter == self.ring * 2 + 1 {
                    self.ring += 1;
                    if rings < self.ring {
                        self.done = Some(Search::Failed);
                        return Search::Failed;
                    }
                    self.counter = 0;
                    self.side = 2;
                }
                // Towards -z.
                *g = [MISS, MISS, MISS, g[0], g[1], g[2], g[3], g[4], g[5]];
                self.centre.z -= s;
                [
                    (cx - s, cz - 2.0 * s, 0),
                    (cx, cz - 2.0 * s, 1),
                    (cx + s, cz - 2.0 * s, 2),
                ]
            }
            2 => {
                self.counter += 1;
                if self.counter == self.ring * 2 - 1 {
                    self.counter = 0;
                    self.side = 3;
                }
                // Towards +x.
                *g = [g[1], g[2], MISS, g[4], g[5], MISS, g[7], g[8], MISS];
                self.centre.x += s;
                [
                    (cx + 2.0 * s, cz - s, 2),
                    (cx + 2.0 * s, cz, 5),
                    (cx + 2.0 * s, cz + s, 8),
                ]
            }
            3 => {
                self.counter += 1;
                if self.counter == self.ring * 2 {
                    self.counter = 0;
                    self.side = 4;
                }
                // Towards +z.
                *g = [g[3], g[4], g[5], g[6], g[7], g[8], MISS, MISS, MISS];
                self.centre.z += s;
                [
                    (cx - s, cz + 2.0 * s, 6),
                    (cx, cz + 2.0 * s, 7),
                    (cx + s, cz + 2.0 * s, 8),
                ]
            }
            _ => {
                self.counter += 1;
                if self.counter == self.ring * 2 {
                    self.counter = 0;
                    self.side = 1;
                }
                // Towards -x.
                *g = [MISS, g[0], g[1], MISS, g[3], g[4], MISS, g[6], g[7]];
                self.centre.x -= s;
                [
                    (cx - 2.0 * s, cz - s, 0),
                    (cx - 2.0 * s, cz, 3),
                    (cx - 2.0 * s, cz + s, 6),
                ]
            }
        };
        // Each cell: the first thing between 10 m over and 10 m under the
        // ground at the target, kept to a 128th of 10 m (a byte).
        let y = self.target.y;
        for (x, z, i) in cells {
            let hit = world.ground(Vec3::new(x, y + 10.0, z), Vec3::new(x, y - 10.0, z));
            self.grid[i] = match hit {
                Some(p) => {
                    let b = ((p.y - y) * 0.1 * 128.0) as i32 as i8;
                    f32::from(b.max(-126)) / 128.0 * 10.0 + y
                }
                None => MISS,
            };
        }
        if !evaluate {
            return Search::Looking;
        }
        // Level and dry enough: the patch's highest (or the water) less
        // its lowest under its slope tolerance.
        let lowest = self.grid.iter().copied().fold(f32::MAX, f32::min);
        let mut highest = self.grid.iter().copied().fold(f32::MIN, f32::max);
        let spot = Vec3::new(self.centre.x, self.centre.y - 10.0, self.centre.z);
        if let Some(w) = world.water(spot) {
            highest = highest.max(w);
        }
        if lowest == MISS || self.slope <= highest - lowest {
            self.centre.y = y + 1.0;
            return Search::Looking;
        }
        self.centre.y = lowest;
        // Clear near the ground (a ball the cell's size from `low` over
        // the spot down to `slope` over it), then clear above (one the
        // clear radius wide from `high` down to `low`).
        let at = |h: f32| Vec3::new(self.centre.x, lowest + h, self.centre.z);
        if world.column(at(self.low), self.low - self.slope, self.step) {
            return Search::Looking;
        }
        if world.column(at(self.high), self.high - self.low, self.clear) {
            return Search::Looking;
        }
        let found = Search::Found(Vec3::new(self.centre.x, lowest, self.centre.z));
        self.done = Some(found);
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_math::glam::Quat;

    /// Level ground at 0 but a block over x < 5 (3 m high), and a pole at
    /// (`pole`) that fills the column checks.
    struct Field {
        pole: Option<Vec3>,
    }

    impl PilotWorld for Field {
        fn ground(&mut self, from: Vec3, to: Vec3) -> Option<Vec3> {
            let h = if from.x < 5.0 { 3.0 } else { 0.0 };
            (from.y >= h && to.y <= h).then(|| Vec3::new(from.x, h, from.z))
        }
        fn sweep(&mut self, _: Vec3, _: Quat, _: Vec3, _: Vec3, _: f32) -> f32 {
            1.0
        }
        fn column(&mut self, top: Vec3, _: f32, radius: f32) -> bool {
            self.pole
                .is_some_and(|p| (p - top).with_y(0.0).length() < radius + 0.5)
        }
    }

    fn search(at: Vec3) -> LandingSearch {
        let size = (Vec3::new(-2.0, 0.0, -6.0), Vec3::new(2.0, 3.0, 6.0));
        LandingSearch::new(at, size, 10.0, 1.0, 1.5)
    }

    // On level open ground it settles on the target itself once its
    // window has filled (three thinks).
    #[test]
    fn on_open_level_ground_it_lands_on_the_point() {
        let mut s = search(Vec3::new(20.0, 0.0, 0.0));
        let mut w = Field { pole: None };
        assert_eq!(s.step(&mut w), Search::Looking);
        assert_eq!(s.step(&mut w), Search::Looking);
        let Search::Found(p) = s.step(&mut w) else {
            panic!("{:?}", s.result());
        };
        assert!((p - Vec3::new(20.0, 0.0, 0.0)).length() < 1e-3, "{p}");
    }

    // At the edge of a 3 m step the patch isn't level: it spirals out to
    // a level patch clear of the edge.
    #[test]
    fn at_a_step_it_spirals_out_to_level_ground() {
        let mut s = search(Vec3::new(5.0, 0.0, 0.0));
        let mut w = Field { pole: None };
        let mut found = None;
        for _ in 0..400 {
            if let Search::Found(p) = s.step(&mut w) {
                found = Some(p);
                break;
            }
        }
        let p = found.expect("a spot");
        assert!(p.x - s.step >= 5.0 || p.x + s.step < 5.0, "{p}");
    }

    // Something standing over the target keeps it off the target; asked
    // for a point far over the ground it fails at once.
    #[test]
    fn it_keeps_clear_of_what_stands_there_and_fails_off_the_ground() {
        let mut s = search(Vec3::new(20.0, 0.0, 0.0));
        let mut w = Field {
            pole: Some(Vec3::new(20.0, 0.0, 0.0)),
        };
        let mut found = None;
        for _ in 0..400 {
            if let Search::Found(p) = s.step(&mut w) {
                found = Some(p);
                break;
            }
        }
        let p = found.expect("a spot");
        assert!(
            (p - Vec3::new(20.0, 0.0, 0.0)).with_y(0.0).length() > s.clear,
            "{p}"
        );
        // Asked far off the ground: it fails at once.
        let mut s = search(Vec3::new(20.0, 30.0, 0.0));
        assert_eq!(s.step(&mut w), Search::Failed);
    }

    /// `pursuitcopter`'s test (its `FindLZ`): inner radius 6, outer 19,
    /// heights 1, 2.5 and 20, searched 40 m out, dry.
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

    // A drop zone test searches with the script's sizes: on open level
    // ground the point itself; by a pole, a spot clear of it by the outer
    // radius, within the search radius; asked off the ground, it fails
    // unless the test says the height doesn't matter.
    #[test]
    fn a_drop_zone_test_searches_with_the_scripts_sizes() {
        let at = Vec3::new(40.0, 0.0, 0.0);
        let mut s = LandingSearch::drop_zone(at, &PURSUIT);
        assert_eq!((s.radius, s.step, s.clear), (40.0, 6.0, 19.0));
        let mut w = Field { pole: None };
        let found = (0..3).map(|_| s.step(&mut w)).last();
        assert_eq!(found, Some(Search::Found(at)));
        let mut s = LandingSearch::drop_zone(at, &PURSUIT);
        let mut w = Field { pole: Some(at) };
        let p = (0..2000)
            .find_map(|_| match s.step(&mut w) {
                Search::Found(p) => Some(p),
                _ => None,
            })
            .expect("a spot");
        let off = (p - at).with_y(0.0).length();
        assert!(off > 19.0 && off <= 40.0 + 2.0 * 6.0, "{p}");
        let high = at + Vec3::Y * 30.0;
        let mut w = Field { pole: None };
        assert_eq!(
            LandingSearch::drop_zone(high, &PURSUIT).step(&mut w),
            Search::Failed
        );
        let free = DropZone {
            no_height_restriction: true,
            ..PURSUIT
        };
        assert_ne!(
            LandingSearch::drop_zone(high, &free).step(&mut w),
            Search::Failed
        );
    }

    // Over open water nothing is found within the search radius: the
    // cells under the water's surface are out of reach or not level with
    // it.
    #[test]
    fn a_dry_drop_zone_test_over_deep_water_fails() {
        struct Sea;
        impl PilotWorld for Sea {
            fn ground(&mut self, from: Vec3, to: Vec3) -> Option<Vec3> {
                (from.y >= -40.0 && to.y <= -40.0).then(|| from.with_y(-40.0))
            }
            fn sweep(&mut self, _: Vec3, _: Quat, _: Vec3, _: Vec3, _: f32) -> f32 {
                1.0
            }
            fn water(&mut self, _: Vec3) -> Option<f32> {
                Some(0.0)
            }
        }
        let mut s = LandingSearch::drop_zone(Vec3::ZERO, &PURSUIT);
        let end = (0..5000)
            .map(|_| s.step(&mut Sea))
            .find(|r| *r != Search::Looking);
        assert_eq!(end, Some(Search::Failed));
    }
}
