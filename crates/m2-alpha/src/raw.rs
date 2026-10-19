//! Clips the scripts play on characters by name (`Human.PlayRawAnimation`).
//!
//! The faction HQs' briefings are cinematics of frames
//! (`WifHq._NextCinematicFrame`), each naming a clip for every actor
//! (`tAnims`: the player, the HQ's starter, `ALL01_Contract_Briefing_Chris`
//! and so on). The game plays the clip once and holds the character in
//! it; the scripts move on when the clip says so: they fade in on its
//! `CinematicStart` event and go to the next frame on the `CinematicEnd`
//! of the actor the frame names (`OnComplete`), or after a time
//! (`OnTime`). What is said is in the clips too: their `vo` events name
//! the voice lines, at the moment they start.
//!
//! Measured over the 258 clips the briefing tables name (`briefclips`
//! example, from samples): all are in the archive, 252 carry
//! `CinematicStart` at 0 s and `CinematicEnd`, 806 `vo` events name lines,
//! and `CameraA`/`CameraB` events (615) cut between two shots, not used
//! here. The six without events are the hero chair's (one track, played
//! with `Object.PlayAnimation`).
//!
//! Here a clip is bound to the character's skeleton by node name and
//! posed over whatever the character would otherwise play
//! ([`crate::scene::Rig::scripted`]), its root motion taken out (the
//! scripts put each actor at a hardpoint of the HQ first). After its end
//! the last frame is held (the scripts' hold flag, read as such: inferred)
//! until the scripts' next clip for that character or the end of their
//! scene (`Player.SetCinematicMode(false)`: inferred, the release is not
//! seen in the data).

use std::sync::Arc;

use m2_assets::animation::{ANIMATION_TYPE, Clip};
use m2_engine::anim::{BoundClip, Skeleton};
use m2_game::mission::PLAYER;

use crate::scene::{Rig, Scene};

/// What a held clip met, for the scripts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Met {
    /// A plain event (`CinematicStart`, `CinematicEnd`).
    Event(String),
    /// A voice line to say (`Chris-Briefing-Contract-All01-72`).
    Line(String),
}

/// A clip held on one character.
#[derive(Debug)]
struct Play {
    guid: u32,
    time: f32,
    /// Its events in time order: (seconds, name, a voice line).
    events: Vec<(f32, String, bool)>,
    /// The next event not yet met.
    next: usize,
}

impl Play {
    /// The events up to its time not yet met, in order.
    fn due(&mut self) -> Vec<Met> {
        let mut out = Vec::new();
        while let Some((t, name, line)) = self.events.get(self.next) {
            if *t > self.time {
                break;
            }
            out.push(if *line {
                Met::Line(name.clone())
            } else {
                Met::Event(name.clone())
            });
            self.next += 1;
        }
        out
    }
}

/// The clips held, by character.
#[derive(Debug, Default)]
pub struct RawClips {
    plays: Vec<Play>,
    met: Vec<(u32, Met)>,
}

/// The rig of the scripts' character `guid`: the player's, or one of the
/// people the scripts stand ([`crate::npc::People::stand_scripted`]).
fn rig_of(scene: &mut Scene, guid: u32) -> Option<&mut Rig> {
    if guid == PLAYER {
        return scene.player.as_mut()?.model.as_mut().map(|(_, rig)| rig);
    }
    if let Some(n) = scene
        .people
        .npcs
        .iter_mut()
        .find(|n| n.spawn == Some(guid) && n.alive)
    {
        return Some(&mut n.rig);
    }
    // Seated on a prop (the PMC HQ's starter at her desk): posed in her
    // seat.
    scene
        .riders
        .iter_mut()
        .find(|r| r.guid == Some(guid))
        .map(|r| &mut r.rig)
}

/// The clip named `name` bound to `rig`'s skeleton: every track on a
/// bone, else (the heroes' and the people's skeletons differ in a few)
/// most of them.
fn bind(scene: &mut Scene, skeleton: &Skeleton, name: &str) -> Result<BoundClip, String> {
    let hash = m2_assets::name_hash(name);
    let source = scene.source.as_mut().ok_or("no archive")?;
    let bytes = source
        .asset(hash, ANIMATION_TYPE)
        .map_err(|e| format!("not in the archive ({e})"))?;
    let clip = Clip::parse(&bytes).map_err(|e| format!("does not parse ({e})"))?;
    let (found, tracks) = BoundClip::tracks_found(&clip, skeleton);
    BoundClip::bind(hash, clip.clone(), skeleton)
        .or_else(|| BoundClip::bind_family(hash, clip, skeleton))
        .ok_or_else(|| format!("{found} of {tracks} tracks on a bone"))
}

impl RawClips {
    /// Plays clip `name` once on character `guid`, from its start; its
    /// length, or why not.
    pub fn start(&mut self, scene: &mut Scene, guid: u32, name: &str) -> Result<f32, String> {
        let skeleton = Arc::clone(
            &rig_of(scene, guid)
                .ok_or("no such character drawn")?
                .skeleton,
        );
        let bound = bind(scene, &skeleton, name)?;
        let length = bound.info.duration;
        let mut events: Vec<(f32, String, bool)> = bound
            .clip
            .events
            .iter()
            .filter(|e| e.category != "camera")
            .map(|e| (e.time, e.name.clone(), e.category == "vo"))
            .collect();
        events.sort_by(|a, b| a.0.total_cmp(&b.0));
        let rig = rig_of(scene, guid).ok_or("no such character drawn")?;
        rig.scripted = Some((Arc::new(bound), 0.0));
        self.plays.retain(|p| p.guid != guid);
        self.plays.push(Play {
            guid,
            time: 0.0,
            events,
            next: 0,
        });
        Ok(length)
    }

    /// Moves each held clip on by `dt` seconds, collecting the events it
    /// meets (those at its very start the first tick).
    pub fn tick(&mut self, scene: &mut Scene, dt: f32) {
        let mut gone = Vec::new();
        for p in &mut self.plays {
            let Some(rig) = rig_of(scene, p.guid) else {
                gone.push(p.guid);
                continue;
            };
            let Some((_, at)) = &mut rig.scripted else {
                // Taken off by someone else (a hijack, a death).
                gone.push(p.guid);
                continue;
            };
            p.time += dt;
            *at = p.time;
            for met in p.due() {
                self.met.push((p.guid, met));
            }
        }
        self.plays.retain(|p| !gone.contains(&p.guid));
    }

    /// Lets every held character go (the scripts' scene is over).
    pub fn release(&mut self, scene: &mut Scene) {
        for p in std::mem::take(&mut self.plays) {
            if let Some(rig) = rig_of(scene, p.guid) {
                rig.scripted = None;
            }
        }
    }

    /// The events met since the last call, by character.
    pub fn take_met(&mut self) -> Vec<(u32, Met)> {
        std::mem::take(&mut self.met)
    }

    /// How many characters are held.
    pub fn held(&self) -> usize {
        self.plays.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_clip_meets_its_events_once_in_order() {
        // As ALL01_Contract_Briefing_Chris begins: its start at 0 s, a
        // line at 2 s (camera cuts are left out when the clip starts).
        let mut p = Play {
            guid: 1,
            time: 0.0,
            events: vec![
                (0.0, "CinematicStart".into(), false),
                (2.0, "Chris-Briefing-Contract-All01-72".into(), true),
                (73.4, "CinematicEnd".into(), false),
            ],
            next: 0,
        };
        p.time = 0.05;
        assert_eq!(p.due(), [Met::Event("CinematicStart".into())]);
        p.time = 1.9;
        assert!(p.due().is_empty());
        p.time = 80.0;
        assert_eq!(
            p.due(),
            [
                Met::Line("Chris-Briefing-Contract-All01-72".into()),
                Met::Event("CinematicEnd".into())
            ]
        );
        // Held past its end: nothing more.
        p.time = 90.0;
        assert!(p.due().is_empty());
    }
}
