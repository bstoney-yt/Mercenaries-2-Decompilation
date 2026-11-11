//! The engine's additions to the scripts' `math` library: rounding,
//! the engine's random numbers, and small vector helpers.
//!
//! From the 360 code (library table `math` in the list at `0x82DE8680`):
//! the engine numbers in 32-bit floats (every result passes through
//! `frsp`), so each function here works in `f32`, and where the 360
//! fuses a multiply and an add (`fmadds`, `fmsubs`) so does the port
//! (`mul_add`), in the same order, so results match to the bit.
//!
//! | Lua name            | 360 native   |
//! |---------------------|--------------|
//! | `math.round`        | `0x82327610` |
//! | `math.randi`        | `0x82327878` |
//! | `math.randf`        | `0x82327948` |
//! | `math.GetXZHeading` | `0x823279E0` |
//! | `math.Normalize`    | `0x82327AA8` |
//! | `math.CrossProduct` | `0x82327B98` |
//! | `math.Length`       | `0x82327C58` |
//! | `math.PolarToRect`  | `0x82327CD8` |
//!
//! The vector natives read their arguments with the engine's vector
//! readers (`0x82308578` two numbers, `0x82308620` three, `0x823082B8`
//! one): each needs that many arguments present and every one a number
//! (or a numeric string), and any other call answers a single `nil`
//! (`0x82206F20` pushes nil). `round`, `randi` and `randf` use
//! `luaL_checknumber` instead, so a missing or non-numeric argument is a
//! Lua error.
//!
//! The random numbers come from the engine's two-step linear
//! congruential generator (`0x821AD4D8` integers, `0x8215E260` reals,
//! multiplier `0x19660D`, increment `0x3C6EF35F`). The scripts' natives
//! use its second stream, which starts at `0x94153A94`; in the game other
//! engine code draws from the same stream, so sequences only match the
//! 360's from a fresh start with no other draws (inferred).
//!
//! `PolarToRect` reads the engine's 8192-entry sine table (`0x82B56558`,
//! filled at start-up). Every use indexes it with `angle * 8192 / 2pi`,
//! and `-table[i - 2048]` stands in for the cosine, so entry `i` is
//! `sin(i * 2pi / 8192)`; the code that fills it wasn't found, so its
//! exact rounding is inferred.

use std::cell::Cell;
use std::rc::Rc;

use mlua::{Lua, MultiValue, Table, Value};

/// Degrees in a radian, as the engine stores it (`0x820F8C78`).
pub const DEG_PER_RAD: f32 = f32::from_bits(0x4265_2EEE);
/// Radians in a degree, as the engine stores it (`0x820F8C74`).
pub const RAD_PER_DEG: f32 = f32::from_bits(0x3C8E_F34D);
/// Entries in the engine's sine table.
pub const SINE_ENTRIES: usize = 8192;
/// The engine's 2pi (`0x82B7D8E4`).
const TWO_PI: f32 = f32::from_bits(0x40C9_0FDB);

/// `math.round(x)`: the nearest whole number, halves up
/// (`floor(x + 0.5)` in single precision).
pub fn round(x: f32) -> f32 {
    ((x + 0.5).floor() as i32) as f32
}

/// The engine's random number generator (one stream).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineRng {
    state: u32,
}

impl EngineRng {
    /// The scripts' stream at start-up (`0x82B5E588`).
    pub const SCRIPT_SEED: u32 = 0x9415_3A94;
    const MUL: u32 = 0x0019_660D;
    const ADD: u32 = 0x3C6E_F35F;

    pub fn new(seed: u32) -> Self {
        Self { state: seed }
    }

    /// Two steps of the generator: the second step's state, and the
    /// first step's top 16 bits.
    fn step(&mut self) -> (u32, u32) {
        let first = self.state.wrapping_mul(Self::MUL).wrapping_add(Self::ADD);
        let second = first.wrapping_mul(Self::MUL).wrapping_add(Self::ADD);
        self.state = second;
        (second, first >> 16)
    }

    /// A random whole number in `[0, n)`, `n > 0` (`0x821AD4D8`).
    pub fn below(&mut self, n: u32) -> u32 {
        let (second, high) = self.step();
        ((second & 0xFFFF_0000) | high) % n
    }

    /// A random real in `[0, 1)` (`0x8215E260`): 23 random bits as the
    /// mantissa of a number in `[1, 2)`, less one.
    pub fn unit(&mut self) -> f32 {
        let (second, high) = self.step();
        f32::from_bits((((second & 0xFFFF_01FF) | high) >> 9) | 0x3F80_0000) - 1.0
    }
}

impl Default for EngineRng {
    fn default() -> Self {
        Self::new(Self::SCRIPT_SEED)
    }
}

/// `math.randi(a, b)`: a random whole number in `[a, b]`, or in `[1, a]`
/// with one argument. Both bounds are truncated to integers; when `b <
/// a` the answer is `a`. The sum is taken as an unsigned 32-bit number,
/// so a range reaching below zero can answer about four billion (as the
/// 360 does).
pub fn randi(rng: &mut EngineRng, lo: f32, hi: f32) -> f32 {
    let lo = lo as i32;
    let span = (hi as i32).wrapping_sub(lo).wrapping_add(1);
    if span < 1 {
        return lo as f32;
    }
    (lo as u32).wrapping_add(rng.below(span as u32)) as f32
}

/// `math.randf(a, b)`: a random real from `a` to just under `b + 1`
/// (the 360 scales by `b - a + 1`); with fewer than two arguments, in
/// `[0, 1)`.
pub fn randf(rng: &mut EngineRng, range: Option<(f32, f32)>) -> f32 {
    let r = rng.unit();
    match range {
        // One fused multiply-add (`fmadds`).
        Some((a, b)) => r.mul_add((b - a) + 1.0, a),
        None => r,
    }
}

/// The engine's `atan2` wrapper (`0x82853F88`): zero for the zero vector.
fn atan2(y: f32, x: f32) -> f32 {
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }
    f64::from(y).atan2(f64::from(x)) as f32
}

/// `math.GetXZHeading(x, z)` or `(x, y, z)`: the heading of a vector in
/// degrees, about the vertical axis: 0 along `+z`, 90 along `+x`
/// (`0x82854F78`, `0x82855048`: `-atan2(-x, z)`).
pub fn xz_heading(x: f32, z: f32) -> f32 {
    -atan2(-x, z) * DEG_PER_RAD
}

/// `math.Normalize(x, y)`: the 2D vector scaled to length 1; the zero
/// vector stays zero.
pub fn normalize2(v: [f32; 2]) -> [f32; 2] {
    let square = v[0].mul_add(v[0], v[1] * v[1]);
    let scale = if 0.0 < square {
        1.0 / square.sqrt()
    } else {
        0.0
    };
    [v[0] * scale, scale * v[1]]
}

/// `math.Normalize(x, y, z)`: the vector scaled to length 1; the zero
/// vector stays zero (`0x82150580`).
pub fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let length = v[2].mul_add(v[2], v[0].mul_add(v[0], v[1] * v[1])).sqrt();
    if length == 0.0 {
        return [0.0; 3];
    }
    let scale = 1.0 / length;
    [v[0] * scale, scale * v[1], scale * v[2]]
}

/// `math.CrossProduct(ax, ay, az, bx, by, bz)`: `a x b`.
pub fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1].mul_add(b[2], -(a[2] * b[1])),
        a[2].mul_add(b[0], -(a[0] * b[2])),
        a[0].mul_add(b[1], -(b[0] * a[1])),
    ]
}

/// `math.Length(x, y, z)`: the vector's length.
pub fn length(v: [f32; 3]) -> f32 {
    v[0].mul_add(v[0], v[1].mul_add(v[1], v[2] * v[2])).sqrt()
}

/// Entry `i` of the engine's sine table (`i` taken modulo 8192).
pub fn sine_table(i: usize) -> f32 {
    let i = i % SINE_ENTRIES;
    (f64::from(i as u32) * std::f64::consts::TAU / SINE_ENTRIES as f64).sin() as f32
}

/// `math.PolarToRect(degrees, radius)`: the point `radius` away at
/// `degrees`, as `(radius cos, radius sin)` from the sine table (the
/// angle rounds to the nearest of its 8192 steps).
pub fn polar_to_rect(degrees: f32, radius: f32) -> (f32, f32) {
    let step = (SINE_ENTRIES as f32 / TWO_PI).mul_add(degrees * RAD_PER_DEG, 0.5);
    let i = step as i32;
    let sin = sine_table((i & 0x1FFF) as usize);
    // `-table[i - 2048]` is the cosine.
    let minus_cos = sine_table((i.wrapping_sub(0x800) & 0x1FFF) as usize);
    (-(minus_cos * radius), sin * radius)
}

/// Lua's `tonumber` for a native's argument (`lua_isnumber` accepts
/// numeric strings).
fn number(v: Option<&Value>) -> Option<f32> {
    match v? {
        Value::Integer(i) => Some(*i as f32),
        Value::Number(n) => Some(*n as f32),
        Value::String(s) => s
            .to_str()
            .ok()?
            .trim()
            .parse::<f64>()
            .ok()
            .map(|n| n as f32),
        _ => None,
    }
}

/// `luaL_checknumber`: argument `i` (from 1) or an error naming it.
fn check(args: &[Value], i: usize, name: &str) -> mlua::Result<f32> {
    number(args.get(i - 1)).ok_or_else(|| {
        mlua::Error::RuntimeError(format!("bad argument #{i} to '{name}' (number expected)"))
    })
}

/// `N` numbers from argument `first` on (from 1), or `None` when any is
/// missing or not a number (the engine's vector readers).
fn read<const N: usize>(args: &[Value], first: usize) -> Option<[f32; N]> {
    let mut out = [0.0; N];
    for (k, slot) in out.iter_mut().enumerate() {
        *slot = number(args.get(first - 1 + k))?;
    }
    Some(out)
}

fn numbers(values: impl IntoIterator<Item = f32>) -> MultiValue {
    values
        .into_iter()
        .map(|n| Value::Number(f64::from(n)))
        .collect()
}

fn nil() -> MultiValue {
    MultiValue::from_vec(vec![Value::Nil])
}

/// Adds the natives to `math` (and the scripts' `Math` alias, if
/// present), sharing one generator.
pub fn install(lua: &Lua, rng: Rc<Cell<EngineRng>>) -> mlua::Result<()> {
    let globals = lua.globals();
    let mut targets: Vec<Table> = vec![globals.get("math")?];
    if let Value::Table(t) = globals.raw_get::<Value>("Math")? {
        targets.push(t);
    }
    for t in &targets {
        t.set(
            "round",
            lua.create_function(|_, args: MultiValue| {
                let args: Vec<Value> = args.into_iter().collect();
                Ok(numbers([round(check(&args, 1, "round")?)]))
            })?,
        )?;
        let r = Rc::clone(&rng);
        t.set(
            "randi",
            lua.create_function(move |_, args: MultiValue| {
                let args: Vec<Value> = args.into_iter().collect();
                let (lo, hi) = if args.len() < 2 {
                    (1.0, check(&args, 1, "randi")?)
                } else {
                    (check(&args, 1, "randi")?, check(&args, 2, "randi")?)
                };
                let mut g = r.get();
                let n = randi(&mut g, lo, hi);
                r.set(g);
                Ok(numbers([n]))
            })?,
        )?;
        let r = Rc::clone(&rng);
        t.set(
            "randf",
            lua.create_function(move |_, args: MultiValue| {
                let args: Vec<Value> = args.into_iter().collect();
                let range = if args.len() < 2 {
                    None
                } else {
                    Some((check(&args, 1, "randf")?, check(&args, 2, "randf")?))
                };
                let mut g = r.get();
                let n = randf(&mut g, range);
                r.set(g);
                Ok(numbers([n]))
            })?,
        )?;
        t.set(
            "GetXZHeading",
            lua.create_function(|_, args: MultiValue| {
                let args: Vec<Value> = args.into_iter().collect();
                Ok(match args.len() {
                    2 => read::<2>(&args, 1).map(|[x, z]| xz_heading(x, z)),
                    3 => read::<3>(&args, 1).map(|[x, _, z]| xz_heading(x, z)),
                    _ => None,
                }
                .map_or_else(nil, |h| numbers([h])))
            })?,
        )?;
        t.set(
            "Normalize",
            lua.create_function(|_, args: MultiValue| {
                let args: Vec<Value> = args.into_iter().collect();
                Ok(match args.len() {
                    2 => read::<2>(&args, 1).map(|v| numbers(normalize2(v))),
                    3 => read::<3>(&args, 1).map(|v| numbers(normalize3(v))),
                    _ => None,
                }
                .unwrap_or_else(nil))
            })?,
        )?;
        t.set(
            "CrossProduct",
            lua.create_function(|_, args: MultiValue| {
                let args: Vec<Value> = args.into_iter().collect();
                Ok(if args.len() == 6 {
                    read::<3>(&args, 1)
                        .zip(read::<3>(&args, 4))
                        .map(|(a, b)| numbers(cross(a, b)))
                } else {
                    None
                }
                .unwrap_or_else(nil))
            })?,
        )?;
        t.set(
            "Length",
            lua.create_function(|_, args: MultiValue| {
                let args: Vec<Value> = args.into_iter().collect();
                Ok(if args.len() == 3 {
                    read::<3>(&args, 1).map(|v| numbers([length(v)]))
                } else {
                    None
                }
                .unwrap_or_else(nil))
            })?,
        )?;
        t.set(
            "PolarToRect",
            lua.create_function(|_, args: MultiValue| {
                let args: Vec<Value> = args.into_iter().collect();
                Ok(read::<1>(&args, 1)
                    .zip(read::<1>(&args, 2))
                    .map(|([d], [r])| {
                        let (x, y) = polar_to_rect(d, r);
                        numbers([x, y])
                    })
                    .unwrap_or_else(nil))
            })?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_takes_halves_up() {
        assert_eq!(round(2.5), 3.0);
        assert_eq!(round(2.49), 2.0);
        assert_eq!(round(-2.5), -2.0);
        assert_eq!(round(-2.51), -3.0);
        assert_eq!(round(-0.4), 0.0);
    }

    #[test]
    fn the_generator_steps_twice_per_draw() {
        // Worked by hand from the seed: 0x94153A94 * 0x19660D + 0x3C6EF35F,
        // twice; the integer draw is (second & 0xFFFF0000) | (first >> 16).
        let mut rng = EngineRng::default();
        assert_eq!(rng.below(10), 7);
        assert_eq!((rng.unit() + 1.0).to_bits(), 0x3FCB_67DB);
    }

    #[test]
    fn randi_spans_both_bounds_and_answers_the_low_bound_when_empty() {
        let mut rng = EngineRng::default();
        for _ in 0..200 {
            let n = randi(&mut rng, 1.0, 3.0);
            assert!((1.0..=3.0).contains(&n) && n.fract() == 0.0);
        }
        // One argument: [1, a], bounds truncated.
        assert_eq!(randi(&mut rng, 1.0, 1.9), 1.0);
        // b < a: the low bound, and no draw.
        let before = rng;
        assert_eq!(randi(&mut rng, 5.0, 2.0), 5.0);
        assert_eq!(rng, before);
    }

    #[test]
    fn randi_below_zero_wraps_as_the_360_does() {
        // [-5, -5]: one value, -5 + 0 taken unsigned.
        let mut rng = EngineRng::default();
        assert_eq!(randi(&mut rng, -5.0, -5.0), 4_294_967_291_u32 as f32);
    }

    #[test]
    fn randf_scales_by_one_more_than_the_range() {
        let mut first = EngineRng::default();
        let r = first.unit();
        let mut rng = EngineRng::default();
        assert_eq!(randf(&mut rng, Some((2.0, 2.0))), r + 2.0);
        let mut rng = EngineRng::default();
        assert_eq!(randf(&mut rng, None), r);
        let mut rng = EngineRng::default();
        for _ in 0..200 {
            let f = randf(&mut rng, Some((-1.0, 1.0)));
            assert!((-1.0..2.0).contains(&f));
        }
    }

    #[test]
    fn heading_is_zero_along_z_and_ninety_along_x() {
        assert_eq!(xz_heading(0.0, 1.0), 0.0);
        assert!((xz_heading(1.0, 0.0) - 90.0).abs() < 1e-4);
        assert!((xz_heading(-1.0, 0.0) + 90.0).abs() < 1e-4);
        // The engine's degrees-per-radian is a little large (57.29583), so
        // straight back reads just over 180.
        assert_eq!(xz_heading(0.0, -1.0), std::f32::consts::PI * DEG_PER_RAD);
        assert!(xz_heading(0.0, -1.0) > 180.0);
        assert_eq!(xz_heading(0.0, 0.0), 0.0);
    }

    #[test]
    fn normalize_leaves_zero_at_zero() {
        assert_eq!(normalize2([3.0, 4.0]), [0.6, 0.8]);
        assert_eq!(normalize2([0.0, 0.0]), [0.0, 0.0]);
        assert_eq!(normalize3([0.0, 0.0, 0.0]), [0.0; 3]);
        assert_eq!(normalize3([0.0, 0.0, -2.0]), [0.0, 0.0, -1.0]);
    }

    #[test]
    fn cross_and_length() {
        assert_eq!(cross([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]), [0.0, 0.0, 1.0]);
        assert_eq!(cross([0.0, 1.0, 0.0], [1.0, 0.0, 0.0]), [0.0, 0.0, -1.0]);
        assert_eq!(cross([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]), [-3.0, 6.0, -3.0]);
        assert_eq!(length([2.0, 3.0, 6.0]), 7.0);
    }

    #[test]
    fn polar_to_rect_reads_the_sine_table() {
        assert_eq!(sine_table(0), 0.0);
        assert_eq!(sine_table(2048), 1.0);
        assert_eq!(sine_table(6144), -1.0);
        assert_eq!(polar_to_rect(0.0, 3.0), (3.0, 0.0));
        let (x, y) = polar_to_rect(90.0, 2.0);
        assert_eq!((x.abs(), y), (0.0, 2.0));
        // Negative angles wrap round the table; the step truncates toward
        // zero (`fctiwz`), so -90 degrees lands one step short of -1.
        let (x, y) = polar_to_rect(-90.0, 1.0);
        assert_eq!(y, sine_table(8192 - 2047));
        assert!(x.abs() < 1e-3 && y > -1.0 && y < -0.9999);
    }

    fn lua() -> Lua {
        let lua = Lua::new();
        install(&lua, Rc::default()).unwrap();
        lua
    }

    #[test]
    fn vector_natives_answer_nil_to_the_wrong_arguments() {
        let lua = lua();
        let nils: (bool, bool, bool, bool, bool) = lua
            .load(
                r#"return math.Length(1, 2) == nil, math.CrossProduct(1, 2, 3, 4, 5) == nil,
                    math.Normalize(3, {}) == nil, math.GetXZHeading(1) == nil,
                    math.PolarToRect(90) == nil"#,
            )
            .eval()
            .unwrap();
        assert_eq!(nils, (true, true, true, true, true));
        let (x, y): (f32, f32) = lua
            .load(r#"return math.Normalize("3", "4")"#)
            .eval()
            .unwrap();
        assert_eq!((x, y), (0.6, 0.8));
        let h: f32 = lua
            .load("return math.GetXZHeading(1, 99, 0)")
            .eval()
            .unwrap();
        assert!((h - 90.0).abs() < 1e-4);
    }

    #[test]
    fn checked_natives_raise_on_a_missing_number() {
        let lua = lua();
        assert!(lua.load("return math.round()").exec().is_err());
        assert!(lua.load("return math.randi()").exec().is_err());
        let n: f32 = lua.load("return math.randf()").eval().unwrap();
        assert!((0.0..1.0).contains(&n));
    }
}
