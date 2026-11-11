//! How fast a vehicle drives a path it is sent along, as the 360's mind
//! does it (`MindPathMove`, 0x822B4538).
//!
//! From the 360 code: each think, by what its mind drives (the mind's
//! +0x14 top byte, set by 0x822BB608), a car moves by `CarMove` 0x822AC458
//! (0x822AC008 toward the path follower's point, the mind's +0x2DC) at 50
//! times the mind's speed factor (+0x7EC; 0x820B7FB8), a tank by
//! 0x822B6150 at 25 times it (0x820F02E4), a boat by 0x822A8028 at 40
//! times it (0x820BAFA8). The factor is 0.4 from the mind's set-up
//! (0x822BDAD0), and the move request's +0x24 in its place when that is
//! over 0 (`MindPathMove`'s enter, 0x822B4908). So a car asks 20 m/s, a
//! tank 10 and a boat 16 unless its request says.
//!
//! Inferred: a placed patrol's `Haste` (0.175 to 0.35 on the install's
//! spawners) is the request's +0x24 (what fills it from a spawner isn't
//! traced).

/// The mind's speed factor before a move request gives one (0x822BDAD0).
pub const DEFAULT_FACTOR: f32 = 0.4;

/// What `MindPathMove` drives a vehicle with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mover {
    /// `CarMove` 0x822AC458.
    Car,
    /// 0x822B6150.
    Tank,
    /// 0x822A8028.
    Boat,
}

impl Mover {
    /// Metres a second for each unit of the speed factor.
    pub fn per_factor(self) -> f32 {
        match self {
            Self::Car => 50.0,
            Self::Tank => 25.0,
            Self::Boat => 40.0,
        }
    }

    /// The speed it asks, m/s, for a request's speed `factor` (its +0x24;
    /// [`DEFAULT_FACTOR`] unless over 0).
    pub fn speed(self, factor: Option<f32>) -> f32 {
        self.per_factor() * factor.filter(|&f| f > 0.0).unwrap_or(DEFAULT_FACTOR)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_moves_ask_their_drivers_speeds() {
        assert_eq!(Mover::Car.speed(None), 20.0);
        assert_eq!(Mover::Tank.speed(None), 10.0);
        assert_eq!(Mover::Boat.speed(None), 16.0);
        assert_eq!(Mover::Car.speed(Some(0.3)), 15.0);
        // A request's 0 leaves the set-up's factor.
        assert_eq!(Mover::Car.speed(Some(0.0)), 20.0);
    }
}
