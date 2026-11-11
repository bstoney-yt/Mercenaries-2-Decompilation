//! The scripts' clocks: `Sys.MainTime`, `Sys.RealTime`, the time stamps
//! (`Sys.MainTimeStamp`, `Sys.RealTimeStamp`, `Sys.TimeStampMark`,
//! `Sys.TimeStampGetElapsed`), `Sys.Clock` and `Sys.DiffTime`.
//!
//! From the 360 code:
//!
//! | Lua name                  | 360 native   |
//! |---------------------------|--------------|
//! | `Sys.MainTime`            | `0x82345250` |
//! | `Sys.RealTime`            | `0x823452C0` |
//! | `Sys.MainTimeStamp`       | `0x82345330` |
//! | `Sys.RealTimeStamp`       | `0x82345378` |
//! | `Sys.TimeStampMark`       | `0x823453C0` |
//! | `Sys.TimeStampGetElapsed` | `0x82345438` |
//! | `Sys.Clock`               | `0x823454B0` |
//! | `Sys.DiffTime`            | `0x82345A48` |
//!
//! The engine keeps two tick counters: the main (game) clock at
//! `0x83567F68` and the real clock at `0x83567F74`. Once a frame
//! (`0x82217EB0`, through `0x828567E0`) each gains its frame time times
//! 3000, truncated to a whole tick *per frame*, times the clock's rate;
//! the main clock takes the game's frame time, the real clock the real
//! one. A time stamp is a small userdata holding a tick count and the
//! clock it reads; its elapsed time is the tick difference over 3000
//! (`0x821754F0`). `MainTime`/`RealTime` read a static stamp taken at
//! tick zero, so they are the clock's seconds since start-up. Every
//! answer is a 32-bit float, as the engine's Lua numbers are.
//!
//! Inferred: the seconds per tick (`0x82E80AD4`, set at start-up by code
//! not found) is 1/3000, as every other use converts seconds to ticks by
//! 3000 (e.g. `0x8239F258`), and the clocks' rate is 1. The port has no
//! time scale yet (`Sys.SetTimeScale` isn't ported), so both clocks step
//! by the same frame time.
//!
//! `Clock` is the C runtime's `clock()`: whole milliseconds since the
//! process started, times 0.001. `DiffTime(t1, t2)` is `difftime` on the
//! two arguments truncated to integers (`t2` defaults to 0); a negative
//! one gives 0.

use std::cell::Cell;
use std::sync::OnceLock;
use std::time::Instant;

use mlua::{AnyUserData, Lua, MultiValue, UserData, Value};

/// Clock ticks in a second.
pub const TICKS_PER_SECOND: f32 = 3000.0;
/// Seconds in a tick (inferred, see the module notes).
const SECONDS_PER_TICK: f32 = 1.0 / TICKS_PER_SECOND;
/// A clock's rate (inferred).
const RATE: f32 = 1.0;

/// Which of the engine's two clocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockKind {
    /// Game time (`0x83567F68`).
    Main,
    /// Real time (`0x83567F74`).
    Real,
}

/// The two tick counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Clocks {
    pub main: i32,
    pub real: i32,
}

impl Clocks {
    /// One frame: `game_dt` seconds of game time, `real_dt` of real time.
    pub fn step(&mut self, game_dt: f32, real_dt: f32) {
        self.main = advance(self.main, game_dt);
        self.real = advance(self.real, real_dt);
    }

    /// The ticks on clock `kind`.
    pub fn ticks(&self, kind: ClockKind) -> i32 {
        match kind {
            ClockKind::Main => self.main,
            ClockKind::Real => self.real,
        }
    }
}

/// `0x828567E0`: a clock's ticks after a frame of `dt` seconds.
fn advance(ticks: i32, dt: f32) -> i32 {
    let whole = (dt * TICKS_PER_SECOND) as i32;
    ((RATE * whole as f32) as i32).wrapping_add(ticks)
}

/// Seconds for a tick count.
pub fn seconds(ticks: i32) -> f32 {
    ticks as f32 * SECONDS_PER_TICK
}

/// A script's time stamp: a tick count on one clock.
#[derive(Debug)]
pub struct TimeStamp {
    pub ticks: Cell<i32>,
    pub clock: ClockKind,
}

impl UserData for TimeStamp {}

/// `Sys.MainTime` / `Sys.RealTime`.
pub fn time(clocks: &Clocks, kind: ClockKind) -> MultiValue {
    number(seconds(clocks.ticks(kind)))
}

/// `Sys.MainTimeStamp` / `Sys.RealTimeStamp`.
pub fn stamp(lua: &Lua, clocks: &Clocks, kind: ClockKind) -> mlua::Result<MultiValue> {
    let stamp = lua.create_userdata(TimeStamp {
        ticks: Cell::new(clocks.ticks(kind)),
        clock: kind,
    })?;
    Ok(MultiValue::from_vec(vec![Value::UserData(stamp)]))
}

/// The stamp in argument 1, if it is one.
fn stamp_arg(args: &[Value]) -> Option<AnyUserData> {
    match args.first()? {
        Value::UserData(u) if u.is::<TimeStamp>() => Some(u.clone()),
        _ => None,
    }
}

/// `Sys.TimeStampMark(stamp)`: the stamp takes its clock's time now.
/// Nothing is answered.
pub fn mark(clocks: &Clocks, args: &[Value]) -> mlua::Result<MultiValue> {
    if let Some(u) = stamp_arg(args) {
        let s = u.borrow::<TimeStamp>()?;
        s.ticks.set(clocks.ticks(s.clock));
    }
    Ok(MultiValue::new())
}

/// `Sys.TimeStampGetElapsed(stamp)`: seconds since the stamp's time, or
/// nothing at all when the argument isn't a stamp.
pub fn elapsed(clocks: &Clocks, args: &[Value]) -> mlua::Result<MultiValue> {
    let Some(u) = stamp_arg(args) else {
        return Ok(MultiValue::new());
    };
    let s = u.borrow::<TimeStamp>()?;
    Ok(number(seconds(
        clocks.ticks(s.clock).wrapping_sub(s.ticks.get()),
    )))
}

/// When the process started, for `Sys.Clock`.
fn process_start() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

/// `clock()` for `ms` milliseconds: seconds, in steps of a millisecond.
pub fn clock_seconds(ms: u128) -> f32 {
    (ms as i32) as f32 * 0.001
}

/// `Sys.Clock()`.
pub fn clock() -> MultiValue {
    number(clock_seconds(process_start().elapsed().as_millis()))
}

/// `difftime(t1, t2)` on whole seconds.
pub fn diff_time(t1: f32, t2: f32) -> f32 {
    let (t1, t2) = (t1 as i64, t2 as i64);
    if t1 < 0 || t2 < 0 {
        0.0
    } else {
        (t1 - t2) as f32
    }
}

/// Lua's `tonumber` for a number argument.
fn to_number(v: Option<&Value>) -> Option<f32> {
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

/// `Sys.DiffTime(t1 [, t2])`: `t1` must be a number; a `t2` given must
/// be one too (`luaL_optnumber`).
pub fn diff_time_native(args: &[Value]) -> mlua::Result<MultiValue> {
    let bad = |i: usize| {
        mlua::Error::RuntimeError(format!("bad argument #{i} to 'DiffTime' (number expected)"))
    };
    let t1 = to_number(args.first()).ok_or_else(|| bad(1))?;
    let t2 = match args.get(1) {
        None | Some(Value::Nil) => 0.0,
        v => to_number(v).ok_or_else(|| bad(2))?,
    };
    Ok(number(diff_time(t1, t2)))
}

fn number(n: f32) -> MultiValue {
    MultiValue::from_vec(vec![Value::Number(f64::from(n))])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thirty_frames_at_thirty_a_second_are_one_second() {
        let mut c = Clocks::default();
        for _ in 0..30 {
            c.step(1.0 / 30.0, 1.0 / 30.0);
        }
        assert_eq!(c.main, 3000);
        assert_eq!(seconds(c.main), 1.0);
    }

    #[test]
    fn each_frame_loses_its_fraction_of_a_tick() {
        // A 16.7 ms frame is 50.1 ticks: 50 a frame, so 60 frames are
        // 3000 ticks (1 s) rather than 1.002 s.
        let mut c = Clocks::default();
        for _ in 0..60 {
            c.step(0.0167, 0.0);
        }
        assert_eq!(c.main, 3000);
        assert_eq!(c.real, 0);
    }

    #[test]
    fn diff_time_truncates_and_refuses_negatives() {
        assert_eq!(diff_time(5.7, 1.2), 4.0);
        assert_eq!(diff_time(-1.0, 0.0), 0.0);
        assert_eq!(diff_time(3.0, -0.5), 3.0);
        // Milliseconds times the float 0.001, as the 360 does.
        assert_eq!(clock_seconds(1234), 1234.0 * 0.001_f32);
        assert_eq!(clock_seconds(1234), 1.234_000_1);
    }

    #[test]
    fn stamps_mark_and_measure_through_lua() {
        let lua = Lua::new();
        let clocks = std::rc::Rc::new(std::cell::RefCell::new(Clocks::default()));
        let sys = lua.create_table().unwrap();
        for (name, kind) in [
            ("MainTimeStamp", ClockKind::Main),
            ("RealTimeStamp", ClockKind::Real),
        ] {
            let c = clocks.clone();
            let f = lua
                .create_function(move |lua, ()| stamp(lua, &c.borrow(), kind))
                .unwrap();
            sys.set(name, f).unwrap();
        }
        let c = clocks.clone();
        let f = lua
            .create_function(move |_, args: MultiValue| {
                let args: Vec<Value> = args.into_iter().collect();
                elapsed(&c.borrow(), &args)
            })
            .unwrap();
        sys.set("TimeStampGetElapsed", f).unwrap();
        let c = clocks.clone();
        let f = lua
            .create_function(move |_, args: MultiValue| {
                let args: Vec<Value> = args.into_iter().collect();
                mark(&c.borrow(), &args)
            })
            .unwrap();
        sys.set("TimeStampMark", f).unwrap();
        lua.globals().set("Sys", sys).unwrap();

        lua.load("main, real = Sys.MainTimeStamp(), Sys.RealTimeStamp()")
            .exec()
            .unwrap();
        clocks.borrow_mut().step(1.0, 0.5);
        let (m, r): (f32, f32) = lua
            .load("return Sys.TimeStampGetElapsed(main), Sys.TimeStampGetElapsed(real)")
            .eval()
            .unwrap();
        assert_eq!((m, r), (1.0, 0.5));
        let n: usize = lua
            .load("Sys.TimeStampMark(main); return select('#', Sys.TimeStampMark(main))")
            .eval()
            .unwrap();
        assert_eq!(n, 0);
        let m: f32 = lua
            .load("return Sys.TimeStampGetElapsed(main)")
            .eval()
            .unwrap();
        assert_eq!(m, 0.0);
        // Not a stamp: no answer at all.
        let n: usize = lua
            .load("return select('#', Sys.TimeStampGetElapsed(42))")
            .eval()
            .unwrap();
        assert_eq!(n, 0);
    }
}
