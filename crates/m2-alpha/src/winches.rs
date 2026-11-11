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
//! - The links' radius is 0.3 ([`m2_engine::physics::LINK_RADIUS`]) and
//!   their mass 102 400 ([`m2_engine::physics::LINK_MASS`]).
//!
//! Not here, for want of 360 evidence or of the pieces it needs:
//! - The player's two buttons (actions 0xA4E4316F `WinchRetract` and
//!   0xB422F25E, 0x82198118): which key or pad button sends them on the
//!   PC is not known, so they are not bound (the winch's own
//!   `press_retract` and `press_deploy` are what they would call).
//! - The magnet (0x82197710) and the scripts' `Object.AttachCargoToWinch`
//!   hooking a body to the cable: both need where the cargo hangs from
//!   (its hardpoints, store 0x83652244, or the fallback point 0x82197C28).
//!   The scripts' cargo still hangs as [`crate::flights`] carries it.
//! - A hardpoint record's own link radius (+0x1F4).

use std::collections::HashMap;

use m2_engine::physics::{CableId, LINK_MASS, LINK_RADIUS, frame_steps};
use m2_engine::winch::{Change, Winch, WinchParams, WinchState};
use m2_engine::{Physics, VehicleId};
use m2_math::glam::Vec3;

/// One helicopter's winch.
#[derive(Clone, Debug)]
pub struct HeliWinch {
    pub winch: Winch,
    /// Its `WinchBone` node, in the helicopter's space (its origin when
    /// it has none).
    pub at: Vec3,
    cable: Option<CableId>,
}

/// The winches of the helicopters in play, by physics id.
#[derive(Debug, Default)]
pub struct Winches {
    by_vehicle: HashMap<VehicleId, HeliWinch>,
    /// The vehicles looked at for a winch, with one or not.
    seen: std::collections::HashSet<VehicleId>,
}

impl Winches {
    /// Gives `id` its winch, if it has none yet: `params` its template's,
    /// `at` its `WinchBone` node in its space.
    pub fn add(&mut self, id: VehicleId, params: WinchParams, at: Vec3) {
        self.by_vehicle.entry(id).or_insert_with(|| HeliWinch {
            winch: Winch::new(&params),
            at,
            cable: None,
        });
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

    /// The frame's winch updates, one per physics step of `dt`'s frame
    /// (0x82195780).
    pub fn step(&mut self, physics: &mut Physics, dt: f32) {
        let (n, each) = frame_steps(dt);
        for _ in 0..n {
            for (&id, w) in &mut self.by_vehicle {
                let stretched = w.cable.is_some_and(|c| physics.cable_stretched(c));
                let c = w.winch.step(each, stretched);
                apply(physics, id, w, c);
            }
        }
    }

    /// Lets the winches of vehicles gone from play go.
    pub fn retain(&mut self, live: impl Fn(VehicleId) -> bool) {
        self.by_vehicle.retain(|&id, _| live(id));
        self.seen.retain(|&id| live(id));
    }
}

/// Carries out what a step or a press asks of the cable.
fn apply(physics: &mut Physics, id: VehicleId, w: &mut HeliWinch, c: Change) {
    if c.drop
        && let Some(cable) = w.cable
    {
        physics.cable_detach(cable);
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
            ws.step(&mut p, 1.0 / 60.0);
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
            ws.step(&mut p, 1.0 / 60.0);
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
        ws.step(&mut p, 1.0 / 60.0);
        let max = Winch::new(&params()).max_links;
        assert_eq!(p.cable_links(ws.by_vehicle[&id].cable.unwrap()), max);
        ws.set_state(id, WinchState::Retracted);
        ws.step(&mut p, 1.0 / 60.0);
        assert_eq!(p.cable_links(ws.by_vehicle[&id].cable.unwrap()), 0);
        // One step per physics step: a 1/20 s frame is three, each paying
        // out 0.625 of a link; the first goes out as two with the end
        // link all the way in (0x821962B8), so the second adds one and
        // the third one more.
        ws.set_state(id, WinchState::Deploying);
        ws.step(&mut p, 1.0 / 20.0);
        assert_eq!(p.cable_links(ws.by_vehicle[&id].cable.unwrap()), 4);
    }
}
