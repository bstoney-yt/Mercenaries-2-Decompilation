//! The radar blips' animations: a blip's icon growing and shrinking, and
//! fading in and out, as the scripts ask (`Hud.Radar:AnimateObjectiveSize`,
//! `:AnimateObjectiveAlpha`, `:UnanimateObjective`).
//!
//! From the 360 code. A minimap objective keeps a list of animators; each
//! tick (`0x82387130`) one with a duration counts it down and is taken
//! off, undoing itself, when it runs out (a duration of 0 or less never
//! runs out); the others step. Adding one of a kind takes off the one of
//! that kind already there (`0x823870A8`).
//!
//! - Size (native `0x82321168`, step `0x82386840`): width and height each
//!   move by their own speed a second (10 unless given); past the largest,
//!   the speed turns negative, or, one way, the size starts again from the
//!   smallest; under the smallest, the speed turns positive, or, one way
//!   and going down, it starts again from the largest. Taken off, the
//!   blip has the size it had when the animation was added.
//! - Alpha (native `0x823213F0`, step `0x823869F0`): the same with one
//!   value, 0 to 1, from the blip's own opacity, at a speed of its own
//!   (the scripts' interface gives 0.5 unless told); turning, it is put
//!   on the limit it passed. Taken off, the blip has its opacity back.
//! - `UnanimateObjective` (native `0x823219D8`): takes off the `size`,
//!   `alpha` or `sonar` animator, or with `all` (the scripts' default)
//!   every one.
//!
//! - Sonar (native `0x82321638`, set-up `0x82386B10`, step `0x82386CA8`,
//!   draw `0x82386EA8`): rings of a texture of their own round the blip.
//!   Each ring's width moves by the grow speed a second (6 unless given;
//!   the scripts' interface gives 5). Until the total number have come, a
//!   new one comes every blip delay (the first at once), at the smallest
//!   width, or at the largest when the speed is negative; with as many up
//!   as may show (8 at most) the oldest goes to make room. A ring past
//!   the largest width or under the smallest goes. With a speed of 0 the
//!   rings stand still, spread evenly from the largest width down. Each
//!   is drawn under the blip's icon as a square half its width each way.
//!   The two alphas, at the smallest and the largest width (each kept
//!   between 0 and 1), and the colour are kept by the set-up; that a
//!   ring's opacity runs between the two with its width is inferred from
//!   their names (the draw read here sets no colour itself).

/// A blip's size animation (see the module).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SizeAnim {
    /// Seconds left; 0 or less: for good.
    pub left: f32,
    pub min: [f32; 2],
    pub max: [f32; 2],
    /// Units a second, width and height; the sign is the way it goes.
    pub speed: [f32; 2],
    pub one_way: bool,
    /// The size to give back.
    pub original: [f32; 2],
}

/// A blip's opacity animation (see the module).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AlphaAnim {
    pub left: f32,
    pub min: f32,
    pub max: f32,
    pub speed: f32,
    pub one_way: bool,
    /// 0 to 1.
    pub value: f32,
    /// The opacity to give back.
    pub original: u8,
}

/// A blip's sonar rings (see the module).
#[derive(Clone, Debug, PartialEq)]
pub struct SonarAnim {
    pub left: f32,
    pub texture: String,
    /// How many rings come in all, and how many show at once.
    pub total: u32,
    pub visible: u32,
    pub min: f32,
    pub max: f32,
    /// Seconds between rings.
    pub delay: f32,
    /// Opacity at the smallest and the largest width, 0 to 1.
    pub alpha_at_min: f32,
    pub alpha_at_max: f32,
    /// Width units a second.
    pub speed: f32,
    pub colour: [u8; 3],
    /// Seconds to the next ring, and how many have come.
    pub timer: f32,
    pub come: u32,
    /// The rings' widths, oldest first.
    pub rings: Vec<f32>,
}

/// The most rings a sonar shows at once.
pub const SONAR_RINGS: u32 = 8;

impl SonarAnim {
    /// A sonar as the set-up makes it: with no speed its rings stand
    /// spread from the largest width down, and none more come.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        left: f32,
        texture: String,
        total: u32,
        visible: u32,
        [min, max]: [f32; 2],
        delay: f32,
        [alpha_at_min, alpha_at_max]: [f32; 2],
        speed: f32,
        colour: [u8; 3],
    ) -> Self {
        let visible = visible.min(SONAR_RINGS);
        let mut a = Self {
            left,
            texture,
            total,
            visible,
            min,
            max,
            delay,
            alpha_at_min: alpha_at_min.clamp(0.0, 1.0),
            alpha_at_max: alpha_at_max.clamp(0.0, 1.0),
            speed,
            colour,
            timer: 0.0,
            come: 0,
            rings: Vec::new(),
        };
        if speed == 0.0 && visible > 0 {
            let step = (max - min) / visible as f32;
            a.rings = (0..visible).map(|i| max - step * i as f32).collect();
            a.come = visible;
            a.total = visible;
        }
        a
    }

    fn step(&mut self, dt: f32) {
        for r in &mut self.rings {
            *r += self.speed * dt;
        }
        if self.come < self.total {
            self.timer -= dt;
            if self.timer < 0.0 {
                self.timer = self.delay;
                self.come += 1;
                if self.rings.len() as u32 >= self.visible && !self.rings.is_empty() {
                    // The oldest makes room.
                    self.rings.remove(0);
                }
                if self.visible > 0 {
                    self.rings
                        .push(if self.speed < 0.0 { self.max } else { self.min });
                }
            }
        }
        let (min, max) = (self.min, self.max);
        self.rings.retain(|&w| w <= max && w >= min);
    }

    /// The rings to draw: each one's width and opacity (0 to 1).
    pub fn rings(&self) -> Vec<(f32, f32)> {
        let span = self.max - self.min;
        self.rings
            .iter()
            .map(|&w| {
                let t = if span > 0.0 {
                    ((w - self.min) / span).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                (
                    w,
                    self.alpha_at_min + (self.alpha_at_max - self.alpha_at_min) * t,
                )
            })
            .collect()
    }
}

/// A blip's animators.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Anims {
    pub size: Option<SizeAnim>,
    pub alpha: Option<AlphaAnim>,
    pub sonar: Option<SonarAnim>,
}

/// One value's step between its limits: the new value and speed.
fn bounce(v: f32, speed: f32, min: f32, max: f32, one_way: bool, clamp: bool) -> (f32, f32) {
    if v > max {
        if one_way && speed >= 0.0 {
            (min, speed)
        } else {
            (if clamp { max } else { v }, -speed.abs())
        }
    } else if v < min {
        if one_way && speed < 0.0 {
            (max, speed)
        } else {
            (if clamp { min } else { v }, speed.abs())
        }
    } else {
        (v, speed)
    }
}

/// Whether an animator with `left` seconds runs out in this tick of `dt`
/// (the 360 counts down first and takes it off without a last step).
fn runs_out(left: &mut f32, dt: f32) -> bool {
    if *left <= 0.0 {
        return false;
    }
    *left -= dt;
    *left <= 0.0
}

impl Anims {
    /// Moves the animations on by `dt` seconds, on the blip's `size` and
    /// `alpha`.
    pub fn step(&mut self, size: &mut [f32; 2], alpha: &mut u8, dt: f32) {
        if let Some(a) = &mut self.size {
            if runs_out(&mut a.left, dt) {
                *size = a.original;
                self.size = None;
            } else {
                for (i, side) in size.iter_mut().enumerate() {
                    let moved = *side + a.speed[i] * dt;
                    (*side, a.speed[i]) =
                        bounce(moved, a.speed[i], a.min[i], a.max[i], a.one_way, false);
                }
            }
        }
        if let Some(a) = &mut self.alpha {
            if runs_out(&mut a.left, dt) {
                *alpha = a.original;
                self.alpha = None;
            } else {
                let moved = a.value + a.speed * dt;
                (a.value, a.speed) = bounce(moved, a.speed, a.min, a.max, a.one_way, true);
                *alpha = (a.value * 255.0).clamp(0.0, 255.0) as u8;
            }
        }
        if let Some(a) = &mut self.sonar {
            if runs_out(&mut a.left, dt) {
                self.sonar = None;
            } else {
                a.step(dt);
            }
        }
    }

    /// Takes off the animators of `kind` (`size`, `alpha`, `sonar`, or
    /// anything else for all), giving the blip back what they changed.
    pub fn remove(&mut self, kind: &str, size: &mut [f32; 2], alpha: &mut u8) {
        let all = !["size", "alpha", "sonar"].contains(&kind);
        if (all || kind == "size")
            && let Some(a) = self.size.take()
        {
            *size = a.original;
        }
        if (all || kind == "alpha")
            && let Some(a) = self.alpha.take()
        {
            *alpha = a.original;
        }
        if all || kind == "sonar" {
            self.sonar = None;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.size.is_none() && self.alpha.is_none() && self.sonar.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(one_way: bool, left: f32) -> Anims {
        Anims {
            size: Some(SizeAnim {
                left,
                min: [4.0, 4.0],
                max: [8.0, 6.0],
                speed: [10.0, 10.0],
                one_way,
                original: [5.0, 5.0],
            }),
            alpha: None,
            sonar: None,
        }
    }

    /// Back and forth: each side turns at its own limits, at its speed.
    #[test]
    fn a_size_goes_back_and_forth_between_its_limits() {
        let mut a = size(false, 0.0);
        let (mut s, mut alpha) = ([5.0, 5.0], 255);
        a.step(&mut s, &mut alpha, 0.1);
        assert_eq!(s, [6.0, 6.0]);
        // The height passes its 6 and turns; the width goes on to 8.
        a.step(&mut s, &mut alpha, 0.1);
        assert_eq!(s, [7.0, 7.0]);
        assert_eq!(a.size.unwrap().speed, [10.0, -10.0]);
        a.step(&mut s, &mut alpha, 0.1);
        assert_eq!(s, [8.0, 6.0]);
        for _ in 0..3 {
            a.step(&mut s, &mut alpha, 0.1);
        }
        // Width turned past 8 and came down; height turned under 4.
        assert!(
            (s[0] - 7.0).abs() < 1e-4 && (s[1] - 3.0).abs() < 1e-4,
            "{s:?}"
        );
        assert_eq!(a.size.unwrap().speed, [-10.0, 10.0]);
        assert_eq!(alpha, 255);
    }

    /// One way it starts again from the smallest; with a duration it is
    /// taken off when that runs out and the blip has its size back.
    #[test]
    fn one_way_starts_again_and_a_duration_ends_it() {
        let mut a = size(true, 0.35);
        let (mut s, mut alpha) = ([7.5, 5.0], 255);
        a.step(&mut s, &mut alpha, 0.1);
        // 8.5 is past 8: from the smallest again. 6.0 is not past 6.
        assert_eq!(s, [4.0, 6.0]);
        a.step(&mut s, &mut alpha, 0.1);
        a.step(&mut s, &mut alpha, 0.1);
        assert!(a.size.is_some());
        a.step(&mut s, &mut alpha, 0.1);
        assert!(a.is_empty());
        assert_eq!(s, [5.0, 5.0]);
    }

    fn sonar(speed: f32, total: u32, visible: u32) -> SonarAnim {
        SonarAnim::new(
            0.0,
            "ring".into(),
            total,
            visible,
            [2.0, 10.0],
            1.0,
            [1.0, 0.0],
            speed,
            [255; 3],
        )
    }

    /// Rings come one every delay, the first at once, grow, and go past
    /// the largest width; no more than the total come.
    #[test]
    fn sonar_rings_come_grow_and_go() {
        let mut a = sonar(4.0, 3, 8);
        a.step(0.5);
        // The first at once, at the smallest width.
        assert_eq!(a.rings, [2.0]);
        a.step(0.5);
        assert_eq!(a.rings, [4.0]);
        a.step(0.5);
        assert_eq!(a.rings, [6.0]);
        a.step(0.5);
        // A second once the delay has run out (past it, not on it).
        assert_eq!(a.rings, [8.0, 2.0]);
        // Its opacity runs from the one at the smallest to the one at the
        // largest width.
        assert_eq!(a.rings(), [(8.0, 0.25), (2.0, 1.0)]);
        for _ in 0..20 {
            a.step(0.5);
        }
        assert!(a.rings.is_empty());
        assert_eq!(a.come, 3);
    }

    /// With as many up as may show, the oldest makes room; shrinking
    /// rings start at the largest width; with no speed they stand spread.
    #[test]
    fn sonar_rings_keep_to_their_number() {
        let mut a = sonar(1.0, 10, 2);
        for _ in 0..3 {
            a.step(1.0);
            a.step(0.001);
        }
        assert_eq!(a.rings.len(), 2);
        assert!(a.rings[0] > a.rings[1]);
        let mut b = sonar(-4.0, 2, 8);
        b.step(0.5);
        assert_eq!(b.rings, [10.0]);
        let c = sonar(0.0, 99, 4);
        assert_eq!(c.rings, [10.0, 8.0, 6.0, 4.0]);
        assert_eq!((c.come, c.total), (4, 4));
        // No more than eight show.
        assert_eq!(sonar(0.0, 99, 20).rings.len(), 8);
    }

    /// The opacity is put on the limit it passes, and given back when the
    /// animator is taken off.
    #[test]
    fn an_opacity_fades_between_its_limits() {
        let mut a = Anims {
            size: None,
            alpha: Some(AlphaAnim {
                left: 0.0,
                min: 0.2,
                max: 1.0,
                speed: -0.5,
                one_way: false,
                value: 1.0,
                original: 255,
            }),
            sonar: None,
        };
        let (mut s, mut alpha) = ([5.0, 5.0], 255);
        a.step(&mut s, &mut alpha, 1.0);
        assert_eq!(alpha, 127);
        a.step(&mut s, &mut alpha, 1.0);
        // 0.0 is under 0.2: on the limit, and turning.
        assert_eq!(alpha, 51);
        assert_eq!(a.alpha.unwrap().speed, 0.5);
        a.step(&mut s, &mut alpha, 1.0);
        assert_eq!(alpha, (0.7_f32 * 255.0) as u8);
        a.remove("size", &mut s, &mut alpha);
        assert!(a.alpha.is_some());
        a.remove("all", &mut s, &mut alpha);
        assert!(a.is_empty());
        assert_eq!((alpha, s), (255, [5.0, 5.0]));
    }
}
