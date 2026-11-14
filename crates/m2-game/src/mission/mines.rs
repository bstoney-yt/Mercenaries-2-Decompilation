//! Mines: the placed ones (`Mine (Human)`, `Mine (Vehicle)`, `Mine
//! (IED)`, `Mine (Water) (Light)`, whose script is `mine`, 0xcf210c94)
//! and the one she plants (`Proximity Mine (Planted)`, whose script is
//! `proximitymine`, 0x75cd1fb9). Both names are from the hashes.
//!
//! From samples (the scripts' listings):
//! - `mine.OnActivate(self, _, data)` blinks it
//!   (`global_weapon_c4land_60thsec`) and, when its data is 2 (`Mine
//!   (Human)`'s template says 2, the others 0), waits for anything
//!   labelled `human` within 1 m (`Event.ObjectProximity` with an
//!   `ObjectFilter` of that label) and kills itself (`Object.Kill`). The
//!   others wait to be killed. `mine.OnDeath(self)` cues
//!   `wpn_bomb_timer_01_finalstage` and, 0.75 s later for one labelled
//!   `HumanMine`, else 0.25 s, spawns `Explosion (Grenade)` (human) or
//!   `Explosion (AT Mine)` where it was.
//! - `proximitymine.OnActivate(self)` waits for anything labelled `human`
//!   within 6 m; then (0.001 s on) it removes itself and fires a `Grenade
//!   MG Projectile` straight up at 8 m/s, fused to go off after 1.8 m
//!   (`Airstrike.SpawnOrdnance`): a bounding mine.
//!
//! From samples (templates): the placed mines have `Health` 1 and
//! `PhysicsPropertyCrashable`; the thrown `Proximity Mine Projectile`'s
//! timer (6 s) spawns `Proximity Mine (Planted)` and removes it
//! (`TimerResponse` +4 = 1; the timer response 0x8227ED30, see
//! [`super::ammo`]).

use super::Mission;

/// `mine`'s module hash.
pub const MINE: u32 = 0xcf21_0c94;

/// `proximitymine`'s module hash.
pub const PROXIMITY_MINE: u32 = 0x75cd_1fb9;

impl Mission {
    /// The placed mines in play and alive (layer entities whose script is
    /// `mine`): guid and where each lies, for the game to kill those a
    /// vehicle runs into or a blast reaches (their script's `OnDeath`
    /// then sets them off).
    pub fn placed_mines(&self) -> Vec<(u32, [f32; 3])> {
        let w = self.world.borrow();
        self.layers
            .values()
            .flatten()
            .filter(|e| e.script == Some(MINE))
            .filter_map(|e| {
                let o = w.objects.get(&e.guid).filter(|o| o.alive)?;
                Some((e.guid, o.position))
            })
            .collect()
    }

    /// Whether `mine` and `proximitymine` are among the scripts run.
    pub fn runs_mine_scripts() -> bool {
        [MINE, PROXIMITY_MINE]
            .iter()
            .all(|h| super::OBJECT_SCRIPTS.iter().any(|(_, s)| s == h))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Command;
    use crate::mission::{Entity, GameData, OBJECT_SCRIPTS, PLAYER};

    #[test]
    fn both_scripts_are_run_by_their_names() {
        for (n, h) in [("mine", MINE), ("proximitymine", PROXIMITY_MINE)] {
            assert_eq!(m2_assets::name_hash(n), h);
            assert!(OBJECT_SCRIPTS.iter().any(|&(a, b)| a == n && b == h));
        }
    }

    fn entity(guid: u32, name: &str, data: u32) -> Entity {
        Entity {
            guid,
            name: name.into(),
            at: [10.0, 0.0, 10.0],
            yaw: 0.0,
            region: None,
            labels: Vec::new(),
            script: Some(MINE),
            script_data: Some(data),
            faction: None,
            path: Vec::new(),
            template: None,
            crew: Vec::new(),
            alarm: false,
            association: None,
        }
    }

    /// The `mine` script's own shape, for tests without the game's data.
    const MINE_SCRIPT: &str = r#"
        mine = {}
        local tEvents = {}
        local uHuman = ObjectFilter.Create()
        ObjectFilter.SetFilter(uHuman, "human")
        function mine.OnActivate(self, _, data)
            if data == 2 then
                tEvents[self] = Event.Create(Event.ObjectProximity,
                    { uHuman, self, "<", 1, false, false }, Object.Kill, { self })
            end
        end
        function mine.OnDeath(self)
            local x, y, z = Object.GetPosition(self)
            Event.Create(Event.TimerRelative, { 0.75 }, function()
                Pg.Spawn("Explosion (Grenade)", x, y, z)
            end)
        end"#;

    // A human mine kills itself when someone labelled human steps on it;
    // its death sets it off 0.75 s later; it is no longer listed.
    #[test]
    fn a_human_mine_goes_off_under_her() {
        let mut data = GameData::default();
        data.layers.insert(
            "mines".into(),
            vec![
                entity(0x100, "Mine (Human)", 2),
                entity(0x101, "Mine (Vehicle)", 0),
            ],
        );
        let mut m = Mission::new(data, "Mattias", [0.0; 3]).unwrap();
        m.lua().load(MINE_SCRIPT).exec().unwrap();
        m.start();
        m.preload("mines");
        m.tick(0.1);
        m.world()
            .borrow_mut()
            .objects
            .get_mut(&PLAYER)
            .unwrap()
            .labels
            .push("human".into());
        assert_eq!(m.placed_mines().len(), 2);
        m.put_player([10.5, 0.0, 10.0]);
        m.take_commands();
        let mut spawned = Vec::new();
        for _ in 0..8 {
            m.tick(0.25);
            spawned.extend(m.take_commands().into_iter().filter_map(|c| match c {
                Command::Spawn { what, position, .. } => Some((what, position)),
                _ => None,
            }));
        }
        assert_eq!(
            spawned,
            [("Explosion (Grenade)".to_owned(), [10.0, 0.0, 10.0])]
        );
        assert_eq!(m.placed_mines(), [(0x101, [10.0, 0.0, 10.0])]);
    }
}
