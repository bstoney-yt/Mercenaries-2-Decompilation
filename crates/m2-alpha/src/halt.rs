//! Notes, for the log, what stops the player's vehicle: a sudden halt (a
//! playtest's El Grande stopped dead from 26 m/s in the middle of the
//! road, then would not move) and being held while she drives on. Each
//! names what her vehicle's body touches: statics by key, other vehicles
//! by name. Logging only: nothing here changes how anything moves.

use m2_engine::{Physics, VehicleId};

use crate::scene::ParkedVehicle;

/// Speed lost in one tick that counts as a halt, m/s: 8 in a sixtieth of
/// a second is some 49 g (chosen for the log; braking hard loses well
/// under 0.5 a tick).
const HALT: f32 = 8.0;
/// Seconds of throttle without getting going that count as held.
const HELD_SECONDS: f32 = 2.0;
/// Under this, m/s, she is not getting going.
const CRAWL: f32 = 0.5;

/// What the watch keeps between ticks.
#[derive(Debug, Default)]
pub struct Halts {
    /// Her vehicle (its index in the scene's) and its speed last tick.
    last: Option<(usize, f32)>,
    /// Seconds she has been held so far, and whether that was logged.
    held: (f32, bool),
}

impl Halts {
    /// Watches her vehicle `id` (`index` in `vehicles`) after a step:
    /// going `speed` with `throttle` asked. Logs a halt or a hold with
    /// what its body touches.
    pub fn watch(
        &mut self,
        physics: &Physics,
        (id, index): (VehicleId, usize),
        vehicles: &[ParkedVehicle],
        speed: f32,
        throttle: f32,
        dt: f32,
    ) {
        let before = self.last.filter(|l| l.0 == index).map(|l| l.1);
        let (halt, held) = self.step(index, speed, throttle, dt);
        if !halt && !held {
            return;
        }
        let label = vehicles.get(index).map_or("?", |v| v.spec.label.as_str());
        let what = touching(physics, id, vehicles);
        if halt {
            eprintln!(
                "halt: the {label} went from {:.1} to {speed:.1} m/s in a tick; {what}",
                before.unwrap_or(speed)
            );
        }
        if held {
            let (down, all) = physics.vehicle_contacts(id);
            eprintln!(
                "halt: the {label} held for {HELD_SECONDS:.0} s at throttle {throttle:.1}; wheels down {down} of {all}; {what}"
            );
        }
    }

    /// One tick of the watch: whether vehicle `index` going `speed`
    /// halted this tick, and whether it has now been held long enough to
    /// log (once a hold).
    fn step(&mut self, index: usize, speed: f32, throttle: f32, dt: f32) -> (bool, bool) {
        let before = match self.last {
            Some((was, s)) if was == index => s,
            _ => {
                self.held = (0.0, false);
                speed
            }
        };
        self.last = Some((index, speed));
        let halt = before.abs() - speed.abs() >= HALT;
        let mut held = false;
        if throttle.abs() > 0.5 && speed.abs() < CRAWL {
            self.held.0 += dt;
            if self.held.0 >= HELD_SECONDS && !self.held.1 {
                self.held.1 = true;
                held = true;
            }
        } else {
            self.held = (0.0, false);
        }
        (halt, held)
    }

    /// Forgets her vehicle (she got out).
    pub fn clear(&mut self) {
        self.last = None;
        self.held = (0.0, false);
    }
}

/// What vehicle `id`'s body touches, for the log.
fn touching(physics: &Physics, id: VehicleId, vehicles: &[ParkedVehicle]) -> String {
    let mut parts: Vec<String> = physics
        .vehicle_bumps(id)
        .into_iter()
        .map(|(them, at, _)| {
            let name = vehicles
                .iter()
                .find(|v| v.id == Some(them))
                .map_or("a vehicle not in the scene", |v| v.spec.label.as_str());
            format!("the {name} at {at:.1}")
        })
        .collect();
    for (key, at, (lo, hi)) in physics.vehicle_touching(id) {
        match key {
            Some(k) => parts.push(format!("{k:?} at {at:.1} (box {lo:.1}..{hi:.1})")),
            // Another vehicle's body, named above; anything else solid
            // and not static (a loose prop) by its box.
            None if physics
                .vehicle_bumps(id)
                .iter()
                .any(|b| b.1.distance(at) < 0.01) => {}
            None => parts.push(format!(
                "something not static at {at:.1} (box {lo:.1}..{hi:.1})"
            )),
        }
    }
    if parts.is_empty() {
        "touching nothing but the ground".to_owned()
    } else {
        format!("touching {}", parts.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dead_stop_and_a_long_hold_are_noted_once() {
        let id = 3;
        let mut h = Halts::default();
        let dt = 1.0 / 60.0;
        assert_eq!(h.step(id, 26.0, 1.0, dt), (false, false));
        // Hard braking loses well under a metre a second a tick.
        assert_eq!(h.step(id, 25.6, -1.0, dt), (false, false));
        // A wall: 25.6 to 0.4 in one tick.
        assert_eq!(h.step(id, 0.4, 1.0, dt), (true, false));
        // Throttle on, going nowhere: noted after two seconds, once.
        let mut noted = 0;
        for _ in 0..(4.0 / dt) as usize {
            noted += usize::from(h.step(id, 0.1, -1.0, dt).1);
        }
        assert_eq!(noted, 1);
        // Free again, then held again: noted again.
        h.step(id, 3.0, -1.0, dt);
        let mut again = 0;
        for _ in 0..(3.0 / dt) as usize {
            again += usize::from(h.step(id, 0.0, 1.0, dt).1);
        }
        assert_eq!(again, 1);
        // Getting out forgets the speed, so getting back in is no halt.
        h.clear();
        assert_eq!(h.step(id, 0.0, 0.0, dt), (false, false));
    }
}
