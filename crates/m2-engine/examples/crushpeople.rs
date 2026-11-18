//! Checks the crush step on a limp body, offscreen and without the game's
//! data ([`m2_engine::crush`]): a 500 kg block dropped onto a ragdoll
//! lying on flat ground, as a `Crusher` (`DamageModifier` 1000) and as
//! plain debris, and the same block set down slowly. Prints a line each
//! and PASS or FAIL.
//!
//! ```text
//! cargo run --release -p m2-engine --example crushpeople
//! ```

use m2_engine::crush::{crush_damage, crush_hit};
use m2_engine::{CollisionShape, LoosePiece, Physics, StaticKey};
use m2_math::glam::{Mat4, Quat, Vec3};

/// A ragdoll of a pelvis, chest and head lying along x at 0.2 m, and a
/// block of `kg` let go `height` m over its chest; the first hit.
fn drop(height: f32, kg: f32) -> Option<m2_engine::physics::DebrisHit> {
    let mut p = Physics::new();
    let h = 50.0;
    let ground = CollisionShape::mesh(
        &[
            Vec3::new(-h, 0.0, -h),
            Vec3::new(h, 0.0, -h),
            Vec3::new(h, 0.0, h),
            Vec3::new(-h, 0.0, h),
        ],
        &[[0, 1, 2], [0, 2, 3]],
    )?;
    p.add_static(StaticKey::Cell(1), &ground, Vec3::ZERO, Quat::IDENTITY);
    let nodes = [
        Mat4::from_translation(Vec3::new(0.0, 0.2, 0.0)),
        Mat4::from_translation(Vec3::new(0.5, 0.2, 0.0)),
        Mat4::from_translation(Vec3::new(1.0, 0.2, 0.0)),
    ];
    let doll = p.add_ragdoll(&[None, Some(0), Some(1)], &nodes, Vec3::ZERO);
    for _ in 0..60 {
        p.step(1.0 / 60.0);
    }
    p.ragdoll_debris_hits(doll);
    let block = LoosePiece::block(None, Vec3::ZERO, Vec3::new(0.4, 0.2, 0.4), kg);
    p.add_debris(
        &block,
        Mat4::from_translation(Vec3::new(0.5, 0.6 + height, 0.0)),
        Vec3::ZERO,
        Vec3::ZERO,
    );
    for _ in 0..180 {
        p.step(1.0 / 60.0);
        if let Some(h) = p.ragdoll_debris_hits(doll).into_iter().next() {
            return Some(h);
        }
    }
    None
}

fn verdict(ok: bool) -> &'static str {
    if ok { "PASS" } else { "FAIL" }
}

fn main() {
    let mut all = true;
    match drop(3.0, 500.0) {
        Some(h) => {
            let crushed = crush_hit(&h, 1000.0);
            let plain = crush_hit(&h, 1.0);
            let ok = crushed.is_some_and(|c| c.crusher == Some(0) && c.modifier == 1000.0)
                && plain.is_some_and(|c| c.crusher.is_none());
            all &= ok;
            println!(
                "crush-people: 500 kg dropped 3 m onto a limp body: size {:.1}, closing {:.1} m/s, \
                 normal up {:.2}; as a Crusher {:.1} damage at cell 1, as plain debris {:.2}: {}",
                h.size,
                h.closing,
                h.normal.y,
                crushed.map_or(0.0, |c| crush_damage(&c, 1.0)),
                plain.map_or(0.0, |c| crush_damage(&c, 1.0)),
                verdict(ok)
            );
        }
        None => {
            all = false;
            println!("crush-people: 500 kg dropped 3 m onto a limp body: no hit: FAIL");
        }
    }
    let set_down = drop(0.05, 500.0).map(|h| crush_hit(&h, 1000.0));
    let ok = match set_down {
        None | Some(None) => true,
        Some(Some(c)) => c.crusher.is_none(),
    };
    all &= ok;
    println!(
        "crush-people: the same Crusher set down 5 cm over it: crusher {}: {}",
        match set_down {
            Some(Some(c)) if c.crusher.is_some() => "stands",
            _ => "does not stand",
        },
        verdict(ok)
    );
    println!("crush-people: {}", verdict(all));
}
