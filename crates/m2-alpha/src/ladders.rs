//! The island's ladders, climbed with the use key ([`m2_engine::ladder`]
//! for the climb itself).
//!
//! A placed building whose template's seats have ladder entrances
//! (`bLadder`: the guard and watch towers, water towers, billboards, the
//! shanty houses' roofs) offers each entrance as a vehicle's are offered
//! (0x82440F30: within its `UseDistance` + 1 m of her, ahead of the
//! camera; the nearest, 0x82299E48). Its seat and the seat that seat
//! links to by `SeatToSeat` are the ladder's ends (0x822A4510).
//!
//! Provenance: the gathering and the pick are from the 360 code, as for
//! vehicles; the ladder's ends from the 360 code and samples (see
//! [`m2_engine::ladder`]); its ends are gathered with the vehicles'
//! entrances and the nearest of all is taken (0x82299E48).

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;

use m2_engine::ladder::{Climb, Ladder, Moves, Off, Phase};
use m2_engine::seats::{self, VehicleSeats};
use m2_engine::{AssetSource, WorldIndex};
use m2_math::glam::{Mat4, Vec3};

use crate::player::{LadderPose, Player};

/// Placed buildings farther than this from her aren't looked at, metres
/// (the longest ladder is 26.4 m; an entrance is reached within 1.8 m).
const LOOK: f32 = 40.0;

/// The ladders' buildings and the nodes of their models.
#[derive(Debug, Default)]
pub struct Ladders {
    /// Placed objects (index into the index's `objects`) whose template
    /// has a ladder entrance, once listed.
    placed: Option<Vec<usize>>,
    /// Models' nodes, model space, by model and name hash.
    nodes: HashMap<u32, Option<Vec<(u32, Mat4)>>>,
}

/// A ladder end she may get on at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reach {
    pub ladder: Ladder,
    /// Getting on at the top.
    pub top: bool,
    pub distance_sq: f32,
    /// Its entrance's `UseDistance`.
    pub use_distance: f32,
    /// The placed object.
    pub object: usize,
}

impl Ladders {
    /// The placed objects with ladders.
    fn placed(&mut self, index: &WorldIndex) -> &[usize] {
        self.placed.get_or_insert_with(|| {
            let db = &index.template_db;
            let list: Vec<usize> = index
                .objects
                .iter()
                .enumerate()
                .filter(|(_, o)| {
                    db.vehicle_seats
                        .get(&o.template)
                        .is_some_and(|s| has_ladder(s))
                })
                .map(|(i, _)| i)
                .collect();
            eprintln!("ladders: {} placed buildings have ladders", list.len());
            list
        })
    }

    /// Every ladder end of placed object `i` she may get on at: the ladder,
    /// whether the end is its top, the entrance's place and its
    /// `UseDistance`.
    pub fn ends(
        &mut self,
        index: &WorldIndex,
        source: Option<&mut AssetSource<BufReader<File>>>,
        i: usize,
    ) -> Vec<(Ladder, bool, Vec3, f32)> {
        let o = &index.objects[i];
        let Some(s) = index.template_db.vehicle_seats.get(&o.template) else {
            return Vec::new();
        };
        let nodes = self.nodes.entry(o.model).or_insert_with(|| {
            let data = source?.model(o.model).ok()?;
            let m = data.parse().ok()?;
            Some(
                (0..m.nodes.len())
                    .filter_map(|k| Some((m.nodes[k].name_hash, m.node_to_model(k as u16)?)))
                    .collect(),
            )
        });
        let Some(nodes) = nodes else {
            return Vec::new();
        };
        let at = Mat4::from_rotation_translation(o.rotation, o.position);
        let node = |h: Option<u32>| {
            let h = h?;
            nodes.iter().find(|n| n.0 == h).map(|n| at * n.1)
        };
        let mut out = Vec::new();
        for seat in &s.seats {
            for row in seat.rows.iter().filter(|r| r.can_enter) {
                let Some(e) = s.entrance(row.link_name).filter(|e| e.params.ladder) else {
                    continue;
                };
                let Some(place) = node(e.hardpoint).map(|m| m.w_axis.truncate()) else {
                    continue;
                };
                let Some(other) = seat.links.first().and_then(|&n| s.seat(n)) else {
                    continue;
                };
                let (Some(a), Some(b)) = (node(seat.hardpoint), node(s.seats[other].hardpoint))
                else {
                    continue;
                };
                let (ra, rb) = (a.w_axis.truncate(), b.w_axis.truncate());
                // Facing the lower seat's way.
                let lower = if ra.y <= rb.y { a } else { b };
                if let Some((ladder, top)) = Ladder::between(ra, rb, lower.z_axis.truncate()) {
                    out.push((ladder, top, place, e.params.use_distance));
                }
            }
        }
        out
    }

    /// The ladder ends she may get on at, standing at `her` with the
    /// camera at `eye` looking along `aim` (gathered as vehicle entrances
    /// are, 0x82440F30; the pick among them and the vehicles' is
    /// [`m2_engine::seats::pick`]).
    pub fn gathered(
        &mut self,
        index: &WorldIndex,
        destroyed: &HashSet<usize>,
        mut source: Option<&mut AssetSource<BufReader<File>>>,
        her: Vec3,
        eye: Vec3,
        aim: Vec3,
    ) -> Vec<Reach> {
        let near: Vec<usize> = self
            .placed(index)
            .iter()
            .copied()
            .filter(|&i| {
                !destroyed.contains(&i)
                    && index.objects[i].position.distance_squared(her) <= LOOK * LOOK
            })
            .collect();
        let mut out = Vec::new();
        for i in near {
            for (ladder, top, place, reach) in self.ends(index, source.as_deref_mut(), i) {
                let d = her.distance_squared(place);
                if seats::in_search(her, eye, aim, place, reach, seats::SEARCH_PAD) {
                    out.push(Reach {
                        ladder,
                        top,
                        distance_sq: d,
                        use_distance: reach,
                        object: i,
                    });
                }
            }
        }
        out
    }

    /// The placed ladder nearest `from` (by its object's place) and its
    /// bottom end's entrance (a testing aid, `--ladder-check`).
    pub fn nearest(
        &mut self,
        index: &WorldIndex,
        mut source: Option<&mut AssetSource<BufReader<File>>>,
        from: Vec3,
    ) -> Option<(usize, Ladder, Vec3)> {
        let mut by: Vec<usize> = self.placed(index).to_vec();
        by.sort_by(|&a, &b| {
            let d = |i: usize| index.objects[i].position.distance_squared(from);
            d(a).total_cmp(&d(b))
        });
        by.into_iter().take(8).find_map(|i| {
            self.ends(index, source.as_deref_mut(), i)
                .into_iter()
                .find(|e| !e.1)
                .map(|(l, _, place, _)| (i, l, place))
        })
    }
}

fn has_ladder(s: &VehicleSeats) -> bool {
    s.entrances.iter().any(|e| e.params.ladder)
}

/// The climb's moves from her clips: their lengths where her rig has
/// them, else Jennifer's (their rises are Jennifer's: from samples).
pub fn moves(player: &Player) -> Moves {
    use m2_engine::ladder::clips;
    let mut m = Moves::default();
    if let Some((_, rig)) = &player.model {
        let length = |h: u32, d: f32| {
            rig.clips
                .iter()
                .find(|c| c.hash == h)
                .map_or(d, |c| c.info.duration)
        };
        m.up_cycle = length(clips::CLIMB_UP, m.up_cycle);
        m.down_cycle = length(clips::CLIMB_DOWN, m.down_cycle);
        m.mount_bottom = length(clips::MOUNT_BOTTOM, m.mount_bottom);
        m.mount_top = length(clips::MOUNT_TOP, m.mount_top);
        m.dismount_bottom = length(clips::DISMOUNT_BOTTOM, m.dismount_bottom);
        m.dismount_top = length(clips::DISMOUNT_TOP, m.dismount_top);
    }
    m
}

/// A climb under way, with what she shows of it.
#[derive(Clone, Copy, Debug)]
pub struct Climbing {
    pub climb: Climb,
    pub moves: Moves,
    step: u32,
    phase: std::mem::Discriminant<Phase>,
    rung: i32,
}

impl Climbing {
    pub fn start(reach: Reach, moves: Moves) -> Self {
        let climb = Climb::start(reach.ladder, reach.top, &moves);
        Self {
            climb,
            moves,
            step: 0,
            phase: std::mem::discriminant(&climb.phase),
            rung: climb.rung,
        }
    }

    /// One tick (`stick` up +, `floor` within the climb's probe under
    /// her feet), `Some` once she is off.
    pub fn tick(&mut self, stick: f32, let_go: bool, floor: bool, dt: f32) -> Option<Off> {
        let off = self.climb.tick(&self.moves, stick, let_go, floor, dt);
        let phase = std::mem::discriminant(&self.climb.phase);
        if phase != self.phase || self.climb.rung != self.rung {
            self.step += 1;
            self.phase = phase;
            self.rung = self.climb.rung;
        }
        off
    }

    /// How she is held this tick.
    pub fn pose(&self) -> LadderPose {
        let f = self.climb.ladder.facing;
        LadderPose {
            feet: self.climb.feet(&self.moves),
            facing: f.x.atan2(f.z),
            clip: self.climb.clip(),
            step: self.step,
        }
    }
}

/// Her body (a capsule 1.8 m tall and 0.4 m round, 0x82B7C0EC
/// and 0x82B7C0F0, as 0x8229A8D8 tests it).
const BODY_RADIUS: f32 = 0.4;

/// Whether a floor is within `depth` under her feet at `feet`: what stops
/// a climb down (0x8229A8D8 asks whether her body, lowered `depth`, is
/// clear of the ground and the world; here rays down under its middle and
/// its rim).
pub fn floor_under(physics: &m2_engine::Physics, feet: Vec3, depth: f32) -> bool {
    let rim = [Vec3::ZERO, Vec3::X, -Vec3::X, Vec3::Z, -Vec3::Z];
    rim.iter().any(|d| {
        let from = feet + *d * BODY_RADIUS * 0.7 + Vec3::Y * 0.3;
        physics.cast_ray(from, -Vec3::Y, 0.3 + depth).is_some()
    })
}
