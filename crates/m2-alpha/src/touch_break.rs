//! Break-on-touch piles ([`m2_engine::touch_break`] has the 360's rule and
//! its provenance): which characters' touches break one, and the
//! `--touch-check` that checks the data and the gate.
//!
//! **From the 360 code:** the character proxy's contact callback
//! (0x82176830) breaks a flagged body only when the toucher's human
//! object answers vfunc +0x30 true (0x8217DC30: its byte +0x40). That
//! byte is set only by the human's event handler vfunc +0xE8
//! (0x82185408, through the setter vfunc +0x2C, 0x8217DC38): for an
//! event of kind 3, 0xB or 7 whose +0x10 entity is a player's character
//! (0x824675F0), about an object whose record in 0x83653E8C has its top
//! bit, it becomes whether any rider of that object (the who-rides-what
//! table 0x836540FC) is a player's character. A body pushable as a
//! dynamic prop (property 0x2009, push factor 0x82178FB0 of 0) is pushed
//! and never reaches the check.
//!
//! So a person walking into a pile is blocked by it and does not break
//! it (Xenia: she was held against `_global_trashpile` 0x100612 for 30 s,
//! nothing broken). **Not established:** what event kinds 3, 0xB and 7
//! are, and what 0x83653E8C's top bit means. **Not built:** the trigger
//! (a character riding with a player brushing through a pile); nothing in
//! the game calls [`touched`] yet.

use std::collections::HashSet;

use m2_engine::touch_break::capsule_meets_box;
use m2_math::glam::{Mat4, Vec3};

/// A character's capsule: feet, radius, total height.
pub type Capsule = (Vec3, f32, f32);

/// A toucher: its capsule, and whether it is riding with a player (the
/// 360 human's byte +0x40).
pub type Toucher = (Capsule, bool);

/// One placed object as the touch test sees it: entity, template,
/// broken already, where it stands, its box (model space).
pub type Placed = (u32, u32, bool, Mat4, (Vec3, Vec3));

/// The unbroken objects of a template in `breakers` (and not in
/// `uncrushable`) that a toucher riding with a player touches, by entity,
/// sorted. Touchers not riding with a player break nothing.
pub fn touched(
    placed: impl IntoIterator<Item = Placed>,
    breakers: &HashSet<u32>,
    uncrushable: &HashSet<u32>,
    touchers: &[Toucher],
) -> Vec<u32> {
    let mut out: Vec<u32> = placed
        .into_iter()
        .filter(|&(_, t, wrecked, _, _)| {
            !wrecked && breakers.contains(&t) && !uncrushable.contains(&t)
        })
        .filter(|&(_, _, _, at, bounds)| {
            touchers
                .iter()
                .filter(|(_, riding)| *riding)
                .any(|&((feet, r, h), _)| capsule_meets_box(feet, r, h, at, bounds))
        })
        .map(|(e, ..)| e)
        .collect();
    out.sort_unstable();
    out
}

/// `--touch-check`: the installation's break-on-touch and uncrushable
/// templates (`names` by id); then a hero walking into a pile (a 2 m box,
/// 0.8 m high, standing in the physics world as the pile's collision)
/// for 3 s: she is held short of it and it does not break. Prints
/// `touch-check:` lines and PASS or FAIL; returns whether all passed.
pub fn check(
    breakers: &HashSet<u32>,
    uncrushable: &HashSet<u32>,
    names: &std::collections::HashMap<u32, String>,
) -> bool {
    use m2_engine::{Character, CollisionShape, Physics, StaticKey};
    use m2_math::glam::Quat;
    let verdict = |ok: bool| if ok { "PASS" } else { "FAIL" };
    let mut all = true;
    let name = |id: &u32| names.get(id).cloned().unwrap_or_else(|| format!("{id:#x}"));
    let mut b: Vec<String> = breakers.iter().map(name).collect();
    b.sort();
    let ok = b.len() == 10
        && b.iter()
            .all(|n| n.starts_with("_global_pile") || n.starts_with("_global_trashpile"));
    all &= ok;
    println!(
        "touch-check: {} templates break on touch: {}: {}",
        b.len(),
        b.join(", "),
        verdict(ok)
    );
    let mut u: Vec<String> = uncrushable.iter().map(name).collect();
    u.sort();
    let ok = u == ["C4", "C4 Projectile"];
    all &= ok;
    println!(
        "touch-check: uncrushable: {}: {}",
        u.join(", "),
        verdict(ok)
    );
    // A pile at x 0 (1 m each way, 0.8 m high) on flat ground; she
    // starts 3 m off and walks into it at 2 m/s for 3 s.
    let pile = breakers.iter().min().copied().unwrap_or(1);
    let bounds = (Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 0.8, 1.0));
    let mut physics = Physics::new();
    let h = 50.0;
    if let Some(ground) = CollisionShape::mesh(
        &[
            Vec3::new(-h, 0.0, -h),
            Vec3::new(h, 0.0, -h),
            Vec3::new(h, 0.0, h),
            Vec3::new(-h, 0.0, h),
        ],
        &[[0, 2, 1], [0, 3, 2]],
    ) {
        physics.add_static(StaticKey::Cell(1), &ground, Vec3::ZERO, Quat::IDENTITY);
    }
    let block = CollisionShape::cuboid(Vec3::new(1.0, 0.4, 1.0));
    physics.add_static(
        StaticKey::Entity(1),
        &block,
        Vec3::new(0.0, 0.4, 0.0),
        Quat::IDENTITY,
    );
    let mut her = Character::new(Vec3::new(-3.0, 0.0, 0.0));
    let dt = 1.0 / 60.0;
    let mut broke = Vec::new();
    for _ in 0..180 {
        physics.move_character(&mut her, Vec3::new(2.0, 0.0, 0.0), None, dt);
        let at = Mat4::IDENTITY;
        let walking: Toucher = ((her.position, her.radius, her.height), false);
        broke.extend(touched(
            [(1, pile, false, at, bounds)],
            breakers,
            uncrushable,
            &[walking],
        ));
    }
    let front = her.position.x + her.radius;
    let held = front < -0.9 && her.position.y < 0.5;
    let ok = held && broke.is_empty();
    all &= ok;
    println!(
        "touch-check: she walks into a pile for 3 s: her front at x {front:.2} (the pile's side at -1.00), \
         broken {broke:?}: {}",
        verdict(ok)
    );
    // The gate: the same touch by one riding with a player breaks it.
    let riding: Toucher = ((Vec3::new(-1.2, 0.0, 0.0), 0.35, 1.8), true);
    let got = touched(
        [(1, pile, false, Mat4::IDENTITY, bounds)],
        breakers,
        uncrushable,
        &[riding],
    );
    let ok = got == [1];
    all &= ok;
    println!(
        "touch-check: the same touch by one riding with a player (the 360's byte +0x40): broken {got:?}: {}",
        verdict(ok)
    );
    println!("touch-check: {}", verdict(all));
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    /// From the 360 code (0x82176830, 0x8217DC30, 0x821859E0): only a
    /// toucher riding with a player breaks a flagged pile; never an
    /// uncrushable or broken one.
    #[test]
    fn only_one_riding_with_a_player_breaks_a_pile() {
        let breakers: HashSet<u32> = [10, 11].into();
        let uncrushable: HashSet<u32> = [11].into();
        let bounds = (Vec3::splat(-1.0), Vec3::new(1.0, 0.8, 1.0));
        let at = |x: f32| Mat4::from_translation(Vec3::new(x, 0.0, 0.0));
        let placed = [
            (1, 10, false, at(0.0), bounds),
            (2, 11, false, at(5.0), bounds),
            (3, 12, false, at(10.0), bounds),
            (4, 10, true, at(15.0), bounds),
            (5, 10, false, at(20.0), bounds),
        ];
        let on = |x: f32, riding| ((Vec3::new(x, 0.0, 0.0), 0.35, 1.8), riding);
        let got = touched(
            placed,
            &breakers,
            &uncrushable,
            &[
                on(0.0, true),
                on(5.0, true),
                on(10.0, true),
                on(15.0, true),
                on(20.0, false),
            ],
        );
        assert_eq!(got, [1]);
    }
}
