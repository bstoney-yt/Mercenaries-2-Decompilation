//! Runs [`m2_engine::particles::push`] on scenarios from a text file and
//! prints each particle's new velocity and position, for comparing with
//! an instruction-level run of the 360 particle step on the same input.
//! One scenario a line, whitespace-separated:
//! `dt px py pz vx vy vz byte`, then value and spread of mass, attractor,
//! vortex, drag, wind and gravity scales, then a force count and per force
//! `kind ax ay az qx qy qz qw strength range decay radial local dx dy dz`
//! (kind 0 wind to 4 vortex as the 360 code numbers them).
use m2_assets::effect::{Attr, ForceKind};
use m2_engine::particles::{EmitterSpec, LiveForce, Spread, push};
use m2_math::glam::{Quat, Vec3};

fn attr() -> Attr {
    Attr {
        name: None,
        kind: 1,
        flags: 0,
        value: [0; 4],
        keys: Vec::new(),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: step_check <scenarios>")?;
    let kinds = [
        ForceKind::Wind,
        ForceKind::Gravity,
        ForceKind::Drag,
        ForceKind::Attractor,
        ForceKind::Vortex,
    ];
    for line in std::fs::read_to_string(path)?.lines() {
        let n: Vec<f32> = line
            .split_whitespace()
            .map(str::parse)
            .collect::<Result<_, _>>()?;
        if n.is_empty() {
            continue;
        }
        let mut it = n.into_iter();
        let mut g = || it.next().unwrap_or(0.0);
        let dt = g();
        let at = Vec3::new(g(), g(), g());
        let v = Vec3::new(g(), g(), g());
        let u = (0.5 - g() / 255.0) * 2.0;
        let mut sp = || Spread {
            value: g(),
            var: g(),
        };
        let (mass, attract, swirl, drag, wind, gravity) = (sp(), sp(), sp(), sp(), sp(), sp());
        let e = EmitterSpec {
            offset: Vec3::ZERO,
            pose: Quat::IDENTITY,
            axis: Vec3::Y,
            shape: Vec::new(),
            size: attr(),
            size_var: 0.0,
            life: attr(),
            life_var: 0.0,
            rate: attr(),
            rate_var: 0.0,
            speed: attr(),
            speed_var: 0.0,
            spread: attr(),
            spread_var: 0.0,
            mass,
            gravity,
            drag,
            wind,
            attract,
            swirl,
            turn: 0.0,
            spin: 0.0,
            frames: Vec::new(),
            textures: 0,
            found: 0,
            colour: Vec::new(),
        };
        let count = g() as usize;
        let forces: Vec<LiveForce> = (0..count)
            .map(|_| {
                let kind = kinds[g() as usize];
                let at = Vec3::new(g(), g(), g());
                let turn = Quat::from_xyzw(g(), g(), g(), g());
                let (strength, range, decay, radial, local) = (g(), g(), g(), g(), g());
                LiveForce {
                    kind,
                    at,
                    turn,
                    strength,
                    range,
                    decay,
                    radial,
                    local: local != 0.0,
                    direction: Vec3::new(g(), g(), g()),
                }
            })
            .collect();
        let (nv, np) = push(&forces, &e, at, v, u, dt);
        println!(
            "{:.7} {:.7} {:.7} {:.7} {:.7} {:.7}",
            nv.x, nv.y, nv.z, np.x, np.y, np.z
        );
    }
    Ok(())
}
