//! Break-on-touch piles: a character touching one destroys it once
//! ([`m2_engine::touch_break`] has the 360's rule and its provenance).
//!
//! Kept beside the props code: it only reads [`Props::shown`] and
//! [`Props::placed_bounds`] and breaks a pile through [`Props::wreck`],
//! the destroy path the scripts' dead objects take. The `--touch-check`
//! flag runs [`check`].

use std::collections::HashSet;

use m2_engine::Physics;
use m2_engine::touch_break::capsule_meets_box;
use m2_math::glam::{Mat4, Vec3};

use crate::props::Props;

/// A character's capsule: feet, radius, total height.
pub type Capsule = (Vec3, f32, f32);

/// One placed object as the touch test sees it: entity, template,
/// broken already, where it stands, its box (model space).
pub type Placed = (u32, u32, bool, Mat4, (Vec3, Vec3));

/// The unbroken objects of a template in `breakers` (and not in
/// `uncrushable`) that any of `characters` touches, by entity, sorted.
pub fn touched(
    placed: impl IntoIterator<Item = Placed>,
    breakers: &HashSet<u32>,
    uncrushable: &HashSet<u32>,
    characters: &[Capsule],
) -> Vec<u32> {
    let mut out: Vec<u32> = placed
        .into_iter()
        .filter(|&(_, t, wrecked, _, _)| {
            !wrecked && breakers.contains(&t) && !uncrushable.contains(&t)
        })
        .filter(|&(_, _, _, at, bounds)| {
            characters
                .iter()
                .any(|&(feet, r, h)| capsule_meets_box(feet, r, h, at, bounds))
        })
        .map(|(e, ..)| e)
        .collect();
    out.sort_unstable();
    out
}

/// Breaks every shown pile `characters` touch (0x821859E0's 1,000,000 on
/// the pile, through its ordinary destroy path, [`Props::wreck`]); the
/// toucher takes nothing. Returns those broken.
pub fn break_touched(
    props: &mut Props,
    physics: &mut Physics,
    breakers: &HashSet<u32>,
    uncrushable: &HashSet<u32>,
    characters: &[Capsule],
) -> Vec<u32> {
    if breakers.is_empty() || characters.is_empty() {
        return Vec::new();
    }
    let placed: Vec<Placed> = props
        .shown()
        .filter(|(_, p)| breakers.contains(&p.template) && !p.wrecked)
        .filter_map(|(e, p)| {
            let (at, bounds) = props.placed_bounds(e)?;
            Some((e, p.template, p.wrecked, at, bounds))
        })
        .collect();
    touched(placed, breakers, uncrushable, characters)
        .into_iter()
        .filter(|&e| props.wreck(physics, e, None))
        .collect()
}

/// `--touch-check`: the installation's break-on-touch and uncrushable
/// templates (`names` by id), then the touch rule on a row of placed
/// piles: one she walks into, one a soldier stands on, one a step away,
/// one of an uncrushable template, one already broken. Prints
/// `touch-check:` lines and PASS or FAIL; returns whether all passed.
pub fn check(
    breakers: &HashSet<u32>,
    uncrushable: &HashSet<u32>,
    names: &std::collections::HashMap<u32, String>,
) -> bool {
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
    // A row of piles 1 m about each way, 0.8 m high, 4 m apart.
    let (pile, other) = (
        breakers.iter().min().copied().unwrap_or(1),
        uncrushable.iter().min().copied().unwrap_or(2),
    );
    let bounds = (Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 0.8, 1.0));
    let at = |x: f32| Mat4::from_translation(Vec3::new(x, 0.0, 0.0));
    let placed = [
        (1, pile, false, at(0.0), bounds),
        (2, pile, false, at(4.0), bounds),
        (3, pile, false, at(8.0), bounds),
        (4, other, false, at(12.0), bounds),
        (5, pile, true, at(16.0), bounds),
    ];
    let (r, h) = (0.35, 1.8);
    let her = (Vec3::new(-1.3, 0.0, 0.0), r, h);
    let soldier = (Vec3::new(4.0, 0.8, 0.0), r, h);
    let near = (Vec3::new(8.0 - 1.0 - r - 0.3, 0.0, 0.0), r, h);
    let on_c4 = (Vec3::new(12.0, 0.0, 0.0), r, h);
    let on_broken = (Vec3::new(16.0, 0.0, 0.0), r, h);
    let got = touched(
        placed,
        breakers,
        uncrushable,
        &[her, soldier, near, on_c4, on_broken],
    );
    let ok = got == [1, 2];
    all &= ok;
    println!(
        "touch-check: she walks into one, a soldier stands on one; one a step off, one uncrushable, one broken: broken {got:?}: {}",
        verdict(ok)
    );
    // Once: the two broken, the same touches break nothing more.
    let again: Vec<Placed> = placed
        .iter()
        .map(|&(e, t, w, a, bx)| (e, t, w || got.contains(&e), a, bx))
        .collect();
    let second = touched(again, breakers, uncrushable, &[her, soldier]);
    let ok = second.is_empty();
    all &= ok;
    println!(
        "touch-check: touched again: broken {second:?}: {}",
        verdict(ok)
    );
    println!("touch-check: {}", verdict(all));
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    /// From the 360 code (0x82176830, 0x821859E0): any character's touch
    /// breaks a flagged pile, once; never an uncrushable one.
    #[test]
    fn a_touch_breaks_a_pile_once_and_never_an_uncrushable_one() {
        let breakers: HashSet<u32> = [10, 11].into();
        let uncrushable: HashSet<u32> = [11].into();
        let bounds = (Vec3::splat(-1.0), Vec3::new(1.0, 0.8, 1.0));
        let at = |x: f32| Mat4::from_translation(Vec3::new(x, 0.0, 0.0));
        let placed = [
            (1, 10, false, at(0.0), bounds),
            (2, 11, false, at(5.0), bounds),
            (3, 12, false, at(10.0), bounds),
            (4, 10, true, at(15.0), bounds),
        ];
        let on = |x: f32| (Vec3::new(x, 0.0, 0.0), 0.35, 1.8);
        let got = touched(
            placed,
            &breakers,
            &uncrushable,
            &[on(0.0), on(5.0), on(10.0), on(15.0)],
        );
        assert_eq!(got, [1]);
        assert!(touched(placed, &breakers, &uncrushable, &[on(30.0)]).is_empty());
    }
}
