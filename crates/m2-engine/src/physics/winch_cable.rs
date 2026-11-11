//! A winch's cable in the physics world: the 360's chain of links
//! (0x821962B8, 0x82196BB8), joined as its ball-socket chain (chain data
//! 0x82637760, chain instance 0x82637D50).
//!
//! From the 360 code (read through a local decode of the VMX128 stretches
//! Ghidra stops on):
//! - Each link is a body of its own with a sphere shape (0x825F7E60) of
//!   radius `+0x1F4` (0.3 unless the winch's hardpoint says otherwise),
//!   given mass 102 400 (0x820F98C8) through the mass helper 0x8268C748
//!   (0x8268C5C8 scales the shape's unit-density properties by the mass
//!   asked over their own).
//! - Next links are joined at pivots half a link along the cable either
//!   way, (0, ±0.08, 0) (0x826378D0's pivot pairs); the chain's tau is 0.6
//!   and its damping 1.0.
//! - The hook is the free end, link 0 (0x82196238 answers its place).
//! - Cargo is joined to the hook link at its attach point (pivot pair: A
//!   the cargo's point, B (0, -(0.08 + 0.005), 0) on the link), and by a
//!   second constraint (0x82638350, atoms 4, 0xF, 0x10, 0x10, through
//!   0x82631390): twist −π..π, its other limits 0.1.
//! - It takes cargo only with seven links out or more (0x82195E58).
//! - Stretched (0x82198308): next links 4 x their spacing squared apart or
//!   more, or on average 1.21 x.
//!
//! Stand-ins (the alpha runs Rapier, not Havok): Rapier's spherical joints
//! with those anchors stand in for Havok's chain solver, and its default
//! solver for tau 0.6 and damping 1.0. Readings not confirmed: the link
//! mass's unit (taken as kg); which two axes the 0.1 limits are on (taken
//! as the cargo's tilt off the hook link, ±0.1 rad about x and z); the
//! links' collision (the 360 puts them on layer 27; here they land on the
//! world and meet nothing else); 0.99 at body +0x184/+0x188 and 320
//! through 0x82629D80 (not named, not applied).

use super::*;

/// Half a link (+0x150).
pub const HALF_LINK: f32 = 0.08;
/// A link's mass (0x820F98C8).
pub const LINK_MASS: f32 = 102_400.0;
/// A link's radius unless the hardpoint says otherwise.
pub const LINK_RADIUS: f32 = 0.3;
/// Links out before cargo may be taken (0x82195E58).
pub const MIN_LINKS_TO_HOOK: usize = 7;
/// The cargo's tilt off the hook link, rad (0x820BB468).
pub const CARGO_TILT: f32 = 0.1;

/// A winch's cable ([`Physics::add_cable`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CableId(usize);

pub(super) struct Cable {
    heli: r::RigidBodyHandle,
    /// The winch's point on the helicopter, in its space.
    winch: Vec3,
    radius: f32,
    mass: f32,
    /// Hook end first.
    links: Vec<r::RigidBodyHandle>,
    /// The top link's joint to the helicopter.
    top: Option<r::ImpulseJointHandle>,
    cargo: Option<[r::ImpulseJointHandle; 2]>,
}

fn to_heli(winch: Vec3) -> r::SphericalJointBuilder {
    r::SphericalJointBuilder::new()
        .local_anchor1(vector(Vec3::Y * HALF_LINK))
        .local_anchor2(vector(winch))
        .contacts_enabled(false)
}

impl Physics {
    /// A cable for `heli`'s winch at `winch` (its space), with links of
    /// `radius` and `mass`, none out.
    pub fn add_cable(
        &mut self,
        heli: VehicleId,
        winch: Vec3,
        radius: f32,
        mass: f32,
    ) -> Option<CableId> {
        let heli = self.vehicles.get(heli.0)?.as_ref()?.controller.chassis;
        self.cables.push(Some(Cable {
            heli,
            winch,
            radius,
            mass,
            links: Vec::new(),
            top: None,
            cargo: None,
        }));
        Some(CableId(self.cables.len() - 1))
    }

    fn cable(&self, id: CableId) -> Option<&Cable> {
        self.cables.get(id.0)?.as_ref()
    }

    /// Pays out `n` links at the winch (0x821962B8).
    pub fn cable_pay_out(&mut self, id: CableId, n: usize) {
        for _ in 0..n {
            let Some(c) = self.cable(id) else {
                return;
            };
            let Some(b) = self.world.bodies.get(c.heli) else {
                return;
            };
            let turn = Quat::from_array(b.position().rotation.to_array());
            let at = vec3(b.position().translation) + turn * c.winch;
            let (radius, mass, heli, winch) = (c.radius, c.mass, c.heli, c.winch);
            let (below, old_top) = (c.links.last().copied(), c.top);
            let body = self.world.insert_body(
                r::RigidBodyBuilder::dynamic().translation(vector(at - Vec3::Y * HALF_LINK)),
            );
            let volume = 4.0 / 3.0 * std::f32::consts::PI * radius.powi(3);
            insert_collider(
                &mut self.world,
                r::ColliderBuilder::ball(radius)
                    .density(mass / volume)
                    .collision_groups(r::InteractionGroups::new(
                        RUBBLE,
                        r::Group::ALL
                            .difference(RUBBLE)
                            .difference(DEBRIS)
                            .difference(VEHICLE)
                            .difference(RAGDOLL),
                        r::InteractionTestMode::And,
                    )),
                Some(body),
            );
            if let Some(j) = old_top {
                self.world.remove_impulse_joint(j);
            }
            if let Some(below) = below {
                self.world.insert_impulse_joint(
                    below,
                    body,
                    r::SphericalJointBuilder::new()
                        .local_anchor1(vector(Vec3::Y * HALF_LINK))
                        .local_anchor2(vector(-Vec3::Y * HALF_LINK))
                        .contacts_enabled(false),
                );
            }
            let top = self.world.insert_impulse_joint(body, heli, to_heli(winch));
            if let Some(c) = self.cables[id.0].as_mut() {
                c.links.push(body);
                c.top = Some(top);
            }
        }
    }

    /// Reels in `n` links at the winch (0x82197360); with the last, any
    /// cargo goes.
    pub fn cable_reel_in(&mut self, id: CableId, n: usize) {
        for _ in 0..n {
            let Some(c) = self.cables.get_mut(id.0).and_then(Option::as_mut) else {
                return;
            };
            let Some(top) = c.links.pop() else {
                return;
            };
            c.top = None;
            let (next, heli, winch) = (c.links.last().copied(), c.heli, c.winch);
            if c.links.is_empty() {
                // Its joints go with the hook link.
                c.cargo = None;
            }
            self.world.remove_body(top);
            if let Some(below) = next {
                let j = self.world.insert_impulse_joint(below, heli, to_heli(winch));
                if let Some(c) = self.cables[id.0].as_mut() {
                    c.top = Some(j);
                }
            }
        }
    }

    /// Links out.
    pub fn cable_links(&self, id: CableId) -> usize {
        self.cable(id).map_or(0, |c| c.links.len())
    }

    /// Where the hook is: link 0 (0x82196238).
    pub fn cable_hook(&self, id: CableId) -> Option<Vec3> {
        let b = self.world.bodies.get(*self.cable(id)?.links.first()?)?;
        Some(vec3(b.position().translation))
    }

    /// 0x82198308's stretch: next links 4 x their spacing squared apart or
    /// more, or on average 1.21 x.
    pub fn cable_stretched(&self, id: CableId) -> bool {
        let Some(c) = self.cable(id) else {
            return false;
        };
        let at: Vec<Vec3> = c
            .links
            .iter()
            .filter_map(|&h| self.world.bodies.get(h))
            .map(|b| vec3(b.position().translation))
            .collect();
        let spacing = 2.0 * HALF_LINK;
        let ratios: Vec<f32> = at
            .windows(2)
            .map(|w| w[0].distance_squared(w[1]) / (spacing * spacing))
            .collect();
        ratios.iter().any(|&r| r >= 4.0)
            || !ratios.is_empty() && ratios.iter().sum::<f32>() / ratios.len() as f32 >= 1.21
    }

    /// Hooks the vehicle `cargo` at `at` (its space) to the hook link
    /// (0x82195E58): only with seven links out or more. Lets go of any
    /// cargo it had first (0x82195C30).
    pub fn cable_attach(&mut self, id: CableId, cargo: VehicleId, at: Vec3) -> bool {
        let Some(body) = self
            .vehicles
            .get(cargo.0)
            .and_then(Option::as_ref)
            .map(|v| v.controller.chassis)
        else {
            return false;
        };
        self.cable_detach(id);
        let Some(&hook) = self
            .cable(id)
            .filter(|c| c.links.len() >= MIN_LINKS_TO_HOOK)
            .and_then(|c| c.links.first())
        else {
            return false;
        };
        let on_link = vector(Vec3::Y * -(HALF_LINK + 0.005));
        let ball = r::SphericalJointBuilder::new()
            .local_anchor1(vector(at))
            .local_anchor2(on_link)
            .contacts_enabled(false);
        let a = self.world.insert_impulse_joint(body, hook, ball);
        let tilt = r::SphericalJointBuilder::new()
            .local_anchor1(vector(at))
            .local_anchor2(on_link)
            .limits(r::JointAxis::AngX, [-CARGO_TILT, CARGO_TILT])
            .limits(r::JointAxis::AngZ, [-CARGO_TILT, CARGO_TILT])
            .contacts_enabled(false);
        let b = self.world.insert_impulse_joint(body, hook, tilt);
        if let Some(c) = self.cables[id.0].as_mut() {
            c.cargo = Some([a, b]);
        }
        true
    }

    /// Lets the cargo go (0x82195FF8).
    pub fn cable_detach(&mut self, id: CableId) {
        let Some(joints) = self
            .cables
            .get_mut(id.0)
            .and_then(Option::as_mut)
            .and_then(|c| c.cargo.take())
        else {
            return;
        };
        for j in joints {
            self.world.remove_impulse_joint(j);
        }
    }

    /// Where vehicle `cargo` hangs from with no `ConnectPoint` of its own
    /// found (0x82197C28), in its hull's space: over the middle of its
    /// box (its bodies' shapes, 0x82182E90), two rays through the box's
    /// height and as much again (from below up, from above down) against
    /// its hull's shapes; the hit nearest `hook` (world space), else the
    /// middle of the box's top.
    pub fn cargo_hang_point(&self, cargo: VehicleId, hook: Vec3) -> Option<Vec3> {
        let chassis = self.vehicles.get(cargo.0)?.as_ref()?.controller.chassis;
        let (pose, b) = self.vehicle_box(chassis)?;
        let (lo, hi) = (vec3(b.mins), vec3(b.maxs));
        let mid = (lo + hi) * 0.5;
        let h = hi.y - lo.y;
        let mut best: Option<(f32, Vec3)> = None;
        for (from, to) in [(mid.y - h, mid.y + h), (mid.y + h, mid.y - h)] {
            let start = Vec3::new(mid.x, from, mid.z);
            let end = Vec3::new(mid.x, to, mid.z);
            let ray = r::Ray::new(vector(start), vector(end - start));
            let body = self.world.bodies.get(chassis)?;
            let hit = body
                .colliders()
                .iter()
                .filter_map(|&c| self.world.colliders.get(c))
                .filter_map(|c| {
                    let local = pose.inv_mul(c.position());
                    c.shape().cast_ray(&local, &ray, 1.0, true)
                })
                .min_by(f32::total_cmp);
            let Some(t) = hit.filter(|t| *t < 1.0) else {
                continue;
            };
            let at = start + (end - start) * t;
            let d = vec3(pose.transform_point(vector(at))).distance_squared(hook);
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, at));
            }
        }
        Some(best.map_or(Vec3::new(mid.x, hi.y, mid.z), |(_, at)| at))
    }

    /// Whether cargo is on it.
    pub fn cable_has_cargo(&self, id: CableId) -> bool {
        self.cable(id).is_some_and(|c| c.cargo.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heli_over_ground() -> (Physics, VehicleId) {
        let mut p = Physics::new();
        let ground = CollisionShape::cuboid(Vec3::new(200.0, 1.0, 200.0));
        p.add_static(
            StaticKey::Cell(1),
            &ground,
            Vec3::new(0.0, -1.0, 0.0),
            Quat::IDENTITY,
        );
        let heli = p.add_vehicle(
            &crate::physics::tests::car(),
            Vec3::new(0.0, 30.0, 0.0),
            0.0,
        );
        (p, heli)
    }

    #[test]
    fn links_pay_out_and_reel_in_at_the_winch() {
        let (mut p, heli) = heli_over_ground();
        let c = p
            .add_cable(heli, Vec3::new(0.0, -1.0, 0.0), LINK_RADIUS, 1.0)
            .unwrap();
        assert_eq!(p.cable_hook(c), None);
        p.cable_pay_out(c, 10);
        assert_eq!(p.cable_links(c), 10);
        let hook = p.cable_hook(c).unwrap();
        assert!(hook.y < 30.0, "{hook}");
        p.cable_reel_in(c, 4);
        assert_eq!(p.cable_links(c), 6);
        p.cable_reel_in(c, 10);
        assert_eq!((p.cable_links(c), p.cable_hook(c)), (0, None));
    }

    #[test]
    fn cargo_with_no_connect_point_hangs_from_its_box_s_face_nearest_the_hook() {
        // 0x82197C28: rays up and down the middle of its box; the hit
        // nearest the hook.
        let p = {
            let mut p = Physics::new();
            p.add_vehicle(&crate::physics::tests::car(), Vec3::ZERO, 0.0);
            p
        };
        let cargo = VehicleId(0);
        let chassis = p.vehicles[0].as_ref().unwrap().controller.chassis;
        let (_, b) = p.vehicle_box(chassis).unwrap();
        let (lo, hi) = (vec3(b.mins), vec3(b.maxs));
        let mid = (lo + hi) * 0.5;
        let top = p
            .cargo_hang_point(cargo, Vec3::new(0.0, 20.0, 0.0))
            .unwrap();
        assert!(
            (top - Vec3::new(mid.x, hi.y, mid.z)).length() < 1e-3,
            "{top} {hi}"
        );
        let under = p
            .cargo_hang_point(cargo, Vec3::new(0.0, -20.0, 0.0))
            .unwrap();
        assert!(
            (under - Vec3::new(mid.x, lo.y, mid.z)).length() < 1e-3,
            "{under} {lo}"
        );
        assert_eq!(p.cargo_hang_point(VehicleId(5), Vec3::ZERO), None);
    }

    #[test]
    fn cargo_is_taken_with_seven_links_out() {
        let (mut p, heli) = heli_over_ground();
        let crate_ = p.add_vehicle(&crate::physics::tests::car(), Vec3::new(0.0, 1.0, 0.0), 0.0);
        let c = p
            .add_cable(heli, Vec3::new(0.0, -1.0, 0.0), LINK_RADIUS, 1.0)
            .unwrap();
        p.cable_pay_out(c, 6);
        assert!(!p.cable_attach(c, crate_, Vec3::ZERO));
        p.cable_pay_out(c, 1);
        assert!(p.cable_attach(c, crate_, Vec3::ZERO));
        assert!(p.cable_has_cargo(c));
        p.cable_detach(c);
        assert!(!p.cable_has_cargo(c));
        assert!(p.cable_attach(c, crate_, Vec3::ZERO));
        for _ in 0..60 {
            p.step(1.0 / 60.0);
        }
        assert!(p.cable_hook(c).is_some_and(|h| h.is_finite()));
        // Reeled in all the way, the cargo goes with the hook link.
        p.cable_reel_in(c, 7);
        assert!(!p.cable_has_cargo(c));
    }

    #[test]
    fn a_cable_pulled_apart_is_stretched() {
        let (mut p, heli) = heli_over_ground();
        let c = p
            .add_cable(heli, Vec3::new(0.0, -1.0, 0.0), LINK_RADIUS, 1.0)
            .unwrap();
        p.cable_pay_out(c, 3);
        assert!(!p.cable_stretched(c));
        // Pull the hook link 0.5 m away: 0.5^2 / 0.16^2 = 9.8 >= 4.
        let hook = p.cables[c.0].as_ref().unwrap().links[0];
        let b = p.world.bodies.get_mut(hook).unwrap();
        let t = vec3(b.position().translation) - Vec3::Y * 0.5;
        b.set_translation(vector(t), true);
        assert!(p.cable_stretched(c));
    }
}
