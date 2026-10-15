//! ActionScript 1 and 2 (AVM1): the scripts a movie's frames run, and the
//! functions the game calls on it.
//!
//! [`Runtime`] plays a [`Player`] with its scripts: the movie's init
//! scripts first, then each frame's as the playhead lands on it, each
//! instance's `onEnterFrame` every frame, intervals (`setInterval`) as
//! they fall due, and functions the host calls by name ([`Runtime::call`]).
//! What the scripts ask of the host (`fscommand`, `getURL("FSCommand:..")`)
//! queues up ([`Runtime::take_commands`]).
//!
//! The interpreter runs the action bytes as the Flash player does: a stack
//! of values; variables looked up through a scope chain (a function's
//! locals, the timeline it was defined on, the globals); objects with
//! prototypes; functions (`DefineFunction`, `DefineFunction2` with its
//! registers) closing over their scope; movie clips as objects whose
//! properties (`_x`, `_alpha`, `_currentframe`...) are their instances' and
//! whose members include their children by name. A library of the built-in
//! objects the game's movies use comes with it: `MovieClip`'s methods
//! (`gotoAndStop`, `attachMovie`, `createEmptyMovieClip`, the drawing API),
//! `Array`, `String`, `Math`, `Object`, `Color`, `Key`, `Stage`,
//! `setInterval`. What is not there reads as `undefined` and does nothing.
//!
//! # Provenance
//!
//! Clean-room from the public SWF file format specification (version 10,
//! its ActionScript action records) and the public ActionScript 2.0
//! language reference for the built-in objects' behaviour. Tested against
//! hand-assembled action bytes; nothing here was read from the game's
//! executable or its movies' scripts.

#![allow(
    clippy::wrong_self_convention,
    clippy::needless_return,
    clippy::collapsible_if,
    clippy::collapsible_match,
    clippy::needless_lifetimes
)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use super::player::{ClipId, Kind, Player};
use super::{ColourTransform, Matrix, Triangle, fill_triangles, push_clipped};

/// A value.
#[derive(Clone, Default)]
pub enum Value {
    #[default]
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    Str(Rc<str>),
    Object(Obj),
}

pub type Obj = Rc<RefCell<Object>>;

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Undefined => write!(f, "undefined"),
            Value::Null => write!(f, "null"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Number(n) => write!(f, "{n}"),
            Value::Str(s) => write!(f, "{s:?}"),
            Value::Object(o) => write!(f, "[object {:?}]", o.borrow().kind),
        }
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Number(n)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Str(s.into())
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

/// A native function: the runtime, `this`, the arguments.
pub type Native = fn(&mut Avm, &mut Player, &Value, &[Value]) -> Value;

/// What an object is besides its properties.
#[derive(Clone)]
pub enum ObjKind {
    Plain,
    Array(Vec<Value>),
    Function(Rc<Function>),
    Native(Native),
    /// A movie clip (or text field): an instance on stage.
    Clip(ClipId),
}

impl fmt::Debug for ObjKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ObjKind::Plain => write!(f, "Object"),
            ObjKind::Array(a) => write!(f, "Array({})", a.len()),
            ObjKind::Function(func) => write!(f, "Function {}", func.name),
            ObjKind::Native(_) => write!(f, "Function (native)"),
            ObjKind::Clip(id) => write!(f, "MovieClip #{id}"),
        }
    }
}

/// An object: properties in the order set, a prototype, and its kind.
#[derive(Clone, Debug)]
pub struct Object {
    keys: Vec<Rc<str>>,
    values: HashMap<Rc<str>, Value>,
    pub proto: Option<Obj>,
    pub kind: ObjKind,
}

impl Object {
    pub fn new(kind: ObjKind, proto: Option<Obj>) -> Obj {
        Rc::new(RefCell::new(Object {
            keys: Vec::new(),
            values: HashMap::new(),
            proto,
            kind,
        }))
    }

    /// Its own property (any case, as SWF 6 and earlier find names).
    pub fn own(&self, name: &str) -> Option<Value> {
        if let Some(v) = self.values.get(name) {
            return Some(v.clone());
        }
        self.keys
            .iter()
            .find(|k| k.eq_ignore_ascii_case(name))
            .and_then(|k| self.values.get(k).cloned())
    }

    fn key(&self, name: &str) -> Option<Rc<str>> {
        if let Some((k, _)) = self.values.get_key_value(name) {
            return Some(Rc::clone(k));
        }
        self.keys
            .iter()
            .find(|k| k.eq_ignore_ascii_case(name))
            .cloned()
    }

    pub fn set(&mut self, name: &str, value: Value) {
        match self.key(name) {
            Some(k) => {
                self.values.insert(k, value);
            }
            None => {
                let k: Rc<str> = name.into();
                self.keys.push(Rc::clone(&k));
                self.values.insert(k, value);
            }
        }
    }

    pub fn delete(&mut self, name: &str) -> bool {
        let Some(k) = self.key(name) else {
            return false;
        };
        self.keys.retain(|x| *x != k);
        self.values.remove(&k);
        true
    }

    pub fn keys(&self) -> Vec<Rc<str>> {
        self.keys.clone()
    }
}

/// A function defined by a script.
#[derive(Debug)]
pub struct Function {
    pub name: String,
    /// Parameters: the register each goes in (0 for a local), and name.
    pub params: Vec<(u8, String)>,
    pub code: Rc<[u8]>,
    pub start: usize,
    pub end: usize,
    pub scope: Rc<Scope>,
    /// The timeline it was defined on.
    pub target: ClipId,
    pub pool: Rc<Vec<Rc<str>>>,
    /// `DefineFunction2`: registers, and what to preload (its flags).
    pub v2: Option<(u8, u16)>,
}

/// A scope chain, innermost first.
#[derive(Debug)]
pub struct Scope {
    pub obj: Obj,
    pub parent: Option<Rc<Scope>>,
}

/// A call or script under way.
struct Frame {
    scope: Rc<Scope>,
    this: Value,
    target: ClipId,
    regs: Vec<Value>,
    pool: Rc<Vec<Rc<str>>>,
    /// The function's own locals (`arguments`, parameters, `var`s).
    locals: Option<Obj>,
}

/// An interval (`setInterval`): when next, every how long, and what.
struct Interval {
    id: i32,
    due: f64,
    every: f64,
    call: (Value, Option<String>),
    args: Vec<Value>,
}

/// Instructions a script may run at most before it is stopped.
const BUDGET: u64 = 2_000_000;
/// Calls nested at most.
const MAX_DEPTH: usize = 64;

/// The interpreter's state: globals, prototypes, clip objects.
pub struct Avm {
    pub global: Obj,
    pub object_proto: Obj,
    pub function_proto: Obj,
    pub array_proto: Obj,
    pub string_proto: Obj,
    pub clip_proto: Obj,
    clips: HashMap<ClipId, Obj>,
    pub commands: Vec<(String, String)>,
    /// Movies asked for (`loadMovie`): the file, the level or clip path, and
    /// the timeline that asked.
    pub loads: Vec<(String, String, ClipId)>,
    pub log: Vec<String>,
    intervals: Vec<Interval>,
    next_interval: i32,
    /// Seconds played.
    pub time: f64,
    budget: u64,
    depth: usize,
    /// The drawing API's state per clip: the path, fill, line.
    pens: HashMap<ClipId, Pen>,
    /// The stage's size, for `Stage.width` and `Stage.height`.
    pub stage: [f32; 2],
    /// Logs each function call, goto, `_visible` change and undefined
    /// variable or member read, for finding what a movie does.
    pub trace: bool,
    /// `Key.addListener`'s objects, told of key presses
    /// ([`Runtime::key_down`]).
    pub key_listeners: Vec<Value>,
    /// The key last pressed (`Key.getCode`) and those held (`Key.isDown`).
    pub key_code: i32,
    pub keys_down: Vec<i32>,
}

#[derive(Default)]
struct Pen {
    at: [f32; 2],
    fill: Option<[f32; 4]>,
    line: Option<(f32, [f32; 4])>,
    path: Vec<([f32; 2], [f32; 2])>,
}

impl fmt::Debug for Avm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Avm")
    }
}

fn native(avm: &Avm, f: Native) -> Value {
    Value::Object(Object::new(
        ObjKind::Native(f),
        Some(Rc::clone(&avm.function_proto)),
    ))
}

impl Avm {
    pub fn new() -> Self {
        let object_proto = Object::new(ObjKind::Plain, None);
        let function_proto = Object::new(ObjKind::Plain, Some(Rc::clone(&object_proto)));
        let array_proto = Object::new(ObjKind::Plain, Some(Rc::clone(&object_proto)));
        let string_proto = Object::new(ObjKind::Plain, Some(Rc::clone(&object_proto)));
        let clip_proto = Object::new(ObjKind::Plain, Some(Rc::clone(&object_proto)));
        let global = Object::new(ObjKind::Plain, Some(Rc::clone(&object_proto)));
        let mut avm = Avm {
            global,
            object_proto,
            function_proto,
            array_proto,
            string_proto,
            clip_proto,
            clips: HashMap::new(),
            commands: Vec::new(),
            loads: Vec::new(),
            log: Vec::new(),
            intervals: Vec::new(),
            next_interval: 1,
            time: 0.0,
            budget: 0,
            depth: 0,
            pens: HashMap::new(),
            stage: [0.0; 2],
            trace: false,
            key_listeners: Vec::new(),
            key_code: 0,
            keys_down: Vec::new(),
        };
        library::install(&mut avm);
        avm
    }

    /// A new plain object.
    pub fn object(&self) -> Obj {
        Object::new(ObjKind::Plain, Some(Rc::clone(&self.object_proto)))
    }

    /// A new array of `items`.
    pub fn array(&self, items: Vec<Value>) -> Value {
        Value::Object(Object::new(
            ObjKind::Array(items),
            Some(Rc::clone(&self.array_proto)),
        ))
    }

    /// The object standing for instance `id`.
    pub fn clip_object(&mut self, id: ClipId) -> Obj {
        Rc::clone(
            self.clips.entry(id).or_insert_with(|| {
                Object::new(ObjKind::Clip(id), Some(Rc::clone(&self.clip_proto)))
            }),
        )
    }

    fn clip_value(&mut self, player: &Player, id: Option<ClipId>) -> Value {
        match id.filter(|&i| player.clip(i).is_some()) {
            Some(i) => Value::Object(self.clip_object(i)),
            None => Value::Undefined,
        }
    }

    // ---- Conversions --------------------------------------------------

    pub fn to_number(&mut self, player: &mut Player, v: &Value) -> f64 {
        match v {
            Value::Undefined | Value::Null => f64::NAN,
            Value::Bool(b) => f64::from(u8::from(*b)),
            Value::Number(n) => *n,
            Value::Str(s) => str_number(s),
            Value::Object(_) => {
                let p = self.to_primitive(player, v);
                match p {
                    Value::Object(_) => f64::NAN,
                    other => self.to_number(player, &other),
                }
            }
        }
    }

    pub fn to_str(&mut self, player: &mut Player, v: &Value) -> Rc<str> {
        match v {
            Value::Undefined => "undefined".into(),
            Value::Null => "null".into(),
            Value::Bool(b) => if *b { "true" } else { "false" }.into(),
            Value::Number(n) => number_str(*n).into(),
            Value::Str(s) => Rc::clone(s),
            Value::Object(o) => {
                let kind = o.borrow().kind.clone();
                match kind {
                    ObjKind::Array(items) => {
                        let parts: Vec<String> = items
                            .iter()
                            .map(|i| self.to_str(player, i).to_string())
                            .collect();
                        parts.join(",").into()
                    }
                    ObjKind::Clip(id) => target_path(player, id).into(),
                    ObjKind::Function(_) | ObjKind::Native(_) => "[type Function]".into(),
                    ObjKind::Plain => {
                        // A script's own toString answers for the object.
                        let to_string = self.get(player, v, "toString");
                        if matches!(to_string, Value::Object(ref f) if matches!(f.borrow().kind, ObjKind::Function(_)))
                        {
                            let r = self.call_value(player, &to_string, v, &[]);
                            if let Value::Str(s) = r {
                                return s;
                            }
                        }
                        "[object Object]".into()
                    }
                }
            }
        }
    }

    fn to_primitive(&mut self, player: &mut Player, v: &Value) -> Value {
        let Value::Object(o) = v else {
            return v.clone();
        };
        let kind = o.borrow().kind.clone();
        match kind {
            ObjKind::Plain => {
                let value_of = self.get(player, v, "valueOf");
                if let Value::Object(_) = value_of {
                    let r = self.call_value(player, &value_of, v, &[]);
                    if !matches!(r, Value::Object(_)) {
                        return r;
                    }
                }
                Value::Str(self.to_str(player, v))
            }
            _ => Value::Str(self.to_str(player, v)),
        }
    }

    pub fn truthy(&self, v: &Value) -> bool {
        match v {
            Value::Undefined | Value::Null => false,
            Value::Bool(b) => *b,
            Value::Number(n) => *n != 0.0 && !n.is_nan(),
            Value::Str(s) => !s.is_empty(),
            Value::Object(_) => true,
        }
    }

    fn equals(&mut self, player: &mut Player, a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Undefined | Value::Null, Value::Undefined | Value::Null) => true,
            (Value::Undefined | Value::Null, _) | (_, Value::Undefined | Value::Null) => false,
            (Value::Number(x), Value::Number(y)) => x == y,
            (Value::Str(x), Value::Str(y)) => x == y,
            (Value::Bool(x), Value::Bool(y)) => x == y,
            (Value::Object(x), Value::Object(y)) => Rc::ptr_eq(x, y),
            (Value::Object(_), _) | (_, Value::Object(_)) => {
                let (pa, pb) = (self.to_primitive(player, a), self.to_primitive(player, b));
                if matches!(pa, Value::Object(_)) || matches!(pb, Value::Object(_)) {
                    return false;
                }
                self.equals(player, &pa, &pb)
            }
            _ => {
                let (x, y) = (self.to_number(player, a), self.to_number(player, b));
                x == y
            }
        }
    }

    fn strict_equals(&self, a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Undefined, Value::Undefined) | (Value::Null, Value::Null) => true,
            (Value::Number(x), Value::Number(y)) => x == y,
            (Value::Str(x), Value::Str(y)) => x == y,
            (Value::Bool(x), Value::Bool(y)) => x == y,
            (Value::Object(x), Value::Object(y)) => Rc::ptr_eq(x, y),
            _ => false,
        }
    }

    fn less(&mut self, player: &mut Player, a: &Value, b: &Value) -> Value {
        let (pa, pb) = (self.to_primitive(player, a), self.to_primitive(player, b));
        if let (Value::Str(x), Value::Str(y)) = (&pa, &pb) {
            return Value::Bool(x < y);
        }
        let (x, y) = (self.to_number(player, &pa), self.to_number(player, &pb));
        if x.is_nan() || y.is_nan() {
            Value::Undefined
        } else {
            Value::Bool(x < y)
        }
    }

    // ---- Members --------------------------------------------------------

    /// `obj[name]`: a clip's properties and children, an array's items and
    /// length, a string's length, else up the prototypes.
    pub fn get(&mut self, player: &mut Player, obj: &Value, name: &str) -> Value {
        match obj {
            Value::Str(s) => {
                if name == "length" {
                    return Value::Number(s.chars().count() as f64);
                }
                let proto = Rc::clone(&self.string_proto);
                return lookup(&proto, name).unwrap_or_default();
            }
            Value::Number(_) | Value::Bool(_) => {
                return lookup(&self.object_proto.clone(), name).unwrap_or_default();
            }
            Value::Object(o) => {
                let kind = o.borrow().kind.clone();
                match kind {
                    ObjKind::Array(items) => {
                        if name == "length" {
                            return Value::Number(items.len() as f64);
                        }
                        if let Ok(i) = name.parse::<usize>() {
                            return items.get(i).cloned().unwrap_or_default();
                        }
                    }
                    ObjKind::Clip(id) => {
                        if let Some(v) = self.clip_property(player, id, name) {
                            return v;
                        }
                        if let Some(v) = o.borrow().own(name) {
                            return v;
                        }
                        if let Some(child) = player.child(id, name) {
                            return Value::Object(self.clip_object(child));
                        }
                    }
                    _ => {}
                }
                lookup(o, name).unwrap_or_default()
            }
            _ => Value::Undefined,
        }
    }

    /// `obj[name] = value`.
    pub fn set(&mut self, player: &mut Player, obj: &Value, name: &str, value: Value) {
        let Value::Object(o) = obj else { return };
        let kind = o.borrow().kind.clone();
        match kind {
            ObjKind::Array(_) => {
                if name == "length" {
                    let n = self.to_number(player, &value).max(0.0) as usize;
                    if let ObjKind::Array(items) = &mut o.borrow_mut().kind {
                        items.resize(n, Value::Undefined);
                    }
                    return;
                }
                if let Ok(i) = name.parse::<usize>() {
                    if let ObjKind::Array(items) = &mut o.borrow_mut().kind {
                        if i >= items.len() {
                            items.resize(i + 1, Value::Undefined);
                        }
                        items[i] = value;
                    }
                    return;
                }
            }
            ObjKind::Clip(id) => {
                if self.set_clip_property(player, id, name, &value) {
                    return;
                }
            }
            _ => {}
        }
        o.borrow_mut().set(name, value);
    }

    fn clip_property(&mut self, player: &mut Player, id: ClipId, name: &str) -> Option<Value> {
        let c = player.clip(id)?;
        let m = c.matrix;
        let corner = text_corner(c);
        let lower = name.to_ascii_lowercase();
        Some(match lower.as_str() {
            "_x" => Value::Number(f64::from(m.tx + corner[0])),
            "_y" => Value::Number(f64::from(m.ty + corner[1])),
            "_xscale" => Value::Number(f64::from(m.a.hypot(m.b) * 100.0)),
            "_yscale" => Value::Number(f64::from(m.c.hypot(m.d) * 100.0)),
            "_rotation" => Value::Number(f64::from(m.b.atan2(m.a).to_degrees())),
            "_alpha" => Value::Number(f64::from(c.colour.mult[3] * 100.0)),
            "_visible" => Value::Bool(c.visible),
            "_currentframe" => Value::Number((c.frame + 1) as f64),
            "_totalframes" | "_framesloaded" => Value::Number(c.frame_count() as f64),
            "_name" => Value::Str(c.name.as_str().into()),
            "_target" => Value::Str(target_path(player, id).into()),
            "_parent" => {
                let p = c.parent;
                return Some(self.clip_value(player, p));
            }
            "_root" | "_level0" => {
                let r = if lower == "_root" {
                    player.root_of(id)
                } else {
                    player.levels.get(&0).copied().unwrap_or(player.root)
                };
                return Some(Value::Object(self.clip_object(r)));
            }
            "_xmouse" | "_ymouse" => Value::Number(0.0),
            "_width" | "_height" => {
                let b = clip_bounds(player, id);
                let v = b.map_or(0.0, |b| {
                    if lower == "_width" {
                        b[2] - b[0]
                    } else {
                        b[3] - b[1]
                    }
                });
                Value::Number(f64::from(v))
            }
            "text" | "htmltext" => match &c.kind {
                Kind::Text(_, s) => Value::Str(s.as_str().into()),
                _ => return None,
            },
            "textwidth" | "textheight" => match &c.kind {
                Kind::Text(def, s) => {
                    // With the fonts, as laid out; else estimated.
                    let (w, h) = match c.lib.texts.get(def) {
                        Some(t) => {
                            let font = player.fonts.as_deref().and_then(|book| {
                                t.font.and_then(|f| Player::font_for(&c.lib, f, book))
                            });
                            match font {
                                Some(font) => super::text::measure(t, s, font),
                                None => t.text_size(s),
                            }
                        }
                        None => (0.0, 12.0 * 1.2),
                    };
                    Value::Number(f64::from(if lower == "textwidth" { w } else { h }))
                }
                _ => return None,
            },
            _ => return None,
        })
    }

    fn set_clip_property(
        &mut self,
        player: &mut Player,
        id: ClipId,
        name: &str,
        value: &Value,
    ) -> bool {
        let lower = name.to_ascii_lowercase();
        let number = |avm: &mut Avm, player: &mut Player| avm.to_number(player, value) as f32;
        match lower.as_str() {
            "_x" | "_y" | "_xscale" | "_yscale" | "_rotation" | "_alpha" | "_width" | "_height" => {
                let n = number(self, player);
                if n.is_nan() {
                    return true;
                }
                let bounds = if lower == "_width" || lower == "_height" {
                    clip_bounds(player, id)
                } else {
                    None
                };
                let corner = player.clip(id).map_or([0.0; 2], text_corner);
                let Some(c) = player.clip_mut(id) else {
                    return true;
                };
                let m = &mut c.matrix;
                match lower.as_str() {
                    "_x" => m.tx = n - corner[0],
                    "_y" => m.ty = n - corner[1],
                    "_xscale" | "_yscale" | "_rotation" => {
                        let (sx, sy, r) = (m.a.hypot(m.b), m.c.hypot(m.d), m.b.atan2(m.a));
                        let (sx, sy, r) = match lower.as_str() {
                            "_xscale" => (n / 100.0, sy, r),
                            "_yscale" => (sx, n / 100.0, r),
                            _ => (sx, sy, n.to_radians()),
                        };
                        let (s, co) = r.sin_cos();
                        m.a = sx * co;
                        m.b = sx * s;
                        m.c = -sy * s;
                        m.d = sy * co;
                    }
                    "_alpha" => c.colour.mult[3] = n / 100.0,
                    "_width" => {
                        if let Some(b) = bounds.filter(|b| b[2] > b[0]) {
                            let k = n / (b[2] - b[0]);
                            m.a *= k;
                            m.b *= k;
                        }
                    }
                    _ => {
                        if let Some(b) = bounds.filter(|b| b[3] > b[1]) {
                            let k = n / (b[3] - b[1]);
                            m.c *= k;
                            m.d *= k;
                        }
                    }
                }
                true
            }
            "_visible" => {
                let v = self.truthy(value);
                if self.trace {
                    let name = player.clip(id).map_or("", |c| c.name.as_str());
                    self.log.push(format!("{name}._visible = {v}"));
                }
                if let Some(c) = player.clip_mut(id) {
                    c.visible = v;
                }
                true
            }
            "_name" => {
                let s = self.to_str(player, value);
                if let Some(c) = player.clip_mut(id) {
                    c.name = s.to_string();
                }
                true
            }
            "text" | "htmltext" => {
                if !matches!(player.clip(id).map(|c| &c.kind), Some(Kind::Text(..))) {
                    return false;
                }
                let s = self.to_str(player, value);
                if let Some(c) = player.clip_mut(id)
                    && let Kind::Text(_, t) = &mut c.kind
                {
                    *t = s.to_string();
                }
                true
            }
            _ => false,
        }
    }

    // ---- Calls ----------------------------------------------------------

    /// Calls `f` with `this` and `args`.
    pub fn call_value(
        &mut self,
        player: &mut Player,
        f: &Value,
        this: &Value,
        args: &[Value],
    ) -> Value {
        let Value::Object(o) = f else {
            return Value::Undefined;
        };
        let kind = o.borrow().kind.clone();
        match kind {
            ObjKind::Native(n) => n(self, player, this, args),
            ObjKind::Function(func) => self.call_function(player, &func, this, args),
            _ => Value::Undefined,
        }
    }

    fn call_function(
        &mut self,
        player: &mut Player,
        func: &Rc<Function>,
        this: &Value,
        args: &[Value],
    ) -> Value {
        if self.depth >= MAX_DEPTH {
            self.log
                .push(format!("{}: calls nested too deep", func.name));
            return Value::Undefined;
        }
        if self.trace {
            self.log.push(format!("call {}{args:?}", func.name));
        }
        let locals = self.object();
        let arguments = self.array(args.to_vec());
        let mut regs = Vec::new();
        match func.v2 {
            Some((count, flags)) => {
                regs = vec![Value::Undefined; usize::from(count).max(1)];
                let mut r = 1usize;
                let mut preload = |bit: u16, v: Value, regs: &mut Vec<Value>| {
                    if flags & bit != 0 {
                        if r < regs.len() {
                            regs[r] = v;
                        }
                        r += 1;
                    }
                };
                let root = Value::Object(self.clip_object(player.root_of(func.target)));
                let parent = player
                    .clip(func.target)
                    .and_then(|c| c.parent)
                    .map_or(Value::Undefined, |p| Value::Object(self.clip_object(p)));
                // The flags as stored: preload this 0x0001 .. global 0x0100,
                // in the order the registers take them.
                preload(0x0001, this.clone(), &mut regs);
                if flags & 0x0004 != 0 {
                    preload(0x0004, arguments.clone(), &mut regs);
                }
                preload(0x0010, Value::Undefined, &mut regs);
                preload(0x0040, root, &mut regs);
                preload(0x0080, parent, &mut regs);
                preload(0x0100, Value::Object(Rc::clone(&self.global)), &mut regs);
                if flags & 0x0008 == 0 {
                    locals.borrow_mut().set("arguments", arguments);
                }
                if flags & 0x0002 == 0 {
                    locals.borrow_mut().set("this", this.clone());
                }
            }
            None => {
                locals.borrow_mut().set("arguments", arguments);
                locals.borrow_mut().set("this", this.clone());
            }
        }
        for (i, (reg, name)) in func.params.iter().enumerate() {
            let v = args.get(i).cloned().unwrap_or_default();
            if *reg != 0 && usize::from(*reg) < regs.len() {
                regs[usize::from(*reg)] = v;
            } else {
                locals.borrow_mut().set(name, v);
            }
        }
        if regs.len() < 4 {
            regs.resize(4, Value::Undefined);
        }
        let mut frame = Frame {
            scope: Rc::new(Scope {
                obj: Rc::clone(&locals),
                parent: Some(Rc::clone(&func.scope)),
            }),
            this: this.clone(),
            target: func.target,
            regs,
            pool: Rc::clone(&func.pool),
            locals: Some(locals),
        };
        self.depth += 1;
        let r = self.execute(player, &func.code, func.start, func.end, &mut frame);
        self.depth -= 1;
        r
    }

    /// Runs a frame script (or init script) on instance `target`.
    pub fn run_script(&mut self, player: &mut Player, target: ClipId, code: &Rc<[u8]>) {
        if player.clip(target).is_none() {
            return;
        }
        let clip = self.clip_object(target);
        let mut frame = Frame {
            scope: Rc::new(Scope {
                obj: Rc::clone(&clip),
                parent: Some(Rc::new(Scope {
                    obj: Rc::clone(&self.global),
                    parent: None,
                })),
            }),
            this: Value::Object(clip),
            target,
            regs: vec![Value::Undefined; 4],
            pool: Rc::new(Vec::new()),
            locals: None,
        };
        self.budget = 0;
        self.execute(player, code, 0, code.len(), &mut frame);
    }

    // ---- Variables ------------------------------------------------------

    fn get_variable(&mut self, player: &mut Player, frame: &Frame, name: &str) -> Value {
        if name.contains(['.', '/', ':']) {
            return self.eval_path(player, frame, name);
        }
        match name.to_ascii_lowercase().as_str() {
            "this" => return frame.this.clone(),
            "_root" => {
                let root = player.root_of(frame.target);
                return Value::Object(self.clip_object(root));
            }
            "_global" => return Value::Object(Rc::clone(&self.global)),
            n if n.starts_with("_level") => {
                let level = n[6..]
                    .parse::<i32>()
                    .ok()
                    .and_then(|l| player.levels.get(&l).copied());
                return self.clip_value(player, level);
            }
            "_parent" => {
                let p = player.clip(frame.target).and_then(|c| c.parent);
                return self.clip_value(player, p);
            }
            _ => {}
        }
        let mut s = Some(Rc::clone(&frame.scope));
        while let Some(scope) = s {
            let kind = scope.obj.borrow().kind.clone();
            if let ObjKind::Clip(id) = kind {
                let v = Value::Object(Rc::clone(&scope.obj));
                if self.clip_property(player, id, name).is_some()
                    || lookup(&scope.obj, name).is_some()
                    || player.child(id, name).is_some()
                {
                    return self.get(player, &v, name);
                }
            } else if let Some(v) = lookup(&scope.obj, name) {
                return v;
            }
            s = scope.parent.clone();
        }
        Value::Undefined
    }

    fn set_variable(&mut self, player: &mut Player, frame: &Frame, name: &str, value: Value) {
        if let Some((path, var)) = split_target(name) {
            let obj = self.eval_path(player, frame, path);
            self.set(player, &obj, var, value);
            return;
        }
        // An existing local or variable on the way out, else the timeline.
        let mut s = Some(Rc::clone(&frame.scope));
        while let Some(scope) = s {
            if Rc::ptr_eq(&scope.obj, &self.global) {
                break;
            }
            let found = scope.obj.borrow().own(name).is_some();
            if found {
                let v = Value::Object(Rc::clone(&scope.obj));
                self.set(player, &v, name, value);
                return;
            }
            s = scope.parent.clone();
        }
        if lookup(&self.global, name).is_some() && self.global.borrow().own(name).is_some() {
            self.global.borrow_mut().set(name, value);
            return;
        }
        let t = Value::Object(self.clip_object(frame.target));
        self.set(player, &t, name, value);
    }

    /// `a.b.c`, `/a/b:c`, `_root.a`, `_parent._parent.b`: from the frame's
    /// timeline.
    fn eval_path(&mut self, player: &mut Player, frame: &Frame, path: &str) -> Value {
        let (path, var) = match path.rfind(':') {
            Some(i) => (&path[..i], Some(&path[i + 1..])),
            None => (path, None),
        };
        let mut at = if path.starts_with('/') {
            Value::Object(self.clip_object(player.root_of(frame.target)))
        } else {
            Value::Object(self.clip_object(frame.target))
        };
        let mut first = true;
        for part in path.split(['/', '.']).filter(|p| !p.is_empty()) {
            at = match part {
                ".." => {
                    let id = clip_id(&at);
                    let p = id.and_then(|i| player.clip(i)).and_then(|c| c.parent);
                    self.clip_value(player, p)
                }
                _ if first => self.get_variable(player, frame, part),
                _ => self.get(player, &at, part),
            };
            first = false;
            if matches!(at, Value::Undefined) {
                return Value::Undefined;
            }
        }
        match var {
            Some(v) => self.get(player, &at, v),
            None => at,
        }
    }

    fn target_of(&mut self, player: &mut Player, frame: &Frame, path: &str) -> Option<ClipId> {
        if path.is_empty() {
            return Some(frame.target);
        }
        clip_id(&self.eval_path(player, frame, path))
    }

    // ---- The interpreter ------------------------------------------------

    #[allow(clippy::too_many_lines)]
    fn execute(
        &mut self,
        player: &mut Player,
        code: &Rc<[u8]>,
        start: usize,
        end: usize,
        frame: &mut Frame,
    ) -> Value {
        let mut stack: Vec<Value> = Vec::new();
        let mut pc = start;
        let end = end.min(code.len());
        // `with` scopes: where each ends.
        let mut withs: Vec<usize> = Vec::new();
        let pop = |stack: &mut Vec<Value>| stack.pop().unwrap_or_default();
        while pc < end {
            while withs.last().is_some_and(|&e| pc >= e) {
                withs.pop();
                if let Some(p) = frame.scope.parent.clone() {
                    frame.scope = p;
                }
            }
            self.budget += 1;
            if self.budget > BUDGET {
                self.log.push("script stopped: too long".into());
                return Value::Undefined;
            }
            let op = code[pc];
            pc += 1;
            let (data, next) = if op >= 0x80 {
                let len = code
                    .get(pc..pc + 2)
                    .map_or(0, |b| usize::from(u16::from_le_bytes([b[0], b[1]])));
                let d = pc + 2;
                (&code[d.min(end)..(d + len).min(end)], d + len)
            } else {
                (&code[0..0], pc)
            };
            pc = next;
            match op {
                0x00 => {}
                0x04 => {
                    if let Some(c) = player.clip(frame.target) {
                        let f = c.frame + 1;
                        player.goto(frame.target, f);
                    }
                    if let Some(c) = player.clip_mut(frame.target) {
                        c.playing = false;
                    }
                }
                0x05 => {
                    if let Some(c) = player.clip(frame.target) {
                        let f = c.frame.saturating_sub(1);
                        player.goto(frame.target, f);
                    }
                    if let Some(c) = player.clip_mut(frame.target) {
                        c.playing = false;
                    }
                }
                0x06 => {
                    if let Some(c) = player.clip_mut(frame.target) {
                        c.playing = true;
                    }
                }
                0x07 => {
                    if let Some(c) = player.clip_mut(frame.target) {
                        c.playing = false;
                    }
                }
                0x08 | 0x09 => {}
                0x0A | 0x47 => {
                    let (b, a) = (pop(&mut stack), pop(&mut stack));
                    let v = if op == 0x0A {
                        Value::Number(self.to_number(player, &a) + self.to_number(player, &b))
                    } else {
                        let (pa, pb) =
                            (self.to_primitive(player, &a), self.to_primitive(player, &b));
                        if matches!(pa, Value::Str(_)) || matches!(pb, Value::Str(_)) {
                            let s =
                                format!("{}{}", self.to_str(player, &pa), self.to_str(player, &pb));
                            Value::Str(s.into())
                        } else {
                            Value::Number(self.to_number(player, &pa) + self.to_number(player, &pb))
                        }
                    };
                    stack.push(v);
                }
                0x0B | 0x0C | 0x0D | 0x3F => {
                    let (b, a) = (pop(&mut stack), pop(&mut stack));
                    let (x, y) = (self.to_number(player, &a), self.to_number(player, &b));
                    stack.push(Value::Number(match op {
                        0x0B => x - y,
                        0x0C => x * y,
                        0x0D => x / y,
                        _ => x % y,
                    }));
                }
                0x0E => {
                    let (b, a) = (pop(&mut stack), pop(&mut stack));
                    let v = self.to_number(player, &a) == self.to_number(player, &b);
                    stack.push(Value::Bool(v));
                }
                0x0F => {
                    let (b, a) = (pop(&mut stack), pop(&mut stack));
                    let v = self.to_number(player, &a) < self.to_number(player, &b);
                    stack.push(Value::Bool(v));
                }
                0x10 | 0x11 => {
                    let (b, a) = (pop(&mut stack), pop(&mut stack));
                    let (x, y) = (self.truthy(&a), self.truthy(&b));
                    stack.push(Value::Bool(if op == 0x10 { x && y } else { x || y }));
                }
                0x12 => {
                    let a = pop(&mut stack);
                    stack.push(Value::Bool(!self.truthy(&a)));
                }
                0x13 => {
                    let (b, a) = (pop(&mut stack), pop(&mut stack));
                    let v = self.to_str(player, &a) == self.to_str(player, &b);
                    stack.push(Value::Bool(v));
                }
                0x14 | 0x31 => {
                    let a = pop(&mut stack);
                    let n = self.to_str(player, &a).chars().count();
                    stack.push(Value::Number(n as f64));
                }
                0x15 | 0x35 => {
                    let (count, index, s) = (pop(&mut stack), pop(&mut stack), pop(&mut stack));
                    let s = self.to_str(player, &s);
                    let i = (self.to_number(player, &index) as i64 - 1).max(0) as usize;
                    let n = self.to_number(player, &count).max(0.0) as usize;
                    let out: String = s.chars().skip(i).take(n).collect();
                    stack.push(Value::Str(out.into()));
                }
                0x17 => {
                    stack.pop();
                }
                0x18 => {
                    let a = pop(&mut stack);
                    let n = self.to_number(player, &a);
                    stack.push(Value::Number(if n.is_nan() { 0.0 } else { n.trunc() }));
                }
                0x1C => {
                    let name = pop(&mut stack);
                    let name = self.to_str(player, &name);
                    let v = self.get_variable(player, frame, &name);
                    if self.trace && matches!(v, Value::Undefined) {
                        self.log.push(format!("{name} is undefined"));
                    }
                    stack.push(v);
                }
                0x1D => {
                    let (value, name) = (pop(&mut stack), pop(&mut stack));
                    let name = self.to_str(player, &name);
                    self.set_variable(player, frame, &name, value);
                }
                0x20 => {
                    let t = pop(&mut stack);
                    let target = match &t {
                        Value::Object(_) => clip_id(&t),
                        other => {
                            let s = self.to_str(player, other);
                            self.target_of(player, frame, &s)
                        }
                    };
                    frame.target = target.unwrap_or(frame.target);
                }
                0x21 => {
                    let (b, a) = (pop(&mut stack), pop(&mut stack));
                    let s = format!("{}{}", self.to_str(player, &a), self.to_str(player, &b));
                    stack.push(Value::Str(s.into()));
                }
                0x22 => {
                    let (index, target) = (pop(&mut stack), pop(&mut stack));
                    let t = self.to_str(player, &target);
                    let i = self.to_number(player, &index) as usize;
                    let v = match (self.target_of(player, frame, &t), PROPERTIES.get(i)) {
                        (Some(id), Some(p)) => {
                            let o = Value::Object(self.clip_object(id));
                            self.get(player, &o, p)
                        }
                        _ => Value::Undefined,
                    };
                    stack.push(v);
                }
                0x23 => {
                    let (value, index, target) =
                        (pop(&mut stack), pop(&mut stack), pop(&mut stack));
                    let t = self.to_str(player, &target);
                    let i = self.to_number(player, &index) as usize;
                    if let (Some(id), Some(p)) =
                        (self.target_of(player, frame, &t), PROPERTIES.get(i))
                    {
                        let o = Value::Object(self.clip_object(id));
                        self.set(player, &o, p, value);
                    }
                }
                0x24 => {
                    let (depth, name, source) = (pop(&mut stack), pop(&mut stack), pop(&mut stack));
                    let s = self.to_str(player, &source);
                    let n = self.to_str(player, &name).to_string();
                    let d = self.to_number(player, &depth) as i32;
                    if let Some(src) = self.target_of(player, frame, &s) {
                        library::duplicate(self, player, src, &n, d);
                    }
                }
                0x25 => {
                    let t = pop(&mut stack);
                    let id = match &t {
                        Value::Object(_) => clip_id(&t),
                        other => {
                            let s = self.to_str(player, other);
                            self.target_of(player, frame, &s)
                        }
                    };
                    if let Some(id) = id {
                        library::remove_clip(player, id);
                    }
                }
                0x26 => {
                    let a = pop(&mut stack);
                    let s = self.to_str(player, &a);
                    self.log.push(format!("trace: {s}"));
                }
                0x27 => {
                    // startDrag: target, lock, constrain?
                    let _target = pop(&mut stack);
                    let _lock = pop(&mut stack);
                    let constrain = pop(&mut stack);
                    if self.truthy(&constrain) {
                        for _ in 0..4 {
                            stack.pop();
                        }
                    }
                }
                0x28 => {}
                0x29 => {
                    let (b, a) = (pop(&mut stack), pop(&mut stack));
                    let v = self.to_str(player, &a) < self.to_str(player, &b);
                    stack.push(Value::Bool(v));
                }
                0x2A => {
                    // throw: the script ends there.
                    return Value::Undefined;
                }
                0x2B => {
                    let (obj, _ctor) = (pop(&mut stack), pop(&mut stack));
                    stack.push(obj);
                }
                0x2C => {
                    let (_ctor, n) = (pop(&mut stack), pop(&mut stack));
                    let n = self.to_number(player, &n).max(0.0) as usize;
                    for _ in 0..n {
                        stack.pop();
                    }
                }
                0x30 => {
                    let max = pop(&mut stack);
                    let m = self.to_number(player, &max).max(1.0);
                    stack.push(Value::Number((library::random() * m).floor()));
                }
                0x32 | 0x36 => {
                    let a = pop(&mut stack);
                    let s = self.to_str(player, &a);
                    stack.push(Value::Number(
                        s.chars().next().map_or(0.0, |c| f64::from(c as u32)),
                    ));
                }
                0x33 | 0x37 => {
                    let a = pop(&mut stack);
                    let n = self.to_number(player, &a) as u32;
                    let s: String = char::from_u32(n).into_iter().collect();
                    stack.push(Value::Str(s.into()));
                }
                0x34 => stack.push(Value::Number((self.time * 1000.0).floor())),
                0x3A => {
                    let (name, obj) = (pop(&mut stack), pop(&mut stack));
                    let name = self.to_str(player, &name);
                    let done = match &obj {
                        Value::Object(o) => o.borrow_mut().delete(&name),
                        _ => false,
                    };
                    stack.push(Value::Bool(done));
                }
                0x3B => {
                    let name = pop(&mut stack);
                    let name = self.to_str(player, &name);
                    let done = frame.scope.obj.borrow_mut().delete(&name);
                    stack.push(Value::Bool(done));
                }
                0x3C | 0x41 => {
                    let value = if op == 0x3C {
                        pop(&mut stack)
                    } else {
                        Value::Undefined
                    };
                    let name = pop(&mut stack);
                    let name = self.to_str(player, &name);
                    match &frame.locals {
                        Some(l) => {
                            if op == 0x3C || l.borrow().own(&name).is_none() {
                                l.borrow_mut().set(&name, value);
                            }
                        }
                        None => {
                            let t = Value::Object(self.clip_object(frame.target));
                            if op == 0x3C {
                                self.set(player, &t, &name, value);
                            }
                        }
                    }
                }
                0x3D => {
                    let name = pop(&mut stack);
                    let args = pop_args(self, player, &mut stack);
                    let name = self.to_str(player, &name);
                    let f = self.get_variable(player, frame, &name);
                    let this = Value::Object(self.clip_object(frame.target));
                    let r = self.call_value(player, &f, &this, &args);
                    stack.push(r);
                }
                0x3E => return pop(&mut stack),
                0x40 | 0x53 => {
                    let (name, obj) = if op == 0x40 {
                        (pop(&mut stack), Value::Undefined)
                    } else {
                        let n = pop(&mut stack);
                        let o = pop(&mut stack);
                        (n, o)
                    };
                    let args = pop_args(self, player, &mut stack);
                    let ctor = if op == 0x40 {
                        let n = self.to_str(player, &name);
                        self.get_variable(player, frame, &n)
                    } else if matches!(name, Value::Undefined)
                        || matches!(&name, Value::Str(s) if s.is_empty())
                    {
                        obj.clone()
                    } else {
                        let n = self.to_str(player, &name);
                        self.get(player, &obj, &n)
                    };
                    let r = self.construct(player, &ctor, &args);
                    stack.push(r);
                }
                0x42 => {
                    let args = pop_args(self, player, &mut stack);
                    let a = self.array(args);
                    stack.push(a);
                }
                0x43 => {
                    let n = pop(&mut stack);
                    let n = self.to_number(player, &n).max(0.0) as usize;
                    let o = self.object();
                    let mut pairs = Vec::new();
                    for _ in 0..n {
                        let v = pop(&mut stack);
                        let k = pop(&mut stack);
                        pairs.push((k, v));
                    }
                    for (k, v) in pairs.into_iter().rev() {
                        let k = self.to_str(player, &k);
                        o.borrow_mut().set(&k, v);
                    }
                    stack.push(Value::Object(o));
                }
                0x44 => {
                    let a = pop(&mut stack);
                    let t = match &a {
                        Value::Undefined => "undefined",
                        Value::Null => "null",
                        Value::Bool(_) => "boolean",
                        Value::Number(_) => "number",
                        Value::Str(_) => "string",
                        Value::Object(o) => match o.borrow().kind {
                            ObjKind::Function(_) | ObjKind::Native(_) => "function",
                            ObjKind::Clip(id) => {
                                if matches!(player.clip(id).map(|c| &c.kind), Some(Kind::Text(..)))
                                {
                                    "object"
                                } else {
                                    "movieclip"
                                }
                            }
                            _ => "object",
                        },
                    };
                    stack.push(Value::from(t));
                }
                0x45 => {
                    let a = pop(&mut stack);
                    let v = clip_id(&a).map_or(Value::Undefined, |id| {
                        Value::Str(target_path(player, id).into())
                    });
                    stack.push(v);
                }
                0x46 | 0x55 => {
                    let obj = if op == 0x46 {
                        let n = pop(&mut stack);
                        let n = self.to_str(player, &n);
                        self.get_variable(player, frame, &n)
                    } else {
                        pop(&mut stack)
                    };
                    stack.push(Value::Null);
                    for k in enumerate(player, &obj) {
                        stack.push(Value::Str(k));
                    }
                }
                0x48 => {
                    let (b, a) = (pop(&mut stack), pop(&mut stack));
                    let v = self.less(player, &a, &b);
                    stack.push(v);
                }
                0x67 => {
                    let (b, a) = (pop(&mut stack), pop(&mut stack));
                    let v = self.less(player, &b, &a);
                    stack.push(v);
                }
                0x49 => {
                    let (b, a) = (pop(&mut stack), pop(&mut stack));
                    let v = self.equals(player, &a, &b);
                    stack.push(Value::Bool(v));
                }
                0x66 => {
                    let (b, a) = (pop(&mut stack), pop(&mut stack));
                    stack.push(Value::Bool(self.strict_equals(&a, &b)));
                }
                0x4A => {
                    let a = pop(&mut stack);
                    let n = self.to_number(player, &a);
                    stack.push(Value::Number(n));
                }
                0x4B => {
                    let a = pop(&mut stack);
                    let s = self.to_str(player, &a);
                    stack.push(Value::Str(s));
                }
                0x4C => {
                    let a = stack.last().cloned().unwrap_or_default();
                    stack.push(a);
                }
                0x4D => {
                    let n = stack.len();
                    if n >= 2 {
                        stack.swap(n - 1, n - 2);
                    }
                }
                0x4E => {
                    let (name, obj) = (pop(&mut stack), pop(&mut stack));
                    let name = self.to_str(player, &name);
                    let v = self.get(player, &obj, &name);
                    if self.trace && matches!(v, Value::Undefined) {
                        self.log.push(format!("{obj:?}.{name} is undefined"));
                    }
                    stack.push(v);
                }
                0x4F => {
                    let (value, name, obj) = (pop(&mut stack), pop(&mut stack), pop(&mut stack));
                    let name = self.to_str(player, &name);
                    self.set(player, &obj, &name, value);
                }
                0x50 | 0x51 => {
                    let a = pop(&mut stack);
                    let n = self.to_number(player, &a);
                    stack.push(Value::Number(if op == 0x50 { n + 1.0 } else { n - 1.0 }));
                }
                0x52 => {
                    let (name, obj) = (pop(&mut stack), pop(&mut stack));
                    let args = pop_args(self, player, &mut stack);
                    let r = if matches!(name, Value::Undefined)
                        || matches!(&name, Value::Str(s) if s.is_empty())
                    {
                        self.call_value(player, &obj, &Value::Undefined, &args)
                    } else {
                        let n = self.to_str(player, &name);
                        let f = self.get(player, &obj, &n);
                        if matches!(f, Value::Undefined) {
                            self.log.push(format!("no method {n} on {obj:?}"));
                        }
                        self.call_value(player, &f, &obj, &args)
                    };
                    stack.push(r);
                }
                0x54 => {
                    let (ctor, obj) = (pop(&mut stack), pop(&mut stack));
                    let v = self.instance_of(player, &obj, &ctor);
                    stack.push(Value::Bool(v));
                }
                0x60..=0x65 => {
                    let (b, a) = (pop(&mut stack), pop(&mut stack));
                    let x = to_i32(self.to_number(player, &a));
                    let y = to_i32(self.to_number(player, &b));
                    stack.push(Value::Number(match op {
                        0x60 => f64::from(x & y),
                        0x61 => f64::from(x | y),
                        0x62 => f64::from(x ^ y),
                        0x63 => f64::from(x.wrapping_shl(y as u32 & 31)),
                        0x64 => f64::from(x.wrapping_shr(y as u32 & 31)),
                        _ => f64::from((x as u32).wrapping_shr(y as u32 & 31)),
                    }));
                }
                0x68 => {
                    let (b, a) = (pop(&mut stack), pop(&mut stack));
                    let v = self.to_str(player, &a) > self.to_str(player, &b);
                    stack.push(Value::Bool(v));
                }
                0x69 => {
                    let (sup, sub) = (pop(&mut stack), pop(&mut stack));
                    let sup_proto = self.get(player, &sup, "prototype");
                    let proto = self.object();
                    if let Value::Object(sp) = &sup_proto {
                        proto.borrow_mut().proto = Some(Rc::clone(sp));
                    }
                    proto.borrow_mut().set("constructor", sup.clone());
                    self.set(player, &sub, "prototype", Value::Object(proto));
                }
                0x81 => {
                    let f = data
                        .get(..2)
                        .map_or(0, |b| usize::from(u16::from_le_bytes([b[0], b[1]])));
                    player.goto(frame.target, f);
                    if let Some(c) = player.clip_mut(frame.target) {
                        c.playing = false;
                    }
                }
                0x83 => {
                    let mut b = super::Bits::new(data);
                    let url = b.cstr();
                    let target = b.cstr();
                    self.get_url(&url, &target, frame.target);
                }
                0x87 => {
                    let r = usize::from(data.first().copied().unwrap_or(0));
                    let v = stack.last().cloned().unwrap_or_default();
                    if r >= frame.regs.len() {
                        frame.regs.resize(r + 1, Value::Undefined);
                    }
                    frame.regs[r] = v;
                }
                0x88 => {
                    let mut b = super::Bits::new(data);
                    let n = b.u16();
                    let pool: Vec<Rc<str>> = (0..n).map(|_| Rc::from(b.cstr())).collect();
                    frame.pool = Rc::new(pool);
                }
                0x8A | 0x8D => {
                    // Everything is loaded: never skips.
                    if op == 0x8D {
                        stack.pop();
                    }
                }
                0x8B => {
                    let name = super::Bits::new(data).cstr();
                    frame.target = self.target_of(player, frame, &name).unwrap_or(frame.target);
                }
                0x8C => {
                    let label = super::Bits::new(data).cstr();
                    if self.trace {
                        self.log.push(format!("goto {label}"));
                    }
                    goto_label(player, frame.target, &label, false);
                }
                0x8E | 0x9B => {
                    let func_end =
                        self.define_function(player, op, data, code, next, frame, &mut stack);
                    pc = func_end;
                }
                0x8F => {
                    // try: the try block runs; catch and finally are skipped
                    // (nothing throws here).
                    let mut b = super::Bits::new(data);
                    let flags = b.u8();
                    let try_size = usize::from(b.u16());
                    let catch_size = usize::from(b.u16());
                    let finally_size = usize::from(b.u16());
                    let _ = flags;
                    let body_end = next + try_size;
                    let r = self.execute(player, code, next, body_end, frame);
                    if !matches!(r, Value::Undefined) {
                        return r;
                    }
                    let fin = body_end + catch_size;
                    let r = self.execute(player, code, fin, fin + finally_size, frame);
                    if !matches!(r, Value::Undefined) {
                        return r;
                    }
                    pc = fin + finally_size;
                }
                0x94 => {
                    let size = data
                        .get(..2)
                        .map_or(0, |b| usize::from(u16::from_le_bytes([b[0], b[1]])));
                    let obj = pop(&mut stack);
                    if let Value::Object(o) = obj {
                        frame.scope = Rc::new(Scope {
                            obj: o,
                            parent: Some(Rc::clone(&frame.scope)),
                        });
                        withs.push(next + size);
                    }
                }
                0x96 => self.push(data, frame, &mut stack),
                0x99 => {
                    let off = data
                        .get(..2)
                        .map_or(0, |b| i16::from_le_bytes([b[0], b[1]]));
                    pc = (next as isize + off as isize).max(0) as usize;
                }
                0x9A => {
                    let (target, url) = (pop(&mut stack), pop(&mut stack));
                    let (u, t) = (self.to_str(player, &url), self.to_str(player, &target));
                    self.get_url(&u, &t, frame.target);
                }
                0x9D => {
                    let off = data
                        .get(..2)
                        .map_or(0, |b| i16::from_le_bytes([b[0], b[1]]));
                    let c = pop(&mut stack);
                    if self.truthy(&c) {
                        pc = (next as isize + off as isize).max(0) as usize;
                    }
                }
                0x9E => {
                    // call(frame): runs that frame's scripts.
                    let f = pop(&mut stack);
                    let label = self.to_str(player, &f);
                    if let Some(c) = player.clip(frame.target)
                        && let Some(t) = &c.timeline
                        && let Some(n) = t.frame_of(&label)
                    {
                        let scripts: Vec<Rc<[u8]>> = t.frames[n]
                            .iter()
                            .filter_map(|c| match c {
                                super::player::Control::Script(s) => Some(Rc::clone(s)),
                                _ => None,
                            })
                            .collect();
                        let target = frame.target;
                        for s in scripts {
                            let saved = self.budget;
                            self.run_script(player, target, &s);
                            self.budget = saved;
                        }
                    }
                }
                0x9F => {
                    let play = data.first().is_some_and(|f| f & 1 != 0);
                    let bias = if data.first().is_some_and(|f| f & 2 != 0) {
                        data.get(1..3)
                            .map_or(0, |b| usize::from(u16::from_le_bytes([b[0], b[1]])))
                    } else {
                        0
                    };
                    let f = pop(&mut stack);
                    match f {
                        Value::Number(n) => {
                            let n = (n as i64 - 1).max(0) as usize + bias;
                            player.goto(frame.target, n);
                            if let Some(c) = player.clip_mut(frame.target) {
                                c.playing = play;
                            }
                        }
                        other => {
                            let s = self.to_str(player, &other);
                            let (target, label) = match s.rfind(':') {
                                Some(i) => (
                                    self.target_of(player, frame, &s[..i]),
                                    s[i + 1..].to_string(),
                                ),
                                None => (Some(frame.target), s.to_string()),
                            };
                            if self.trace {
                                self.log.push(format!("goto {s}"));
                            }
                            if let Some(t) = target {
                                goto_label(player, t, &label, play);
                            }
                        }
                    }
                }
                _ => {
                    self.log.push(format!("action {op:02x} not run"));
                }
            }
        }
        Value::Undefined
    }

    fn push(&mut self, data: &[u8], frame: &Frame, stack: &mut Vec<Value>) {
        let mut b = super::Bits::new(data);
        while !b.done() {
            let v = match b.u8() {
                0 => Value::Str(b.cstr().into()),
                1 => {
                    let v = [b.u8(), b.u8(), b.u8(), b.u8()];
                    Value::Number(f64::from(f32::from_le_bytes(v)))
                }
                2 => Value::Null,
                3 => Value::Undefined,
                4 => {
                    let r = usize::from(b.u8());
                    frame.regs.get(r).cloned().unwrap_or_default()
                }
                5 => Value::Bool(b.u8() != 0),
                6 => {
                    // Two 32-bit halves, the high one first.
                    let hi = [b.u8(), b.u8(), b.u8(), b.u8()];
                    let lo = [b.u8(), b.u8(), b.u8(), b.u8()];
                    let bits = (u64::from(u32::from_le_bytes(hi)) << 32)
                        | u64::from(u32::from_le_bytes(lo));
                    Value::Number(f64::from_bits(bits))
                }
                7 => {
                    let v = [b.u8(), b.u8(), b.u8(), b.u8()];
                    Value::Number(f64::from(i32::from_le_bytes(v)))
                }
                8 => {
                    let i = usize::from(b.u8());
                    frame
                        .pool
                        .get(i)
                        .map_or(Value::Undefined, |s| Value::Str(Rc::clone(s)))
                }
                9 => {
                    let i = usize::from(b.u16());
                    frame
                        .pool
                        .get(i)
                        .map_or(Value::Undefined, |s| Value::Str(Rc::clone(s)))
                }
                _ => break,
            };
            stack.push(v);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn define_function(
        &mut self,
        player: &mut Player,
        op: u8,
        data: &[u8],
        code: &Rc<[u8]>,
        body: usize,
        frame: &Frame,
        stack: &mut Vec<Value>,
    ) -> usize {
        let mut b = super::Bits::new(data);
        let name = b.cstr();
        let n = b.u16();
        let mut params = Vec::new();
        let mut v2 = None;
        if op == 0x8E {
            let regs = b.u8();
            let f1 = b.u8();
            let f2 = b.u8();
            // As stored, most significant first: preload parent, root,
            // suppress super, preload super, suppress arguments, preload
            // arguments, suppress this, preload this; then preload global.
            let mut flags = 0u16;
            let map = [
                (0x80, 0x0080),
                (0x40, 0x0040),
                (0x20, 0x0020),
                (0x10, 0x0010),
                (0x08, 0x0008),
                (0x04, 0x0004),
                (0x02, 0x0002),
                (0x01, 0x0001),
            ];
            for (bit, v) in map {
                if f1 & bit != 0 {
                    flags |= v;
                }
            }
            if f2 & 1 != 0 {
                flags |= 0x0100;
            }
            for _ in 0..n {
                let reg = b.u8();
                params.push((reg, b.cstr()));
            }
            v2 = Some((regs, flags));
        } else {
            for _ in 0..n {
                params.push((0, b.cstr()));
            }
        }
        let size = usize::from(b.u16());
        let func = Function {
            name: name.clone(),
            params,
            code: Rc::clone(code),
            start: body,
            end: body + size,
            scope: Rc::clone(&frame.scope),
            target: frame.target,
            pool: Rc::clone(&frame.pool),
            v2,
        };
        let f = self.function_object(func);
        if name.is_empty() {
            stack.push(f);
        } else {
            match &frame.locals {
                Some(l) => l.borrow_mut().set(&name, f),
                None => {
                    let t = Value::Object(self.clip_object(frame.target));
                    self.set(player, &t, &name, f);
                }
            }
        }
        body + size
    }

    /// A function object for `func`, with a `prototype` of its own.
    pub fn function_object(&self, func: Function) -> Value {
        let f = Object::new(
            ObjKind::Function(Rc::new(func)),
            Some(Rc::clone(&self.function_proto)),
        );
        let proto = self.object();
        proto
            .borrow_mut()
            .set("constructor", Value::Object(Rc::clone(&f)));
        f.borrow_mut().set("prototype", Value::Object(proto));
        Value::Object(f)
    }

    /// `new ctor(args)`.
    pub fn construct(&mut self, player: &mut Player, ctor: &Value, args: &[Value]) -> Value {
        let Value::Object(c) = ctor else {
            return Value::Undefined;
        };
        let kind = c.borrow().kind.clone();
        if let ObjKind::Native(n) = kind {
            // Built-in constructors make their own objects (`this` absent).
            return n(self, player, &Value::Undefined, args);
        }
        let proto = match self.get(player, ctor, "prototype") {
            Value::Object(p) => Some(p),
            _ => Some(Rc::clone(&self.object_proto)),
        };
        let obj = Value::Object(Object::new(ObjKind::Plain, proto));
        let r = self.call_value(player, ctor, &obj, args);
        match r {
            Value::Object(_) => r,
            _ => obj,
        }
    }

    fn instance_of(&mut self, player: &mut Player, obj: &Value, ctor: &Value) -> bool {
        let Value::Object(target) = self.get(player, ctor, "prototype") else {
            return false;
        };
        let Value::Object(o) = obj else { return false };
        let mut p = o.borrow().proto.clone();
        while let Some(x) = p {
            if Rc::ptr_eq(&x, &target) {
                return true;
            }
            p = x.borrow().proto.clone();
        }
        false
    }

    fn get_url(&mut self, url: &str, target: &str, from: ClipId) {
        let lower = url.to_ascii_lowercase();
        if let Some(cmd) = lower.strip_prefix("fscommand:") {
            let cmd = &url[url.len() - cmd.len()..];
            self.commands.push((cmd.to_owned(), target.to_owned()));
        } else if !url.is_empty() {
            // loadMovieNum (a level) or loadMovie (a clip, by its path).
            if self.trace {
                self.log
                    .push(format!("getURL {url:?} into {target:?} from #{from}"));
            }
            self.loads.push((url.to_owned(), target.to_owned(), from));
        }
    }

    /// Calls each instance's `onEnterFrame`, then intervals due by `time`.
    fn enter_frame(&mut self, player: &mut Player) {
        for id in player.order() {
            let Some(o) = self.clips.get(&id).cloned() else {
                continue;
            };
            let h = o.borrow().own("onEnterFrame");
            if let Some(f @ Value::Object(_)) = h {
                self.budget = 0;
                self.call_value(player, &f, &Value::Object(o), &[]);
            }
        }
    }

    fn run_intervals(&mut self, player: &mut Player) {
        let due: Vec<usize> = (0..self.intervals.len())
            .filter(|&i| self.intervals[i].due <= self.time)
            .collect();
        for i in due {
            let Some(iv) = self.intervals.get_mut(i) else {
                continue;
            };
            iv.due += iv.every.max(0.001);
            let (call, args, id) = (iv.call.clone(), iv.args.clone(), iv.id);
            self.budget = 0;
            match call {
                (f, None) => {
                    self.call_value(player, &f, &Value::Undefined, &args);
                }
                (obj, Some(method)) => {
                    let f = self.get(player, &obj, &method);
                    self.call_value(player, &f, &obj, &args);
                }
            }
            let _ = id;
        }
    }

    /// Draws `pen`'s path into `id`'s drawing.
    fn flush_fill(&mut self, player: &mut Player, id: ClipId) {
        let Some(pen) = self.pens.get_mut(&id) else {
            return;
        };
        let Some(colour) = pen.fill else {
            pen.path.clear();
            return;
        };
        let mut segments = std::mem::take(&mut pen.path);
        if let (Some(first), Some(last)) =
            (segments.first().map(|s| s.0), segments.last().map(|s| s.1))
            && first != last
        {
            segments.push((last, first));
        }
        // Either way round fills: nonzero of the path as drawn.
        let tris = fill_triangles(&segments);
        if let Some(c) = player.clip_mut(id) {
            for corners in tris {
                push_clipped(&mut c.drawing, corners, [[0.0; 2]; 3], colour, None, None);
            }
        }
    }
}

impl Default for Avm {
    fn default() -> Self {
        Self::new()
    }
}

/// The properties `GetProperty`/`SetProperty` number.
const PROPERTIES: [&str; 22] = [
    "_x",
    "_y",
    "_xscale",
    "_yscale",
    "_currentframe",
    "_totalframes",
    "_alpha",
    "_visible",
    "_width",
    "_height",
    "_rotation",
    "_target",
    "_framesloaded",
    "_name",
    "_droptarget",
    "_url",
    "_highquality",
    "_focusrect",
    "_soundbuftime",
    "_quality",
    "_xmouse",
    "_ymouse",
];

fn lookup(o: &Obj, name: &str) -> Option<Value> {
    let mut at = Some(Rc::clone(o));
    let mut n = 0;
    while let Some(x) = at {
        if let Some(v) = x.borrow().own(name) {
            return Some(v);
        }
        at = x.borrow().proto.clone();
        n += 1;
        if n > 64 {
            break;
        }
    }
    None
}

fn clip_id(v: &Value) -> Option<ClipId> {
    match v {
        Value::Object(o) => match o.borrow().kind {
            ObjKind::Clip(id) => Some(id),
            _ => None,
        },
        _ => None,
    }
}

fn pop_args(avm: &mut Avm, player: &mut Player, stack: &mut Vec<Value>) -> Vec<Value> {
    let n = stack.pop().unwrap_or_default();
    let n = avm.to_number(player, &n);
    let n = if n.is_nan() {
        0
    } else {
        (n.max(0.0) as usize).min(stack.len())
    };
    (0..n).map(|_| stack.pop().unwrap_or_default()).collect()
}

fn split_target(name: &str) -> Option<(&str, &str)> {
    if let Some(i) = name.rfind(':') {
        return Some((&name[..i], &name[i + 1..]));
    }
    let i = name.rfind('.')?;
    Some((&name[..i], &name[i + 1..]))
}

fn enumerate(player: &Player, obj: &Value) -> Vec<Rc<str>> {
    let Value::Object(o) = obj else {
        return Vec::new();
    };
    let o = o.borrow();
    let mut keys: Vec<Rc<str>> = match &o.kind {
        ObjKind::Array(items) => (0..items.len()).map(|i| Rc::from(i.to_string())).collect(),
        _ => Vec::new(),
    };
    keys.extend(o.keys());
    if let ObjKind::Clip(id) = o.kind
        && let Some(c) = player.clip(id)
    {
        for &k in c.children.values() {
            if let Some(k) = player.clip(k).filter(|k| !k.name.is_empty()) {
                keys.push(k.name.as_str().into());
            }
        }
    }
    keys
}

/// The instance's path from the root (`_level0.a.b`).
pub fn target_path(player: &Player, id: ClipId) -> String {
    let mut names = Vec::new();
    let mut at = Some(id);
    while let Some(i) = at {
        let Some(c) = player.clip(i) else { break };
        if c.parent.is_none() {
            break;
        }
        names.push(c.name.clone());
        at = c.parent;
    }
    names.reverse();
    let mut s = String::from("_level0");
    for n in names {
        s.push('.');
        s.push_str(&n);
    }
    s
}

fn goto_label(player: &mut Player, id: ClipId, label: &str, play: bool) {
    let frame = player
        .clip(id)
        .and_then(|c| c.timeline.as_ref())
        .and_then(|t| t.frame_of(label));
    if let Some(f) = frame {
        player.goto(id, f);
        if let Some(c) = player.clip_mut(id) {
            c.playing = play;
        }
    }
}

/// The box round what instance `id` draws, in its parent's space.
/// Where a text field's `_x`/`_y` sit from its origin: its box's top-left
/// corner (the box's `-2, -2` from `DefineEditText`), through its matrix's
/// scale and turn; other clips sit at their origin. Inferred from footage
/// (the loading screen's hint lines, placed by `_y`, sit 2 px lower than
/// with `_y` as the origin: 10 lines of 4 hints, tops 2.4 -> 0.4 px off
/// with [`text::measure`](super::text::measure)'s last-line rule); Flash's
/// rule itself not read.
fn text_corner(c: &super::player::Clip) -> [f32; 2] {
    match &c.kind {
        Kind::Text(def, _) => c.lib.texts.get(def).map_or([0.0; 2], |t| {
            let [x, y] = [t.bounds[0], t.bounds[1]];
            let m = &c.matrix;
            [m.a * x + m.c * y, m.b * x + m.d * y]
        }),
        _ => [0.0; 2],
    }
}

fn clip_bounds(player: &Player, id: ClipId) -> Option<[f32; 4]> {
    let c = player.clip(id)?;
    let mut tris = Vec::new();
    collect(player, id, &c.matrix, &mut tris, 0);
    super::bounds(&tris).or_else(|| {
        // A text field: its box.
        if let Kind::Text(def, _) = &c.kind {
            let b = c.lib.texts.get(def)?.bounds;
            let p = [c.matrix.apply([b[0], b[1]]), c.matrix.apply([b[2], b[3]])];
            Some([p[0][0], p[0][1], p[1][0], p[1][1]])
        } else {
            None
        }
    })
}

fn collect(player: &Player, id: ClipId, m: &Matrix, out: &mut Vec<Triangle>, nesting: usize) {
    let Some(c) = player.clip(id) else { return };
    if nesting > 32 {
        return;
    }
    if let Kind::Shape(s) = c.kind
        && let Some((shape, mesh)) = c.lib.shapes.get(&s)
    {
        super::draw_shape(
            shape,
            mesh,
            &c.lib.images,
            m,
            &ColourTransform::IDENTITY,
            None,
            out,
        );
    }
    for t in &c.drawing {
        out.push(Triangle {
            corners: t.corners.map(|p| m.apply(p)),
            ..t.clone()
        });
    }
    for &k in c.children.values() {
        if let Some(kc) = player.clip(k) {
            collect(player, k, &m.then(&kc.matrix), out, nesting + 1);
        }
    }
}

fn to_i32(n: f64) -> i32 {
    if n.is_finite() { (n as i64) as i32 } else { 0 }
}

fn str_number(s: &str) -> f64 {
    let t = s.trim();
    if t.is_empty() {
        return f64::NAN;
    }
    if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return i64::from_str_radix(h, 16).map_or(f64::NAN, |v| v as f64);
    }
    t.parse::<f64>().unwrap_or(f64::NAN)
}

/// A number as ActionScript writes it.
pub fn number_str(n: f64) -> String {
    if n.is_nan() {
        "NaN".into()
    } else if n.is_infinite() {
        if n > 0.0 { "Infinity" } else { "-Infinity" }.into()
    } else if n == n.trunc() && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        let s = format!("{n}");
        s
    }
}

/// Finds a movie by file name (`map.gfx`, without the extension, any case)
/// for `loadMovie`.
pub type Loader = Box<dyn FnMut(&str) -> Option<Rc<super::Library>>>;

/// A movie with its scripts running.
pub struct Runtime {
    pub player: Player,
    pub avm: Avm,
    clock: f32,
    loader: Option<Loader>,
}

impl fmt::Debug for Runtime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Runtime")
    }
}

impl Runtime {
    /// Starts `player`'s movie: its init scripts, then its first frame's.
    pub fn new(mut player: Player) -> Self {
        let mut avm = Avm::new();
        avm.stage = player.lib.size;
        let inits = player.lib.init_scripts.clone();
        let root = player.root;
        for (_, code) in inits {
            avm.run_script(&mut player, root, &code);
        }
        let mut rt = Runtime {
            player,
            avm,
            clock: 0.0,
            loader: None,
        };
        rt.run_due();
        rt
    }

    /// Movies loaded by name from now on (and those asked for already).
    pub fn set_loader(&mut self, loader: Loader) {
        self.loader = Some(loader);
        self.run_due();
    }

    /// Runs the scripts due (and those they make due), in order; loads the
    /// movies asked for.
    pub fn run_due(&mut self) {
        for _ in 0..64 {
            self.load_movies();
            let due = self.player.take_scripts();
            if due.is_empty() {
                break;
            }
            for (id, code) in due {
                self.avm.run_script(&mut self.player, id, &code);
            }
        }
    }

    /// Plays `dt` seconds: each frame, the instances' `onEnterFrame`, the
    /// timelines on, their scripts, then intervals due.
    pub fn step(&mut self, dt: f32) {
        let frame = 1.0 / self.player.lib.frame_rate;
        self.clock += dt.max(0.0);
        let mut played = 0;
        while self.clock >= frame {
            self.clock -= frame;
            if played == 4 {
                self.clock = 0.0;
                break;
            }
            self.avm.time += f64::from(frame);
            self.avm.enter_frame(&mut self.player);
            self.run_due();
            self.player.advance();
            self.run_due();
            self.avm.run_intervals(&mut self.player);
            self.run_due();
            played += 1;
        }
    }

    /// Calls the function at `path` (`MapIntialize`, `_root.map_mc.f`) with
    /// `args`, then runs the scripts that makes due.
    pub fn call(&mut self, path: &str, args: &[Value]) -> Value {
        let root = Value::Object(self.avm.clip_object(self.player.root));
        let (owner, name) = match split_target(path) {
            Some((p, n)) => {
                let mut at = root;
                for part in p
                    .split(['.', '/'])
                    .filter(|s| !s.is_empty() && *s != "_root" && *s != "_level0")
                {
                    at = self.avm.get(&mut self.player, &at, part);
                }
                (at, n.to_owned())
            }
            None => (root, path.to_owned()),
        };
        let mut f = self.avm.get(&mut self.player, &owner, &name);
        if matches!(f, Value::Undefined) {
            f = lookup(&self.avm.global, &name).unwrap_or_default();
        }
        if matches!(f, Value::Undefined) {
            self.avm.log.push(format!("call {path}: no such function"));
            return Value::Undefined;
        }
        self.avm.budget = 0;
        let r = self.avm.call_value(&mut self.player, &f, &owner, args);
        self.run_due();
        r
    }

    /// A key goes down (a Flash key code, as `Key.getCode` gives it): each
    /// `Key` listener's `onKeyDown` runs.
    pub fn key_down(&mut self, code: i32) {
        self.avm.key_code = code;
        if !self.avm.keys_down.contains(&code) {
            self.avm.keys_down.push(code);
        }
        self.key_event("onKeyDown");
    }

    /// A key comes up: each `Key` listener's `onKeyUp` runs.
    pub fn key_up(&mut self, code: i32) {
        self.avm.key_code = code;
        self.avm.keys_down.retain(|&k| k != code);
        self.key_event("onKeyUp");
    }

    fn key_event(&mut self, name: &str) {
        for listener in self.avm.key_listeners.clone() {
            let f = self.avm.get(&mut self.player, &listener, name);
            if !matches!(f, Value::Undefined) {
                self.avm.budget = 0;
                self.avm.call_value(&mut self.player, &f, &listener, &[]);
            }
        }
        self.run_due();
    }

    /// What the scripts asked of the host since the last call:
    /// `fscommand(command, argument)`.
    pub fn take_commands(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.avm.commands)
    }

    /// Sets a variable on the movie's root (as the host does).
    pub fn set_variable(&mut self, name: &str, value: Value) {
        let root = Value::Object(self.avm.clip_object(self.player.root));
        self.avm.set(&mut self.player, &root, name, value);
    }

    /// Text fields bound to variables show them.
    pub fn sync_text(&mut self) {
        for id in self.player.order() {
            let Some(c) = self.player.clip(id) else {
                continue;
            };
            let Kind::Text(def, _) = c.kind else { continue };
            let Some(var) = c.lib.texts.get(&def).map(|t| t.variable.clone()) else {
                continue;
            };
            if var.is_empty() {
                continue;
            }
            let Some(parent) = c.parent else { continue };
            let owner = Value::Object(self.avm.clip_object(parent));
            let (owner, name) = match split_target(&var) {
                Some((p, n)) => {
                    let mut at = owner;
                    for part in p.split(['.', '/']).filter(|s| !s.is_empty()) {
                        at = self.avm.get(&mut self.player, &at, part);
                    }
                    (at, n.to_owned())
                }
                None => (owner, var),
            };
            let v = self.avm.get(&mut self.player, &owner, &name);
            if matches!(v, Value::Undefined) {
                continue;
            }
            let s = self.avm.to_str(&mut self.player, &v).to_string();
            if let Some(c) = self.player.clip_mut(id)
                && let Kind::Text(_, t) = &mut c.kind
            {
                *t = s;
            }
        }
    }
}

impl Runtime {
    /// The movies the scripts asked for, loaded into their levels or clips
    /// (with their init scripts run).
    fn load_movies(&mut self) {
        let loads = std::mem::take(&mut self.avm.loads);
        for (url, target, from) in loads {
            let name = url
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(&url)
                .split('.')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            let Some(loader) = &mut self.loader else {
                self.avm.log.push(format!("load {url}: no loader"));
                continue;
            };
            let Some(lib) = loader(&name) else {
                self.avm.log.push(format!("load {url}: not found"));
                continue;
            };
            let lower = target.to_ascii_lowercase();
            let into = if let Some(level) = lower
                .strip_prefix("_level")
                .and_then(|l| l.parse::<i32>().ok())
            {
                Some(self.player.load_level(level, Rc::clone(&lib)))
            } else {
                let clip = resolve_clip(&self.player, from, &target);
                if let Some(c) = clip {
                    self.player.load_into(c, Rc::clone(&lib));
                }
                clip
            };
            if let Some(into) = into {
                self.avm.log.push(format!("loaded {name} into {target}"));
                for (_, code) in lib.init_scripts.clone() {
                    self.avm.run_script(&mut self.player, into, &code);
                }
            }
        }
    }
}

/// The clip `path` names from `from` (`_root.a.b`, `_parent`, `a/b`).
fn resolve_clip(player: &Player, from: ClipId, path: &str) -> Option<ClipId> {
    let mut at = if path.starts_with('/') {
        player.root_of(from)
    } else {
        from
    };
    for part in path.split(['.', '/']).filter(|p| !p.is_empty()) {
        at = match part.to_ascii_lowercase().as_str() {
            "_root" => player.root_of(at),
            "this" => at,
            "_parent" | ".." => player.clip(at)?.parent?,
            other => player.child(at, other)?,
        };
    }
    Some(at)
}
mod library;
