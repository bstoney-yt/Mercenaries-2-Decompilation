//! The island's benches, chairs and the like calling people over to use
//! them ([`m2_engine::social_use`] for the rules).
//!
//! A placed object whose template has a `SocialUse` record and seats
//! keeps a timer while it is near her. When the timer runs out the
//! object takes the first of the tick's idle people within its `Range`,
//! who walks to a free seat's entrance, gets on the seat as a vehicle's
//! is got on, sits the record's duration out and gets off
//! ([`crate::npc::UseJob`]).
//!
//! Provenance: the timer, the pick and the time seated are from the 360
//! code (see [`m2_engine::social_use`]); the seats and their clips from
//! the game's data (the template's `SeatLink`s and
//! `VehicleAnimationSet`, the stance and action table's `InVehicle`
//! `EnterVehicle`, `Idle` and `ExitVehicle` rows and the vehicle
//! animation table).
//!
//! **Inferred**: a seat is free when nobody here uses or walks to it
//! (the 360's seat finder, 0x8229D840, was read for its use, not its
//! test of a seat). A person sits at the seat's hardpoint, turned as it
//! is, as a vehicle's riders do.
//!
//! **Stand-ins**: a timer runs only for objects within [`LOOK`] of her
//! and is forgotten beyond it. The 360 has no such distance: every
//! loaded usable object's timer runs (0x822F96B8), made when the object
//! comes in (0x82400440), and the port does not follow the objects'
//! streaming. And a person gives a thing up after 30 s of not reaching
//! it ([`crate::npc`]), where the 360's task fails with its path.
//!
//! **Not here**: the scripts' `Use` and `UnUse` hooks (the 360 calls
//! `Use` as the person gets on; every such script body is empty in the
//! game's data); the things without seats; the lighter civilians'
//! `LivingWorldObject` path.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;

use m2_assets::animation_table::{self as table, AnimationTable};
use m2_engine::social_use::{self, UseTimer};
use m2_engine::{AssetSource, WorldIndex};
use m2_math::glam::{Mat4, Vec3};

use crate::npc::{People, UseJob};

/// Usable things farther than this from her keep no timer, metres
/// (invented).
const LOOK: f32 = 80.0;

/// The usable things placed on the island, their timers and the nodes
/// of their models.
#[derive(Debug, Default)]
pub struct Social {
    /// Placed objects (index into the index's `objects`) whose template
    /// has a `SocialUse` record and a seat, once listed.
    placed: Option<Vec<usize>>,
    /// Models' nodes, model space, by model and name hash.
    nodes: HashMap<u32, Option<Vec<(u32, Mat4)>>>,
    /// The timers of those near her, by placed object.
    timers: HashMap<usize, UseTimer>,
    /// The stance and action table and the vehicle animation table, once
    /// read (`None`: the archive has not got them).
    tables: Option<Option<(AnimationTable, AnimationTable)>>,
    /// How many people have been called over, for the log.
    called: usize,
}

/// A seat's place: where it is got on, where its rider sits, and the
/// heading faced there.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SeatPlace {
    entrance: Vec3,
    /// The entrance's `UseDistance`.
    reach: f32,
    seat_at: Vec3,
    seat_yaw: f32,
}

/// The heading (radians clockwise from `+z` seen from above) a node's
/// matrix faces along its own `+z`.
fn yaw_of(m: &Mat4) -> f32 {
    let z = m.z_axis.truncate();
    z.x.atan2(z.z)
}

impl Social {
    fn placed(&mut self, index: &WorldIndex) -> &[usize] {
        self.placed.get_or_insert_with(|| {
            let db = &index.template_db;
            let list: Vec<usize> = index
                .objects
                .iter()
                .enumerate()
                .filter(|(_, o)| {
                    db.social_use.contains_key(&o.template)
                        && db
                            .vehicle_seats
                            .get(&o.template)
                            .is_some_and(|s| !s.seats.is_empty())
                })
                .map(|(i, _)| i)
                .collect();
            eprintln!(
                "social: {} placed objects are usable and have seats",
                list.len()
            );
            list
        })
    }

    /// Where seat `seat` of placed object `i` is got on and sat on, when
    /// its model has the nodes.
    fn seat_place(
        &mut self,
        index: &WorldIndex,
        source: Option<&mut AssetSource<BufReader<File>>>,
        i: usize,
        seat: usize,
    ) -> Option<SeatPlace> {
        let o = &index.objects[i];
        let s = index.template_db.vehicle_seats.get(&o.template)?;
        let seat = s.seats.get(seat)?;
        let nodes = self.nodes.entry(o.model).or_insert_with(|| {
            let data = source?.model(o.model).ok()?;
            let m = data.parse().ok()?;
            Some(
                (0..m.nodes.len())
                    .filter_map(|k| Some((m.nodes[k].name_hash, m.node_to_model(k as u16)?)))
                    .collect(),
            )
        });
        let nodes = nodes.as_ref()?;
        let at = Mat4::from_rotation_translation(o.rotation, o.position);
        let node = |h: Option<u32>| {
            let h = h?;
            nodes.iter().find(|n| n.0 == h).map(|n| at * n.1)
        };
        let sat = node(seat.hardpoint)?;
        // Its first entrance with a place; with none, the seat itself.
        let (entrance, reach) = seat
            .rows
            .iter()
            .filter(|r| r.can_enter)
            .find_map(|r| {
                let e = s.entrance(r.link_name)?;
                let reach = Some(e.params.use_distance).filter(|&d| d > 0.0);
                Some((node(e.hardpoint)?, reach.unwrap_or(crate::npc::USE_REACH)))
            })
            .map_or((sat.w_axis.truncate(), crate::npc::USE_REACH), |(m, d)| {
                (m.w_axis.truncate(), d)
            });
        Some(SeatPlace {
            entrance,
            reach,
            seat_at: sat.w_axis.truncate(),
            seat_yaw: yaw_of(&sat),
        })
    }

    /// One tick: the timers of the usable things within [`LOOK`] of
    /// `her` count down, and one that runs out calls a person over.
    pub fn step(
        &mut self,
        index: &WorldIndex,
        destroyed: &HashSet<usize>,
        mut source: Option<&mut AssetSource<BufReader<File>>>,
        people: &mut People,
        her: Vec3,
        dt: f32,
    ) {
        let near: Vec<usize> = self
            .placed(index)
            .iter()
            .copied()
            .filter(|&i| {
                !destroyed.contains(&i)
                    && index.objects[i].position.distance_squared(her) <= LOOK * LOOK
            })
            .collect();
        self.timers.retain(|i, _| near.contains(i));
        // A thing destroyed is no longer usable: those on it or on
        // their way give it up.
        for n in &mut people.npcs {
            if n.using().is_some_and(|u| destroyed.contains(&u.0)) {
                n.drop_use();
            }
        }
        if near.is_empty() {
            return;
        }
        // The tick's candidates, made once and used up as they are
        // taken, as the 360's list is.
        let mut candidates: Option<Vec<(usize, Vec3)>> = None;
        for i in near {
            let o = &index.objects[i];
            let Some(record) = index.template_db.social_use.get(&o.template).copied() else {
                continue;
            };
            let (r0, r1) = (people.random(), people.random());
            let timer = self
                .timers
                .entry(i)
                .or_insert_with(|| UseTimer::new(&record, r0));
            let mut taken = None;
            timer.step(dt, r1, || {
                let list = candidates.get_or_insert_with(|| people.use_candidates());
                taken = social_use::recruit(o.position, record.range, list);
                taken.is_some()
            });
            let Some(who) = taken else { continue };
            match self.job(
                index,
                source.as_deref_mut(),
                people,
                i,
                who,
                record.duration,
            ) {
                Some(job) => {
                    self.called += 1;
                    eprintln!(
                        "social: object {i} (template {:08x}) calls person {who} over to seat {:08x}, {:.1} m off ({} so far)",
                        o.template,
                        job.seat,
                        people.npcs[who].character.position.distance(job.entrance),
                        self.called
                    );
                    people.npcs[who].start_use(job);
                }
                None => eprintln!(
                    "social: object {i} (template {:08x}) has no free seat with clips for person {who}",
                    o.template
                ),
            }
        }
    }

    /// For checking (`--use-nearest`): the usable thing with seats
    /// nearest `her` calls the person nearest it over at once, whatever
    /// its timer and the candidate rules say (anyone alive, on their
    /// feet and free). The thing and the person, if it took.
    pub fn force_call(
        &mut self,
        index: &WorldIndex,
        destroyed: &HashSet<usize>,
        mut source: Option<&mut AssetSource<BufReader<File>>>,
        people: &mut People,
        her: Vec3,
    ) -> Option<(usize, usize)> {
        let mut things: Vec<(f32, usize)> = self
            .placed(index)
            .iter()
            .copied()
            .filter(|i| !destroyed.contains(i))
            .map(|i| (index.objects[i].position.distance(her), i))
            .collect();
        things.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (d, i) in things.into_iter().take(5) {
            let o = &index.objects[i];
            let duration = index
                .template_db
                .social_use
                .get(&o.template)
                .map_or(30.0, |r| r.duration);
            let free = |n: &crate::npc::Npc| {
                n.alive && n.landed && !n.held && n.down.is_none() && n.using().is_none()
            };
            let who = people
                .npcs
                .iter()
                .enumerate()
                .filter(|(_, n)| free(n))
                .min_by(|a, b| {
                    let d = |n: &crate::npc::Npc| n.character.position.distance(o.position);
                    d(a.1).total_cmp(&d(b.1))
                })
                .map(|(k, _)| k)?;
            match self.job(index, source.as_deref_mut(), people, i, who, duration) {
                Some(job) => {
                    eprintln!(
                        "use-check: object {i} (template {:08x}, {d:.0} m from her) calls person {who} over to seat {:08x}, {:.1} m off",
                        o.template,
                        job.seat,
                        people.npcs[who].character.position.distance(job.entrance)
                    );
                    people.npcs[who].start_use(job);
                    return Some((i, who));
                }
                None => eprintln!(
                    "use-check: object {i} (template {:08x}) has no free seat with clips for person {who}",
                    o.template
                ),
            }
        }
        eprintln!("use-check: no usable thing took anyone");
        None
    }

    /// The job of person `who` called over to placed object `i`: its
    /// first seat nobody uses, with a place and a seated clip on their
    /// rig.
    fn job(
        &mut self,
        index: &WorldIndex,
        mut source: Option<&mut AssetSource<BufReader<File>>>,
        people: &People,
        i: usize,
        who: usize,
        duration: f32,
    ) -> Option<UseJob> {
        let o = &index.objects[i];
        let db = &index.template_db;
        let seats = db.vehicle_seats.get(&o.template)?;
        let set = db.animation_sets.get(&o.template).copied()?;
        let taken: Vec<u32> = people
            .npcs
            .iter()
            .filter_map(|n| n.using().filter(|u| u.0 == i).map(|u| u.1))
            .collect();
        if self.tables.is_none() {
            let read = source.as_deref_mut().and_then(|s| {
                let mut one = |hash| {
                    let data = s.asset(hash, table::ANIMATION_TABLE_TYPE).ok()?;
                    AnimationTable::parse(&data).ok()
                };
                Some((one(table::ACTIONS)?, one(table::VEHICLE_ANIMATIONS)?))
            });
            if read.is_none() {
                eprintln!("social: no animation tables: nobody sits");
            }
            self.tables = Some(read);
        }
        for (k, seat) in seats.seats.iter().enumerate() {
            if taken.contains(&seat.name) {
                continue;
            }
            let Some(place) = self.seat_place(index, source.as_deref_mut(), i, k) else {
                continue;
            };
            let (actions, vehicles) = self.tables.as_ref()?.as_ref()?;
            let rig = &people.npcs[who].rig;
            let clip = |action: &str| {
                let hash =
                    table::seat_clip(actions, vehicles, "InVehicle", action, set, Some(seat.name))?;
                let hash = rig.their_clip(hash);
                rig.clips.iter().position(|c| c.hash == hash)
            };
            let Some(idle) = clip("Idle") else { continue };
            return Some(UseJob {
                object: i,
                seat: seat.name,
                entrance: place.entrance,
                reach: place.reach,
                seat_at: place.seat_at,
                seat_yaw: place.seat_yaw,
                duration,
                enter: clip("EnterVehicle"),
                idle,
                exit: clip("ExitVehicle"),
            });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seat_faces_along_its_nodes_z() {
        assert!(yaw_of(&Mat4::IDENTITY).abs() < 1e-6);
        let quarter = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2);
        assert!((yaw_of(&quarter) - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
    }
}
