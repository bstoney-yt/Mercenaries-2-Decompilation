//! Summary of an m2-alpha autoplay run, read from its stderr log
//! (`<name>.err`), so a playtest thread doesn't have to read the log by hand.
//!
//! The summary is generic: it reports what the log says (contract states,
//! objectives, markers, control locks, hops, Lua errors, the frame rate and
//! how the run ended) plus two tooling heuristics over her logged positions
//! (the longest stretch she stayed put, and A-B-A movement loops). It
//! encodes no mission's success rule.
//!
//! Times are seconds of play. Lines that carry no time of their own
//! (`mission:`, `hijack:`, `game saved ...`) take the last play time seen
//! before them, so theirs are approximate.

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// Thresholds for the position heuristics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Config {
    /// A stretch counts as "no progress" when she stays within
    /// `still_radius` metres of where it began for at least this long.
    pub stall_seconds: f64,
    /// Metres she may drift and still count as standing still.
    pub still_radius: f64,
    /// Metres within which a return to an earlier spot counts as the same
    /// spot, for A-B-A loops; the B in between must be farther than this.
    pub osc_radius: f64,
    /// Returns needed before a loop is reported (A-B-A is one return).
    pub osc_min_returns: usize,
    /// The run's `--mission-seconds`, if known: a run whose autoplay played
    /// (nearly) this long ended on the time limit.
    pub mission_seconds: Option<f64>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            stall_seconds: 60.0,
            still_radius: 2.0,
            osc_radius: 5.0,
            osc_min_returns: 2,
            mission_seconds: None,
        }
    }
}

/// A position: x, y, z in metres.
pub type Pos = [f64; 3];

/// One timed event in a list (a transition, a save state, a hop).
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub t: f64,
    pub what: String,
}

/// One objective line's life.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectiveItem {
    pub name: String,
    pub first: f64,
    /// When it last went away; `None` if still shown at the end.
    pub gone: Option<f64>,
    /// Times it appeared (more than 1 when it flickers in and out).
    pub appearances: usize,
}

/// The quoted strings of a `["a", "b"]` list, with any `(n/m)` count at
/// the end dropped.
fn objective_names(list: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = list.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut item = String::new();
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    if let Some(n) = chars.next() {
                        item.push(n);
                    }
                }
                '"' => break,
                c => item.push(c),
            }
        }
        let name = match item.rfind(" (") {
            Some(i)
                if item.ends_with(')')
                    && item[i + 2..item.len() - 1]
                        .split('/')
                        .all(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())) =>
            {
                item[..i].to_string()
            }
            _ => item,
        };
        if !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// A marker the scripts showed, by kind, name and entity.
#[derive(Debug, Clone, PartialEq)]
pub struct Marker {
    pub kind: String,
    pub name: String,
    pub id: String,
    pub first: f64,
    pub last: f64,
    pub lines: usize,
}

/// A span between two times, with a count of lines inside it.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub start: f64,
    /// `None` if the log ended inside the window.
    pub end: Option<f64>,
    pub label: String,
    /// Autoplay action lines logged inside the window (what she did while
    /// locked); status lines (`at [..]`, `HQ state`, `on contract`,
    /// `objectives`, tallies, `controls`) are not counted.
    pub action_lines: usize,
}

/// A stretch where she stayed within the still radius.
#[derive(Debug, Clone, PartialEq)]
pub struct Still {
    pub start: f64,
    pub end: f64,
    pub at: Pos,
    pub samples: usize,
}

/// A run of A-B-A returns between two spots.
#[derive(Debug, Clone, PartialEq)]
pub struct Loop {
    pub start: f64,
    pub end: f64,
    pub a: Pos,
    pub b: Pos,
    pub returns: usize,
}

/// A distinct Lua error message.
#[derive(Debug, Clone, PartialEq)]
pub struct LuaError {
    pub message: String,
    pub first: f64,
    pub count: usize,
}

/// Min, max and count of a logged quantity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Range {
    pub min: f64,
    pub max: f64,
    pub count: usize,
}

impl Range {
    fn add(range: &mut Option<Range>, v: f64) {
        match range {
            Some(r) => {
                r.min = r.min.min(v);
                r.max = r.max.max(v);
                r.count += 1;
            }
            None => {
                *range = Some(Range {
                    min: v,
                    max: v,
                    count: 1,
                })
            }
        }
    }
}

/// How the run ended.
#[derive(Debug, Clone, PartialEq)]
pub struct End {
    /// `clean`, `panic`, `error` or `truncated` (no end-of-run summary:
    /// killed, timed out from outside, or crashed without a message).
    pub verdict: String,
    /// The `autoplay: N s played` line's play time.
    pub played: Option<f64>,
    /// Hops the end-of-run summary counts.
    pub hops: Option<usize>,
    /// The end-of-run summary said the contract was not completed.
    pub not_completed: bool,
    /// Played up to `--mission-seconds` without completing.
    pub time_limit: bool,
    /// The last line is `wrote <picture>` (the `--screenshot`).
    pub screenshot_written: bool,
    /// The panic or error line, if any.
    pub message: Option<String>,
}

/// Everything the summary reports.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub config: Config,
    pub lines: usize,
    pub last_time: f64,
    /// `on contract` changes: the contract autoplay is on, or `none`.
    pub contract_states: Vec<Event>,
    /// `accept: X is active` lines.
    pub accepted: Vec<Event>,
    /// `contract N completed` and `job X done` lines.
    pub completed: Vec<Event>,
    /// `game saved with ...` states, consecutive repeats dropped.
    pub saves: Vec<Event>,
    /// `load: ... the scripts hold ...` states.
    pub loads: Vec<Event>,
    /// `objectives [...]` changes.
    pub objectives: Vec<Event>,
    /// Each objective line, by its text without a trailing `(n/m)` count:
    /// when it first appeared and when it last went away.
    pub objective_items: Vec<ObjectiveItem>,
    pub markers: Vec<Marker>,
    /// `controls off` .. `controls on` spans from autoplay's lines.
    pub locks: Vec<Window>,
    /// `hijack: X started` .. `hijack: done` spans.
    pub hijacks: Vec<Window>,
    /// `no headway ... hopped to` lines.
    pub hops: Vec<Event>,
    /// Times of `nothing to do` lines.
    pub idle: Vec<f64>,
    pub positions: usize,
    pub longest_still: Option<Still>,
    /// Still stretches of at least `stall_seconds`.
    pub stalls: Vec<Still>,
    pub loops: Vec<Loop>,
    pub lua_errors: Vec<LuaError>,
    /// Lua memory in KB, from any line naming Lua memory with a size.
    pub lua_memory_kb: Option<Range>,
    /// Frames a second from `pace:` lines.
    pub fps: Option<Range>,
    /// Slowest frame in ms from `pace:` lines.
    pub slowest_frame_ms: Option<Range>,
    pub end: End,
}

/// The time and body of a line that starts with a play time, such as
/// `autoplay 12.5 s: ...` or `accept: 3.0 s: ...`.
fn timed(line: &str) -> Option<(f64, &str)> {
    const PREFIXES: [&str; 7] = [
        "autoplay tally ",
        "autoplay checks ",
        "after contract tally ",
        "after contract checks ",
        "after contract: ",
        "accept: ",
        "autoplay ",
    ];
    for p in PREFIXES {
        let Some(rest) = line.strip_prefix(p) else {
            continue;
        };
        let Some((num, after)) = rest.split_once(' ') else {
            continue;
        };
        let Ok(t) = num.parse::<f64>() else {
            continue;
        };
        let Some(body) = after
            .strip_prefix("s:")
            .or_else(|| after.strip_prefix("s,"))
        else {
            continue;
        };
        return Some((t, body.trim_start()));
    }
    None
}

/// The number in `after <T> s of play`.
fn after_play(line: &str) -> Option<f64> {
    let i = line.find("after ")?;
    let rest = &line[i + 6..];
    let (num, after) = rest.split_once(' ')?;
    after.starts_with("s of play").then_some(())?;
    num.parse().ok()
}

/// The `[x, y, z]` right after the first `pat` in `s` (`pat` ends in `[`).
fn triple_after(s: &str, pat: &str) -> Option<Pos> {
    let i = s.find(pat)? + pat.len();
    triple(&s[i..])
}

/// The `[x, y, z]` after the last `pat` in `s`.
fn triple_after_last(s: &str, pat: &str) -> Option<Pos> {
    let i = s.rfind(pat)? + pat.len();
    triple(&s[i..])
}

/// `x, y, z]` at the start of `s`.
fn triple(s: &str) -> Option<Pos> {
    let inner = &s[..s.find(']')?];
    let mut it = inner.split(',').map(|v| v.trim().parse::<f64>());
    let p = [it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?];
    it.next().is_none().then_some(p)
}

fn dist(a: Pos, b: Pos) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// The text after `start` up to the next `end`.
fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let i = s.find(start)? + start.len();
    let rest = &s[i..];
    Some(&rest[..rest.find(end)?])
}

/// The first number before one of the size units in `s`, in KB.
fn size_kb(s: &str) -> Option<f64> {
    let lower = s.to_ascii_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| c.is_whitespace() || c == ',' || c == '(' || c == ')' || c == '=')
        .filter(|w| !w.is_empty())
        .collect();
    for (i, w) in words.iter().enumerate() {
        // "123kb" or "123 kb".
        let split = w
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(w.len());
        let (num, unit) = if split == w.len() {
            (*w, words.get(i + 1).copied().unwrap_or(""))
        } else {
            w.split_at(split)
        };
        let Ok(v) = num.parse::<f64>() else {
            continue;
        };
        let scale = match unit {
            "b" | "bytes" => 1.0 / 1024.0,
            "kb" | "kib" | "k" => 1.0,
            "mb" | "mib" => 1024.0,
            _ => continue,
        };
        return Some(v * scale);
    }
    None
}

#[derive(Default)]
struct Parser {
    s: Option<Summary>,
    clock: f64,
    open_lock: Option<Window>,
    open_hijack: Option<Window>,
    /// A hijack whose `done` line came; it ends at the next timed line.
    hijack_done: Option<Window>,
    shown_objectives: Vec<String>,
    samples: Vec<(f64, Pos)>,
    markers: BTreeMap<(String, String), Marker>,
    lua: Vec<LuaError>,
    last_line: String,
    current_contract: Option<String>,
    last_objectives: Option<String>,
}

/// Summarizes the log text of one run.
pub fn summarize(log: &str, config: Config) -> Summary {
    let mut p = Parser {
        s: Some(Summary {
            config,
            lines: 0,
            last_time: 0.0,
            contract_states: Vec::new(),
            accepted: Vec::new(),
            completed: Vec::new(),
            saves: Vec::new(),
            loads: Vec::new(),
            objectives: Vec::new(),
            objective_items: Vec::new(),
            markers: Vec::new(),
            locks: Vec::new(),
            hijacks: Vec::new(),
            hops: Vec::new(),
            idle: Vec::new(),
            positions: 0,
            longest_still: None,
            stalls: Vec::new(),
            loops: Vec::new(),
            lua_errors: Vec::new(),
            lua_memory_kb: None,
            fps: None,
            slowest_frame_ms: None,
            end: End {
                verdict: String::new(),
                played: None,
                hops: None,
                not_completed: false,
                time_limit: false,
                screenshot_written: false,
                message: None,
            },
        }),
        ..Parser::default()
    };
    for line in log.lines() {
        p.line(line.trim_end_matches('\r'));
    }
    p.finish()
}

impl Parser {
    fn line(&mut self, line: &str) {
        let s = self.s.as_mut().expect("summary");
        s.lines += 1;
        if !line.trim().is_empty() {
            self.last_line = line.to_string();
        }
        let timed = timed(line);
        if let Some((t, _)) = timed {
            self.clock = t;
            if let Some(mut w) = self.hijack_done.take() {
                w.end = Some(t);
                s.hijacks.push(w);
            }
        } else if let Some(t) = after_play(line) {
            self.clock = t;
        }
        let t = self.clock;

        // Panics and fatal errors.
        if line.contains("panicked at") || line.starts_with("Error: ") {
            if s.end.message.is_none() {
                s.end.message = Some(clip(line, 200));
            }
            s.end.verdict = if line.contains("panicked at") {
                "panic".into()
            } else {
                "error".into()
            };
        }

        // Lua errors: runtime errors and the `attempt to` family.
        if !line.starts_with('\t')
            && (line.contains("runtime error") || line.contains("attempt to "))
        {
            let message = line
                .find("runtime error: ")
                .map(|i| &line[i + 15..])
                .unwrap_or(line);
            let message = clip(message.trim(), 160);
            match self.lua.iter_mut().find(|e| e.message == message) {
                Some(e) => e.count += 1,
                None => self.lua.push(LuaError {
                    message,
                    first: t,
                    count: 1,
                }),
            }
        }

        // Lua memory, if a line logs it.
        let lower = line.to_ascii_lowercase();
        if lower.contains("lua")
            && (lower.contains("memory") || lower.contains(" mem") || lower.contains("heap"))
            && let Some(kb) = size_kb(line)
        {
            Range::add(&mut s.lua_memory_kb, kb);
        }

        // Frame rate.
        if let Some(rest) = line.strip_prefix("pace: ") {
            if let Some(fps) = rest
                .split_once(" fps")
                .and_then(|(n, _)| n.trim().parse::<f64>().ok())
            {
                Range::add(&mut s.fps, fps);
            }
            if let Some(ms) = between(rest, "slowest frame ", " ms").and_then(|n| n.parse().ok()) {
                Range::add(&mut s.slowest_frame_ms, ms);
            }
        }

        // Contract states.
        if let Some(rest) = line.strip_prefix("accept: ")
            && let Some((id, _)) = rest.split_once(" is active")
            && !id.contains(' ')
        {
            s.accepted.push(Event {
                t,
                what: id.to_string(),
            });
        }
        if line.starts_with("autoplay: contract ") && line.contains(" completed after ") {
            let n = line["autoplay: contract ".len()..]
                .split(' ')
                .next()
                .unwrap_or("?");
            s.completed.push(Event {
                t,
                what: format!("contract {n}"),
            });
        }
        if let Some(rest) = line.strip_prefix("autoplay: job ")
            && rest.contains(" done after ")
        {
            let id = rest.split(' ').next().unwrap_or("?");
            s.completed.push(Event {
                t,
                what: format!("job {id}"),
            });
        }
        if let Some(state) = line.strip_prefix("game saved with ")
            && s.saves.last().is_none_or(|e| e.what != state)
        {
            s.saves.push(Event {
                t,
                what: state.to_string(),
            });
        }
        if line.starts_with("load: ")
            && let Some(state) = between(line, "the scripts hold ", "; she is")
        {
            s.loads.push(Event {
                t,
                what: state.to_string(),
            });
        }

        // Markers: `mission: marker <kind> on "<name>" (<id>)`.
        if let Some(rest) = line.strip_prefix("mission: marker ")
            && let Some((kind, rest)) = rest.split_once(" on \"")
            && let Some((name, rest)) = rest.split_once('"')
        {
            let id = between(rest, "(", ")").unwrap_or("").to_string();
            let m = self
                .markers
                .entry((kind.to_string(), id.clone() + name))
                .or_insert_with(|| Marker {
                    kind: kind.to_string(),
                    name: name.to_string(),
                    id,
                    first: t,
                    last: t,
                    lines: 0,
                });
            m.last = t;
            m.lines += 1;
        }

        // Hijack spans.
        if let Some(rest) = line.strip_prefix("hijack: ") {
            if let Some(vehicle) = rest.strip_suffix(" started")
                && !vehicle.contains(' ')
            {
                self.open_hijack = Some(Window {
                    start: t,
                    end: None,
                    label: vehicle.to_string(),
                    action_lines: 0,
                });
            } else if rest == "done" {
                self.hijack_done = self.open_hijack.take();
            }
        }

        // The end-of-run summary: `autoplay: <T> s played, <N> hops...`.
        if let Some(rest) = line.strip_prefix("autoplay: ")
            && let Some((num, after)) = rest.split_once(' ')
            && after.starts_with("s played, ")
            && let Ok(played) = num.parse::<f64>()
        {
            s.end.played = Some(played);
            s.end.hops = between(after, "s played, ", " hops").and_then(|n| n.parse().ok());
            s.end.not_completed = line.contains("the contract was not completed");
        }

        let Some((t, body)) = timed.filter(|_| line.starts_with("autoplay ")) else {
            // Positions also come from `after contract: T s, she is at [..]`.
            if line.starts_with("after contract: ")
                && let Some(pos) = triple_after(line, "she is at [")
            {
                self.samples.push((t, pos));
            }
            return;
        };
        let is_tally = line.starts_with("autoplay tally ") || line.starts_with("autoplay checks ");

        // Locks: autoplay's own `controls off` / `controls on`.
        if body.starts_with("controls off") {
            if self.open_lock.is_none() {
                self.open_lock = Some(Window {
                    start: t,
                    end: None,
                    label: body.to_string(),
                    action_lines: 0,
                });
            }
        } else if body.starts_with("controls on") {
            if let Some(mut w) = self.open_lock.take() {
                w.end = Some(t);
                s.locks.push(w);
            }
        } else if !is_tally
            && !["HQ state:", "on contract", "objectives ", "at ["]
                .iter()
                .any(|p| body.starts_with(p))
        {
            if let Some(w) = self.open_lock.as_mut() {
                w.action_lines += 1;
            }
            if let Some(w) = self.open_hijack.as_mut() {
                w.action_lines += 1;
            }
        }
        if is_tally {
            return;
        }

        if let Some(rest) = body.strip_prefix("on contract ") {
            let now = between(rest, "Some((\"", "\"").map(str::to_string);
            if now != self.current_contract || s.contract_states.is_empty() {
                s.contract_states.push(Event {
                    t,
                    what: now.clone().unwrap_or_else(|| "none".into()),
                });
                self.current_contract = now;
            }
        }
        if let Some(list) = body.strip_prefix("objectives ")
            && self.last_objectives.as_deref() != Some(list)
        {
            s.objectives.push(Event {
                t,
                what: list.to_string(),
            });
            self.last_objectives = Some(list.to_string());
            let now = objective_names(list);
            for name in &now {
                if self.shown_objectives.contains(name) {
                    continue;
                }
                match s.objective_items.iter_mut().find(|o| &o.name == name) {
                    Some(o) => {
                        o.appearances += 1;
                        o.gone = None;
                    }
                    None => s.objective_items.push(ObjectiveItem {
                        name: name.clone(),
                        first: t,
                        gone: None,
                        appearances: 1,
                    }),
                }
            }
            for name in &self.shown_objectives {
                if !now.contains(name)
                    && let Some(o) = s.objective_items.iter_mut().find(|o| &o.name == name)
                {
                    o.gone = Some(t);
                }
            }
            self.shown_objectives = now;
        }
        if body.starts_with("nothing to do") || body.contains(", nothing to do;") {
            s.idle.push(t);
        }

        // Her position.
        if body.starts_with("at [") {
            if let Some(pos) = triple_after(body, "at [") {
                self.samples.push((t, pos));
            }
        } else if body.starts_with("no headway") {
            let from = triple_after(body, " at [");
            let to = triple_after_last(body, "hopped to [");
            if let Some(a) = from {
                self.samples.push((t, a));
            }
            if let Some(b) = to {
                self.samples.push((t, b));
                s.hops.push(Event {
                    t,
                    what: between(body, "no headway for ", " s")
                        .map(|n| format!("no headway {n} s"))
                        .unwrap_or_default(),
                });
            }
        } else if body.starts_with("HQ state:") {
            if let Some(pos) = triple_after_last(body, ", at [") {
                self.samples.push((t, pos));
            }
        } else if let Some(pos) = triple_after_last(body, " from [") {
            self.samples.push((t, pos));
        }
    }

    fn finish(mut self) -> Summary {
        let mut s = self.s.take().expect("summary");
        s.last_time = self.clock;
        if let Some(w) = self.open_lock.take() {
            s.locks.push(w);
        }
        if let Some(mut w) = self.hijack_done.take() {
            w.end = Some(self.clock);
            s.hijacks.push(w);
        }
        if let Some(w) = self.open_hijack.take() {
            s.hijacks.push(w);
        }
        s.markers = self.markers.into_values().collect();
        s.markers
            .sort_by(|a, b| a.first.total_cmp(&b.first).then(a.kind.cmp(&b.kind)));
        s.lua_errors = self.lua;
        s.positions = self.samples.len();
        let (longest, stalls) = still_windows(&self.samples, &s.config);
        s.longest_still = longest;
        s.stalls = stalls;
        s.loops = loops(&self.samples, &s.config);

        s.end.screenshot_written = self.last_line.starts_with("wrote ");
        if s.end.verdict.is_empty() {
            s.end.verdict = if s.end.played.is_some() {
                "clean".into()
            } else {
                "truncated".into()
            };
        }
        if let (Some(limit), Some(played)) = (s.config.mission_seconds, s.end.played) {
            // Autoplay's clock starts a few seconds after the run's, so allow
            // 15 s (or 1%) of slack.
            s.end.time_limit = played + (limit * 0.01).max(15.0) >= limit;
        }
        s
    }
}

/// Stretches where she stayed within `still_radius` of where each began:
/// the longest, and every one of at least `stall_seconds`.
fn still_windows(samples: &[(f64, Pos)], c: &Config) -> (Option<Still>, Vec<Still>) {
    let mut longest: Option<Still> = None;
    let mut stalls = Vec::new();
    let mut i = 0;
    while i < samples.len() {
        let (t0, anchor) = samples[i];
        let mut j = i;
        while j + 1 < samples.len() && dist(samples[j + 1].1, anchor) <= c.still_radius {
            j += 1;
        }
        let w = Still {
            start: t0,
            end: samples[j].0,
            at: anchor,
            samples: j - i + 1,
        };
        let len = w.end - w.start;
        if len >= c.stall_seconds {
            stalls.push(w.clone());
        }
        if longest.as_ref().is_none_or(|l| len > l.end - l.start) {
            longest = Some(w);
        }
        i = j + 1;
    }
    (longest, stalls)
}

/// A-B-A loops: her path cut into spots at least `osc_radius` apart, then
/// runs where each spot is back within `osc_radius` of the one two before.
fn loops(samples: &[(f64, Pos)], c: &Config) -> Vec<Loop> {
    let mut spots: Vec<(f64, Pos)> = Vec::new();
    for &(t, p) in samples {
        if spots.last().is_none_or(|&(_, q)| dist(p, q) > c.osc_radius) {
            spots.push((t, p));
        }
    }
    let mut out = Vec::new();
    let mut k = 2;
    while k < spots.len() {
        if dist(spots[k].1, spots[k - 2].1) <= c.osc_radius {
            let start = k - 2;
            let mut end = k;
            while end + 1 < spots.len() && dist(spots[end + 1].1, spots[end - 1].1) <= c.osc_radius
            {
                end += 1;
            }
            let returns = end - start - 1;
            if returns >= c.osc_min_returns {
                out.push(Loop {
                    start: spots[start].0,
                    end: spots[end].0,
                    a: spots[start].1,
                    b: spots[start + 1].1,
                    returns,
                });
            }
            k = end + 1;
        } else {
            k += 1;
        }
    }
    out
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n).collect();
        out.push_str("...");
        out
    }
}

fn pos(p: Pos) -> String {
    format!("[{:.1}, {:.1}, {:.1}]", p[0], p[1], p[2])
}

fn opt_t(t: Option<f64>) -> String {
    t.map_or_else(|| "end".into(), |t| format!("{t:.1}"))
}

/// Text summary: a few lines a thread report can quote.
pub fn to_text(s: &Summary, name: &str) -> String {
    let mut o = String::new();
    let e = &s.end;
    let _ = writeln!(
        o,
        "run {name}: {} lines, last play time {:.1} s; end: {}{}{}{}",
        s.lines,
        s.last_time,
        e.verdict,
        e.played
            .map(|p| format!(", {p:.1} s played, {} hops", e.hops.unwrap_or(0)))
            .unwrap_or_default(),
        if e.time_limit {
            ", time limit reached"
        } else if e.not_completed {
            ", contract not completed"
        } else {
            ""
        },
        if e.screenshot_written {
            ", screenshot written"
        } else {
            ""
        }
    );
    if let Some(m) = &e.message {
        let _ = writeln!(o, "  fatal: {m}");
    }
    let events = |v: &[Event]| {
        v.iter()
            .map(|e| format!("{:.1} {}", e.t, e.what))
            .collect::<Vec<_>>()
            .join(" | ")
    };
    let _ = writeln!(o, "contracts on: {}", or_none(events(&s.contract_states)));
    let _ = writeln!(o, "accepted: {}", or_none(events(&s.accepted)));
    let _ = writeln!(o, "completed: {}", or_none(events(&s.completed)));
    if let Some(l) = s.loads.first() {
        let _ = writeln!(o, "loaded: {}", l.what);
    }
    match (s.saves.first(), s.saves.last()) {
        (Some(a), Some(b)) if s.saves.len() > 1 => {
            let _ = writeln!(
                o,
                "saves: {} states; first ~{:.1} {}; last ~{:.1} {}",
                s.saves.len(),
                a.t,
                a.what,
                b.t,
                b.what
            );
        }
        (Some(a), _) => {
            let _ = writeln!(o, "saves: 1 state ~{:.1} {}", a.t, a.what);
        }
        _ => {}
    }
    let shown: Vec<String> = if s.objectives.len() > 16 {
        let mut v: Vec<String> = s.objectives[..12]
            .iter()
            .map(|e| format!("{:.1} {}", e.t, e.what))
            .collect();
        v.push(format!("... {} more ...", s.objectives.len() - 15));
        v.extend(
            s.objectives[s.objectives.len() - 3..]
                .iter()
                .map(|e| format!("{:.1} {}", e.t, e.what)),
        );
        v
    } else {
        s.objectives
            .iter()
            .map(|e| format!("{:.1} {}", e.t, e.what))
            .collect()
    };
    let _ = writeln!(
        o,
        "objectives ({} changes): {}",
        s.objectives.len(),
        or_none(shown.join(" | "))
    );
    let _ = writeln!(
        o,
        "objective lines: {}",
        or_none(
            s.objective_items
                .iter()
                .map(|i| format!(
                    "{} {:.1}-{}{}",
                    i.name,
                    i.first,
                    opt_t(i.gone),
                    if i.appearances > 1 {
                        format!(" (shown {} times)", i.appearances)
                    } else {
                        String::new()
                    }
                ))
                .collect::<Vec<_>>()
                .join(" | ")
        )
    );
    let mut kinds: BTreeMap<&str, (usize, f64, f64)> = BTreeMap::new();
    for m in &s.markers {
        let k = kinds.entry(&m.kind).or_insert((0, m.first, m.last));
        k.0 += 1;
        k.1 = k.1.min(m.first);
        k.2 = k.2.max(m.last);
    }
    let _ = writeln!(
        o,
        "markers: {} distinct{}",
        s.markers.len(),
        kinds
            .iter()
            .map(|(k, (n, a, b))| format!("; {k} {n} (~{a:.1}..~{b:.1})"))
            .collect::<String>()
    );
    let windows = |v: &[Window]| {
        v.iter()
            .map(|w| {
                format!(
                    "{:.1}-{} {} ({} action lines)",
                    w.start,
                    opt_t(w.end),
                    w.label,
                    w.action_lines
                )
            })
            .collect::<Vec<_>>()
            .join(" | ")
    };
    let _ = writeln!(o, "locks: {}", or_none(windows(&s.locks)));
    let _ = writeln!(o, "hijacks (~): {}", or_none(windows(&s.hijacks)));
    let _ = writeln!(
        o,
        "hops: {}{}",
        s.hops.len(),
        if s.hops.is_empty() {
            String::new()
        } else {
            format!(
                " at {}",
                s.hops
                    .iter()
                    .take(20)
                    .map(|h| format!("{:.1}", h.t))
                    .collect::<Vec<_>>()
                    .join(", ")
                    + if s.hops.len() > 20 { ", ..." } else { "" }
            )
        }
    );
    if let (Some(a), Some(b)) = (s.idle.first(), s.idle.last()) {
        let _ = writeln!(o, "nothing to do: {} lines, {a:.1}..{b:.1}", s.idle.len());
    }
    let c = &s.config;
    match &s.longest_still {
        Some(w) => {
            let _ = writeln!(
                o,
                "longest still ({} positions; radius {} m): {:.1}-{:.1} ({:.1} s) at {}; {} stretch(es) >= {} s",
                s.positions,
                c.still_radius,
                w.start,
                w.end,
                w.end - w.start,
                pos(w.at),
                s.stalls.len(),
                c.stall_seconds
            );
        }
        None => {
            let _ = writeln!(o, "longest still: no positions logged");
        }
    }
    let _ = writeln!(
        o,
        "A-B-A loops (radius {} m, >= {} returns): {}",
        c.osc_radius,
        c.osc_min_returns,
        or_none(
            s.loops
                .iter()
                .map(|l| format!(
                    "{:.1}-{:.1} {}<->{} x{}",
                    l.start,
                    l.end,
                    pos(l.a),
                    pos(l.b),
                    l.returns
                ))
                .collect::<Vec<_>>()
                .join(" | ")
        )
    );
    let _ = writeln!(
        o,
        "lua errors: {}",
        or_none(
            s.lua_errors
                .iter()
                .map(|e| format!("x{} from ~{:.1}: {}", e.count, e.first, e.message))
                .collect::<Vec<_>>()
                .join(" | ")
        )
    );
    let range = |r: &Option<Range>, unit: &str| {
        r.map_or_else(
            || "not logged".to_string(),
            |r| format!("{:.0}..{:.0} {unit} ({} lines)", r.min, r.max, r.count),
        )
    };
    let _ = writeln!(o, "lua memory: {}", range(&s.lua_memory_kb, "KB"));
    let _ = writeln!(
        o,
        "pace: {}; slowest frame {}",
        range(&s.fps, "fps"),
        range(&s.slowest_frame_ms, "ms")
    );
    o
}

fn or_none(s: String) -> String {
    if s.is_empty() { "none".into() } else { s }
}

/// JSON string escaping.
fn js(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for ch in s.chars() {
        match ch {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn jn(v: f64) -> String {
    if v.is_finite() {
        let r = (v * 1000.0).round() / 1000.0;
        format!("{r}")
    } else {
        "null".into()
    }
}

fn jopt(v: Option<f64>) -> String {
    v.map_or_else(|| "null".into(), jn)
}

fn jpos(p: Pos) -> String {
    format!("[{},{},{}]", jn(p[0]), jn(p[1]), jn(p[2]))
}

fn jarr<T>(v: &[T], f: impl Fn(&T) -> String) -> String {
    format!("[{}]", v.iter().map(f).collect::<Vec<_>>().join(","))
}

fn jevents(v: &[Event]) -> String {
    jarr(v, |e| {
        format!("{{\"t\":{},\"what\":{}}}", jn(e.t), js(&e.what))
    })
}

fn jwindows(v: &[Window]) -> String {
    jarr(v, |w| {
        format!(
            "{{\"start\":{},\"end\":{},\"label\":{},\"action_lines\":{}}}",
            jn(w.start),
            jopt(w.end),
            js(&w.label),
            w.action_lines
        )
    })
}

fn jstill(w: &Still) -> String {
    format!(
        "{{\"start\":{},\"end\":{},\"seconds\":{},\"at\":{},\"samples\":{}}}",
        jn(w.start),
        jn(w.end),
        jn(w.end - w.start),
        jpos(w.at),
        w.samples
    )
}

fn jrange(r: &Option<Range>) -> String {
    r.map_or_else(
        || "null".into(),
        |r| {
            format!(
                "{{\"min\":{},\"max\":{},\"count\":{}}}",
                jn(r.min),
                jn(r.max),
                r.count
            )
        },
    )
}

/// The summary as one JSON object.
pub fn to_json(s: &Summary, name: &str) -> String {
    let c = &s.config;
    let e = &s.end;
    let fields = [
        ("log", js(name)),
        (
            "config",
            format!(
                "{{\"stall_seconds\":{},\"still_radius\":{},\"osc_radius\":{},\"osc_min_returns\":{},\"mission_seconds\":{}}}",
                jn(c.stall_seconds),
                jn(c.still_radius),
                jn(c.osc_radius),
                c.osc_min_returns,
                jopt(c.mission_seconds)
            ),
        ),
        ("lines", s.lines.to_string()),
        ("last_time", jn(s.last_time)),
        (
            "end",
            format!(
                "{{\"verdict\":{},\"played\":{},\"hops\":{},\"not_completed\":{},\"time_limit\":{},\"screenshot_written\":{},\"message\":{}}}",
                js(&e.verdict),
                jopt(e.played),
                e.hops.map_or_else(|| "null".into(), |h| h.to_string()),
                e.not_completed,
                e.time_limit,
                e.screenshot_written,
                e.message.as_deref().map_or_else(|| "null".into(), js)
            ),
        ),
        ("contract_states", jevents(&s.contract_states)),
        ("accepted", jevents(&s.accepted)),
        ("completed", jevents(&s.completed)),
        ("saves", jevents(&s.saves)),
        ("loads", jevents(&s.loads)),
        ("objectives", jevents(&s.objectives)),
        (
            "objective_items",
            jarr(&s.objective_items, |i| {
                format!(
                    "{{\"name\":{},\"first\":{},\"gone\":{},\"appearances\":{}}}",
                    js(&i.name),
                    jn(i.first),
                    jopt(i.gone),
                    i.appearances
                )
            }),
        ),
        (
            "markers",
            jarr(&s.markers, |m| {
                format!(
                    "{{\"kind\":{},\"name\":{},\"id\":{},\"first\":{},\"last\":{},\"lines\":{}}}",
                    js(&m.kind),
                    js(&m.name),
                    js(&m.id),
                    jn(m.first),
                    jn(m.last),
                    m.lines
                )
            }),
        ),
        ("locks", jwindows(&s.locks)),
        ("hijacks", jwindows(&s.hijacks)),
        ("hops", jevents(&s.hops)),
        ("idle", jarr(&s.idle, |t| jn(*t))),
        ("positions", s.positions.to_string()),
        (
            "longest_still",
            s.longest_still
                .as_ref()
                .map_or_else(|| "null".into(), jstill),
        ),
        ("stalls", jarr(&s.stalls, jstill)),
        (
            "loops",
            jarr(&s.loops, |l| {
                format!(
                    "{{\"start\":{},\"end\":{},\"a\":{},\"b\":{},\"returns\":{}}}",
                    jn(l.start),
                    jn(l.end),
                    jpos(l.a),
                    jpos(l.b),
                    l.returns
                )
            }),
        ),
        (
            "lua_errors",
            jarr(&s.lua_errors, |e| {
                format!(
                    "{{\"message\":{},\"first\":{},\"count\":{}}}",
                    js(&e.message),
                    jn(e.first),
                    e.count
                )
            }),
        ),
        ("lua_memory_kb", jrange(&s.lua_memory_kb)),
        ("fps", jrange(&s.fps)),
        ("slowest_frame_ms", jrange(&s.slowest_frame_ms)),
    ];
    let body = fields
        .iter()
        .map(|(k, v)| format!("{}:{v}", js(k)))
        .collect::<Vec<_>>()
        .join(",\n  ");
    format!("{{\n  {body}\n}}\n")
}

#[cfg(test)]
mod tests;
