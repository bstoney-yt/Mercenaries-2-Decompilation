//! The contract tracker: which of the story's contracts play through in
//! this build, kept in `docs/contracts-status.toml` and shown on the
//! launcher's Contracts tab (also a public roadmap).
//!
//! The file holds only our own text: contract ids, a status, a short note,
//! issue numbers and where the status came from. No game text, pictures or
//! 360 data.
//!
//! A status is only as good as its evidence: `playable` needs a recorded
//! autoplay or playtest result ([`validate`] refuses it otherwise), and a
//! contract with no recorded result is `unknown`. The nightly autoplay
//! runner's `night.json` ([`update_from_night`]) moves statuses on from
//! its runs; entries marked `locked` (a playtest's word) are left alone.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The story's 52 contracts in the PC release, in story order (the ids of
/// the contract scripts in `vz.wad`; the nightly runner lists them from
/// the install the same way). From samples.
pub const CONTRACTS: [&str; 52] = [
    "VzaCon001",
    "PmcCon001",
    "PmcCon002",
    "PmcCon003",
    "PmcCon004",
    "PmcCon013",
    "PmcCon015",
    "PmcCon016",
    "PmcCon018",
    "PmcCon031",
    "PmcCon032",
    "PmcCon033",
    "PmcCon034",
    "MecCon001",
    "JetCon001",
    "OilCon001",
    "OilCon002",
    "OilCon003",
    "OilCon005",
    "OilCon020",
    "OilCon021",
    "OilCon050",
    "OilCon051",
    "OilCon052",
    "GurCon001",
    "GurCon002",
    "GurCon003",
    "GurCon005",
    "GurCon050",
    "GurCon052",
    "GurCon053",
    "PirCon001",
    "PirCon002",
    "PirCon003",
    "PirCon004",
    "PirCon051",
    "PirCon052",
    "AllCon001",
    "AllCon002",
    "AllCon003",
    "AllCon008",
    "AllCon050",
    "AllCon052",
    "AllCon053",
    "ChiCon001",
    "ChiCon002",
    "ChiCon003",
    "ChiCon008",
    "ChiCon009",
    "ChiCon050",
    "ChiCon051",
    "ChiCon053",
];

/// Who gives a contract, by its id's prefix, in our own words.
pub fn group(id: &str) -> &'static str {
    match id.get(..3) {
        Some("Vza") => "Opening",
        Some("Pmc") => "Your PMC",
        Some("Mec") => "Crew: mechanic",
        Some("Jet") => "Crew: pilot",
        Some("Oil") => "Oil company",
        Some("Gur") => "Guerrillas",
        Some("Pir") => "Pirates",
        Some("All") => "Allies",
        Some("Chi") => "China",
        _ => "Other",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// Plays through: a recorded run completed it with nothing flagged.
    Playable,
    /// Completes, but with known gaps (test shortcuts, flagged faults).
    PlayableWithGaps,
    /// A recorded run did not get through it.
    NotYet,
    /// No recorded result.
    Unknown,
}

impl Status {
    pub const ALL: [Status; 4] = [
        Status::Playable,
        Status::PlayableWithGaps,
        Status::NotYet,
        Status::Unknown,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Status::Playable => "Playable",
            Status::PlayableWithGaps => "Playable with gaps",
            Status::NotYet => "Not yet",
            Status::Unknown => "Unknown",
        }
    }
}

/// Where a status came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Evidence {
    /// No recorded result.
    None,
    /// An autoplay run (a sweep or a thread's run) before the nightly.
    Autoplay,
    /// The nightly autoplay runner.
    Nightly,
    /// Someone played it.
    Playtest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Contract {
    pub id: String,
    pub status: Status,
    /// One or two sentences, our own words.
    #[serde(default)]
    pub note: String,
    /// GitHub issue and PR numbers.
    #[serde(default)]
    pub issues: Vec<u32>,
    #[serde(default = "no_evidence")]
    pub evidence: Evidence,
    /// When the evidence was recorded, `YYYY-MM-DD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<String>,
    /// The commit the evidence ran on, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    /// Kept as written: the nightly doesn't change it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
}

fn no_evidence() -> Evidence {
    Evidence::None
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tracker {
    /// When the file last changed, `YYYY-MM-DD`.
    pub updated: String,
    #[serde(rename = "contract")]
    pub contracts: Vec<Contract>,
}

impl Tracker {
    pub fn parse(text: &str) -> Result<Self, String> {
        let t: Tracker = toml::from_str(text).map_err(|e| e.to_string())?;
        let problems = validate(&t);
        if problems.is_empty() {
            Ok(t)
        } else {
            Err(problems.join("; "))
        }
    }

    pub fn to_toml(&self) -> String {
        let mut s = String::from(
            "# Which contracts play through in this build: docs/contract-tracker.md.\n\
             # Statuses: playable, playable-with-gaps, not-yet, unknown. Our own text only.\n\n",
        );
        s.push_str(&toml::to_string_pretty(self).expect("the tracker always serialises"));
        s
    }

    /// How many contracts have each status.
    pub fn counts(&self) -> BTreeMap<Status, usize> {
        let mut m: BTreeMap<Status, usize> = Status::ALL.iter().map(|&s| (s, 0)).collect();
        for c in &self.contracts {
            *m.entry(c.status).or_default() += 1;
        }
        m
    }
}

fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

/// What is wrong with a tracker; empty when it is fine.
pub fn validate(t: &Tracker) -> Vec<String> {
    let mut out = Vec::new();
    if !is_date(&t.updated) {
        out.push(format!("updated {:?} is not YYYY-MM-DD", t.updated));
    }
    let mut seen = std::collections::HashSet::new();
    for c in &t.contracts {
        let id = &c.id;
        if !CONTRACTS.contains(&id.as_str()) {
            out.push(format!("{id}: not one of the 52 contracts"));
        }
        if !seen.insert(id.clone()) {
            out.push(format!("{id}: listed twice"));
        }
        let recorded = c.evidence != Evidence::None;
        if matches!(
            c.status,
            Status::Playable | Status::PlayableWithGaps | Status::NotYet
        ) && !recorded
        {
            out.push(format!(
                "{id}: {} needs a recorded autoplay or playtest result",
                c.status.label()
            ));
        }
        if recorded && !c.checked.as_deref().is_some_and(is_date) {
            out.push(format!("{id}: evidence needs a checked date YYYY-MM-DD"));
        }
        if c.status != Status::Playable && c.status != Status::Unknown && c.note.trim().is_empty() {
            out.push(format!("{id}: {} needs a note", c.status.label()));
        }
        if c.note.len() > 300 {
            out.push(format!("{id}: note over 300 characters"));
        }
        if c.issues.contains(&0) {
            out.push(format!("{id}: issue number 0"));
        }
    }
    for id in CONTRACTS {
        if !seen.contains(id) {
            out.push(format!("{id}: missing"));
        }
    }
    out
}

// ---- the nightly runner's results -------------------------------------------

/// The fields of the nightly runner's `night.json` read here (the rest is
/// ignored, so the formats can grow apart).
#[derive(Debug, Clone, Deserialize)]
pub struct Night {
    pub run_id: String,
    pub head: String,
    pub started_unix: u64,
    pub results: Vec<NightResult>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NightResult {
    pub contract: String,
    /// `pass`, `fail`, `skipped` or `aborted`.
    pub outcome: String,
    #[serde(default)]
    pub issue_kinds: Vec<String>,
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub hops: Option<usize>,
}

/// `YYYY-MM-DD` of a unix time (UTC).
pub fn date(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The status one nightly run gives: a pass with no hops is playable; a
/// pass with hops, or a fail that still completed (a frame-time cliff), is
/// playable with gaps; a fail that didn't complete is not yet. Skipped and
/// aborted runs say nothing.
pub fn status_of(r: &NightResult) -> Option<(Status, String)> {
    let hops = r.hops.unwrap_or(0);
    let kinds = r.issue_kinds.join(", ");
    match r.outcome.as_str() {
        "pass" if hops == 0 => Some((
            Status::Playable,
            "The nightly autoplay completed it with nothing flagged.".into(),
        )),
        "pass" => Some((
            Status::PlayableWithGaps,
            format!("The nightly autoplay completed it, with {hops} test-shortcut hop(s)."),
        )),
        "fail" if r.completed => Some((
            Status::PlayableWithGaps,
            format!("The nightly autoplay completed it but flagged: {kinds}."),
        )),
        "fail" => Some((
            Status::NotYet,
            format!("The nightly autoplay did not complete it ({kinds})."),
        )),
        _ => None,
    }
}

/// Moves the tracker on from a night's runs; returns the ids changed.
/// Locked entries and entries with newer evidence are left alone; issue
/// numbers are kept.
pub fn update_from_night(t: &mut Tracker, n: &Night) -> Vec<String> {
    let day = date(n.started_unix);
    let mut changed = Vec::new();
    for r in &n.results {
        let Some((status, note)) = status_of(r) else {
            continue;
        };
        let Some(c) = t.contracts.iter_mut().find(|c| c.id == r.contract) else {
            continue;
        };
        if c.locked || c.checked.as_deref().is_some_and(|d| d > day.as_str()) {
            continue;
        }
        let before = (c.status, c.note.clone(), c.checked.clone());
        c.status = status;
        c.note = note;
        c.evidence = Evidence::Nightly;
        c.checked = Some(day.clone());
        c.head = Some(n.head.chars().take(10).collect());
        if (c.status, c.note.clone(), c.checked.clone()) != before {
            changed.push(c.id.clone());
        }
    }
    if !changed.is_empty() && day > t.updated {
        t.updated = day;
    }
    changed
}

#[cfg(test)]
mod tests;
