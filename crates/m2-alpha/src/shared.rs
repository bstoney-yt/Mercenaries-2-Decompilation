//! Online co-op, stage 3: the host's people on the friend's game.
//!
//! The host's game runs the island's people as ever and shares those near
//! the friend's hero ([`HostShare`]): each kind once (template or role
//! name and model hash), each person as they come within
//! [`SHARE_RADIUS`] (with their look), then their places [`SHARE_RATE`]
//! times a second until they go. The friend's game spawns and thinks for
//! no one of its own; it stands the host's people where they are sent
//! and plays their clips ([`Mirror`]), each played back as the other
//! hero is ([`m2_net::Playback`]).
//!
//! Invented for co-op. Not shared yet: vehicles and their crews, the
//! mission's objectives, hits from the friend's game.

use std::collections::{HashMap, HashSet};

use m2_math::glam::Vec3;
use m2_net::proto::PEOPLE_PER_DATAGRAM;
use m2_net::{HeroState, KindInfo, Msg, PeopleBatch, PersonIn, PersonState, Playback, Session};

use crate::game::Game;
use crate::npc::{Brain, Npc};

/// People within this of the friend's hero are shared, metres.
pub const SHARE_RADIUS: f32 = 120.0;
/// How many times a second their places are sent.
pub const SHARE_RATE: f64 = 10.0;

/// The host's side: what has been shared with the friend.
#[derive(Debug, Default)]
pub struct HostShare {
    kinds: HashSet<usize>,
    people: HashSet<u32>,
    last: f64,
    /// People shared now, and the most at once (for the log).
    pub shared: usize,
    pub most: usize,
    pub sent_in: u32,
    pub sent_out: u32,
}

impl HostShare {
    /// Shares the people near `friend` (the other hero), at most
    /// [`SHARE_RATE`] times a second.
    pub fn tick(&mut self, game: &Game, friend: Vec3, session: &Session, now: f64) {
        if now - self.last < 1.0 / SHARE_RATE {
            return;
        }
        self.last = now;
        let people = &game.scene.people;
        let mut near = HashSet::new();
        let mut states = Vec::new();
        for n in &people.npcs {
            if !n.landed || n.character.position.distance(friend) > SHARE_RADIUS {
                continue;
            }
            let Some(kind) = people.kinds.get(n.kind) else {
                continue;
            };
            if self.kinds.insert(n.kind) {
                session.send(
                    Msg::Kind(KindInfo {
                        id: n.kind as u16,
                        name: kind.role.clone(),
                        model: kind.model.hash,
                        named: kind.named,
                    }),
                    true,
                );
            }
            if self.people.insert(n.net_id) {
                self.sent_in += 1;
                session.send(
                    Msg::PersonIn(PersonIn {
                        net_id: n.net_id,
                        kind: n.kind as u16,
                        outfit: n.outfit.clone(),
                        armed: n.gun.is_some() || n.soldier || n.faction.is_some(),
                    }),
                    true,
                );
            }
            near.insert(n.net_id);
            let mut flags = 0;
            if n.alive {
                flags |= PersonState::ALIVE;
            }
            if n.rig.aim.is_some() {
                flags |= PersonState::AIMING;
            }
            if n.down.is_some() {
                flags |= PersonState::DOWN;
            }
            if n.swim.is_some() {
                flags |= PersonState::SWIMMING;
            }
            states.push(PersonState {
                net_id: n.net_id,
                position: n.character.position.to_array(),
                facing: n.facing,
                gait: crate::puppet::wire_gait(n.rig.gait),
                flags,
            });
        }
        let gone: Vec<u32> = self.people.difference(&near).copied().collect();
        for id in gone {
            self.people.remove(&id);
            self.sent_out += 1;
            session.send(Msg::PersonOut(id), true);
        }
        for chunk in states.chunks(PEOPLE_PER_DATAGRAM) {
            session.send(
                Msg::People(PeopleBatch {
                    time: now,
                    people: chunk.to_vec(),
                }),
                false,
            );
        }
        self.shared = near.len();
        self.most = self.most.max(self.shared);
    }
}

/// One of the host's people as the friend's game has them.
#[derive(Debug)]
struct Mirrored {
    info: PersonIn,
    playback: Playback,
    /// Where it was last placed, and when (its speed sets the clips').
    last: Option<(Vec3, f64)>,
}

/// The friend's side: the host's people, stood in for the game's own.
#[derive(Debug, Default)]
pub struct Mirror {
    kinds: HashMap<u16, KindInfo>,
    people: HashMap<u32, Mirrored>,
    started: bool,
    /// People made here, places received, and the most stood at once.
    pub made: u32,
    pub places: u32,
    pub most: usize,
    /// Kinds asked for whose model this game didn't have (logged once).
    missing: HashSet<u16>,
}

impl Mirror {
    /// A reliable message from the host.
    pub fn message(&mut self, msg: Msg) {
        match msg {
            Msg::Kind(k) => {
                eprintln!(
                    "coop: the host's people include {} ({:08x}{})",
                    k.name,
                    k.model,
                    if k.named { ", its own model" } else { "" }
                );
                self.kinds.insert(k.id, k);
            }
            Msg::PersonIn(p) => {
                self.people.insert(
                    p.net_id,
                    Mirrored {
                        info: p,
                        playback: Playback::new(),
                        last: None,
                    },
                );
            }
            Msg::PersonOut(id) => {
                self.people.remove(&id);
            }
            other => eprintln!("coop: unexpected message {other:?}"),
        }
    }

    /// The host's people's places, arrived at `at`.
    pub fn batch(&mut self, b: PeopleBatch, at: f64) {
        // Numbered by the host's clock, in milliseconds: each person is
        // in one datagram a send, so later is larger.
        let seq = (b.time * 1000.0) as u32;
        for p in b.people {
            if let Some(m) = self.people.get_mut(&p.net_id) {
                self.places += 1;
                m.playback.push(
                    HeroState {
                        seq,
                        time: b.time,
                        position: p.position,
                        velocity: [0.0; 3],
                        facing: p.facing,
                        gait: p.gait,
                        flags: p.flags,
                    },
                    at,
                );
            }
        }
    }

    /// Stands the host's people where they are now (`now` on the session
    /// clock) and plays their clips on by `dt`.
    pub fn tick(&mut self, game: &mut Game, now: f64, dt: f32) {
        let people = &mut game.scene.people;
        if !self.started {
            // Our own people go: the host's stand in for them.
            self.started = true;
            people.mirror = true;
            let ours = people.npcs.len();
            people.npcs.clear();
            eprintln!("coop: showing the host's people ({ours} of our own let go)");
        }
        people.npcs.retain(|n| self.people.contains_key(&n.net_id));
        for (id, m) in &mut self.people {
            let Some(pose) = m.playback.sample(now) else {
                continue;
            };
            let at = Vec3::from_array(pose.position);
            let i = match people.npcs.iter().position(|n| n.net_id == *id) {
                Some(i) => i,
                None => {
                    let Some(kind) = self.kinds.get(&m.info.kind) else {
                        continue;
                    };
                    let k = if kind.named {
                        people.rider_kind(&kind.name, Some(kind.model))
                    } else {
                        people
                            .kinds
                            .iter()
                            .position(|k| !k.named && k.role == kind.name)
                    };
                    let Some(k) = k else {
                        // A named model loads in a while; a role this game
                        // lacks never will.
                        if !kind.named && self.missing.insert(m.info.kind) {
                            eprintln!("coop: no {} here to show the host's as", kind.name);
                        }
                        continue;
                    };
                    let rig = people.kinds[k].rig.clone();
                    let mut n = Npc::new(
                        k,
                        &rig,
                        at,
                        pose.facing,
                        Brain::Post {
                            facing: pose.facing,
                        },
                        None,
                    );
                    n.net_id = *id;
                    n.outfit = m.info.outfit.clone();
                    n.soldier = m.info.armed;
                    n.gun = if m.info.armed {
                        people.kinds[k].gun
                    } else {
                        None
                    };
                    n.landed = true;
                    people.npcs.push(n);
                    self.made += 1;
                    people.npcs.len() - 1
                }
            };
            let n = &mut people.npcs[i];
            n.character.position = at;
            n.facing = pose.facing;
            if pose.flags & PersonState::ALIVE == 0 && n.alive {
                n.hurt(f32::INFINITY, Vec3::ZERO);
            }
            let moved = match m.last {
                Some((last, t)) if now - t > 1e-4 => {
                    let d = at - last;
                    (d.x * d.x + d.z * d.z).sqrt() / (now - t) as f32
                }
                _ => 0.0,
            };
            m.last = Some((at, now));
            let gait = crate::puppet::gait(pose.gait);
            n.mirror_step(gait, pose.flags & PersonState::AIMING != 0, moved, dt);
        }
        self.most = self.most.max(people.npcs.len());
    }

    /// The host's people known now (whether stood yet or not).
    pub fn known(&self) -> usize {
        self.people.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(id: u32) -> Msg {
        Msg::PersonIn(PersonIn {
            net_id: id,
            kind: 0,
            outfit: vec![true, false],
            armed: true,
        })
    }

    #[test]
    fn people_come_go_and_take_places_only_once_in() {
        let mut m = Mirror::default();
        m.message(Msg::Kind(KindInfo {
            id: 0,
            name: "vz_hum_soldier".into(),
            model: 0,
            named: false,
        }));
        m.message(person(1));
        m.message(person(2));
        assert_eq!(m.known(), 2);
        let place = |id| PersonState {
            net_id: id,
            flags: PersonState::ALIVE,
            ..Default::default()
        };
        // A place for someone not (or no longer) in is let go.
        m.batch(
            PeopleBatch {
                time: 1.0,
                people: vec![place(1), place(9)],
            },
            1.05,
        );
        assert_eq!(m.places, 1);
        m.message(Msg::PersonOut(1));
        assert_eq!(m.known(), 1);
        m.batch(
            PeopleBatch {
                time: 1.1,
                people: vec![place(1), place(2)],
            },
            1.15,
        );
        assert_eq!(m.places, 2);
    }
}
