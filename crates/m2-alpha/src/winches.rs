//! The helicopters' winches in play: each helicopter whose template has a
//! `Winch` gets the 360's winch state ([`m2_engine::winch::Winch`]) and,
//! once it pays out, its cable of links in the physics world
//! ([`m2_engine::Physics::add_cable`]).
//!
//! # Provenance
//!
//! From the 360 code:
//! - Which helicopters have one: a template's `Winch` component (the
//!   actor `PhysicsActorWinch` is built for it, 0x823FE8B0 ->
//!   0x821952E8), its params from that record.
//! - Where the cable hangs from (+0x1D0, 0x82198568, each step): the
//!   hardpoint `WinchBone` names, found by 0x8241E478 as a node of the
//!   helicopter's model by its name hash; finding none (no such node, no
//!   `WinchBone`), the helicopter's origin, and it still has its winch.
//! - Each physics step runs the winch's update (0x82195780): its state
//!   work (pay out, reel in, the stretch check 0x82198308) and what that
//!   asks of the links and the cargo. Here it runs as many times a frame
//!   as the physics steps ([`m2_engine::physics::frame_steps`]), each with
//!   a step's time.
//! - `Object.SetWinchState` sets the state (+0x140) and the update does
//!   the rest: deployed with no links pays them all out, retracted with
//!   links takes them in.
//! - Taking cargo (0x82195C30, for `Object.AttachCargoToWinch`
//!   0x82333460 and the magnet 0x82197710): only out (deployed or
//!   deploying), not a helicopter; it hangs from the cargo's
//!   `ConnectPoint` (store 0x83652244: `Name`, `BoneName`) whose node is
//!   nearest the hook, else the fallback point (0x82197C28,
//!   [`m2_engine::Physics::cargo_hang_point`]). Letting go (0x82195FF8)
//!   with cargo on starts the time hung empty again (+0x168), which
//!   grows each step while out and empty (0x82195780); the magnet looks
//!   once it is over two seconds. A step: the state's work, the magnet,
//!   the time hung empty, then the stretch check (0x82198308).
//! - `Object.IsWinched` (0x82333330) answers from the cargo's body's
//!   constraints the winch whose cargo (+0x170) it is: so the scripts
//!   hear of what a magnet took ([`Winches::told`]).
//! - The links' radius is 0.3 ([`m2_engine::physics::LINK_RADIUS`]) and
//!   their mass 102 400 ([`m2_engine::physics::LINK_MASS`]).
//!
//! Not here, for want of 360 evidence or of the pieces it needs:
//! - The player's two buttons (actions 0xA4E4316F `WinchRetract` and
//!   0xB422F25E, 0x82198118): which key or pad button sends them on the
//!   PC is not known, so they are not bound (the winch's own
//!   `press_retract` and `press_deploy` are what they would call).
//! - Cargo other than vehicles (props, crates, people): the cable hooks
//!   vehicles' bodies only. The scripts' cargo on their AI flights still
//!   hangs as [`crate::flights`] carries it.
//! - A hardpoint record's own link radius (+0x1F4).

use std::collections::HashMap;

use m2_engine::physics::{CableId, LINK_MASS, LINK_RADIUS, frame_steps};
use m2_engine::winch::{
    Change, ConnectPoint, MAGNET_WAIT, MagnetCandidate, Winch, WinchParams, WinchState,
};
use m2_engine::{Physics, VehicleId};
use m2_math::glam::Vec3;

/// One helicopter's winch.
#[derive(Clone, Debug)]
pub struct HeliWinch {
    pub params: WinchParams,
    pub winch: Winch,
    /// Its `WinchBone` node, in the helicopter's space (its origin when
    /// it has none).
    pub at: Vec3,
    cable: Option<CableId>,
    /// What hangs on it.
    cargo: Option<VehicleId>,
    /// How long it has hung empty while out (+0x168), s.
    empty: f32,
}

/// Something a winch might take: a vehicle in play, its `ConnectPoint`s
/// with where each one's node is in its space (`None`: its model lacks
/// it), and whether it may be carried at all (no helicopter, kind 4;
/// every vehicle's vtable +0xC0, 0x822A2F80, answers yes).
#[derive(Clone, Debug)]
pub struct Cargo {
    pub id: VehicleId,
    pub points: Vec<(ConnectPoint, Option<Vec3>)>,
    pub carriable: bool,
}

/// The winches of the helicopters in play, by physics id.
#[derive(Debug, Default)]
pub struct Winches {
    by_vehicle: HashMap<VehicleId, HeliWinch>,
    /// The vehicles looked at for a winch, with one or not.
    seen: std::collections::HashSet<VehicleId>,
    /// Each model's nodes (name hash, place in its space), as looked up.
    nodes: HashMap<u32, Vec<(u32, Vec3)>>,
    /// The cargo (guids) the scripts were told hangs on a winch.
    told: std::collections::HashSet<u32>,
}

impl Winches {
    /// Gives `id` its winch, if it has none yet: `params` its template's,
    /// `at` its `WinchBone` node in its space.
    pub fn add(&mut self, id: VehicleId, params: WinchParams, at: Vec3) {
        self.by_vehicle.entry(id).or_insert_with(|| HeliWinch {
            params,
            winch: Winch::new(&params),
            at,
            cable: None,
            cargo: None,
            empty: 0.0,
        });
    }

    /// Model `model`'s nodes, looked up once by `find`.
    pub fn nodes(&mut self, model: u32, find: impl FnOnce() -> Vec<(u32, Vec3)>) -> &[(u32, Vec3)] {
        self.nodes.entry(model).or_insert_with(find)
    }

    /// Whether `id` is yet to be looked at for a winch (true once).
    pub fn unseen(&mut self, id: VehicleId) -> bool {
        self.seen.insert(id)
    }

    /// `id`'s winch's state.
    pub fn state(&self, id: VehicleId) -> Option<WinchState> {
        self.by_vehicle.get(&id).map(|w| w.winch.state)
    }

    /// `Object.SetWinchState` (0x823337E8): the state as asked; the next
    /// steps pay out or reel in to match.
    pub fn set_state(&mut self, id: VehicleId, state: WinchState) {
        if let Some(w) = self.by_vehicle.get_mut(&id) {
            w.winch.state = state;
        }
    }

    /// Whether a winch's magnet could look now (0x82197710's own test:
    /// out, empty, seven links or more, empty over two seconds), so that
    /// what it might take is worth gathering.
    pub fn magnet_ready(&self) -> bool {
        self.by_vehicle.values().any(ready)
    }

    /// Notes that the scripts were told `cargo` (a guid) hangs on a
    /// winch, or no longer; answers whether they had been told it did.
    pub fn told(&mut self, cargo: u32, hangs: bool) -> bool {
        if hangs {
            !self.told.insert(cargo)
        } else {
            self.told.remove(&cargo)
        }
    }

    /// The helicopter whose winch `cargo` hangs on.
    pub fn carrier_of(&self, cargo: VehicleId) -> Option<VehicleId> {
        self.by_vehicle
            .iter()
            .find(|(_, w)| w.cargo == Some(cargo))
            .map(|(&id, _)| id)
    }

    /// `Object.AttachCargoToWinch` (0x82333460 -> 0x82195C30): hooks
    /// `cargo` on `carrier`'s winch at its hang point. False when the
    /// winch is not out or has too few links, or `cargo` may not hang.
    pub fn attach(&mut self, physics: &mut Physics, carrier: VehicleId, cargo: &Cargo) -> bool {
        let Some(w) = self.by_vehicle.get_mut(&carrier) else {
            return false;
        };
        hook_on(physics, w, cargo)
    }

    /// `Object.DetachCargoFromWinch` (0x82333640 -> 0x82195FF8): lets go.
    pub fn detach(&mut self, physics: &mut Physics, carrier: VehicleId) {
        if let Some(w) = self.by_vehicle.get_mut(&carrier) {
            let_go(physics, w);
        }
    }

    /// The frame's winch updates, one per physics step of `dt`'s frame
    /// (0x82195780): the state's work, then the magnet (0x82197710)
    /// among `cargo`, then the time hung empty. Answers what each winch
    /// took, (carrier, cargo).
    pub fn step(
        &mut self,
        physics: &mut Physics,
        dt: f32,
        cargo: &[Cargo],
    ) -> Vec<(VehicleId, VehicleId)> {
        let mut took = Vec::new();
        let (n, each) = frame_steps(dt);
        for _ in 0..n {
            for (&id, w) in &mut self.by_vehicle {
                let c = w.winch.step_state(each);
                apply(physics, id, w, c);
                if let Some(taken) = magnet(physics, id, w, cargo) {
                    took.push((id, taken));
                }
                if w.cargo.is_none() && w.winch.state != WinchState::Retracted {
                    w.empty += each;
                }
                let stretched = w.cable.is_some_and(|c| physics.cable_stretched(c));
                let c = w.winch.step_stretch(each, stretched);
                apply(physics, id, w, c);
            }
        }
        took
    }

    /// Lets the winches of vehicles gone from play go.
    pub fn retain(&mut self, live: impl Fn(VehicleId) -> bool) {
        self.by_vehicle.retain(|&id, _| live(id));
        self.seen.retain(|&id| live(id));
    }
}

/// 0x82197710's own test before it looks.
fn ready(w: &HeliWinch) -> bool {
    matches!(w.winch.state, WinchState::Deployed | WinchState::Deploying)
        && w.cargo.is_none()
        && w.winch.links >= 7
        && w.empty > MAGNET_WAIT
}

/// 0x82195FF8: lets the cargo go; with some on, the time hung empty
/// starts again (+0x168 to 0).
fn let_go(physics: &mut Physics, w: &mut HeliWinch) {
    if let Some(cable) = w.cable {
        physics.cable_detach(cable);
    }
    if w.cargo.take().is_some() {
        w.empty = 0.0;
    }
    w.winch.cargo = false;
}

/// Where `cargo` hangs from in its space, with the hook at `hook`
/// (0x82195C30): its `ConnectPoint` nearest the hook, else the fallback
/// (0x82197C28); `None` when it may not hang.
fn hang_point(physics: &Physics, cargo: &Cargo, hook: Vec3) -> Option<Vec3> {
    if !cargo.carriable {
        return None;
    }
    let t = physics.vehicle_transform(cargo.id)?;
    match m2_engine::winch::hang_point(&cargo.points, hook, |p| t.transform_point3(p))? {
        Some(at) => Some(at),
        None => physics.cargo_hang_point(cargo.id, hook),
    }
}

/// 0x82195C30: out (deployed or deploying), it lets go of what it had
/// and hooks `cargo` (0x82195E58: seven links out or more).
fn hook_on(physics: &mut Physics, w: &mut HeliWinch, cargo: &Cargo) -> bool {
    if !matches!(w.winch.state, WinchState::Deployed | WinchState::Deploying) {
        return false;
    }
    let Some(cable) = w.cable else {
        return false;
    };
    let Some(hook) = physics.cable_hook(cable) else {
        return false;
    };
    let Some(at) = hang_point(physics, cargo, hook) else {
        return false;
    };
    let_go(physics, w);
    if !physics.cable_attach(cable, cargo.id, at) {
        return false;
    }
    w.cargo = Some(cargo.id);
    w.winch.cargo = true;
    true
}

/// 0x82197710: the magnet takes the nearest eligible thing under the
/// hook ([`m2_engine::winch::magnet`]), its hang point as the hook
/// would take it.
fn magnet(
    physics: &mut Physics,
    id: VehicleId,
    w: &mut HeliWinch,
    cargo: &[Cargo],
) -> Option<VehicleId> {
    if !ready(w) {
        return None;
    }
    let hook = physics.cable_hook(w.cable?)?;
    let winch = physics.vehicle_transform(id)?.transform_point3(w.at);
    let others: Vec<&Cargo> = cargo.iter().filter(|c| c.id != id).collect();
    let candidates: Vec<MagnetCandidate> = others
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let t = physics.vehicle_transform(c.id)?;
            let at = hang_point(physics, c, hook);
            Some(MagnetCandidate {
                id: i as u32,
                at: at.map_or(t.w_axis.truncate(), |a| t.transform_point3(a)),
                eligible: at.is_some(),
            })
        })
        .collect();
    let (i, _) = m2_engine::winch::magnet(
        &w.winch,
        w.empty,
        w.params.magnet_radius,
        hook,
        winch,
        &candidates,
    )?;
    let c = others[i as usize];
    hook_on(physics, w, c).then_some(c.id)
}

/// Carries out what a step or a press asks of the cable.
fn apply(physics: &mut Physics, id: VehicleId, w: &mut HeliWinch, c: Change) {
    if c.drop {
        let_go(physics, w);
    }
    if c.add > 0 {
        if w.cable.is_none() {
            w.cable = physics.add_cable(id, w.at, LINK_RADIUS, LINK_MASS);
        }
        if let Some(cable) = w.cable {
            physics.cable_pay_out(cable, c.add);
        }
    }
    if c.take > 0
        && let Some(cable) = w.cable
    {
        physics.cable_reel_in(cable, c.take);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A body held still where a helicopter would hover (a tank's hull
    /// with no wheels: what flies it does not matter here).
    fn heli(p: &mut Physics) -> VehicleId {
        let spec = m2_engine::VehicleSpec {
            wheels: Vec::new(),
            tracked: false,
            ..crate::npc::tests::tank(Default::default())
        };
        let id = p.add_vehicle(&spec, Vec3::new(0.0, 40.0, 0.0), 0.0);
        p.hold_vehicle(id, true);
        id
    }

    fn params() -> WinchParams {
        WinchParams {
            length: 5.0,
            ..WinchParams::default()
        }
    }

    #[test]
    fn a_deployed_winch_pays_out_its_links_and_reels_them_back_in() {
        let mut p = Physics::new();
        let id = heli(&mut p);
        let mut ws = Winches::default();
        ws.add(id, params(), Vec3::new(0.0, -1.0, 0.0));
        assert_eq!(ws.state(id), Some(WinchState::Retracted));
        let hook =
            |ws: &Winches, p: &Physics| ws.by_vehicle[&id].cable.and_then(|c| p.cable_hook(c));
        assert!(hook(&ws, &p).is_none());
        // Paid out at 6 m/s: 5 m (32 links) in a second, then deployed.
        ws.by_vehicle.get_mut(&id).unwrap().winch.press_deploy();
        for _ in 0..90 {
            ws.step(&mut p, 1.0 / 60.0, &[]);
            p.step_frame(1.0 / 60.0);
        }
        assert_eq!(ws.state(id), Some(WinchState::Deployed));
        let max = Winch::new(&params()).max_links;
        let w = &ws.by_vehicle[&id];
        assert_eq!(p.cable_links(w.cable.unwrap()), max);
        assert!(hook(&ws, &p).unwrap().y < 40.0);
        // Reeled in at 3 m/s: in after two seconds.
        ws.by_vehicle.get_mut(&id).unwrap().winch.press_retract();
        for _ in 0..150 {
            ws.step(&mut p, 1.0 / 60.0, &[]);
            p.step_frame(1.0 / 60.0);
        }
        assert_eq!(ws.state(id), Some(WinchState::Retracted));
        assert!(hook(&ws, &p).is_none());
    }

    #[test]
    fn the_scripts_setting_it_deployed_pay_it_all_out_at_once() {
        let mut p = Physics::new();
        let id = heli(&mut p);
        let mut ws = Winches::default();
        ws.add(id, params(), Vec3::ZERO);
        ws.set_state(id, WinchState::Deployed);
        ws.step(&mut p, 1.0 / 60.0, &[]);
        let max = Winch::new(&params()).max_links;
        assert_eq!(p.cable_links(ws.by_vehicle[&id].cable.unwrap()), max);
        ws.set_state(id, WinchState::Retracted);
        ws.step(&mut p, 1.0 / 60.0, &[]);
        assert_eq!(p.cable_links(ws.by_vehicle[&id].cable.unwrap()), 0);
        // One step per physics step: a 1/20 s frame is three, each paying
        // out 0.625 of a link; the first goes out as two with the end
        // link all the way in (0x821962B8), so the second adds one and
        // the third one more.
        ws.set_state(id, WinchState::Deploying);
        ws.step(&mut p, 1.0 / 20.0, &[]);
        assert_eq!(p.cable_links(ws.by_vehicle[&id].cable.unwrap()), 4);
    }

    #[test]
    fn the_magnet_takes_what_hangs_under_the_hook_by_its_box_s_top() {
        let mut p = Physics::new();
        let id = heli(&mut p);
        let mut ws = Winches::default();
        ws.add(id, params(), Vec3::ZERO);
        // Retracted, nothing is taken.
        let spec = crate::npc::tests::tank(Default::default());
        let load = p.add_vehicle(&spec, Vec3::new(50.0, 0.0, 0.0), 0.0);
        p.hold_vehicle(load, true);
        let cargo = [Cargo {
            id: load,
            points: Vec::new(),
            carriable: true,
        }];
        assert!(!ws.attach(&mut p, id, &cargo[0]));
        ws.by_vehicle.get_mut(&id).unwrap().winch.press_deploy();
        for _ in 0..60 {
            ws.step(&mut p, 1.0 / 60.0, &[]);
            p.step_frame(1.0 / 60.0);
        }
        // Under the hook, its box's top half a metre down: the magnet
        // looks once the winch has hung empty two seconds (the cable
        // swinging on, the hook within a metre of it).
        let hook = ws.by_vehicle[&id]
            .cable
            .and_then(|c| p.cable_hook(c))
            .unwrap();
        let w = &ws.by_vehicle[&id];
        assert!(w.empty > 0.9 && w.empty < 1.1, "{}", w.empty);
        let top = p.cargo_hang_point(load, hook).unwrap().y;
        let load2 = p.add_vehicle(&spec, hook - Vec3::Y * (top + 0.5), 0.0);
        p.hold_vehicle(load2, true);
        let cargo = [Cargo {
            id: load2,
            points: Vec::new(),
            carriable: true,
        }];
        let mut took = Vec::new();
        for _ in 0..90 {
            took.extend(ws.step(&mut p, 1.0 / 60.0, &cargo));
            if !took.is_empty() {
                break;
            }
            p.step_frame(1.0 / 60.0);
        }
        assert_eq!(took, vec![(id, load2)]);
        // The scripts hear of it once, and once it goes.
        assert!(!ws.told(9, true));
        assert!(ws.told(9, true));
        assert!(ws.told(9, false));
        assert!(!ws.told(9, false));
        assert_eq!(ws.carrier_of(load2), Some(id));
        assert!(ws.by_vehicle[&id].winch.cargo);
        // Let go: the time hung empty starts again.
        ws.detach(&mut p, id);
        assert_eq!(ws.carrier_of(load2), None);
        assert_eq!(ws.by_vehicle[&id].empty, 0.0);
        // A helicopter is never taken.
        let cargo = [Cargo {
            id: load2,
            points: Vec::new(),
            carriable: false,
        }];
        assert!(!ws.attach(&mut p, id, &cargo[0]));
    }
}
