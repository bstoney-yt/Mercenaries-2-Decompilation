//! How an `Ai.Goal` with `Goal = "PathMove"` has its path taken.
//!
//! From the 360 code (the goal's reader, 8230EB48, called from `Ai.Goal`
//! 8230FFB8 through 8230F920 by the hash of the goal's name):
//!
//! - `Target` is the path, and is needed; a path with no points gives no
//!   goal.
//! - `Start` says where on the path the goal begins: `"Nearest"`, or left
//!   out, the point of the path nearest to who has the goal; `"First"`
//!   and `"Last"` an end of it, counted the way it is travelled (with
//!   `Reverse`, `"First"` is the path's last point).
//! - `Reverse` (a boolean) travels the path's points last to first.
//! - `Mode` is `"Oneway"` (0, also when left out or unknown), `"Loop"`
//!   (1) or `"Bounce"` (2).
//! - `Haste` and `Timeout` are numbers, 0 when left out; `Investigate` is
//!   a boolean. The three ride in the goal as they are.
//! - Any goal's `Priority` is a number or one of `"loPri"` (0.25),
//!   `"medPri"` (3.0) and `"hiPri"` (14.0), and 4.5 when it is left out
//!   or below 0 (8230CC50, 8230F920). The names are matched by their hash,
//!   as written: `"HiPri"` is not `"hiPri"`. Nothing here uses it yet.
//!
//! Inferred: that after a `Nearest` start the goal goes on through the
//! points after it in the direction of travel (the reader only stores the
//! start and the flags; what walks the path was not read).

use mlua::{Table, Value};

/// Where on its path a `PathMove` begins (`Start`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PathStart {
    /// The point of the path nearest to who has the goal.
    #[default]
    Nearest,
    First,
    Last,
}

/// What a `PathMove` does at its path's end (`Mode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PathMode {
    #[default]
    Oneway,
    /// Back to its first point, and round again.
    Loop,
    /// Back the way it came, and so on.
    Bounce,
}

/// A `PathMove`'s own settings, as the scripts gave them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PathOrder {
    pub start: PathStart,
    pub reverse: bool,
    pub mode: PathMode,
    pub investigate: bool,
    /// Seconds as given, 0 for none (what runs out isn't known).
    pub timeout: f32,
}

impl PathOrder {
    /// Reads a goal's table.
    pub fn read(t: &Table) -> mlua::Result<Self> {
        let name = |k: &str| t.raw_get::<Option<String>>(k).ok().flatten();
        let flag = |k: &str| matches!(t.raw_get::<Value>(k), Ok(Value::Boolean(true)));
        Ok(Self {
            start: match name("Start").as_deref() {
                Some("First") => PathStart::First,
                Some("Last") => PathStart::Last,
                _ => PathStart::Nearest,
            },
            reverse: flag("Reverse"),
            mode: match name("Mode").as_deref() {
                Some("Loop") => PathMode::Loop,
                Some("Bounce") => PathMode::Bounce,
                _ => PathMode::Oneway,
            },
            investigate: flag("Investigate"),
            timeout: match t.raw_get::<Value>("Timeout")? {
                Value::Number(n) => n as f32,
                Value::Integer(i) => i as f32,
                _ => 0.0,
            },
        })
    }

    /// The path's points in the order they are travelled: `points` as
    /// they are, or last to first with `Reverse`.
    pub fn lap(&self, points: &[[f32; 3]]) -> Vec<[f32; 3]> {
        let mut lap = points.to_vec();
        if self.reverse {
            lap.reverse();
        }
        lap
    }

    /// The way from the goal's start to the path's end as it is
    /// travelled, for one at `from`: the whole lap from `First`, its last
    /// point alone from `Last`, and from `Nearest` the point of the path
    /// nearest `from` and the points after it.
    pub fn route(&self, points: &[[f32; 3]], from: [f32; 3]) -> Vec<[f32; 3]> {
        let lap = self.lap(points);
        match self.start {
            PathStart::First => lap,
            PathStart::Last => lap.last().map(|&p| vec![p]).unwrap_or_default(),
            PathStart::Nearest => {
                let Some((segment, at)) = nearest(&lap, from) else {
                    return lap;
                };
                let mut route = vec![at];
                route.extend(lap[segment + 1..].iter().copied().filter(|&p| p != at));
                route
            }
        }
    }
}

/// The point of the line through `lap` nearest `from`, and the index of
/// the point its stretch starts at. `None` for no points.
fn nearest(lap: &[[f32; 3]], from: [f32; 3]) -> Option<(usize, [f32; 3])> {
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    if lap.len() == 1 {
        return Some((0, lap[0]));
    }
    lap.windows(2)
        .enumerate()
        .map(|(i, w)| {
            let along = sub(w[1], w[0]);
            let length = dot(along, along);
            let t = if length > 0.0 {
                (dot(sub(from, w[0]), along) / length).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let at = [
                w[0][0] + along[0] * t,
                w[0][1] + along[1] * t,
                w[0][2] + along[2] * t,
            ];
            let off = sub(from, at);
            (i, at, dot(off, off))
        })
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(i, at, _)| (i, at))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATH: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [100.0, 0.0, 50.0]];

    fn order(lua: &mlua::Lua, table: &str) -> PathOrder {
        PathOrder::read(&lua.load(table).eval::<Table>().unwrap()).unwrap()
    }

    #[test]
    fn a_goal_left_bare_starts_nearest_and_goes_one_way() {
        let lua = mlua::Lua::new();
        assert_eq!(order(&lua, "{ Goal = 'PathMove' }"), PathOrder::default());
        // PirCon001's blocking ships, as their script orders them.
        let ships = order(
            &lua,
            "{ Goal = 'PathMove', Mode = 'Oneway', Reverse = false, Haste = 0.475, \
             Priority = 'hiPri' }",
        );
        assert_eq!(ships, PathOrder::default());
        let all = order(
            &lua,
            "{ Start = 'Last', Reverse = true, Mode = 'Bounce', Investigate = true, Timeout = 30 }",
        );
        assert_eq!(
            all,
            PathOrder {
                start: PathStart::Last,
                reverse: true,
                mode: PathMode::Bounce,
                investigate: true,
                timeout: 30.0,
            }
        );
        assert_eq!(order(&lua, "{ Mode = 'Loop' }").mode, PathMode::Loop);
        // Names are matched as written.
        assert_eq!(order(&lua, "{ Mode = 'loop' }").mode, PathMode::Oneway);
    }

    #[test]
    fn nearest_joins_the_path_where_it_passes_closest() {
        let o = PathOrder::default();
        // Beside the first stretch, 40 m along it.
        assert_eq!(
            o.route(&PATH, [40.0, 0.0, -7.0]),
            [[40.0, 0.0, 0.0], [100.0, 0.0, 0.0], [100.0, 0.0, 50.0]]
        );
        // Before the path's first point: the whole of it.
        assert_eq!(o.route(&PATH, [-20.0, 0.0, 5.0]), PATH);
        // By the corner: from the corner on, not the corner twice.
        assert_eq!(
            o.route(&PATH, [110.0, 0.0, -5.0]),
            [[100.0, 0.0, 0.0], [100.0, 0.0, 50.0]]
        );
        assert!(o.route(&[], [0.0; 3]).is_empty());
        assert_eq!(o.route(&PATH[..1], [9.0; 3]), [PATH[0]]);
    }

    #[test]
    fn first_and_last_are_counted_the_way_it_is_travelled() {
        let from = [50.0, 0.0, -7.0];
        let first = PathOrder {
            start: PathStart::First,
            ..PathOrder::default()
        };
        assert_eq!(first.route(&PATH, from), PATH);
        let back = PathOrder {
            reverse: true,
            ..first
        };
        // With Reverse, "First" is the path's last point.
        assert_eq!(back.route(&PATH, from), [PATH[2], PATH[1], PATH[0]]);
        let last = PathOrder {
            start: PathStart::Last,
            ..PathOrder::default()
        };
        assert_eq!(last.route(&PATH, from), [PATH[2]]);
        let last_back = PathOrder {
            reverse: true,
            ..last
        };
        assert_eq!(last_back.route(&PATH, from), [PATH[0]]);
        // Nearest, reversed: back towards the path's first point.
        let near_back = PathOrder {
            reverse: true,
            ..PathOrder::default()
        };
        assert_eq!(
            near_back.route(&PATH, from),
            [[50.0, 0.0, 0.0], [0.0, 0.0, 0.0]]
        );
    }
}
