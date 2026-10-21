//! The scripts' `Report` library: witnesses reporting the player.
//!
//! The faction manager (`MrxFactionManager.Setup`) gives the engine its
//! reporter callback, `Report.Init({ Callback = HandleReporter,
//! SimultaneousReporters = 1, LossThreshold = 1, GoalPriority = 10 })`.
//! When the player wrongs a faction (hurts or kills one of theirs) the
//! game picks a witness of that faction and offers him to the callback
//! (`HandleReporter(reporter, 0)`): the manager either turns him down
//! (`Report.Failed`) or has him call in (`HandleReporter0`, his
//! `Reporting.CallIn` line). Once he gets through (state 1,
//! `HandleReporter1`) it starts the HUD's reporting timer
//! (`Hud.FactionDisplay:StartTimer { nDuration = 10, fCallback =
//! FinishedReporting }`); when that runs out the manager asks
//! `Report.GetInfractions(reporter)` for what he saw, docks the PMC's
//! standing with that faction by it (`DamagePerson` 3, `DestroyPerson` 50,
//! `DamageObject` 1, `DestroyObject` 25, `Trespassing` 20, `Hijack` 10
//! each, at most 60) and calls `Report.Completed`. A reporter stopped
//! before then is state 2 (`HandleReporter2`, which takes the display
//! down; `DisableReporting(true)` sends it too, as the HQ does on entry).
//!
//! # Provenance
//!
//! From the scripts (`MrxFactionManager`'s `Setup`, `HandleReporter`,
//! `HandleReporter0/1/2`, `ValidateReporter`, `DisableReporting` and
//! `FinishedReporting`): the calls, what each state does and the table's
//! keys (each `{ _, count }`, the count second). Inferred: what the first
//! of each pair holds (0 here). Invented (by the alpha, which drives it):
//! who is picked as the reporter and how long he takes to get through.

use std::collections::HashMap;

use mlua::{Function, Lua, MultiValue, Table, Value};

/// The kinds of infraction the faction manager tallies, as its
/// `GetInfractions` table keys them.
pub const KINDS: [&str; 7] = [
    "DamageObject",
    "DestroyObject",
    "Trespassing",
    "Hijack",
    "SpecialEvent",
    "DestroyPerson",
    "DamagePerson",
];

/// The reporting under way and what each faction has against her.
#[derive(Debug, Default)]
pub struct Reports {
    /// The faction manager's reporter callback (`Report.Init`).
    pub callback: Option<Function>,
    /// What each faction (by its label: `OC`, `Allied`) has seen her do
    /// since its last report, by kind.
    pub seen: HashMap<String, HashMap<&'static str, u32>>,
    /// Reporters reporting, each with what they report.
    pub reporting: HashMap<u32, HashMap<&'static str, u32>>,
    /// How much of what a faction's witnesses saw they report, by the
    /// faction's label in lower case (`Ai.SetInfractionMultiplier`: 0.2
    /// for a faction while the PMC holds a contract of theirs, 0.1 or 0
    /// in three missions); 1 if never set. Read as a scale on the counts
    /// `GetInfractions` answers: inferred from the name and the values,
    /// the engine's use of it is not seen.
    pub multipliers: HashMap<String, f32>,
    /// The scale each reporter's counts are answered with (his
    /// faction's multiplier when he was offered).
    pub scale: HashMap<u32, f32>,
    /// Reporters the manager said were done or turned down, since the
    /// game last asked ([`Reports::take_ended`]): whether completed.
    pub ended: Vec<(u32, bool)>,
}

impl Reports {
    /// She did an infraction of `kind` to faction `faction`.
    pub fn record(&mut self, faction: &str, kind: &'static str) {
        *self
            .seen
            .entry(faction.to_owned())
            .or_default()
            .entry(kind)
            .or_default() += 1;
    }

    /// Reporters turned down or done since the last call.
    pub fn take_ended(&mut self) -> Vec<(u32, bool)> {
        std::mem::take(&mut self.ended)
    }

    /// What a reporter reports, as `GetInfractions` answers it.
    pub fn infractions(&self, lua: &Lua, reporter: u32) -> mlua::Result<Table> {
        let t = lua.create_table()?;
        let seen = self.reporting.get(&reporter);
        let scale = self.scale.get(&reporter).copied().unwrap_or(1.0);
        for kind in KINDS {
            let n = seen.and_then(|s| s.get(kind)).copied().unwrap_or(0);
            let count = if scale == 1.0 {
                Value::Integer(n as mlua::Integer)
            } else {
                Value::Number(f64::from(n) * f64::from(scale))
            };
            t.set(kind, lua.create_sequence_from([Value::Integer(0), count])?)?;
        }
        Ok(t)
    }

    /// Reporter `reporter` of the faction labelled `faction` is offered
    /// with what it has seen; his counts are scaled by its multiplier.
    pub fn offer(&mut self, reporter: u32, faction: &str) {
        let seen = self.seen.remove(faction).unwrap_or_default();
        self.reporting.insert(reporter, seen);
        let m = self.multipliers.get(&faction.to_ascii_lowercase());
        self.scale.insert(reporter, m.copied().unwrap_or(1.0));
    }
}

/// Calls the reporter callback for `reporter` with `state` (0 offered,
/// 1 through and reporting, 2 stopped), with the world released.
pub fn tell(world: &crate::api::Shared, reporter: u32, state: i64) -> mlua::Result<()> {
    let callback = world.borrow().reports.callback.clone();
    if let Some(f) = callback {
        f.call::<()>(MultiValue::from_vec(vec![
            crate::events::guid_value(reporter),
            Value::Integer(state as mlua::Integer),
        ]))?;
    }
    Ok(())
}
