//! Her vehicle disguise: driving a faction's vehicle, she passes for one
//! of theirs, as long as she doesn't give herself away.
//!
//! What the game's data decides:
//! - Disguises work only while the scripts switch them on
//!   (`Player.SetVehicleDisguise`, `MrxMissionFlow.SetVehicleDisguiseEnabled`).
//! - She is disguised as the faction of the vehicle she rides
//!   (`MrxFactionManager.GetPerceivedFaction`: `Object.IsDisguised(her)`,
//!   then the faction of `Vehicle.GetFromRider(her)`), told to the HUD's
//!   gauge as a level from 0 to 100 and whether it holds
//!   (`MrxGuiHudVehicleDisguise.SetDisguiseLevel` clamps to 0..100).
//! - What gives it away (Fiona's line `Fiona-In-Mission-Contract-Vz01-217`):
//!   honking the horn, staying near people too long, getting out of the
//!   car, running someone over.
//!
//! Invented: how much each costs and how it comes back ([`HONK_COST`],
//! [`LINGER`], [`RECOVER`], [`NEAR`]); that hurting one of theirs any
//! other way gives it away too; that once given away it stays so until
//! she gets out.

/// What a honk costs her disguise, of 100 (invented).
pub const HONK_COST: f32 = 30.0;
/// What each second near the faction's people costs, and what each
/// second away from them gives back (invented).
pub const LINGER: f32 = 12.0;
pub const RECOVER: f32 = 5.0;
/// How near counts as near, metres (invented).
pub const NEAR: f32 = 12.0;

/// Her disguise as it stands.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Disguise {
    /// The faction she passes for (by the faction manager's
    /// abbreviation, `All`), and how much of the disguise is left.
    pub faction: Option<&'static str>,
    pub level: f32,
    /// Given away: she shows as herself until she gets out.
    pub blown: bool,
    /// The vehicle the disguise is of (scene index).
    vehicle: Option<usize>,
}

/// What the disguise hears of a tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tick {
    /// Whether the scripts have disguises on.
    pub enabled: bool,
    /// The vehicle she rides (scene index), and its faction's
    /// abbreviation (`None` for a civilian's or her own).
    pub vehicle: Option<usize>,
    pub faction: Option<&'static str>,
    /// Horn presses this tick.
    pub honks: u32,
    /// The faction's people within [`NEAR`] of her.
    pub near: usize,
    /// She hurt someone of a faction this tick (run over, or otherwise).
    pub hurt_someone: bool,
}

impl Disguise {
    /// Runs the disguise for `dt` seconds of `t`; what the HUD and the
    /// scripts are told: how much is left and whether it holds, `None`
    /// out of a disguise.
    pub fn update(&mut self, t: Tick, dt: f32) -> Option<(f32, bool)> {
        let (Some(vehicle), Some(faction), true) = (t.vehicle, t.faction, t.enabled) else {
            *self = Self::default();
            return None;
        };
        if self.vehicle != Some(vehicle) || self.faction != Some(faction) {
            *self = Self {
                faction: Some(faction),
                level: 100.0,
                blown: false,
                vehicle: Some(vehicle),
            };
        }
        if !self.blown {
            self.level -= HONK_COST * t.honks as f32;
            self.level += if t.near > 0 { -LINGER } else { RECOVER } * dt;
            self.level = self.level.min(100.0);
            if t.hurt_someone || self.level <= 0.0 {
                self.level = 0.0;
                self.blown = true;
                eprintln!("disguise: given away ({faction})");
            }
        }
        Some((self.level, !self.blown))
    }

    /// The faction she passes for now, if the disguise holds.
    pub fn holds(&self) -> Option<&'static str> {
        self.faction.filter(|_| !self.blown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick(faction: Option<&'static str>) -> Tick {
        Tick {
            enabled: true,
            vehicle: Some(3),
            faction,
            ..Tick::default()
        }
    }

    #[test]
    fn a_factions_vehicle_disguises_her_till_she_gets_out() {
        let mut d = Disguise::default();
        assert_eq!(d.update(tick(Some("All")), 0.1), Some((100.0, true)));
        assert_eq!(d.holds(), Some("All"));
        // Out of it (or in a civilian's, or with disguises off): none.
        assert_eq!(d.update(Tick::default(), 0.1), None);
        assert_eq!(d.holds(), None);
        assert_eq!(d.update(tick(None), 0.1), None);
        let off = Tick {
            enabled: false,
            ..tick(Some("All"))
        };
        assert_eq!(d.update(off, 0.1), None);
    }

    #[test]
    fn honks_lingering_and_hurting_people_give_it_away() {
        let mut d = Disguise::default();
        d.update(tick(Some("Chi")), 0.0);
        let honk = Tick {
            honks: 1,
            ..tick(Some("Chi"))
        };
        assert_eq!(d.update(honk, 0.0), Some((100.0 - HONK_COST, true)));
        // Near their people it runs down, and away from them comes back.
        let near = Tick {
            near: 2,
            ..tick(Some("Chi"))
        };
        let (level, _) = d.update(near, 1.0).unwrap();
        assert!((level - (100.0 - HONK_COST - LINGER)).abs() < 1e-3);
        let (back, _) = d.update(tick(Some("Chi")), 1.0).unwrap();
        assert!((back - level - RECOVER).abs() < 1e-3);
        // Running someone over gives it away, for good in this vehicle.
        let hurt = Tick {
            hurt_someone: true,
            ..tick(Some("Chi"))
        };
        assert_eq!(d.update(hurt, 0.0), Some((0.0, false)));
        assert_eq!(d.update(tick(Some("Chi")), 10.0), Some((0.0, false)));
        assert_eq!(d.holds(), None);
        // Another vehicle: a fresh disguise.
        let other = Tick {
            vehicle: Some(4),
            ..tick(Some("Chi"))
        };
        assert_eq!(d.update(other, 0.0), Some((100.0, true)));
    }

    #[test]
    fn honking_it_to_nothing_gives_it_away() {
        let mut d = Disguise::default();
        let honk = Tick {
            honks: 4,
            ..tick(Some("Gur"))
        };
        assert_eq!(d.update(honk, 0.0), Some((0.0, false)));
    }
}
