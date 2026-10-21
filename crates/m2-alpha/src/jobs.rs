//! The job she was skipped to (`--skip-to AllJob002`), followed as a
//! player takes it on: once the scripts have it on, she tracks it on the
//! PDA's map (which marks its targets), and goes for what it wants
//! ([`m2_game::jobs`] has the four kinds).
//!
//! A set job's marks are its own targets only (the other jobs' markers
//! stay up all over the island). A bounty or a collection names no
//! targets: what it is over (its label's objects in play) is her quarry,
//! the [`QUARRY`] nearest, marked for her as a destroy objective's
//! targets are and destroyed by her blasts as they are.

use m2_game::jobs::{JobView, is_job_id, read};
use m2_game::mission::Mission;
use m2_math::glam::Vec3;

use crate::mission::{Mark, MarkFor};

/// How many of a bounty's or a collection's objects she has in view at
/// once, nearest first (invented).
pub const QUARRY: usize = 6;

/// The job followed, and where it stands.
#[derive(Debug, Default)]
pub struct JobRun {
    pub id: String,
    /// Tracked on the PDA's map.
    pub tracked: bool,
    pub view: JobView,
    /// A bounty's or a collection's objects she goes for, by guid.
    pub quarry: Vec<u32>,
    /// What the log said last: targets counted, completed.
    said: (i64, bool),
    /// Seconds since the quarry was chosen.
    since_chosen: f32,
    /// She is put this far from its nearest target once it is tracked
    /// (`--job-near`, a test shortcut), and whether she has been.
    pub near: Option<f32>,
    placed: bool,
}

/// Seconds between choosing the quarry again (invented).
const CHOOSE_EVERY: f32 = 2.0;
/// She picks up what a collection is over from this near, metres
/// (invented: a pickup's reach).
const PICK_UP: f32 = 3.0;

impl JobRun {
    /// The job `--skip-to` names, if it names one.
    pub fn for_skip(id: &str) -> Option<Self> {
        is_job_id(id).then(|| Self {
            id: id.to_owned(),
            ..Self::default()
        })
    }

    /// A collection: what it is over is walked up to and picked up.
    pub fn is_collection(&self) -> bool {
        self.view.class.as_deref() == Some("MrxTaskJobCollectType")
    }

    /// What she destroys for the job without its being marked by the
    /// scripts: a bounty's quarry (a collection's is picked up).
    pub fn bounty_quarry(&self) -> &[u32] {
        if self.view.is_bounty() {
            &self.quarry
        } else {
            &[]
        }
    }

    /// Done: a set job or a collection completed, or a bounty paid once
    /// (it never completes).
    pub fn finished(&self) -> bool {
        self.view.completed || (self.view.is_bounty() && self.view.counted > 0)
    }

    /// Reads where it stands, tracks it once it is on, and chooses her
    /// quarry near `her`. Lines for the log as it changes.
    pub fn update(&mut self, mission: &mut Mission, her: Vec3, dt: f32) -> Vec<String> {
        let mut lines = Vec::new();
        self.view = read(mission, &self.id);
        if self.view.active && !self.tracked {
            self.tracked = mission.track_mission(&self.id);
            lines.push(format!(
                "jobs: {} is on ({}, {} targets, label {:?}); {}",
                self.id,
                self.view.class.as_deref().unwrap_or("?"),
                self.view.targets.len(),
                self.view.label,
                if self.tracked {
                    "tracked on the PDA's map"
                } else {
                    "the PDA's map can't track it"
                }
            ));
        }
        self.since_chosen += dt;
        if self.view.active
            && self.view.targets.is_empty()
            && (self.since_chosen >= CHOOSE_EVERY || self.quarry.is_empty())
            && let Some(label) = &self.view.label
        {
            self.since_chosen = 0.0;
            let w = mission.world().borrow();
            let mut near: Vec<(u32, f32)> = mission
                .labelled(label)
                .into_iter()
                .filter(|(g, _)| w.objects.get(g).is_some_and(|o| o.alive && !o.hidden))
                .map(|(g, at)| (g, Vec3::from(at).distance(her)))
                .collect();
            near.sort_by(|a, b| a.1.total_cmp(&b.1));
            self.quarry = near.into_iter().take(QUARRY).map(|q| q.0).collect();
        }
        // A collection's things she walks up to, she picks up: gone, as
        // her picking one up takes it from play (inferred: the job counts
        // each one's death, `MrxTaskJobCollectType` over
        // `MrxTaskObjectiveDestroy`).
        if self.is_collection() {
            let hero = mission.world().borrow().player;
            let reached: Vec<(u32, String)> = {
                let w = mission.world().borrow();
                self.quarry
                    .iter()
                    .filter_map(|&g| {
                        let o = w.objects.get(&g).filter(|o| o.alive)?;
                        (Vec3::from(o.position).distance(her) <= PICK_UP)
                            .then(|| (g, o.name.clone()))
                    })
                    .collect()
            };
            for (g, name) in reached {
                if let Some(h) = hero {
                    mission.kill_by(g, h);
                    lines.push(format!("jobs: she picks up {name} ({g:#x})"));
                }
            }
        }
        let now = (self.view.counted, self.view.completed);
        if now != self.said {
            lines.push(format!(
                "jobs: {} counts {} target{}{}",
                self.id,
                now.0,
                if now.0 == 1 { "" } else { "s" },
                if now.1 { "; completed" } else { "" }
            ));
            // A target done: with `--job-near` she is put by the next.
            if now.0 > self.said.0 {
                self.placed = false;
            }
            self.said = now;
        }
        lines
    }

    /// Once it is tracked, with [`near`](Self::near) set: where to put
    /// her, that far from its nearest target (or quarry) on her side of
    /// it, and the target's name. Once, and again after each target
    /// done; not if she is that near
    /// already.
    pub fn start_near(&mut self, mission: &Mission, her: Vec3) -> Option<(String, Vec3)> {
        let near = self.near?;
        if self.placed || !self.tracked {
            return None;
        }
        let w = mission.world().borrow();
        let (name, at) = self
            .view
            .targets
            .iter()
            .chain(&self.quarry)
            .filter_map(|g| {
                let o = w.objects.get(g)?;
                Some((o.name.clone(), Vec3::from(o.position)))
            })
            .min_by(|a, b| a.1.distance(her).total_cmp(&b.1.distance(her)))?;
        self.placed = true;
        let flat = (her - at).with_y(0.0);
        if flat.length() <= near * 2.0 {
            return None;
        }
        Some((name, at + flat.normalize() * near + Vec3::Y * 2.0))
    }

    /// Of `marks`, those for this job: its targets' (a verify mark stays
    /// on a body), else marks made for her quarry: to destroy, or to go
    /// to and pick up (`deliverable`).
    pub fn marks(&self, marks: Vec<Mark>, mission: &Mission) -> Vec<Mark> {
        if !self.view.targets.is_empty() {
            return marks
                .into_iter()
                .filter(|m| self.view.targets.contains(&m.target))
                .collect();
        }
        let w = mission.world().borrow();
        self.quarry
            .iter()
            .filter_map(|&g| {
                let o = w.objects.get(&g)?;
                Some(Mark {
                    target: g,
                    kind: if self.is_collection() {
                        "deliverable".into()
                    } else {
                        "destroy".into()
                    },
                    name: o.name.clone(),
                    at: Vec3::from(o.position),
                    alive: o.alive,
                    mission: Some(MarkFor::Id(self.id.clone())),
                    secondary: true,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_job_is_followed() {
        assert_eq!(
            JobRun::for_skip("AllJob002").map(|j| j.id),
            Some("AllJob002".into())
        );
        assert!(JobRun::for_skip("AllCon002").is_none());
    }

    #[test]
    fn a_bounty_is_done_once_paid_a_set_job_once_complete() {
        let mut j = JobRun::for_skip("AllJob003").unwrap();
        j.view.class = Some("MrxTaskJobDestroyType".into());
        assert!(!j.finished());
        j.view.counted = 1;
        assert!(j.finished(), "a standing bounty never completes");
        let mut s = JobRun::for_skip("AllJob020").unwrap();
        s.view.class = Some("MrxTaskJobDestroySet".into());
        s.view.counted = 5;
        assert!(!s.finished());
        s.view.completed = true;
        assert!(s.finished());
    }
}
