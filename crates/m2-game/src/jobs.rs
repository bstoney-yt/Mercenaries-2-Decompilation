//! The jobs: work a faction offers besides its contracts (`AllJob002`,
//! `PmcJob001`), as the game's scripts run them.
//!
//! The mission flow binds 16 (`WifMissionFlow`); each is a module of its
//! own over one of four classes of `MrxTaskJob` (from the scripts):
//!
//! - `MrxTaskJobVerifySet`: people to kill and verify, by name
//!   (`AllJob002_01_Target`), each with its own layers;
//! - `MrxTaskJobDestroySet`: things to destroy, by name;
//! - `MrxTaskJobDestroyType`: a standing bounty on whatever has a label
//!   (`"China"`, `"Billboard"`), paid for each one that the player herself
//!   destroys (`_SetHeroOnly`); it has no quota, so it never completes;
//! - `MrxTaskJobCollectType`: a collection, things of a label picked up
//!   (`PmcJob001`: 100 of `"SpareParts"`).
//!
//! The flow unlocks a job as the story gets there and it starts at once
//! (`MrxMissionFlow` activates it; no starter offers it). Its targets are
//! marked while it is tracked on the PDA's map
//! ([`Mission::track_mission`]). Each target done pays
//! (`MrxRewardData.GrantRewardKey("<job>_PerTarget")`), and the last one
//! completes the job, which awards its key (`HasKey("AllJob002")`).
//! [`read`] reads where a job stands from the scripts' own state.

use crate::events::as_guid;
use crate::mission::Mission;

/// Where a job stands, from its mission object (`WifMissionFlow.
/// _tActiveMissions[id].oMission`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JobView {
    /// It is on: unlocked and running.
    pub active: bool,
    /// Done: its key is awarded.
    pub completed: bool,
    /// Its class (`"MrxTaskJobVerifySet"`), when it is on.
    pub class: Option<String>,
    /// Targets done so far (`_nTargetsComplete`).
    pub counted: i64,
    /// What a bounty or a collection is over (`"China"`).
    pub label: Option<String>,
    /// How many a collection wants (`_nQuota`); none for a bounty.
    pub quota: Option<i64>,
    /// A set job's targets still to do, by guid.
    pub targets: Vec<u32>,
}

impl JobView {
    /// A standing bounty: paid for each, never complete.
    pub fn is_bounty(&self) -> bool {
        self.class.as_deref() == Some("MrxTaskJobDestroyType")
    }

    /// Its targets are people to verify.
    pub fn is_verify(&self) -> bool {
        self.class.as_deref() == Some("MrxTaskJobVerifySet")
    }
}

/// Whether `id` names a job as the mission flow does: a faction's three
/// letters, `Job`, three digits (`AllJob002`).
pub fn is_job_id(id: &str) -> bool {
    let b = id.as_bytes();
    b.len() == 9
        && b[0].is_ascii_uppercase()
        && b[1..3].iter().all(u8::is_ascii_lowercase)
        && &b[3..6] == b"Job"
        && b[6..].iter().all(u8::is_ascii_digit)
}

const STATUS: &str = r#"
local id = ...
local flow = rawget(_G, "WifMissionFlow")
if type(flow) ~= "table" then return false, false end
local t = flow._tActiveMissions and flow._tActiveMissions[id]
local o = t and t.oMission
local done = type(flow.HasKey) == "function" and flow.HasKey(id) and true or false
if type(o) ~= "table" then return false, done end
local class = nil
for _, c in ipairs({ "MrxTaskJobVerifySet", "MrxTaskJobDestroySet",
    "MrxTaskJobDestroyType", "MrxTaskJobCollectType" }) do
    local m = rawget(_G, c)
    if type(m) == "table" and o._Go == m._Go then class = c end
end
local targets = {}
if type(o._tTargets) == "table" then
    local names = {}
    for name, d in pairs(o._tTargets) do
        if type(d) == "table" and not d.bComplete then names[#names + 1] = name end
    end
    table.sort(names)
    for _, name in ipairs(names) do
        local g = Pg.GetGuidByName(name)
        if g then targets[#targets + 1] = g end
    end
end
return o:IsActive(), done or o:IsCompleted(), class, o._nTargetsComplete or 0,
    o._sLabelFilter, o._nQuota, targets
"#;

/// Where job `id` stands. Not on (and not completed) if the scripts have
/// not unlocked it.
pub fn read(mission: &Mission, id: &str) -> JobView {
    type Row = (
        bool,
        bool,
        Option<String>,
        Option<i64>,
        Option<String>,
        Option<i64>,
        Option<Vec<mlua::Value>>,
    );
    let row = mission
        .lua()
        .load(STATUS)
        .set_name("=job status")
        .call::<Row>(id);
    let Ok((active, completed, class, counted, label, quota, targets)) = row else {
        return JobView::default();
    };
    JobView {
        active,
        completed,
        class,
        counted: counted.unwrap_or(0),
        label,
        quota,
        targets: targets
            .unwrap_or_default()
            .iter()
            .filter_map(as_guid)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_ids_are_told_from_contracts() {
        assert!(is_job_id("AllJob002"));
        assert!(is_job_id("PmcJob001"));
        assert!(!is_job_id("AllCon002"));
        assert!(!is_job_id("AllJob02"));
        assert!(!is_job_id("alljob002"));
    }

    #[test]
    fn classes_say_what_kind_of_job() {
        let bounty = JobView {
            class: Some("MrxTaskJobDestroyType".into()),
            ..JobView::default()
        };
        assert!(bounty.is_bounty() && !bounty.is_verify());
        let verify = JobView {
            class: Some("MrxTaskJobVerifySet".into()),
            ..JobView::default()
        };
        assert!(verify.is_verify() && !verify.is_bounty());
    }
}
