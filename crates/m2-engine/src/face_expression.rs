//! The faces people pull as they go about: the original's runtime
//! facial expressions, each a weight on one of the head model's blend
//! shapes that rises and falls while the expression is on.
//!
//! # Provenance
//!
//! From the 360 code:
//!
//! - **The expressions** are a table of six (0x82B76068), each naming
//!   the blend shape it drives and giving the weights it runs between,
//!   a period, the seconds it blends in and out over, and for `Blink` a
//!   pair of random spans it is on and off for ([`Expression`],
//!   [`TABLE`]).
//! - **Starting one** (the event's handler 0x8241DF40, the instance set
//!   up at 0x8241DA30): an event names a person, an expression, a weight
//!   (clamped to 0..1), and seconds to last (under 0: until told
//!   otherwise). The instance blends in from the shape's weight at that
//!   moment, with two random phases of up to 10 s. An event for an
//!   expression already running replaces it. A weight of 0 is how one is
//!   stopped: it blends to nothing and is dropped.
//! - **Each frame** (0x8241DD58): the wave's value times the weight;
//!   while blending in, that far from the starting weight; while
//!   blending out, that far to nothing. When its seconds run out it
//!   blends out and is dropped, or, with on and off spans, goes off for
//!   a random span and then comes on again.
//! - **The wave** (0x8241DB50): a cosine between the two weights whose
//!   phase advances at a rate that itself swings between
//!   [`RATE_LOW`] and [`RATE_LOW`] + [`RATE_SWING`] periods' worth, by a
//!   second cosine; the cosine's 0..1 height `u` is shaped as
//!   `3 (1 - 1 / (1 + u / 2))`, clamped to 0..1.
//! - **Who turns them on**: `Blink` for everyone when their animation
//!   driver is set up (0x8241F360), off again when a face animation
//!   starts on them (0x824201A8); `Flap` by the voice system for a
//!   speaker while a line plays (0x822782D8, 0x82278440); `Angry` for
//!   [`ANGRY_SECONDS`] on a stagger, a fall's impact or a knockdown and
//!   for [`ANGRY_BASH_SECONDS`] on a melee bash, and `Scared` through a
//!   threat reaction (the driver's state-change handler 0x8241FE38).
//!
//! From samples: 45 of the 3,007 models of `vz.wad` have blend shapes;
//! all but one have `Angry`, `Blink`, `Flap` and `Scared`, 26 `Idle` too.
//!
//! **Not established**: the sixth expression's name (hash 0x473E33B1, on
//! the `Blink` shape with no spans); who turns `Idle` on; the damage
//! code's `Angry` (0x82284678: for whom was not read).

/// One expression of the table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Expression {
    /// Its name's hash, and the hash of the blend shape it drives.
    pub name: u32,
    pub shape: u32,
    /// The weights its wave runs between.
    pub low: f32,
    pub high: f32,
    /// The wave's period.
    pub period: f32,
    /// Seconds it blends in and out over.
    pub blend: f32,
    /// The least and most seconds it is on, then off, over and over.
    pub spans: Option<[f32; 4]>,
}

pub const ANGRY: u32 = 0xd3d1_37bc;
pub const SCARED: u32 = 0xf3f0_0055;
pub const IDLE: u32 = 0xb4da_003b;
pub const FLAP: u32 = 0xd388_246c;
pub const BLINK: u32 = 0x416c_2f99;

/// The expressions, in the table's order.
pub const TABLE: [Expression; 6] = [
    Expression {
        name: ANGRY,
        shape: ANGRY,
        low: 0.5,
        high: 1.0,
        period: 8.0,
        blend: 0.2,
        spans: None,
    },
    Expression {
        name: SCARED,
        shape: SCARED,
        low: 1.0,
        high: 1.0,
        period: 8.0,
        blend: 0.2,
        spans: None,
    },
    Expression {
        name: IDLE,
        shape: IDLE,
        low: 0.0,
        high: 1.0,
        period: 4.0,
        blend: 0.2,
        spans: None,
    },
    Expression {
        name: FLAP,
        shape: FLAP,
        low: 0.0,
        high: 1.0,
        period: 2.4,
        blend: 0.2,
        spans: None,
    },
    Expression {
        name: BLINK,
        shape: BLINK,
        low: 1.0,
        high: 1.0,
        period: 1.0,
        blend: 0.1,
        spans: Some([0.2, 0.2, 2.0, 4.0]),
    },
    Expression {
        name: 0x473e_33b1,
        shape: BLINK,
        low: 1.0,
        high: 1.0,
        period: 1.0,
        blend: 0.1,
        spans: None,
    },
];

/// How long `Angry` shows on a stagger, a fall's impact or a knockdown,
/// and on a melee bash, seconds.
pub const ANGRY_SECONDS: f32 = 2.0;
pub const ANGRY_BASH_SECONDS: f32 = 4.0;

/// The wave's phase advances by between this and this plus
/// [`RATE_SWING`] a frame at 30 a second.
pub const RATE_LOW: f32 = 0.08;
pub const RATE_SWING: f32 = 0.05;

/// The random phases a starting expression is given are under this.
pub const PHASE_MOST: f32 = 10.0;

/// The expression named `name`.
pub fn expression(name: u32) -> Option<&'static Expression> {
    TABLE.iter().find(|e| e.name == name)
}

/// The original's cosine table read at `turns`: `-cos` of that many
/// whole turns (its sine table is entered a quarter turn back).
fn wave_at(turns: f32) -> f32 {
    -(turns * std::f32::consts::TAU).cos()
}

/// Where a running expression is at.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Fade {
    /// Fully on.
    Hold,
    /// Blending in from the shape's weight when it started, for this
    /// long so far.
    In { from: f32, t: f32 },
    /// Blending out, for this long so far.
    Out { t: f32 },
}

/// One expression running on one person.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Running {
    pub expression: &'static Expression,
    weight: f32,
    /// Seconds left; under 0: until stopped.
    left: f32,
    fade: Fade,
    /// With spans: off between two showings.
    off: bool,
    /// The wave's clock and its phase.
    clock: f32,
    phase: f32,
}

impl Running {
    /// `expression` starting at `weight` for `seconds` (under 0: until
    /// stopped), the shape's weight now being `from`; `r` four random
    /// numbers in [0, 1).
    pub fn start(
        expression: &'static Expression,
        weight: f32,
        seconds: f32,
        from: f32,
        r: [f32; 4],
    ) -> Self {
        let mut run = Self {
            expression,
            weight: weight.clamp(0.0, 1.0),
            left: seconds,
            fade: Fade::In { from, t: 0.0 },
            off: false,
            clock: r[0] * PHASE_MOST,
            phase: r[1] * PHASE_MOST,
        };
        if let Some(spans) = expression.spans {
            run.left = spans[0] + r[2] * (spans[1] - spans[0]);
        }
        run
    }

    /// The wave's value, its clocks moved on `dt` seconds (0x8241DB50).
    fn wave(&mut self, dt: f32) -> f32 {
        let e = self.expression;
        let swing = 0.5 - 0.5 * wave_at(self.clock / e.period * 0.8);
        self.phase += (swing * RATE_SWING + RATE_LOW) * dt * 30.0;
        let u = 0.5 - 0.5 * wave_at(self.phase / e.period);
        self.clock += dt;
        let shaped = ((1.0 - 1.0 / (u * 0.5 + 1.0)) * 3.0).clamp(0.0, 1.0);
        e.low + (e.high - e.low) * shaped
    }

    /// One frame of `dt` seconds, `r` two random numbers in [0, 1): the
    /// shape's weight to set, if any this frame, and whether the
    /// expression is over (0x8241DD58).
    pub fn step(&mut self, dt: f32, r: [f32; 2]) -> (Option<f32>, bool) {
        if self.weight <= 1e-4 && !matches!(self.fade, Fade::In { .. }) {
            return (Some(0.0), true);
        }
        let blend = self.expression.blend.max(1e-6);
        let mut over = false;
        let mut set = None;
        if !self.off {
            let full = self.wave(dt) * self.weight;
            let value = match self.fade {
                Fade::Hold => full,
                Fade::In { from, t } => {
                    let k = ((t + dt) / blend).clamp(0.0, 1.0);
                    if k <= 0.9999 {
                        self.fade = Fade::In { from, t: t + dt };
                        from + (full - from) * k
                    } else {
                        self.fade = Fade::Hold;
                        full
                    }
                }
                Fade::Out { t } => {
                    let k = ((t + dt) / blend).clamp(0.0, 1.0);
                    if k <= 0.9999 {
                        self.fade = Fade::Out { t: t + dt };
                        full - full * k
                    } else {
                        match self.expression.spans {
                            Some(spans) => {
                                self.off = true;
                                self.left = spans[2] + r[0] * (spans[3] - spans[2]);
                            }
                            None => over = true,
                        }
                        0.0
                    }
                }
            };
            set = Some(value);
        }
        if self.left >= 0.0 {
            self.left -= dt;
            if self.left < 0.0 {
                match (self.off, self.expression.spans) {
                    (true, Some(spans)) => {
                        self.off = false;
                        self.left = spans[0] + r[0] * (spans[1] - spans[0]);
                        self.fade = Fade::In { from: 0.0, t: 0.0 };
                        self.clock = r[0] * PHASE_MOST;
                        self.phase = r[1] * PHASE_MOST;
                    }
                    _ => self.fade = Fade::Out { t: 0.0 },
                }
            }
        }
        (set, over)
    }
}

/// The expressions running on one person, and the weights they give
/// the head's blend shapes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Face {
    running: Vec<Running>,
    /// Each driven shape's weight (by the shape's name hash).
    weights: Vec<(u32, f32)>,
}

impl Face {
    /// The weight of the blend shape named `shape`.
    pub fn weight(&self, shape: u32) -> f32 {
        self.weights
            .iter()
            .find(|w| w.0 == shape)
            .map_or(0.0, |w| w.1)
    }

    /// Every driven shape and its weight.
    pub fn weights(&self) -> &[(u32, f32)] {
        &self.weights
    }

    /// Whether every weight is 0 and nothing runs: the head is as
    /// modelled.
    pub fn at_rest(&self) -> bool {
        self.running.is_empty() && self.weights.iter().all(|w| w.1 == 0.0)
    }

    fn set(&mut self, shape: u32, value: f32) {
        match self.weights.iter_mut().find(|w| w.0 == shape) {
            Some(w) => w.1 = value,
            None => self.weights.push((shape, value)),
        }
    }

    /// An expression event: `name` at `weight` for `seconds` (under 0:
    /// until stopped; a weight of 0 stops it), `r` four random numbers
    /// in [0, 1). Unknown names do nothing.
    pub fn play(&mut self, name: u32, weight: f32, seconds: f32, r: [f32; 4]) {
        let Some(e) = expression(name) else { return };
        let from = self.weight(e.shape);
        self.running.retain(|x| x.expression.name != name);
        self.running
            .push(Running::start(e, weight, seconds, from, r));
    }

    /// Every expression off at once (`DisableAll`): the weights go to 0.
    pub fn disable_all(&mut self) {
        self.running.clear();
        for w in &mut self.weights {
            w.1 = 0.0;
        }
    }

    /// Whether `name` is running.
    pub fn plays(&self, name: u32) -> bool {
        self.running.iter().any(|x| x.expression.name == name)
    }

    /// One frame of `dt` seconds, `random` giving numbers in [0, 1).
    pub fn step(&mut self, dt: f32, mut random: impl FnMut() -> f32) {
        let mut k = 0;
        while k < self.running.len() {
            let r = [random(), random()];
            let shape = self.running[k].expression.shape;
            let (set, over) = self.running[k].step(dt, r);
            if let Some(v) = set {
                self.set(shape, v);
            }
            if over {
                self.running.remove(k);
            } else {
                k += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_assets::name_hash;

    #[test]
    fn the_names_hash_to_the_tables_words() {
        for (hash, name) in [
            (ANGRY, "Angry"),
            (SCARED, "Scared"),
            (IDLE, "Idle"),
            (FLAP, "Flap"),
            (BLINK, "Blink"),
        ] {
            assert_eq!(hash, name_hash(name), "{name}");
        }
        assert!(expression(FLAP).is_some_and(|e| e.period == 2.4));
        assert!(expression(0x1234).is_none());
    }

    #[test]
    fn the_wave_stays_between_its_weights_and_reaches_both() {
        let mut run = Running::start(expression(FLAP).unwrap(), 1.0, -1.0, 0.0, [0.0; 4]);
        let (mut low, mut high) = (f32::INFINITY, 0.0f32);
        for _ in 0..600 {
            let v = run.wave(1.0 / 60.0);
            low = low.min(v);
            high = high.max(v);
        }
        assert!((0.0..0.02).contains(&low), "{low}");
        assert!(high <= 1.0 && high > 0.98, "{high}");
        // Angry never drops under its half.
        let mut angry = Running::start(expression(ANGRY).unwrap(), 1.0, -1.0, 0.0, [0.0; 4]);
        for _ in 0..600 {
            let v = angry.wave(1.0 / 60.0);
            assert!((0.5..=1.0).contains(&v), "{v}");
        }
    }

    #[test]
    fn the_flap_moves_the_mouth_about_once_to_twice_a_second() {
        // Phase runs 0.08 to 0.13 a frame at 30 a second: 2.4 to 3.9 a
        // second, over a period of 2.4.
        let mut run = Running::start(expression(FLAP).unwrap(), 1.0, -1.0, 0.0, [0.0; 4]);
        let mut peaks = 0;
        let (mut a, mut b) = (0.0, 0.0);
        for _ in 0..600 {
            let v = run.wave(1.0 / 60.0);
            if b > a && b > v && b > 0.9 {
                peaks += 1;
            }
            (a, b) = (b, v);
        }
        assert!((9..=17).contains(&peaks), "{peaks} peaks in 10 s");
    }

    #[test]
    fn a_scared_face_blends_in_holds_and_blends_out_when_stopped() {
        let mut f = Face::default();
        assert!(f.at_rest());
        f.play(SCARED, 1.0, -1.0, [0.0; 4]);
        f.step(0.1, || 0.0);
        assert!(
            (f.weight(SCARED) - 0.5).abs() < 1e-4,
            "{}",
            f.weight(SCARED)
        );
        f.step(0.15, || 0.0);
        assert_eq!(f.weight(SCARED), 1.0);
        for _ in 0..100 {
            f.step(0.1, || 0.0);
        }
        assert_eq!(f.weight(SCARED), 1.0);
        // Stopped: a weight of 0, blended to from where it is.
        f.play(SCARED, 0.0, -1.0, [0.0; 4]);
        f.step(0.1, || 0.0);
        assert!((f.weight(SCARED) - 0.5).abs() < 1e-4);
        f.step(0.15, || 0.0);
        f.step(0.1, || 0.0);
        assert_eq!(f.weight(SCARED), 0.0);
        assert!(f.at_rest());
    }

    #[test]
    fn angry_shows_its_seconds_and_goes() {
        let mut f = Face::default();
        f.play(ANGRY, 1.0, ANGRY_SECONDS, [0.0; 4]);
        for _ in 0..19 {
            f.step(0.1, || 0.0);
        }
        assert!(f.weight(ANGRY) >= 0.5);
        for _ in 0..5 {
            f.step(0.1, || 0.0);
        }
        assert_eq!(f.weight(ANGRY), 0.0);
        assert!(!f.plays(ANGRY) && f.at_rest());
    }

    #[test]
    fn a_blink_shuts_a_fifth_of_a_second_every_two_to_four() {
        let mut f = Face::default();
        f.play(BLINK, 1.0, -1.0, [0.0; 4]);
        let (mut shut, mut blinks, mut was) = (0, 0, false);
        let ticks = 60 * 60;
        for _ in 0..ticks {
            f.step(1.0 / 60.0, || 0.5);
            let now = f.weight(BLINK) > 0.5;
            shut += usize::from(now);
            blinks += usize::from(now && !was);
            was = now;
        }
        // Off 3 s each time with the random number at a half: a blink
        // every 3.3 s or so.
        assert!((15..=20).contains(&blinks), "{blinks} blinks a minute");
        let each = shut as f32 / 60.0 / blinks as f32;
        assert!((0.1..=0.3).contains(&each), "{each} s shut each");
        assert!(f.plays(BLINK));
        f.disable_all();
        assert!(f.at_rest());
    }

    #[test]
    fn a_new_event_replaces_the_running_one_from_where_it_is() {
        let mut f = Face::default();
        f.play(SCARED, 1.0, -1.0, [0.0; 4]);
        for _ in 0..5 {
            f.step(0.1, || 0.0);
        }
        f.play(SCARED, 0.5, -1.0, [0.0; 4]);
        f.step(0.1, || 0.0);
        assert!((f.weight(SCARED) - 0.75).abs() < 1e-4);
        for _ in 0..5 {
            f.step(0.1, || 0.0);
        }
        assert!((f.weight(SCARED) - 0.5).abs() < 1e-4);
    }
}
