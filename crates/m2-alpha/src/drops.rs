//! What a soldier leaves when killed: the game's `Soldier` object script
//! (75e8c74d, named by soldier templates' `ObjectScript`) picks a pickup
//! on its death, as decoded in round 138.
//!
//! Every death counts, once per session. A soldier killed in a seat leaves
//! nothing; a heavy soldier always leaves something, any other only every
//! third death. Rocket soldiers leave rockets, heavy ones bullets, the
//! rest a small ammunition pickup; but if a player's grenades are fewer
//! than a roll in [0, 8) it is grenades instead, and if their health is
//! under a roll in [0, 80) it is a health pickup. It lies where the body
//! fell.

/// What a dead soldier was, by its template's labels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Soldier {
    pub heavy: bool,
    pub rocket: bool,
    pub machine_gun: bool,
    /// Killed in a vehicle's seat.
    pub seated: bool,
}

/// The pickups' templates, by name.
pub const SMALL: &str = "Ammo Pickup (Small)";
pub const ROCKET: &str = "Ammo Pickup (Rocket)";
pub const BULLET: &str = "Ammo Pickup (Bullet)";
pub const GRENADES: &str = "Ammo Pickup (Grenades)";
pub const HEALTH: &str = "Health Pickup";

/// The script's count of deaths and its random rolls.
#[derive(Clone, Debug)]
pub struct Drops {
    deaths: u32,
    seed: u64,
}

impl Default for Drops {
    fn default() -> Self {
        Self {
            deaths: 0,
            seed: 0x9e37_79b9_7f4a_7c15,
        }
    }
}

impl Drops {
    /// A roll in [0, 1).
    fn roll(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 40) as f32 / (1u64 << 24) as f32
    }

    /// The pickup `soldier` leaves on dying, if any: the player has
    /// `grenades` (if she carries them at all) and `health` of 100.
    pub fn on_death(
        &mut self,
        soldier: Soldier,
        grenades: Option<u32>,
        health: f32,
    ) -> Option<&'static str> {
        self.deaths += 1;
        if soldier.seated || !(soldier.heavy || self.deaths.is_multiple_of(3)) {
            return None;
        }
        let mut pick = if soldier.rocket && !soldier.machine_gun {
            ROCKET
        } else if soldier.heavy {
            BULLET
        } else {
            SMALL
        };
        if grenades.is_some_and(|g| (g as f32) < self.roll() * 8.0) {
            pick = GRENADES;
        }
        if health < self.roll() * 80.0 {
            pick = HEALTH;
        }
        Some(pick)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_third_soldier_leaves_grenades_to_her_without() {
        let mut drops = Drops::default();
        let plain = Soldier::default();
        let left: Vec<_> = (0..6)
            .map(|_| drops.on_death(plain, Some(0), 100.0))
            .collect();
        assert_eq!(
            left,
            [None, None, Some(GRENADES), None, None, Some(GRENADES)]
        );
    }

    #[test]
    fn heavies_always_leave_something_and_full_players_get_ammo() {
        let mut drops = Drops::default();
        let heavy = Soldier {
            heavy: true,
            ..Soldier::default()
        };
        for _ in 0..5 {
            assert_eq!(drops.on_death(heavy, Some(9), 100.0), Some(BULLET));
        }
        let seated = Soldier {
            seated: true,
            ..heavy
        };
        assert_eq!(drops.on_death(seated, Some(9), 100.0), None);
        let rocket = Soldier {
            rocket: true,
            heavy: true,
            ..Soldier::default()
        };
        assert_eq!(drops.on_death(rocket, None, 100.0), Some(ROCKET));
    }

    #[test]
    fn the_hurt_mostly_get_health() {
        let mut drops = Drops::default();
        let heavy = Soldier {
            heavy: true,
            ..Soldier::default()
        };
        let health = (0..100)
            .filter(|_| drops.on_death(heavy, Some(9), 1.0) == Some(HEALTH))
            .count();
        assert!(health > 95, "{health}");
    }
}
