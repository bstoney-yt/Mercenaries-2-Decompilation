//! The `AntiAir` and `HomingMissile` object scripts' side of homing locks:
//! the engine's four lock events handed to `AntiAir`'s handlers, and the
//! missile fired on a lock made an object of the scripts' world, so its
//! own script can blip it.
//!
//! **From the 360 code**: the lock update (`0x82294318`) and the launch
//! (`0x82293660`) send `HomingLockStart`, `HomingLockUpdate` (with
//! `nPercent`, the share of the lock done, 0 to 1), `HomingLockClear` and
//! `HomingLaunched` through `0x822956C0`, which fills `uPlayerGuid` (the
//! target's player), `uOwnerGuid` (who holds the weapon), `uVehicleGuid`
//! (the vehicle it is on) and `uAmmoGuid` (the missile), leaving out any
//! that is unset.
//!
//! **From samples** (the scripts' listings, data): `AntiAir` (`0xdede89ef`)
//! takes them in `_HomingLockStart`, `_HomingLockUpdate`,
//! `_HomingLockClear` and `_HomingLaunched`, each `(_, tData)`; the last
//! hands the data to `HomingMissile._HomingLaunched`, which finds the
//! missile by `GetFromGuid(uAmmoGuid)` and blips it, flashing every 0.1 s.
//! A locker whose vehicle runs `AntiAir` itself has its own blip blink
//! faster as the lock comes on (`_HomingLockUpdate`'s `AddObjective`).
//!
//! **Inferred**: on the 360 the four events reach `AntiAir` through the
//! HUD layout's radar widget, which binds those handlers; here they are
//! called straight in the mission's state, where the objects live. The
//! missile's object is made the way a script spawn is (its template's
//! object script noted), but the game is not told to make anything: it
//! already flies the missile and keeps the object where it is.

use mlua::{Function, Table, Value};

use super::{Mission, OBJECT_SCRIPTS, first_lines, object_script};
use crate::api::Object;
use crate::events::guid_value;

/// `AntiAir`'s module hash: the script of the anti-air vehicles and
/// emplacements (`M113 AA (VZ) (Driver)`, `Emplaced ZU23 (Driver)
/// (Seatbelted)`, `HMMWV (Avenger) (DriverGunner)`, ...), whose module
/// also takes the engine's homing lock events.
pub const ANTI_AIR: u32 = 0xdede_89ef;

/// `HomingMissile`'s module hash: the script of the missile templates
/// (`AA Missile`, `Vehicle AA Missile`, `AT Missile`, ...).
pub const HOMING_MISSILE: u32 = 0xb397_c517;

/// One of the four lock events.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HomingKind {
    /// `HomingLockStart`.
    Start,
    /// `HomingLockUpdate`, with `nPercent`: the share done, 0 to 1.
    Update(f32),
    /// `HomingLockClear`.
    Clear,
    /// `HomingLaunched`.
    Launched,
}

impl HomingKind {
    /// The event's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Start => "HomingLockStart",
            Self::Update(_) => "HomingLockUpdate",
            Self::Clear => "HomingLockClear",
            Self::Launched => "HomingLaunched",
        }
    }

    /// `AntiAir`'s handler of it.
    pub fn handler(self) -> &'static str {
        match self {
            Self::Start => "_HomingLockStart",
            Self::Update(_) => "_HomingLockUpdate",
            Self::Clear => "_HomingLockClear",
            Self::Launched => "_HomingLaunched",
        }
    }
}

/// A lock event for the scripts, with the guids `0x822956C0` sends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HomingPost {
    pub kind: HomingKind,
    /// The target's player (`uPlayerGuid`).
    pub player: u32,
    /// Who holds the weapon (`uOwnerGuid`).
    pub owner: u32,
    /// The vehicle the weapon is on (`uVehicleGuid`), if one.
    pub vehicle: Option<u32>,
    /// The missile (`uAmmoGuid`), for `HomingLaunched`.
    pub ammo: Option<u32>,
}

impl Mission {
    /// Hands a lock event to `AntiAir`'s handler of it. Whether it ran;
    /// a failure is logged.
    pub fn homing_post(&mut self, post: HomingPost) -> bool {
        let lua = self.host.lua();
        let result = event_table(lua, &post).and_then(|data| {
            lua.globals()
                .get::<Table>("AntiAir")?
                .get::<Function>(post.kind.handler())?
                .call::<()>((Value::Nil, data))
        });
        match result {
            Ok(()) => true,
            Err(e) => {
                self.log.push(format!(
                    "  AntiAir.{}: {}",
                    post.kind.handler(),
                    first_lines(&e.to_string())
                ));
                false
            }
        }
    }

    /// Makes the missile a lock fired (`template`, at `at`) an object of
    /// the scripts' world and runs its template's object script on it now
    /// (`OnActivate`), so `HomingLaunched` finds it. Its guid; move it with
    /// [`place`](Self::place) and take it away with
    /// [`round_gone`](Self::round_gone).
    pub fn homing_round(&mut self, template: &str, at: [f32; 3]) -> u32 {
        let (guid, script) = {
            let mut w = self.world.borrow_mut();
            let guid = w.spawn_guid();
            w.insert(guid, Object::new(template, at));
            w.note_script(guid, &template.to_ascii_lowercase());
            // Run now rather than at the next tick.
            let i = w.script_spawns.iter().position(|s| s.0 == guid);
            (guid, i.map(|i| w.script_spawns.remove(i)))
        };
        if let Some((_, script, data)) = script
            && let Some(Err(f)) = object_script(self.host.lua(), script, true, guid, data)
        {
            self.log.push(format!("  {f} (missile {guid:#x})"));
        }
        guid
    }

    /// The missile `guid` ([`homing_round`](Self::homing_round)) burst or
    /// went: out of the scripts' world, its script told.
    pub fn round_gone(&mut self, guid: u32) {
        self.world.borrow_mut().remove_object(guid);
    }

    /// Whether `AntiAir` and `HomingMissile` are among the scripts run.
    pub fn runs_homing_scripts() -> bool {
        [ANTI_AIR, HOMING_MISSILE]
            .iter()
            .all(|h| OBJECT_SCRIPTS.iter().any(|(_, s)| s == h))
    }
}

/// The event's table, as `0x822956C0` fills it.
fn event_table(lua: &mlua::Lua, post: &HomingPost) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("EventType", post.kind.name())?;
    t.set("uPlayerGuid", guid_value(post.player))?;
    t.set("uOwnerGuid", guid_value(post.owner))?;
    if let Some(v) = post.vehicle {
        t.set("uVehicleGuid", guid_value(v))?;
    }
    if let Some(a) = post.ammo {
        t.set("uAmmoGuid", guid_value(a))?;
    }
    if let HomingKind::Update(share) = post.kind {
        t.set("nPercent", share)?;
    }
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mission::GameData;

    fn mission() -> Mission {
        let mut data = GameData::default();
        data.template_scripts
            .insert("aa missile".into(), (HOMING_MISSILE, None));
        Mission::new(data, "Mattias", [0.0; 3]).unwrap()
    }

    #[test]
    fn both_scripts_are_run() {
        assert!(Mission::runs_homing_scripts());
    }

    // Each event goes to its handler as (_, tData), with the guids set and
    // the unset ones left out.
    #[test]
    fn lock_events_reach_antiair_handlers() {
        let mut m = mission();
        m.lua()
            .load(
                r#"heard = {}
                AntiAir = {}
                for _, n in ipairs({"_HomingLockStart", "_HomingLockUpdate",
                        "_HomingLockClear", "_HomingLaunched"}) do
                    AntiAir[n] = function(_, t)
                        table.insert(heard, n .. " " .. t.EventType .. " "
                            .. tostring(t.nPercent) .. " "
                            .. tostring(t.uVehicleGuid ~= nil) .. " "
                            .. tostring(t.uAmmoGuid ~= nil) .. " "
                            .. tostring(t.uOwnerGuid == t.uPlayerGuid))
                    end
                end"#,
            )
            .exec()
            .unwrap();
        let post = |kind, vehicle, ammo| HomingPost {
            kind,
            player: 1,
            owner: 0x50,
            vehicle,
            ammo,
        };
        assert!(m.homing_post(post(HomingKind::Start, Some(0x50), None)));
        assert!(m.homing_post(post(HomingKind::Update(0.5), None, None)));
        assert!(m.homing_post(post(HomingKind::Clear, None, None)));
        assert!(m.homing_post(post(HomingKind::Launched, None, Some(9))));
        let heard: Vec<String> = m.lua().globals().get("heard").unwrap();
        assert_eq!(
            heard,
            [
                "_HomingLockStart HomingLockStart nil true false false",
                "_HomingLockUpdate HomingLockUpdate 0.5 false false false",
                "_HomingLockClear HomingLockClear nil false false false",
                "_HomingLaunched HomingLaunched nil false true false",
            ]
        );
    }

    // The missile's object runs its script's OnActivate as it is made, so
    // HomingLaunched finds it; it goes from the world when it is gone.
    #[test]
    fn a_missile_is_activated_before_launch_is_heard() {
        let mut m = mission();
        m.lua()
            .load(
                r#"order = {}
                HomingMissile = { OnActivate = function(g)
                    table.insert(order, "activate") ; seen = g end }
                AntiAir = { _HomingLaunched = function(_, t)
                    table.insert(order, "launched " .. tostring(t.uAmmoGuid == seen)) end }"#,
            )
            .exec()
            .unwrap();
        let g = m.homing_round("AA Missile", [1.0, 2.0, 3.0]);
        assert!(m.world().borrow().objects.contains_key(&g));
        m.homing_post(HomingPost {
            kind: HomingKind::Launched,
            player: 1,
            owner: 0x50,
            vehicle: None,
            ammo: Some(g),
        });
        let order: Vec<String> = m.lua().globals().get("order").unwrap();
        assert_eq!(order, ["activate", "launched true"]);
        m.round_gone(g);
        assert!(!m.world().borrow().objects.contains_key(&g));
    }

    // A handler's error is logged, not raised.
    #[test]
    fn a_failing_handler_is_logged() {
        let mut m = mission();
        m.lua().load("AntiAir = {}").exec().unwrap();
        m.take_log();
        assert!(!m.homing_post(HomingPost {
            kind: HomingKind::Start,
            player: 1,
            owner: 2,
            vehicle: None,
            ammo: None,
        }));
        assert!(m.take_log().iter().any(|l| l.contains("_HomingLockStart")));
    }
}
