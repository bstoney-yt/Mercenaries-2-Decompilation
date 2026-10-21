//! The built-in objects: globals, `MovieClip`, `Array`, `String`, `Math`,
//! `Color`, `Key`, `Stage`, intervals and the drawing API.

#![allow(
    clippy::wrong_self_convention,
    clippy::needless_return,
    clippy::collapsible_if,
    clippy::collapsible_match,
    clippy::needless_lifetimes
)]

use std::cell::Cell;
use std::rc::Rc;

use super::super::player::{ClipId, Kind, Player, TIMELINE_DEPTH};
use super::super::text::Substitution;
use super::super::{Matrix, push_clipped};
use super::{Avm, Interval, Native, Obj, ObjKind, Object, Pen, Value, clip_id, native, number_str};

fn def(avm: &Avm, obj: &Obj, name: &str, f: Native) {
    obj.borrow_mut().set(name, native(avm, f));
}

fn same_object(a: &Value, b: &Value) -> bool {
    matches!((a, b), (Value::Object(x), Value::Object(y)) if Rc::ptr_eq(x, y))
}

/// One of `setImageSubstitutions`' objects.
fn substitution(avm: &mut Avm, player: &mut Player, item: &Value) -> Option<Substitution> {
    let get = |avm: &mut Avm, player: &mut Player, v: &Value, k: &str| avm.get(player, v, k);
    let text = get(avm, player, item, "subString");
    let text = avm.to_str(player, &text).to_string();
    let image = get(avm, player, item, "image");
    let Value::Str(texture) = get(avm, player, &image, "texture") else {
        return None;
    };
    let number = |avm: &mut Avm, player: &mut Player, v: &Value, k: &str| {
        let x = get(avm, player, v, k);
        match x {
            Value::Undefined => None,
            x => Some(avm.to_number(player, &x) as f32),
        }
    };
    let part = [
        number(avm, player, &image, "u0").unwrap_or(0.0),
        number(avm, player, &image, "v0").unwrap_or(0.0),
        number(avm, player, &image, "u1").unwrap_or(1.0),
        number(avm, player, &image, "v1").unwrap_or(1.0),
    ];
    let natural = [
        number(avm, player, &image, "width").unwrap_or(0.0),
        number(avm, player, &image, "height").unwrap_or(0.0),
    ];
    let size = [
        number(avm, player, item, "width").unwrap_or(natural[0]),
        number(avm, player, item, "height").unwrap_or(natural[1]),
    ];
    let baseline = number(avm, player, item, "baseLineY").unwrap_or(size[1]);
    let id = get(avm, player, item, "id");
    let id = match id {
        Value::Undefined => String::new(),
        v => avm.to_str(player, &v).to_string(),
    };
    (!text.is_empty()).then(|| Substitution {
        text,
        texture: texture.to_string(),
        part,
        size,
        baseline,
        id,
    })
}

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).cloned().unwrap_or_default()
}

fn num(avm: &mut Avm, player: &mut Player, args: &[Value], i: usize) -> f64 {
    let v = arg(args, i);
    avm.to_number(player, &v)
}

fn text(avm: &mut Avm, player: &mut Player, v: &Value) -> String {
    avm.to_str(player, v).to_string()
}

/// Sets up the globals and prototypes.
pub fn install(avm: &mut Avm) {
    let g = Rc::clone(&avm.global);
    // Object.prototype and Function.prototype.
    let op = Rc::clone(&avm.object_proto);
    def(avm, &op, "toString", object_to_string);
    def(avm, &op, "valueOf", |_, _, this, _| this.clone());
    def(avm, &op, "hasOwnProperty", |avm, player, this, args| {
        let n = arg(args, 0);
        let n = text(avm, player, &n);
        match this {
            Value::Object(o) => Value::Bool(o.borrow().own(&n).is_some()),
            _ => Value::Bool(false),
        }
    });
    def(avm, &op, "addProperty", |_, _, _, _| Value::Bool(false));
    def(avm, &op, "watch", |_, _, _, _| Value::Bool(false));
    let fp = Rc::clone(&avm.function_proto);
    def(avm, &fp, "call", |avm, player, this, args| {
        let t = arg(args, 0);
        avm.call_value(player, this, &t, args.get(1..).unwrap_or(&[]))
    });
    def(avm, &fp, "apply", |avm, player, this, args| {
        let t = arg(args, 0);
        let list = match arg(args, 1) {
            Value::Object(o) => match &o.borrow().kind {
                ObjKind::Array(a) => a.clone(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        avm.call_value(player, this, &t, &list)
    });

    // Constructors.
    let object_ctor = native(avm, |avm, _, _, _| Value::Object(avm.object()));
    set_proto(&object_ctor, &avm.object_proto);
    g.borrow_mut().set("Object", object_ctor);
    let array_ctor = native(avm, |avm, player, _, args| {
        if args.len() == 1
            && let Value::Number(n) = args[0]
        {
            let _ = player;
            return avm.array(vec![Value::Undefined; n.max(0.0) as usize]);
        }
        avm.array(args.to_vec())
    });
    set_proto(&array_ctor, &avm.array_proto);
    g.borrow_mut().set("Array", array_ctor);
    let string_ctor = native(avm, |avm, player, _, args| {
        let v = arg(args, 0);
        Value::Str(avm.to_str(player, &v))
    });
    set_proto(&string_ctor, &avm.string_proto);
    g.borrow_mut().set("String", string_ctor);
    def(avm, &g, "Number", |avm, player, _, args| {
        if args.is_empty() {
            return Value::Number(0.0);
        }
        Value::Number(num(avm, player, args, 0))
    });
    def(avm, &g, "Boolean", |avm, _, _, args| {
        Value::Bool(avm.truthy(&arg(args, 0)))
    });
    let clip_ctor = native(avm, |_, _, _, _| Value::Undefined);
    set_proto(&clip_ctor, &avm.clip_proto);
    g.borrow_mut().set("MovieClip", clip_ctor.clone());
    g.borrow_mut().set("TextField", clip_ctor);
    def(avm, &g, "TextFormat", |avm, _, _, _| {
        Value::Object(avm.object())
    });
    def(avm, &g, "Color", |avm, _, _, args| {
        let o = avm.object();
        o.borrow_mut().set("target", arg(args, 0));
        def(avm, &o, "setRGB", color_set_rgb);
        def(avm, &o, "getRGB", |_, _, _, _| Value::Number(0.0));
        def(avm, &o, "setTransform", color_set_transform);
        def(avm, &o, "getTransform", |avm, _, _, _| {
            Value::Object(avm.object())
        });
        Value::Object(o)
    });

    // Functions.
    def(avm, &g, "isNaN", |avm, player, _, args| {
        Value::Bool(num(avm, player, args, 0).is_nan())
    });
    def(avm, &g, "isFinite", |avm, player, _, args| {
        Value::Bool(num(avm, player, args, 0).is_finite())
    });
    def(avm, &g, "parseInt", |avm, player, _, args| {
        let s = arg(args, 0);
        let s = text(avm, player, &s);
        let radix = if args.len() > 1 {
            num(avm, player, args, 1) as u32
        } else {
            10
        };
        let t = s.trim();
        let (neg, t) = match t.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, t.strip_prefix('+').unwrap_or(t)),
        };
        let (radix, t) = match (radix, t.strip_prefix("0x").or_else(|| t.strip_prefix("0X"))) {
            (10 | 16, Some(h)) => (16, h),
            (r, _) => (if (2..=36).contains(&r) { r } else { 10 }, t),
        };
        let digits: String = t.chars().take_while(|c| c.is_digit(radix)).collect();
        match i64::from_str_radix(&digits, radix) {
            Ok(v) => Value::Number(if neg { -(v as f64) } else { v as f64 }),
            Err(_) => Value::Number(f64::NAN),
        }
    });
    def(avm, &g, "parseFloat", |avm, player, _, args| {
        let s = arg(args, 0);
        let s = text(avm, player, &s);
        let t = s.trim();
        let mut end = 0;
        for i in (1..=t.len()).rev() {
            if t.is_char_boundary(i) && t[..i].parse::<f64>().is_ok() {
                end = i;
                break;
            }
        }
        Value::Number(if end == 0 {
            f64::NAN
        } else {
            t[..end].parse().unwrap_or(f64::NAN)
        })
    });
    def(avm, &g, "escape", |avm, player, _, args| {
        let s = arg(args, 0);
        Value::Str(avm.to_str(player, &s))
    });
    def(avm, &g, "unescape", |avm, player, _, args| {
        let s = arg(args, 0);
        Value::Str(avm.to_str(player, &s))
    });
    def(avm, &g, "trace", |avm, player, _, args| {
        let s = arg(args, 0);
        let s = text(avm, player, &s);
        avm.log.push(format!("trace: {s}"));
        Value::Undefined
    });
    def(avm, &g, "getTimer", |avm, _, _, _| {
        Value::Number((avm.time * 1000.0).floor())
    });
    def(avm, &g, "updateAfterEvent", |_, _, _, _| Value::Undefined);
    def(avm, &g, "ASSetPropFlags", |_, _, _, _| Value::Undefined);
    def(avm, &g, "fscommand", |avm, player, _, args| {
        let (c, a) = (arg(args, 0), arg(args, 1));
        let (c, a) = (text(avm, player, &c), text(avm, player, &a));
        avm.commands.push((c, a));
        Value::Undefined
    });
    def(avm, &g, "setInterval", set_interval);
    def(avm, &g, "clearInterval", |avm, player, _, args| {
        let id = num(avm, player, args, 0) as i32;
        avm.intervals.retain(|i| i.id != id);
        Value::Undefined
    });

    // Math.
    let math = avm.object();
    math.borrow_mut()
        .set("PI", Value::Number(std::f64::consts::PI));
    math.borrow_mut()
        .set("E", Value::Number(std::f64::consts::E));
    math.borrow_mut()
        .set("SQRT2", Value::Number(std::f64::consts::SQRT_2));
    def(avm, &math, "abs", |a, p, _, x| {
        Value::Number(num(a, p, x, 0).abs())
    });
    def(avm, &math, "floor", |a, p, _, x| {
        Value::Number(num(a, p, x, 0).floor())
    });
    def(avm, &math, "ceil", |a, p, _, x| {
        Value::Number(num(a, p, x, 0).ceil())
    });
    def(avm, &math, "round", |a, p, _, x| {
        Value::Number((num(a, p, x, 0) + 0.5).floor())
    });
    def(avm, &math, "sqrt", |a, p, _, x| {
        Value::Number(num(a, p, x, 0).sqrt())
    });
    def(avm, &math, "sin", |a, p, _, x| {
        Value::Number(num(a, p, x, 0).sin())
    });
    def(avm, &math, "cos", |a, p, _, x| {
        Value::Number(num(a, p, x, 0).cos())
    });
    def(avm, &math, "tan", |a, p, _, x| {
        Value::Number(num(a, p, x, 0).tan())
    });
    def(avm, &math, "asin", |a, p, _, x| {
        Value::Number(num(a, p, x, 0).asin())
    });
    def(avm, &math, "acos", |a, p, _, x| {
        Value::Number(num(a, p, x, 0).acos())
    });
    def(avm, &math, "atan", |a, p, _, x| {
        Value::Number(num(a, p, x, 0).atan())
    });
    def(avm, &math, "exp", |a, p, _, x| {
        Value::Number(num(a, p, x, 0).exp())
    });
    def(avm, &math, "log", |a, p, _, x| {
        Value::Number(num(a, p, x, 0).ln())
    });
    def(avm, &math, "atan2", |a, p, _, x| {
        let (y, xx) = (num(a, p, x, 0), num(a, p, x, 1));
        Value::Number(y.atan2(xx))
    });
    def(avm, &math, "pow", |a, p, _, x| {
        let (b, e) = (num(a, p, x, 0), num(a, p, x, 1));
        Value::Number(b.powf(e))
    });
    def(avm, &math, "min", |a, p, _, x| {
        let v: Vec<f64> = (0..x.len()).map(|i| num(a, p, x, i)).collect();
        Value::Number(if v.iter().any(|n| n.is_nan()) {
            f64::NAN
        } else {
            v.into_iter().fold(f64::INFINITY, f64::min)
        })
    });
    def(avm, &math, "max", |a, p, _, x| {
        let v: Vec<f64> = (0..x.len()).map(|i| num(a, p, x, i)).collect();
        Value::Number(if v.iter().any(|n| n.is_nan()) {
            f64::NAN
        } else {
            v.into_iter().fold(f64::NEG_INFINITY, f64::max)
        })
    });
    def(avm, &math, "random", |_, _, _, _| Value::Number(random()));
    g.borrow_mut().set("Math", Value::Object(math));

    // Scaleform's `flash.display.BitmapData.loadBitmap(name)`: an image the
    // movie exports by name, for `TextField.setImageSubstitutions`.
    let bitmap_data = avm.object();
    def(avm, &bitmap_data, "loadBitmap", |avm, player, _, args| {
        let name = arg(args, 0);
        let name = text(avm, player, &name).to_ascii_lowercase();
        let image = player.levels.values().find_map(|&root| {
            let lib = &player.clip(root)?.lib;
            lib.images.get(lib.exports.get(&name)?).cloned()
        });
        let Some(image) = image else {
            return Value::Undefined;
        };
        let o = avm.object();
        {
            let mut b = o.borrow_mut();
            b.set("width", Value::Number(f64::from(image.width)));
            b.set("height", Value::Number(f64::from(image.height)));
            b.set("texture", Value::Str(image.texture.as_str().into()));
            for (k, v) in ["u0", "v0", "u1", "v1"].into_iter().zip(image.part) {
                b.set(k, Value::Number(f64::from(v)));
            }
        }
        Value::Object(o)
    });
    let display = avm.object();
    display
        .borrow_mut()
        .set("BitmapData", Value::Object(bitmap_data));
    let flash = avm.object();
    flash.borrow_mut().set("display", Value::Object(display));
    g.borrow_mut().set("flash", Value::Object(flash));

    // Key, Mouse, Stage.
    let key = avm.object();
    for (n, v) in [
        ("LEFT", 37),
        ("UP", 38),
        ("RIGHT", 39),
        ("DOWN", 40),
        ("ENTER", 13),
        ("SPACE", 32),
        ("ESCAPE", 27),
        ("TAB", 9),
        ("SHIFT", 16),
        ("CONTROL", 17),
    ] {
        key.borrow_mut().set(n, Value::Number(f64::from(v)));
    }
    def(avm, &key, "isDown", |avm, p, _, a| {
        let code = num(avm, p, a, 0) as i32;
        Value::Bool(avm.keys_down.contains(&code))
    });
    def(avm, &key, "isToggled", |_, _, _, _| Value::Bool(false));
    def(avm, &key, "getCode", |avm, _, _, _| {
        Value::Number(f64::from(avm.key_code))
    });
    def(avm, &key, "getAscii", |_, _, _, _| Value::Number(0.0));
    def(avm, &key, "addListener", |avm, _, _, a| {
        let l = arg(a, 0);
        if matches!(l, Value::Object(_)) && !avm.key_listeners.iter().any(|k| same_object(k, &l)) {
            avm.key_listeners.push(l);
        }
        Value::Undefined
    });
    def(avm, &key, "removeListener", |avm, _, _, a| {
        let l = arg(a, 0);
        let before = avm.key_listeners.len();
        avm.key_listeners.retain(|k| !same_object(k, &l));
        Value::Bool(avm.key_listeners.len() != before)
    });
    g.borrow_mut().set("Key", Value::Object(key));
    let mouse = avm.object();
    for n in ["hide", "show"] {
        def(avm, &mouse, n, |_, _, _, _| Value::Undefined);
    }
    def(avm, &mouse, "addListener", |avm, _, _, a| {
        let l = arg(a, 0);
        if matches!(l, Value::Object(_)) && !avm.mouse_listeners.iter().any(|k| same_object(k, &l))
        {
            avm.mouse_listeners.push(l);
        }
        Value::Undefined
    });
    def(avm, &mouse, "removeListener", |avm, _, _, a| {
        let l = arg(a, 0);
        let before = avm.mouse_listeners.len();
        avm.mouse_listeners.retain(|k| !same_object(k, &l));
        Value::Bool(avm.mouse_listeners.len() != before)
    });
    g.borrow_mut().set("Mouse", Value::Object(mouse));
    let stage = avm.object();
    def(avm, &stage, "addListener", |_, _, _, _| Value::Undefined);
    def(avm, &stage, "removeListener", |_, _, _, _| {
        Value::Bool(true)
    });
    g.borrow_mut().set("Stage", Value::Object(stage));

    install_array(avm);
    install_string(avm);
    install_clip(avm);
}

fn set_proto(ctor: &Value, proto: &Obj) {
    if let Value::Object(c) = ctor {
        c.borrow_mut()
            .set("prototype", Value::Object(Rc::clone(proto)));
        proto.borrow_mut().set("constructor", ctor.clone());
    }
}

/// `Object.prototype.toString`, and what numbers and strings answer.
pub fn object_to_string(avm: &mut Avm, player: &mut Player, this: &Value, _: &[Value]) -> Value {
    match this {
        Value::Object(o) => {
            let kind = o.borrow().kind.clone();
            match kind {
                ObjKind::Plain => Value::from("[object Object]"),
                _ => Value::Str(avm.to_str(player, this)),
            }
        }
        other => Value::Str(avm.to_str(player, other)),
    }
}

thread_local! {
    static SEED: Cell<u64> = const { Cell::new(0x2545_f491_4f6c_dd1d) };
}

/// A pseudo-random number in 0..1 (the same sequence every run).
pub fn random() -> f64 {
    SEED.with(|s| {
        let mut x = s.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.set(x);
        (x >> 11) as f64 / (1u64 << 53) as f64
    })
}

fn set_interval(avm: &mut Avm, player: &mut Player, _: &Value, args: &[Value]) -> Value {
    let (call, rest) = match (arg(args, 0), arg(args, 1)) {
        (f @ Value::Object(_), Value::Str(m)) => {
            ((f, Some(m.to_string())), args.get(2..).unwrap_or(&[]))
        }
        (f, _) => ((f, None), args.get(1..).unwrap_or(&[])),
    };
    let ms = rest.first().map_or(0.0, |v| avm.to_number(player, v));
    let id = avm.next_interval;
    avm.next_interval += 1;
    let every = (if ms.is_nan() { 0.0 } else { ms } / 1000.0).max(0.001);
    avm.intervals.push(Interval {
        id,
        due: avm.time + every,
        every,
        call,
        args: rest.get(1..).unwrap_or(&[]).to_vec(),
    });
    Value::Number(f64::from(id))
}

fn color_target(avm: &mut Avm, player: &mut Player, this: &Value) -> Option<ClipId> {
    let t = avm.get(player, this, "target");
    clip_id(&t)
}

fn color_set_rgb(avm: &mut Avm, player: &mut Player, this: &Value, args: &[Value]) -> Value {
    let rgb = num(avm, player, args, 0) as u32;
    if let Some(id) = color_target(avm, player, this)
        && let Some(c) = player.clip_mut(id)
    {
        for (i, shift) in [16, 8, 0].into_iter().enumerate() {
            c.colour.mult[i] = 0.0;
            c.colour.add[i] = ((rgb >> shift) & 255) as f32 / 255.0;
        }
    }
    Value::Undefined
}

fn color_set_transform(avm: &mut Avm, player: &mut Player, this: &Value, args: &[Value]) -> Value {
    let t = arg(args, 0);
    let Some(id) = color_target(avm, player, this) else {
        return Value::Undefined;
    };
    let mut fields = [None; 8];
    for (i, n) in ["ra", "rb", "ga", "gb", "ba", "bb", "aa", "ab"]
        .iter()
        .enumerate()
    {
        let v = avm.get(player, &t, n);
        if !matches!(v, Value::Undefined) {
            fields[i] = Some(avm.to_number(player, &v) as f32);
        }
    }
    if let Some(c) = player.clip_mut(id) {
        for ch in 0..4 {
            if let Some(m) = fields[ch * 2] {
                c.colour.mult[ch] = m / 100.0;
            }
            if let Some(a) = fields[ch * 2 + 1] {
                c.colour.add[ch] = a / 255.0;
            }
        }
    }
    Value::Undefined
}

// ---- Array ------------------------------------------------------------

fn items(this: &Value) -> Vec<Value> {
    match this {
        Value::Object(o) => match &o.borrow().kind {
            ObjKind::Array(a) => a.clone(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn set_items(this: &Value, v: Vec<Value>) {
    if let Value::Object(o) = this
        && let ObjKind::Array(a) = &mut o.borrow_mut().kind
    {
        *a = v;
    }
}

fn index(
    avm: &mut Avm,
    player: &mut Player,
    v: Option<&Value>,
    len: usize,
    default: usize,
) -> usize {
    let Some(v) = v.filter(|v| !matches!(v, Value::Undefined)) else {
        return default;
    };
    let n = avm.to_number(player, v);
    if n.is_nan() {
        return 0;
    }
    let n = n as i64;
    if n < 0 {
        (len as i64 + n).max(0) as usize
    } else {
        (n as usize).min(len)
    }
}

fn install_array(avm: &mut Avm) {
    let ap = Rc::clone(&avm.array_proto);
    def(avm, &ap, "push", |_, _, this, args| {
        let mut v = items(this);
        v.extend(args.iter().cloned());
        let n = v.len();
        set_items(this, v);
        Value::Number(n as f64)
    });
    def(avm, &ap, "pop", |_, _, this, _| {
        let mut v = items(this);
        let r = v.pop().unwrap_or_default();
        set_items(this, v);
        r
    });
    def(avm, &ap, "shift", |_, _, this, _| {
        let mut v = items(this);
        let r = if v.is_empty() {
            Value::Undefined
        } else {
            v.remove(0)
        };
        set_items(this, v);
        r
    });
    def(avm, &ap, "unshift", |_, _, this, args| {
        let mut v = args.to_vec();
        v.extend(items(this));
        let n = v.len();
        set_items(this, v);
        Value::Number(n as f64)
    });
    def(avm, &ap, "splice", |avm, player, this, args| {
        let mut v = items(this);
        let start = index(avm, player, args.first(), v.len(), 0);
        let count = match args.get(1) {
            Some(c) => (avm.to_number(player, c).max(0.0) as usize).min(v.len() - start),
            None => v.len() - start,
        };
        let removed: Vec<Value> = v
            .splice(
                start..start + count,
                args.get(2..).unwrap_or(&[]).iter().cloned(),
            )
            .collect();
        set_items(this, v);
        avm.array(removed)
    });
    def(avm, &ap, "slice", |avm, player, this, args| {
        let v = items(this);
        let s = index(avm, player, args.first(), v.len(), 0);
        let e = index(avm, player, args.get(1), v.len(), v.len());
        avm.array(if s < e { v[s..e].to_vec() } else { Vec::new() })
    });
    def(avm, &ap, "concat", |avm, _, this, args| {
        let mut v = items(this);
        for a in args {
            match a {
                Value::Object(o) if matches!(o.borrow().kind, ObjKind::Array(_)) => {
                    v.extend(items(a))
                }
                other => v.push(other.clone()),
            }
        }
        avm.array(v)
    });
    def(avm, &ap, "join", |avm, player, this, args| {
        let sep = match args.first() {
            Some(Value::Undefined) | None => ",".to_owned(),
            Some(s) => text(avm, player, s),
        };
        let v = items(this);
        let parts: Vec<String> = v
            .iter()
            .map(|i| match i {
                Value::Undefined | Value::Null => String::new(),
                other => text(avm, player, other),
            })
            .collect();
        Value::Str(parts.join(&sep).into())
    });
    def(avm, &ap, "toString", |avm, player, this, _| {
        Value::Str(avm.to_str(player, this))
    });
    def(avm, &ap, "reverse", |_, _, this, _| {
        let mut v = items(this);
        v.reverse();
        set_items(this, v);
        this.clone()
    });
    def(avm, &ap, "sort", array_sort);
    def(avm, &ap, "sortOn", |avm, player, this, args| {
        let field = arg(args, 0);
        let field = text(avm, player, &field);
        let opts = if args.len() > 1 {
            num(avm, player, args, 1) as u32
        } else {
            0
        };
        let mut v: Vec<(Value, Value)> = items(this)
            .into_iter()
            .map(|i| {
                let k = avm.get(player, &i, &field);
                (k, i)
            })
            .collect();
        let numeric = opts & 16 != 0;
        let mut keyed: Vec<(f64, String, Value)> = v
            .drain(..)
            .map(|(k, i)| (avm.to_number(player, &k), text(avm, player, &k), i))
            .collect();
        keyed.sort_by(|a, b| {
            if numeric {
                a.0.total_cmp(&b.0)
            } else if opts & 1 != 0 {
                a.1.to_lowercase().cmp(&b.1.to_lowercase())
            } else {
                a.1.cmp(&b.1)
            }
        });
        if opts & 2 != 0 {
            keyed.reverse();
        }
        set_items(this, keyed.into_iter().map(|k| k.2).collect());
        this.clone()
    });
}

fn array_sort(avm: &mut Avm, player: &mut Player, this: &Value, args: &[Value]) -> Value {
    let mut v = items(this);
    let (compare, opts) = match (arg(args, 0), arg(args, 1)) {
        (f @ Value::Object(_), o) => (Some(f), avm.to_number(player, &o)),
        (o, _) => (None, avm.to_number(player, &o)),
    };
    let opts = if opts.is_nan() { 0 } else { opts as u32 };
    // An insertion sort: the compare function may be a script.
    for i in 1..v.len() {
        let mut j = i;
        while j > 0 {
            let less = match &compare {
                Some(f) => {
                    let r = avm.call_value(
                        player,
                        f,
                        &Value::Undefined,
                        &[v[j].clone(), v[j - 1].clone()],
                    );
                    avm.to_number(player, &r) < 0.0
                }
                None if opts & 16 != 0 => {
                    avm.to_number(player, &v[j]) < avm.to_number(player, &v[j - 1])
                }
                None => {
                    let (a, b) = (text(avm, player, &v[j]), text(avm, player, &v[j - 1]));
                    if opts & 1 != 0 {
                        a.to_lowercase() < b.to_lowercase()
                    } else {
                        a < b
                    }
                }
            };
            if !less {
                break;
            }
            v.swap(j, j - 1);
            j -= 1;
        }
    }
    if opts & 2 != 0 {
        v.reverse();
    }
    set_items(this, v);
    this.clone()
}

// ---- String -----------------------------------------------------------

fn install_string(avm: &mut Avm) {
    let sp = Rc::clone(&avm.string_proto);
    def(avm, &sp, "toString", |avm, player, this, _| {
        Value::Str(avm.to_str(player, this))
    });
    def(avm, &sp, "valueOf", |avm, player, this, _| {
        Value::Str(avm.to_str(player, this))
    });
    def(avm, &sp, "toUpperCase", |avm, player, this, _| {
        Value::Str(text(avm, player, this).to_uppercase().into())
    });
    def(avm, &sp, "toLowerCase", |avm, player, this, _| {
        Value::Str(text(avm, player, this).to_lowercase().into())
    });
    def(avm, &sp, "charAt", |avm, player, this, args| {
        let s = text(avm, player, this);
        let i = num(avm, player, args, 0);
        let c: String = if i < 0.0 || i.is_nan() {
            String::new()
        } else {
            s.chars().nth(i as usize).into_iter().collect()
        };
        Value::Str(c.into())
    });
    def(avm, &sp, "charCodeAt", |avm, player, this, args| {
        let s = text(avm, player, this);
        let i = num(avm, player, args, 0);
        Value::Number(
            s.chars()
                .nth(i.max(0.0) as usize)
                .map_or(f64::NAN, |c| f64::from(c as u32)),
        )
    });
    def(avm, &sp, "indexOf", |avm, player, this, args| {
        let s: Vec<char> = text(avm, player, this).chars().collect();
        let n = arg(args, 0);
        let n: Vec<char> = text(avm, player, &n).chars().collect();
        let from = if args.len() > 1 {
            num(avm, player, args, 1).max(0.0) as usize
        } else {
            0
        };
        let r = (from..=s.len().saturating_sub(n.len()))
            .find(|&i| s.get(i..i + n.len()) == Some(&n[..]));
        Value::Number(r.map_or(-1.0, |i| i as f64))
    });
    def(avm, &sp, "lastIndexOf", |avm, player, this, args| {
        let s: Vec<char> = text(avm, player, this).chars().collect();
        let n = arg(args, 0);
        let n: Vec<char> = text(avm, player, &n).chars().collect();
        let r = (0..=s.len().saturating_sub(n.len()))
            .rev()
            .find(|&i| s.get(i..i + n.len()) == Some(&n[..]));
        Value::Number(r.map_or(-1.0, |i| i as f64))
    });
    def(avm, &sp, "substring", |avm, player, this, args| {
        let s: Vec<char> = text(avm, player, this).chars().collect();
        let clamp = |v: f64| {
            if v.is_nan() {
                0
            } else {
                (v.max(0.0) as usize).min(s.len())
            }
        };
        let a = clamp(num(avm, player, args, 0));
        let b = if args.len() > 1 && !matches!(args[1], Value::Undefined) {
            clamp(num(avm, player, args, 1))
        } else {
            s.len()
        };
        let (a, b) = (a.min(b), a.max(b));
        Value::Str(s[a..b].iter().collect::<String>().into())
    });
    def(avm, &sp, "substr", |avm, player, this, args| {
        let s: Vec<char> = text(avm, player, this).chars().collect();
        let start = index(avm, player, args.first(), s.len(), 0);
        let n = if args.len() > 1 {
            num(avm, player, args, 1).max(0.0) as usize
        } else {
            s.len()
        };
        let end = (start + n).min(s.len());
        Value::Str(s[start..end].iter().collect::<String>().into())
    });
    def(avm, &sp, "slice", |avm, player, this, args| {
        let s: Vec<char> = text(avm, player, this).chars().collect();
        let a = index(avm, player, args.first(), s.len(), 0);
        let b = index(avm, player, args.get(1), s.len(), s.len());
        Value::Str(
            if a < b {
                s[a..b].iter().collect::<String>()
            } else {
                String::new()
            }
            .into(),
        )
    });
    def(avm, &sp, "split", |avm, player, this, args| {
        let s = text(avm, player, this);
        let parts: Vec<Value> = match args.first() {
            None | Some(Value::Undefined) => vec![Value::Str(s.into())],
            Some(sep) => {
                let sep = text(avm, player, sep);
                if sep.is_empty() {
                    s.chars()
                        .map(|c| Value::Str(c.to_string().into()))
                        .collect()
                } else {
                    s.split(sep.as_str())
                        .map(|p| Value::Str(p.into()))
                        .collect()
                }
            }
        };
        avm.array(parts)
    });
    def(avm, &sp, "concat", |avm, player, this, args| {
        let mut s = text(avm, player, this);
        for a in args {
            s.push_str(&text(avm, player, a));
        }
        Value::Str(s.into())
    });
}

// ---- MovieClip ----------------------------------------------------------

fn this_clip(this: &Value) -> Option<ClipId> {
    clip_id(this)
}

fn frame_arg(avm: &mut Avm, player: &mut Player, id: ClipId, v: &Value) -> Option<usize> {
    match v {
        Value::Number(n) => Some((*n as i64 - 1).max(0) as usize),
        other => {
            let s = text(avm, player, other);
            player.clip(id)?.timeline.as_ref()?.frame_of(&s)
        }
    }
}

fn goto(avm: &mut Avm, player: &mut Player, this: &Value, args: &[Value], play: bool) -> Value {
    if let Some(id) = this_clip(this) {
        let f = arg(args, 0);
        if avm.trace {
            let name = player.clip(id).map_or("", |c| c.name.as_str());
            avm.log.push(format!("{name}.goto {f:?}"));
        }
        if let Some(n) = frame_arg(avm, player, id, &f) {
            player.goto(id, n);
        }
        if let Some(c) = player.clip_mut(id) {
            c.playing = play;
        }
    }
    Value::Undefined
}

/// A copy of `src` at `depth` under its parent, named `name`.
pub fn duplicate(avm: &mut Avm, player: &mut Player, src: ClipId, name: &str, depth: i32) -> Value {
    let Some(s) = player.clip(src).cloned() else {
        return Value::Undefined;
    };
    let Some(parent) = s.parent else {
        return Value::Undefined;
    };
    let id = place_new(player, parent, depth, s.character, name);
    if let Some(c) = player.clip_mut(id) {
        c.matrix = s.matrix;
        c.colour = s.colour;
        c.drawing = s.drawing.clone();
    }
    Value::Object(avm.clip_object(id))
}

fn place_new(
    player: &mut Player,
    parent: ClipId,
    depth: i32,
    character: u16,
    name: &str,
) -> ClipId {
    if let Some(old) = player
        .clip(parent)
        .and_then(|c| c.children.get(&depth))
        .copied()
    {
        player.remove(parent, depth, old);
    }
    let id = player.add(parent, depth, character);
    if let Some(c) = player.clip_mut(id) {
        c.name = name.to_owned();
    }
    if player.clip(id).is_some_and(|c| c.timeline.is_some()) {
        player.goto(id, 0);
    }
    id
}

/// Takes a clip a script placed off the stage (not one a timeline placed).
pub fn remove_clip(player: &mut Player, id: ClipId) {
    let Some(c) = player.clip(id) else { return };
    if c.from_timeline {
        return;
    }
    if let Some(parent) = c.parent {
        let depth = c.depth;
        player.remove(parent, depth, id);
    }
}

fn pen<'a>(avm: &'a mut Avm, id: ClipId) -> &'a mut Pen {
    avm.pens.entry(id).or_default()
}

fn colour_of(
    avm: &mut Avm,
    player: &mut Player,
    args: &[Value],
    rgb: usize,
    alpha: usize,
) -> [f32; 4] {
    let c = num(avm, player, args, rgb);
    let c = if c.is_nan() { 0 } else { c as u32 };
    let a = if args.len() > alpha {
        num(avm, player, args, alpha)
    } else {
        100.0
    };
    let a = if a.is_nan() { 100.0 } else { a };
    [
        ((c >> 16) & 255) as f32 / 255.0,
        ((c >> 8) & 255) as f32 / 255.0,
        (c & 255) as f32 / 255.0,
        (a / 100.0).clamp(0.0, 1.0) as f32,
    ]
}

fn draw_to(avm: &mut Avm, player: &mut Player, id: ClipId, to: [f32; 2]) {
    let p = pen(avm, id);
    let from = p.at;
    p.at = to;
    if p.fill.is_some() {
        p.path.push((from, to));
    }
    if let Some((w, colour)) = p.line
        && let Some(c) = player.clip_mut(id)
    {
        for corners in super::super::stroke(from, to, w.max(1.0) / 2.0) {
            push_clipped(&mut c.drawing, corners, [[0.0; 2]; 3], colour, None, None);
        }
    }
}

fn install_clip(avm: &mut Avm) {
    let cp = Rc::clone(&avm.clip_proto);
    def(avm, &cp, "gotoAndStop", |a, p, t, x| {
        goto(a, p, t, x, false)
    });
    def(avm, &cp, "gotoAndPlay", |a, p, t, x| goto(a, p, t, x, true));
    def(avm, &cp, "play", |_, p, t, _| {
        if let Some(c) = this_clip(t).and_then(|i| p.clip_mut(i)) {
            c.playing = true;
        }
        Value::Undefined
    });
    def(avm, &cp, "stop", |_, p, t, _| {
        if let Some(c) = this_clip(t).and_then(|i| p.clip_mut(i)) {
            c.playing = false;
        }
        Value::Undefined
    });
    def(avm, &cp, "nextFrame", |_, p, t, _| {
        if let Some(id) = this_clip(t) {
            let f = p.clip(id).map_or(0, |c| c.frame + 1);
            p.goto(id, f);
            if let Some(c) = p.clip_mut(id) {
                c.playing = false;
            }
        }
        Value::Undefined
    });
    def(avm, &cp, "prevFrame", |_, p, t, _| {
        if let Some(id) = this_clip(t) {
            let f = p.clip(id).map_or(0, |c| c.frame.saturating_sub(1));
            p.goto(id, f);
            if let Some(c) = p.clip_mut(id) {
                c.playing = false;
            }
        }
        Value::Undefined
    });
    def(avm, &cp, "attachMovie", |avm, player, this, args| {
        let Some(parent) = this_clip(this) else {
            return Value::Undefined;
        };
        let (link, name) = (arg(args, 0), arg(args, 1));
        let (link, name) = (text(avm, player, &link), text(avm, player, &name));
        let depth = num(avm, player, args, 2);
        let depth = if depth.is_nan() { 0 } else { depth as i32 };
        let exports = player.clip(parent).map(|c| Rc::clone(&c.lib));
        let Some(&character) = exports
            .as_ref()
            .and_then(|l| l.exports.get(&link.to_ascii_lowercase()))
        else {
            avm.log.push(format!("attachMovie {link}: no such symbol"));
            return Value::Undefined;
        };
        let id = place_new(player, parent, depth, character, &name);
        let obj = Value::Object(avm.clip_object(id));
        if let Some(Value::Object(init)) = args.get(3) {
            let keys = init.borrow().keys();
            for k in keys {
                let v = init.borrow().own(&k).unwrap_or_default();
                avm.set(player, &obj, &k, v);
            }
        }
        obj
    });
    def(
        avm,
        &cp,
        "createEmptyMovieClip",
        |avm, player, this, args| {
            let Some(parent) = this_clip(this) else {
                return Value::Undefined;
            };
            let name = arg(args, 0);
            let name = text(avm, player, &name);
            let depth = num(avm, player, args, 1);
            let depth = if depth.is_nan() { 0 } else { depth as i32 };
            let id = player.add_empty(parent, depth, &name);
            Value::Object(avm.clip_object(id))
        },
    );
    def(avm, &cp, "createTextField", |avm, player, this, args| {
        // No text definition to show it with: an empty clip, placed.
        let Some(parent) = this_clip(this) else {
            return Value::Undefined;
        };
        let name = arg(args, 0);
        let name = text(avm, player, &name);
        let depth = num(avm, player, args, 1) as i32;
        let (x, y) = (
            num(avm, player, args, 2) as f32,
            num(avm, player, args, 3) as f32,
        );
        let id = player.add_empty(parent, depth, &name);
        if let Some(c) = player.clip_mut(id) {
            c.matrix = Matrix {
                tx: x,
                ty: y,
                ..Matrix::IDENTITY
            };
        }
        Value::Object(avm.clip_object(id))
    });
    def(avm, &cp, "duplicateMovieClip", |avm, player, this, args| {
        let Some(src) = this_clip(this) else {
            return Value::Undefined;
        };
        let name = arg(args, 0);
        let name = text(avm, player, &name);
        let depth = num(avm, player, args, 1) as i32;
        duplicate(avm, player, src, &name, depth)
    });
    def(avm, &cp, "removeMovieClip", |_, player, this, _| {
        if let Some(id) = this_clip(this) {
            remove_clip(player, id);
        }
        Value::Undefined
    });
    def(avm, &cp, "getNextHighestDepth", |_, player, this, _| {
        let d = this_clip(this)
            .and_then(|id| player.clip(id))
            .and_then(|c| c.children.keys().next_back().copied())
            .map_or(0, |d| (d + 1).max(0));
        Value::Number(f64::from(d))
    });
    def(avm, &cp, "getDepth", |_, player, this, _| {
        Value::Number(
            this_clip(this)
                .and_then(|id| player.clip(id))
                .map_or(0.0, |c| f64::from(c.depth)),
        )
    });
    def(avm, &cp, "getInstanceAtDepth", |avm, player, this, args| {
        let d = num(avm, player, args, 0) as i32;
        let k = this_clip(this)
            .and_then(|id| player.clip(id))
            .and_then(|c| c.children.get(&d).copied());
        avm.clip_value(player, k)
    });
    def(avm, &cp, "swapDepths", |avm, player, this, args| {
        let Some(id) = this_clip(this) else {
            return Value::Undefined;
        };
        let Some((parent, from)) = player.clip(id).and_then(|c| c.parent.map(|p| (p, c.depth)))
        else {
            return Value::Undefined;
        };
        let to = match arg(args, 0) {
            v @ Value::Object(_) => clip_id(&v).and_then(|o| player.clip(o)).map(|c| c.depth),
            v => {
                let n = avm.to_number(player, &v);
                (!n.is_nan()).then_some(n as i32)
            }
        };
        let Some(to) = to else {
            return Value::Undefined;
        };
        let other = player
            .clip(parent)
            .and_then(|p| p.children.get(&to).copied());
        if let Some(p) = player.clip_mut(parent) {
            p.children.remove(&from);
            if let Some(o) = other {
                p.children.insert(from, o);
            }
            p.children.insert(to, id);
        }
        if let Some(o) = other.and_then(|o| player.clip_mut(o)) {
            o.depth = from;
            o.from_timeline = false;
        }
        if let Some(c) = player.clip_mut(id) {
            c.depth = to;
            c.from_timeline = false;
        }
        Value::Undefined
    });
    // `hitTest(x, y, shapeFlag)` (stage pixels) or `hitTest(target)`: the
    // two boxes overlap.
    def(avm, &cp, "hitTest", |avm, player, this, a| {
        let Some(id) = this_clip(this) else {
            return Value::Bool(false);
        };
        if a.len() >= 2 {
            let x = num(avm, player, a, 0) as f32;
            let y = num(avm, player, a, 1) as f32;
            let shape = a.get(2).is_some_and(|v| avm.truthy(v));
            return Value::Bool(super::hit_test(player, id, [x, y], shape));
        }
        let other = clip_id(&arg(a, 0));
        let boxes =
            other.and_then(|o| super::world_bounds(player, o).zip(super::world_bounds(player, id)));
        Value::Bool(
            boxes
                .is_some_and(|(p, q)| p[0] <= q[2] && q[0] <= p[2] && p[1] <= q[3] && q[1] <= p[3]),
        )
    });
    def(avm, &cp, "getBounds", |avm, player, this, _| {
        let o = avm.object();
        let b = this_clip(this).and_then(|id| {
            let saved = player.clip(id)?.matrix;
            if let Some(c) = player.clip_mut(id) {
                c.matrix = Matrix::IDENTITY;
            }
            let b = super::clip_bounds(player, id);
            if let Some(c) = player.clip_mut(id) {
                c.matrix = saved;
            }
            b
        });
        let b = b.unwrap_or([0.0; 4]);
        for (k, v) in ["xMin", "yMin", "xMax", "yMax"].iter().zip(b) {
            o.borrow_mut().set(k, Value::Number(f64::from(v)));
        }
        Value::Object(o)
    });
    def(avm, &cp, "localToGlobal", |avm, player, this, args| {
        convert_point(avm, player, this, args, false);
        Value::Undefined
    });
    def(avm, &cp, "globalToLocal", |avm, player, this, args| {
        convert_point(avm, player, this, args, true);
        Value::Undefined
    });
    def(avm, &cp, "lineStyle", |avm, player, this, args| {
        if let Some(id) = this_clip(this) {
            let line = if args.is_empty() || matches!(args[0], Value::Undefined) {
                None
            } else {
                let w = num(avm, player, args, 0) as f32;
                Some((
                    if w.is_nan() { 1.0 } else { w },
                    colour_of(avm, player, args, 1, 2),
                ))
            };
            pen(avm, id).line = line;
        }
        Value::Undefined
    });
    def(avm, &cp, "beginFill", |avm, player, this, args| {
        if let Some(id) = this_clip(this) {
            avm.flush_fill(player, id);
            let c = (!args.is_empty()).then(|| colour_of(avm, player, args, 0, 1));
            let p = pen(avm, id);
            p.fill = c;
            p.path.clear();
        }
        Value::Undefined
    });
    def(avm, &cp, "endFill", |avm, player, this, _| {
        if let Some(id) = this_clip(this) {
            avm.flush_fill(player, id);
            pen(avm, id).fill = None;
        }
        Value::Undefined
    });
    def(avm, &cp, "moveTo", |avm, player, this, args| {
        if let Some(id) = this_clip(this) {
            let at = [
                num(avm, player, args, 0) as f32,
                num(avm, player, args, 1) as f32,
            ];
            let p = pen(avm, id);
            // A new contour of the same fill: close the last one.
            if p.fill.is_some()
                && let (Some(first), Some(last)) =
                    (p.path.first().map(|s| s.0), p.path.last().map(|s| s.1))
                && first != last
            {
                p.path.push((last, first));
            }
            p.at = at;
        }
        Value::Undefined
    });
    def(avm, &cp, "lineTo", |avm, player, this, args| {
        if let Some(id) = this_clip(this) {
            let to = [
                num(avm, player, args, 0) as f32,
                num(avm, player, args, 1) as f32,
            ];
            draw_to(avm, player, id, to);
        }
        Value::Undefined
    });
    def(avm, &cp, "curveTo", |avm, player, this, args| {
        if let Some(id) = this_clip(this) {
            let c = [
                num(avm, player, args, 0) as f32,
                num(avm, player, args, 1) as f32,
            ];
            let to = [
                num(avm, player, args, 2) as f32,
                num(avm, player, args, 3) as f32,
            ];
            let from = pen(avm, id).at;
            for i in 1..=8 {
                let t = i as f32 / 8.0;
                let u = 1.0 - t;
                let p =
                    std::array::from_fn(|k| u * u * from[k] + 2.0 * u * t * c[k] + t * t * to[k]);
                draw_to(avm, player, id, p);
            }
        }
        Value::Undefined
    });
    def(avm, &cp, "clear", |avm, player, this, _| {
        if let Some(id) = this_clip(this) {
            avm.pens.remove(&id);
            if let Some(c) = player.clip_mut(id) {
                c.drawing.clear();
            }
        }
        Value::Undefined
    });
    def(avm, &cp, "loadMovie", |avm, player, this, args| {
        if let Some(id) = this_clip(this) {
            let url = arg(args, 0);
            let url = text(avm, player, &url);
            avm.loads.push((url, String::new(), id));
        }
        Value::Undefined
    });
    for n in [
        "setMask",
        "startDrag",
        "stopDrag",
        "setTextFormat",
        "setNewTextFormat",
        "unloadMovie",
    ] {
        def(avm, &cp, n, |_, _, _, _| Value::Undefined);
    }
    // Scaleform's inline images in text: `{ subString, image, width,
    // height, baseLineY, id }` each (an array of them, or one).
    def(
        avm,
        &cp,
        "setImageSubstitutions",
        |avm, player, this, args| {
            let Some(id) = clip_id(this) else {
                return Value::Undefined;
            };
            let list = match arg(args, 0) {
                Value::Object(o) => match &o.borrow().kind {
                    ObjKind::Array(items) => items.clone(),
                    _ => vec![Value::Object(Rc::clone(&o))],
                },
                _ => Vec::new(),
            };
            let mut subs = Vec::new();
            for item in &list {
                if let Some(sub) = substitution(avm, player, item) {
                    subs.push(sub);
                }
            }
            if let Some(c) = player.clip_mut(id) {
                c.substitutions = subs;
            }
            Value::Undefined
        },
    );
    // (id, image): that substitution shows another image.
    def(
        avm,
        &cp,
        "updateImageSubstitution",
        |avm, player, this, args| {
            let (Some(clip), name) = (clip_id(this), text(avm, player, &arg(args, 0))) else {
                return Value::Undefined;
            };
            let image = arg(args, 1);
            let texture = avm.get(player, &image, "texture");
            let part = ["u0", "v0", "u1", "v1"].map(|k| {
                let v = avm.get(player, &image, k);
                avm.to_number(player, &v) as f32
            });
            let texture = match texture {
                Value::Str(t) => Some(t.to_string()),
                _ => None,
            };
            if let Some(c) = player.clip_mut(clip) {
                for s in c.substitutions.iter_mut().filter(|s| s.id == name) {
                    match &texture {
                        Some(t) => {
                            s.texture = t.clone();
                            s.part = part;
                        }
                        None => s.texture.clear(),
                    }
                }
            }
            Value::Undefined
        },
    );
    def(avm, &cp, "getTextFormat", |avm, _, _, _| {
        Value::Object(avm.object())
    });
    def(avm, &cp, "getBytesLoaded", |_, _, _, _| Value::Number(1.0));
    def(avm, &cp, "getBytesTotal", |_, _, _, _| Value::Number(1.0));
    let _ = Kind::Sprite;
    let _ = TIMELINE_DEPTH;
    let _ = Object::new;
}

fn convert_point(avm: &mut Avm, player: &mut Player, this: &Value, args: &[Value], inverse: bool) {
    let Some(id) = this_clip(this) else { return };
    let pt = arg(args, 0);
    let x = avm.get(player, &pt, "x");
    let y = avm.get(player, &pt, "y");
    let p = [
        avm.to_number(player, &x) as f32,
        avm.to_number(player, &y) as f32,
    ];
    let m = player.world_matrix(id);
    let m = if inverse {
        m.inverse().unwrap_or(Matrix::IDENTITY)
    } else {
        m
    };
    let [qx, qy] = m.apply(p);
    avm.set(player, &pt, "x", Value::Number(f64::from(qx)));
    avm.set(player, &pt, "y", Value::Number(f64::from(qy)));
    let _ = number_str;
}
