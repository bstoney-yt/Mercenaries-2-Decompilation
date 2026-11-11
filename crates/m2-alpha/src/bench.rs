//! A fixed flight over the island for measuring the frame rate
//! (`--bench <seconds>`): the free camera flies the same loop every time,
//! at a fixed speed and a height over the ground the world's index gives
//! (no streaming needed to know it), one simulation tick a frame, so two
//! builds draw and stream the same frames and can be compared.
//!
//! The route, speed, height and look are invented for measuring; they are
//! not the game's.

use m2_engine::WorldIndex;
use m2_math::glam::{Vec2, Vec3};

use crate::camera::FlyCamera;

/// The loop's corners, (x, z): the coast town north-west of the start,
/// the road east of it, the outpost by the jungle and the farmland north
/// of that, and back. Places runs have been seen in, low ground.
pub const ROUTE: [(f32, f32); 4] = [
    (-450.0, 1100.0),
    (46.0, 576.0),
    (540.0, 884.0),
    (676.0, 1288.0),
];

/// Metres a second along the route: a car's.
pub const SPEED: f32 = 20.0;

/// Metres over the highest ground the index knows near each point.
pub const CLEARANCE: f32 = 30.0;

/// How far round each point the ground is looked at, metres.
const AROUND: f32 = 150.0;

/// Metres between the route's samples.
const STEP: f32 = 25.0;

/// The camera looks at the route this far ahead, metres, and this far
/// down below its own height.
const LOOK_AHEAD: f32 = 80.0;
const LOOK_DOWN: f32 = 20.0;

/// The route flown: samples every [`STEP`] metres round the loop, each at
/// its flying height.
#[derive(Clone, Debug)]
pub struct Route {
    points: Vec<Vec3>,
}

impl Route {
    /// The loop over `index`'s ground.
    pub fn new(index: &WorldIndex) -> Self {
        Self::over(|x, z| index.rough_height(x, z))
    }

    /// The loop over ground of height `ground(x, z)`.
    pub fn over(ground: impl Fn(f32, f32) -> f32) -> Self {
        let corners: Vec<Vec2> = ROUTE.iter().map(|&(x, z)| Vec2::new(x, z)).collect();
        let mut flat = Vec::new();
        for (i, &a) in corners.iter().enumerate() {
            let b = corners[(i + 1) % corners.len()];
            let n = ((b - a).length() / STEP).ceil().max(1.0) as usize;
            flat.extend((0..n).map(|k| a.lerp(b, k as f32 / n as f32)));
        }
        let high = |p: Vec2| {
            [(0.0, 0.0), (1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)]
                .iter()
                .map(|&(dx, dz)| ground(p.x + dx * AROUND, p.y + dz * AROUND))
                .fold(f32::MIN, f32::max)
        };
        let heights: Vec<f32> = flat.iter().map(|&p| high(p)).collect();
        // Each sample at the highest of its neighbours', so the climb
        // starts before a rise.
        let n = heights.len();
        let points = (0..n)
            .map(|i| {
                let h = (0..5)
                    .map(|d| heights[(i + n + d - 2) % n])
                    .fold(f32::MIN, f32::max);
                Vec3::new(flat[i].x, h + CLEARANCE, flat[i].y)
            })
            .collect();
        Self { points }
    }

    /// Metres once round.
    pub fn length(&self) -> f32 {
        self.points.len() as f32 * STEP
    }

    /// Where along the loop `d` metres in (round and round).
    fn at(&self, d: f32) -> Vec3 {
        let n = self.points.len();
        let s = d.rem_euclid(self.length()) / STEP;
        let i = s.floor() as usize % n;
        self.points[i].lerp(self.points[(i + 1) % n], s.fract())
    }

    /// The camera `seconds` into the flight.
    pub fn pose(&self, seconds: f32) -> FlyCamera {
        let d = seconds * SPEED;
        let eye = self.at(d);
        let ahead = self.at(d + LOOK_AHEAD) - Vec3::Y * LOOK_DOWN;
        FlyCamera::looking_at(eye, ahead)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flight_keeps_over_the_ground_and_comes_round() {
        // A hill of 100 m under the third corner.
        let hill = Vec2::new(ROUTE[2].0, ROUTE[2].1);
        let route = Route::over(|x, z| {
            if Vec2::new(x, z).distance(hill) < 200.0 {
                100.0
            } else {
                0.0
            }
        });
        let at_hill = route.pose(0.0);
        assert!((at_hill.position.y - CLEARANCE).abs() < 1e-3);
        let lap = route.length() / SPEED;
        // Over the hill it flies at least its clearance over it.
        let over = (0..1000)
            .map(|k| route.pose(lap * k as f32 / 1000.0).position)
            .filter(|p| Vec2::new(p.x, p.z).distance(hill) < 50.0)
            .map(|p| p.y)
            .fold(f32::MAX, f32::min);
        assert!(over >= 100.0 + CLEARANCE - 1e-3, "{over}");
        // Once round, it is back where it started, looking down a little.
        let back = route.pose(lap);
        assert!(back.position.distance(at_hill.position) < 1e-2);
        assert!(back.pitch < 0.0);
        // About 2.9 km: a little over two minutes.
        assert!(
            (2800.0..3000.0).contains(&route.length()),
            "{}",
            route.length()
        );
    }
}
