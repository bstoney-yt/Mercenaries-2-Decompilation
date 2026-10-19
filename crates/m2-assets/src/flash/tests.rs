use std::rc::Rc;

use super::avm1::{ObjKind, Object, Runtime, Value};
use super::player::{Control, Library, Place, Player, TIMELINE_DEPTH, Timeline};
use super::text::{EditText, Font, FontBook, plain};
use super::*;

/// Writes SWF fields: bytes little-endian, bit fields most significant
/// bit first.
#[derive(Default)]
struct Writer {
    bytes: Vec<u8>,
    bit: usize,
}

impl Writer {
    fn ub(&mut self, n: usize, v: u32) {
        for i in (0..n).rev() {
            if self.bit.is_multiple_of(8) {
                self.bytes.push(0);
            }
            if (v >> i) & 1 == 1 {
                let last = self.bytes.len() - 1;
                self.bytes[last] |= 0x80 >> (self.bit % 8);
            }
            self.bit += 1;
        }
    }
    fn sb(&mut self, n: usize, v: i32) {
        self.ub(n, (v as u32) & ((1u64 << n) - 1) as u32);
    }
    fn align(&mut self) {
        self.bit = self.bit.div_ceil(8) * 8;
    }
    fn u8(&mut self, v: u8) {
        self.align();
        self.bytes.push(v);
        self.bit += 8;
    }
    fn u16(&mut self, v: u16) {
        self.u8(v as u8);
        self.u8((v >> 8) as u8);
    }
    fn rect(&mut self, x0: i32, x1: i32, y0: i32, y1: i32) {
        self.align();
        self.ub(5, 16);
        for v in [x0, x1, y0, y1] {
            self.sb(16, v);
        }
        self.align();
    }
    fn style_change(
        &mut self,
        move_to: Option<(i32, i32)>,
        fill0: Option<u32>,
        fill1: Option<u32>,
    ) {
        self.ub(1, 0);
        self.ub(1, 0);
        self.ub(1, 0);
        self.ub(1, u32::from(fill1.is_some()));
        self.ub(1, u32::from(fill0.is_some()));
        self.ub(1, u32::from(move_to.is_some()));
        if let Some((x, y)) = move_to {
            self.ub(5, 16);
            self.sb(16, x);
            self.sb(16, y);
        }
        if let Some(f) = fill0 {
            self.ub(2, f);
        }
        if let Some(f) = fill1 {
            self.ub(2, f);
        }
    }
    fn line_to(&mut self, dx: i32, dy: i32) {
        self.ub(1, 1);
        self.ub(1, 1);
        self.ub(4, 14);
        self.ub(1, 1);
        self.sb(16, dx);
        self.sb(16, dy);
    }
    fn curve_to(&mut self, cx: i32, cy: i32, ax: i32, ay: i32) {
        self.ub(1, 1);
        self.ub(1, 0);
        self.ub(4, 14);
        for v in [cx, cy, ax, ay] {
            self.sb(16, v);
        }
    }
    fn end(&mut self) {
        self.ub(6, 0);
        self.align();
    }
}

/// A `DefineShape3` with one solid red fill and the edges `draw` writes.
fn shape(draw: impl FnOnce(&mut Writer)) -> Shape {
    let mut w = Writer::default();
    w.u16(7);
    w.rect(0, 400, 0, 400);
    w.u8(1);
    w.u8(0x00);
    for v in [255, 0, 0, 255] {
        w.u8(v);
    }
    w.u8(0);
    w.align();
    w.ub(4, 2);
    w.ub(4, 0);
    draw(&mut w);
    w.end();
    let (id, s) = Shape::parse(&w.bytes, 3);
    assert_eq!(id, 7);
    s
}

fn area(tris: &[ShapeTriangle]) -> f32 {
    tris.iter()
        .map(|t| {
            let [a, b, c] = t.corners;
            ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs() / 2.0
        })
        .sum()
}

fn square(w: &mut Writer, x: i32, y: i32, side: i32, fill0: Option<u32>, fill1: Option<u32>) {
    w.style_change(Some((x, y)), fill0, fill1);
    w.line_to(side, 0);
    w.line_to(0, side);
    w.line_to(-side, 0);
    w.line_to(0, -side);
}

#[test]
fn a_square_fills_its_area() {
    let s = shape(|w| square(w, 0, 0, 200, None, Some(1)));
    assert_eq!(s.edges.len(), 4);
    assert_eq!(s.fills[1], Fill::Solid([1.0, 0.0, 0.0, 1.0]));
    assert!((area(&s.mesh()) - 100.0).abs() < 1e-3);
    let s = shape(|w| square(w, 0, 0, 200, Some(1), None));
    assert!((area(&s.mesh()) - 100.0).abs() < 1e-3);
}

#[test]
fn an_inner_square_drawn_the_other_way_is_a_hole() {
    let s = shape(|w| {
        square(w, 0, 0, 400, None, Some(1));
        square(w, 100, 100, 200, Some(1), Some(0));
    });
    assert!(
        (area(&s.mesh()) - 300.0).abs() < 1e-3,
        "{}",
        area(&s.mesh())
    );
    let s = shape(|w| {
        square(w, 0, 0, 200, None, Some(1));
        w.style_change(Some((0, 100)), Some(1), Some(1));
        w.line_to(200, 0);
    });
    assert!((area(&s.mesh()) - 100.0).abs() < 1e-3);
}

#[test]
fn curves_are_flattened_inside_their_hull() {
    let s = shape(|w| {
        w.style_change(Some((0, 0)), None, Some(1));
        w.line_to(400, 0);
        w.curve_to(0, 400, -400, -400);
    });
    assert!(s.edges[1].control.is_some());
    let a = area(&s.mesh());
    assert!((a - 133.33).abs() < 8.0, "{a}");
}

#[test]
fn transforms_compose_and_invert() {
    let half = ColourTransform {
        mult: [0.5; 4],
        add: [0.0; 4],
    };
    let lift = ColourTransform {
        mult: [1.0; 4],
        add: [0.25, 0.0, 0.0, 0.0],
    };
    assert_eq!(half.then(&lift).apply([1.0; 4]), [0.625, 0.5, 0.5, 0.5]);
    let m = Matrix {
        a: 2.0,
        d: 2.0,
        tx: 5.0,
        ..Matrix::IDENTITY
    };
    let inv = m.inverse().unwrap();
    assert_eq!(inv.apply(m.apply([3.0, 4.0])), [3.0, 4.0]);
}

// ---- Timelines ----------------------------------------------------------

fn at(x: f32, y: f32) -> Matrix {
    Matrix {
        tx: x,
        ty: y,
        ..Matrix::IDENTITY
    }
}

fn put(depth: u16, character: u16, m: Matrix, name: &str) -> Control {
    Control::Place(Place {
        depth,
        character: Some(character),
        matrix: Some(m),
        name: (!name.is_empty()).then(|| name.to_owned()),
        ..Place::default()
    })
}

fn moved(depth: u16, m: Matrix) -> Control {
    Control::Place(Place {
        depth,
        matrix: Some(m),
        moving: true,
        ..Place::default()
    })
}

/// A library: shape 1 (a 20 px red square), sprite 2 (shape 1 on frame
/// one, moved on frame two, a label "two"), and `root` as the movie.
fn library(root: Vec<Vec<Control>>, sprite: Vec<Vec<Control>>) -> Rc<Library> {
    let mut lib = Library {
        size: [100.0, 100.0],
        frame_rate: 10.0,
        ..Library::default()
    };
    let s = shape(|w| square(w, 0, 0, 400, None, Some(1)));
    let mesh = s.mesh();
    lib.shapes.insert(1, (s, mesh));
    let mut sprite = Timeline {
        frames: sprite,
        ..Timeline::default()
    };
    sprite.labels.insert("two".into(), 1);
    lib.sprites.insert(2, Rc::new(sprite));
    lib.exports.insert("box".into(), 2);
    lib.root = Rc::new(Timeline {
        frames: root,
        ..Timeline::default()
    });
    Rc::new(lib)
}

fn sprite_frames() -> Vec<Vec<Control>> {
    vec![
        vec![put(1, 1, at(0.0, 0.0), "square")],
        vec![moved(1, at(50.0, 0.0))],
    ]
}

#[test]
fn timelines_play_frame_by_frame_and_loop() {
    let lib = library(
        vec![
            vec![put(1, 2, at(10.0, 10.0), "clip")],
            vec![],
            vec![Control::Remove(1)],
        ],
        sprite_frames(),
    );
    let mut p = Player::new(lib);
    let clip = p.child(p.root, "clip").unwrap();
    let square = p.child(clip, "square").unwrap();
    assert_eq!(p.clip(square).unwrap().matrix.tx, 0.0);
    // A tenth of a second a frame.
    assert_eq!(p.step(0.1), 1);
    assert_eq!(p.clip(p.root).unwrap().frame, 1);
    // The sprite's own playhead moved on: its square moved.
    assert_eq!(p.clip(square).unwrap().matrix.tx, 50.0);
    p.step(0.1);
    // Frame three takes the sprite away.
    assert!(p.child(p.root, "clip").is_none());
    // Back to the start: placed again, fresh.
    p.step(0.1);
    assert_eq!(p.clip(p.root).unwrap().frame, 0);
    let clip = p.child(p.root, "clip").unwrap();
    let square = p.child(clip, "square").unwrap();
    assert_eq!(p.clip(square).unwrap().matrix.tx, 0.0);
    let mut out = Vec::new();
    p.draw(&Matrix::IDENTITY, &FontBook::default(), &mut out);
    assert_eq!(bounds(&out).unwrap(), [10.0, 10.0, 30.0, 30.0]);
}

#[test]
fn going_back_rebuilds_and_labels_name_frames() {
    let lib = library(vec![vec![put(1, 2, at(0.0, 0.0), "clip")]], sprite_frames());
    let mut p = Player::new(lib);
    let clip = p.child(p.root, "clip").unwrap();
    let two = p
        .clip(clip)
        .unwrap()
        .timeline
        .as_ref()
        .unwrap()
        .frame_of("TWO");
    assert_eq!(two, Some(1));
    p.goto(clip, 1);
    let square = p.child(clip, "square").unwrap();
    assert_eq!(p.clip(square).unwrap().matrix.tx, 50.0);
    p.goto(clip, 0);
    let square = p.child(clip, "square").unwrap();
    assert_eq!(p.clip(square).unwrap().matrix.tx, 0.0);
    // Timeline depths are where scripts see them.
    assert!(
        p.clip(clip)
            .unwrap()
            .children
            .contains_key(&(1 + TIMELINE_DEPTH))
    );
}

#[test]
fn a_mask_clips_what_it_masks() {
    let lib = library(
        vec![vec![
            Control::Place(Place {
                depth: 1,
                character: Some(1),
                matrix: Some(at(0.0, 0.0)),
                clip_depth: Some(3),
                ..Place::default()
            }),
            put(2, 1, at(10.0, 10.0), ""),
            put(4, 1, at(30.0, 0.0), ""),
        ]],
        sprite_frames(),
    );
    let p = Player::new(lib);
    let mut out = Vec::new();
    p.draw(&Matrix::IDENTITY, &FontBook::default(), &mut out);
    // The masked square is cut to the mask's 20 px; the one past the
    // mask's depths is whole; the mask itself is not drawn.
    let (cut, whole): (Vec<Triangle>, Vec<Triangle>) = out
        .into_iter()
        .partition(|t| t.corners.iter().all(|c| c[0] <= 20.0));
    assert_eq!(bounds(&cut).unwrap(), [10.0, 10.0, 20.0, 20.0]);
    assert_eq!(bounds(&whole).unwrap(), [30.0, 0.0, 50.0, 20.0]);
}

#[test]
fn a_mask_shows_through_its_shape_not_its_box() {
    // A 20 px textured square (uv = corners / 20) under a mask that is the
    // square's lower-left half, as two triangles of a shape's mesh.
    let square = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| Triangle {
        corners: [a, b, c],
        uv: [a, b, c].map(|p| [p[0] / 20.0, p[1] / 20.0]),
        colour: [1.0; 4],
        texture: Some("t".into()),
        multiply: true,
    };
    let content = [
        square([0.0, 0.0], [20.0, 0.0], [20.0, 20.0]),
        square([0.0, 0.0], [20.0, 20.0], [0.0, 20.0]),
    ];
    let mask = [
        [[0.0, 0.0], [10.0, 10.0], [0.0, 20.0]],
        [[0.0, 20.0], [20.0, 20.0], [10.0, 10.0]],
    ];
    let cut = super::cut_to_mask(&content, &mask);
    let area: f32 = cut
        .iter()
        .map(|t| {
            let [a, b, c] = t.corners;
            ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs() / 2.0
        })
        .sum();
    // Half the square (its box would have let all 400 through).
    assert!((area - 200.0).abs() < 1e-3, "area {area}");
    for t in &cut {
        for (p, uv) in t.corners.iter().zip(t.uv) {
            // Nothing above the diagonal; texture kept where it was.
            assert!(p[1] >= p[0] - 1e-4, "{p:?} outside the mask");
            assert!((uv[0] - p[0] / 20.0).abs() < 1e-5 && (uv[1] - p[1] / 20.0).abs() < 1e-5);
        }
        assert!(t.multiply && t.texture.as_deref() == Some("t"));
    }
    // Nothing under an empty mask.
    assert!(super::cut_to_mask(&content, &[]).is_empty());
}

// ---- ActionScript ---------------------------------------------------------

/// Assembles action bytes.
#[derive(Clone, Default)]
struct Asm(Vec<u8>);

impl Asm {
    fn op(&mut self, op: u8) -> &mut Self {
        self.0.push(op);
        self
    }
    fn act(&mut self, op: u8, data: &[u8]) -> &mut Self {
        self.0.push(op);
        self.0.extend((data.len() as u16).to_le_bytes());
        self.0.extend(data);
        self
    }
    fn str(&mut self, s: &str) -> &mut Self {
        let mut d = vec![0u8];
        d.extend(s.as_bytes());
        d.push(0);
        self.act(0x96, &d)
    }
    fn num(&mut self, n: f64) -> &mut Self {
        let mut d = vec![6u8];
        let bits = n.to_bits();
        d.extend(((bits >> 32) as u32).to_le_bytes());
        d.extend((bits as u32).to_le_bytes());
        self.act(0x96, &d)
    }
    fn int(&mut self, n: i32) -> &mut Self {
        let mut d = vec![7u8];
        d.extend(n.to_le_bytes());
        self.act(0x96, &d)
    }
    fn reg(&mut self, r: u8) -> &mut Self {
        self.act(0x96, &[4, r])
    }
    /// `name = value` on the timeline (value pushed by `value`).
    fn set(&mut self, name: &str, value: impl FnOnce(&mut Asm)) -> &mut Self {
        self.str(name);
        value(self);
        self.op(0x1D)
    }
    fn bytes(&self) -> Rc<[u8]> {
        self.0.clone().into()
    }
}

/// A runtime whose movie's first frame runs `script`, over `library`'s
/// shapes and sprite.
fn run(script: &Asm, root_more: Vec<Vec<Control>>) -> Runtime {
    let mut frames = vec![vec![Control::Script(script.bytes())]];
    frames.extend(root_more);
    let lib = library(frames, sprite_frames());
    Runtime::new(Player::new(lib))
}

fn var(rt: &mut Runtime, name: &str) -> Value {
    let root = Value::Object(rt.avm.clip_object(rt.player.root));
    rt.avm.get(&mut rt.player, &root, name)
}

fn number(v: &Value) -> f64 {
    match v {
        Value::Number(n) => *n,
        other => panic!("not a number: {other:?}"),
    }
}

fn string(v: &Value) -> String {
    match v {
        Value::Str(s) => s.to_string(),
        other => panic!("not a string: {other:?}"),
    }
}

#[test]
fn scripts_stop_their_timeline_and_keep_variables() {
    let mut a = Asm::default();
    a.set("x", |a| {
        a.int(1).int(2).op(0x47);
    })
    .set("s", |a| {
        a.str("map").int(3).op(0x47);
    })
    .op(0x07);
    let mut rt = run(&a, vec![vec![], vec![]]);
    assert_eq!(number(&var(&mut rt, "x")), 3.0);
    assert_eq!(string(&var(&mut rt, "s")), "map3");
    rt.step(1.0);
    assert_eq!(
        rt.player.clip(rt.player.root).unwrap().frame,
        0,
        "stop() held it"
    );
}

/// `function name(a, b) { return a * b + base; }` as `DefineFunction`.
fn define_multiply(a: &mut Asm) {
    let mut body = Asm::default();
    body.str("a")
        .op(0x1C)
        .str("b")
        .op(0x1C)
        .op(0x0C)
        .str("base")
        .op(0x1C)
        .op(0x47)
        .op(0x3E);
    let mut d = Vec::new();
    d.extend(b"multiply\0");
    d.extend(2u16.to_le_bytes());
    d.extend(b"a\0b\0");
    d.extend((body.0.len() as u16).to_le_bytes());
    a.act(0x9B, &d);
    a.0.extend(&body.0);
}

#[test]
fn functions_called_by_the_host_see_the_timeline() {
    let mut a = Asm::default();
    a.set("base", |a| {
        a.int(10);
    });
    define_multiply(&mut a);
    let mut rt = run(&a, Vec::new());
    let r = rt.call("multiply", &[Value::Number(3.0), Value::Number(4.0)]);
    assert_eq!(number(&r), 22.0);
    // Called from a script too, `multiply(5, 6)`.
    let mut b = Asm::default();
    b.set("y", |b| {
        b.int(6).int(5).int(2).str("multiply").op(0x3D);
    });
    let root = rt.player.root;
    rt.avm.run_script(&mut rt.player, root, &b.bytes());
    assert_eq!(number(&var(&mut rt, "y")), 40.0);
}

#[test]
fn define_function2_keeps_parameters_in_registers() {
    // function add(r1, r2) { return r1 + r2 } with this preloaded? No:
    // parameters in registers 1 and 2.
    let mut body = Asm::default();
    body.reg(1).reg(2).op(0x47).op(0x3E);
    let mut d = Vec::new();
    d.extend(b"add\0");
    d.extend(2u16.to_le_bytes());
    d.push(3); // registers
    d.extend([0u8, 0u8]); // flags
    d.extend([1u8]);
    d.extend(b"p\0");
    d.extend([2u8]);
    d.extend(b"q\0");
    d.extend((body.0.len() as u16).to_le_bytes());
    let mut a = Asm::default();
    a.act(0x8E, &d);
    a.0.extend(&body.0);
    let mut rt = run(&a, Vec::new());
    let r = rt.call("add", &[Value::from("a"), Value::from("b")]);
    assert_eq!(string(&r), "ab");
}

#[test]
fn clips_are_objects_with_properties_and_methods() {
    // clip._x = 40; clip._alpha = 50; clip.gotoAndStop("two");
    let mut a = Asm::default();
    a.str("clip").op(0x1C).str("_x").int(40).op(0x4F);
    a.str("clip").op(0x1C).str("_alpha").int(50).op(0x4F);
    a.str("two")
        .int(1)
        .str("clip")
        .op(0x1C)
        .str("gotoAndStop")
        .op(0x52)
        .op(0x17);
    // n = clip._currentframe
    a.set("n", |a| {
        a.str("clip").op(0x1C).str("_currentframe").op(0x4E);
    });
    let mut rt = run(&a, Vec::new());
    // The script runs before the clip is placed on frame one? It is placed
    // by the same frame, before the script.
    let clip = rt.player.child(rt.player.root, "clip");
    assert!(clip.is_none(), "this movie places no clip");
    // Place one first, then run the script.
    let lib = library(
        vec![vec![
            put(1, 2, at(0.0, 0.0), "clip"),
            Control::Script(a.bytes()),
        ]],
        sprite_frames(),
    );
    rt = Runtime::new(Player::new(lib));
    let clip = rt.player.child(rt.player.root, "clip").unwrap();
    let c = rt.player.clip(clip).unwrap();
    assert_eq!(
        (c.matrix.tx, c.colour.mult[3], c.frame, c.playing),
        (40.0, 0.5, 1, false)
    );
    assert_eq!(number(&var(&mut rt, "n")), 2.0);
}

#[test]
fn arrays_strings_and_objects() {
    let mut a = Asm::default();
    // list = [1, 2]; list.push(3); joined = list.join("-");
    a.set("list", |a| {
        a.int(2).int(1).int(2).op(0x42);
    });
    a.int(3)
        .int(1)
        .str("list")
        .op(0x1C)
        .str("push")
        .op(0x52)
        .op(0x17);
    a.set("joined", |a| {
        a.str("-").int(1).str("list").op(0x1C).str("join").op(0x52);
    });
    // parts = "a,b,c".split(","); count = parts.length;
    a.set("count", |a| {
        a.str(",")
            .int(1)
            .str("a,b,c")
            .str("split")
            .op(0x52)
            .str("length")
            .op(0x4E);
    });
    // o = {k: "v"}; got = o.k;
    a.set("o", |a| {
        a.str("k").str("v").int(1).op(0x43);
    });
    a.set("got", |a| {
        a.str("o").op(0x1C).str("k").op(0x4E);
    });
    // r = Math.floor(2.7)
    a.set("r", |a| {
        a.num(2.7).int(1).str("Math").op(0x1C).str("floor").op(0x52);
    });
    let mut rt = run(&a, Vec::new());
    assert_eq!(string(&var(&mut rt, "joined")), "1-2-3");
    assert_eq!(number(&var(&mut rt, "count")), 3.0);
    assert_eq!(string(&var(&mut rt, "got")), "v");
    assert_eq!(number(&var(&mut rt, "r")), 2.0);
}

#[test]
fn branches_loop_until_done() {
    // i = 0; do { i = i + 1 } while (i < 5)
    let mut a = Asm::default();
    a.set("i", |a| {
        a.int(0);
    });
    let top = a.0.len();
    a.set("i", |a| {
        a.str("i").op(0x1C).int(1).op(0x47);
    });
    a.str("i").op(0x1C).int(5).op(0x48);
    // If jumps back to `top`: the offset is from after the If.
    let after = a.0.len() + 5;
    let off = top as i32 - after as i32;
    a.act(0x9D, &(off as i16).to_le_bytes());
    let mut rt = run(&a, Vec::new());
    assert_eq!(number(&var(&mut rt, "i")), 5.0);
}

#[test]
fn attach_movie_makes_an_instance_by_its_linkage_name() {
    // mc = this.attachMovie("box", "b1", 5); mc._y = 7;
    let mut a = Asm::default();
    a.set("mc", |a| {
        a.int(5)
            .str("b1")
            .str("box")
            .int(3)
            .str("this")
            .op(0x1C)
            .str("attachMovie")
            .op(0x52);
    });
    a.str("mc").op(0x1C).str("_y").int(7).op(0x4F);
    let rt = run(&a, Vec::new());
    let b1 = rt.player.child(rt.player.root, "b1").unwrap();
    let c = rt.player.clip(b1).unwrap();
    assert_eq!((c.depth, c.matrix.ty, c.character), (5, 7.0, 2));
    // Its first frame placed its square.
    assert!(rt.player.child(b1, "square").is_some());
}

#[test]
fn fscommands_enter_frame_and_intervals_reach_the_host() {
    let mut a = Asm::default();
    // getURL("FSCommand:closeFanfare", "now")
    let mut d = Vec::new();
    d.extend(b"FSCommand:closeFanfare\0now\0");
    a.act(0x83, &d);
    // count = 0; this.onEnterFrame = function() { count = count + 1 };
    a.set("count", |a| {
        a.int(0);
    });
    let mut body = Asm::default();
    body.set("count", |b| {
        b.str("count").op(0x1C).int(1).op(0x47);
    });
    let mut f = Vec::new();
    f.extend(b"\0");
    f.extend(0u16.to_le_bytes());
    f.extend((body.0.len() as u16).to_le_bytes());
    a.str("this").op(0x1C).str("onEnterFrame");
    a.act(0x9B, &f);
    a.0.extend(&body.0);
    a.op(0x4F);
    a.op(0x07);
    let mut rt = run(&a, Vec::new());
    assert_eq!(
        rt.take_commands(),
        vec![("closeFanfare".to_owned(), "now".to_owned())]
    );
    rt.step(0.35);
    assert_eq!(number(&var(&mut rt, "count")), 3.0);
}

#[test]
fn drawing_api_fills_a_rectangle() {
    // this.beginFill(0x00ff00, 50); moveTo(0,0); lineTo(10,0); lineTo(10,10);
    // lineTo(0,10); endFill();
    let mut a = Asm::default();
    let call = |a: &mut Asm, name: &str, args: &[i32]| {
        for &v in args.iter().rev() {
            a.int(v);
        }
        a.int(args.len() as i32)
            .str("this")
            .op(0x1C)
            .str(name)
            .op(0x52)
            .op(0x17);
    };
    call(&mut a, "beginFill", &[0x00ff00, 50]);
    call(&mut a, "moveTo", &[0, 0]);
    call(&mut a, "lineTo", &[10, 0]);
    call(&mut a, "lineTo", &[10, 10]);
    call(&mut a, "lineTo", &[0, 10]);
    call(&mut a, "endFill", &[]);
    let rt = run(&a, Vec::new());
    let mut out = Vec::new();
    rt.player
        .draw(&Matrix::IDENTITY, &FontBook::default(), &mut out);
    assert_eq!(bounds(&out).unwrap(), [0.0, 0.0, 10.0, 10.0]);
    assert_eq!(out[0].colour, [0.0, 1.0, 0.0, 0.5]);
}

// ---- Text -------------------------------------------------------------

#[test]
fn text_fields_lay_out_in_texture_glyphs() {
    let mut font = Font {
        name: "Gunplay".into(),
        ascent: 0.75,
        descent: 0.25,
        advances: vec![0.5, 0.5],
        ..Font::default()
    };
    font.codes.insert('A', 0);
    font.codes.insert(' ', 1);
    // Glyph A: a 24 x 48 box in a 512 x 256 texture, origin at its bottom
    // left; drawn at 48 px nominal, so 32 px to the em.
    let g = text::TextureGlyph {
        uv: [0.0, 0.0, 24.0 / 512.0, 48.0 / 256.0],
        origin: [0.0, 36.0 / 256.0],
    };
    font.texture = Some(("font_f0".into(), [512.0, 256.0], 48.0, vec![Some(g), None]));
    let def = EditText {
        bounds: [0.0, 0.0, 100.0, 20.0],
        height: 24.0,
        colour: [1.0; 4],
        ..EditText::default()
    };
    let mut out = Vec::new();
    text::layout(
        &def,
        "A A",
        &[],
        &font,
        &Matrix::IDENTITY,
        &ColourTransform::IDENTITY,
        None,
        &mut out,
    );
    // Two glyphs, two triangles each; the second 12 px (half an em of
    // 24) after a space.
    assert_eq!(out.len(), 4);
    assert_eq!(out[0].texture.as_deref(), Some("font_f0"));
    let b0 = bounds(&out[..2]).unwrap();
    let b1 = bounds(&out[2..]).unwrap();
    // A 24 px field takes 24 of the texture's 32 px em: 18 x 36.
    assert!((b0[2] - b0[0] - 18.0).abs() < 1e-4 && (b0[3] - b0[1] - 36.0).abs() < 1e-4);
    assert!((b1[0] - b0[0] - 24.0).abs() < 1e-4);
    // An inline image for "[f]", 10 px wide, sitting on the baseline: the
    // second glyph moves along by its width.
    let flag = text::Substitution {
        text: "[f]".into(),
        texture: "flag".into(),
        part: [0.0, 0.0, 1.0, 1.0],
        size: [10.0, 8.0],
        baseline: 8.0,
        id: String::new(),
    };
    out.clear();
    text::layout(
        &def,
        "A[f]A",
        &[flag],
        &font,
        &Matrix::IDENTITY,
        &ColourTransform::IDENTITY,
        None,
        &mut out,
    );
    assert_eq!(out.len(), 6);
    assert_eq!(out[2].texture.as_deref(), Some("flag"));
    let image = bounds(&out[2..4]).unwrap();
    assert!((image[2] - image[0] - 10.0).abs() < 1e-4);
    let a1 = bounds(&out[4..]).unwrap();
    assert!((a1[0] - b0[0] - 22.0).abs() < 1e-4);
    assert_eq!(
        plain(
            "<p align='left'><font color='#ff0000'>Marga</font></p>",
            true
        ),
        ("Marga".to_owned(), Some([1.0, 0.0, 0.0, 1.0]))
    );
}

#[test]
fn a_font_texture_reads_its_padding_and_nominal_size() {
    let mut d = Vec::new();
    d.extend_from_slice(&[1, 0, 0, 0, 0, 0]);
    d.push(9);
    d.extend_from_slice(b"f0_F0.tga");
    for v in [512u16, 256] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    d.push(2);
    d.extend_from_slice(&48u16.to_le_bytes());
    d.extend_from_slice(&1u16.to_le_bytes());
    for v in [0.0f32, 0.0, 0.5, 0.25, 0.0, 0.2] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    for v in [1u16, 7, 1, 3, 0] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    let t = text::FontTexture::parse(&d).unwrap();
    assert_eq!(t.name, "f0_F0");
    assert_eq!((t.size, t.pad, t.nominal), ([512.0, 256.0], 2.0, 48.0));
    assert_eq!(t.glyphs[0].origin, [0.0, 0.2]);
    assert_eq!(t.maps[&7][&3], 0);
    // Its glyphs were drawn 32 px to the em.
    assert!((t.nominal * text::TEXTURE_EM - 32.0).abs() < 1e-4);
}

#[test]
fn a_movie_on_another_level_has_its_own_root() {
    // _root.n = 1, in the movie loaded on _level1.
    let mut a = Asm::default();
    a.str("_root").op(0x1C).str("n").int(1).op(0x4F);
    let level1 = library(vec![vec![Control::Script(a.bytes())]], sprite_frames());
    let mut rt = run(&Asm::default(), Vec::new());
    rt.run_due();
    let id = rt.player.load_level(1, level1);
    rt.run_due();
    assert_eq!(rt.player.root_of(id), id);
    let root1 = Value::Object(rt.avm.clip_object(id));
    assert_eq!(number(&rt.avm.get(&mut rt.player, &root1, "n")), 1.0);
    assert!(matches!(var(&mut rt, "n"), Value::Undefined));
}

#[test]
fn load_bitmap_gives_an_exported_image() {
    // b = flash.display.BitmapData.loadBitmap("Pic")
    let mut a = Asm::default();
    a.set("b", |a| {
        a.str("Pic")
            .int(1)
            .str("flash")
            .op(0x1C)
            .str("display")
            .op(0x4E)
            .str("BitmapData")
            .op(0x4E)
            .str("loadBitmap")
            .op(0x52);
    });
    let mut lib = Library::default();
    lib.images.insert(
        7,
        super::Image {
            texture: "pic_tex".into(),
            width: 16.0,
            height: 8.0,
            part: [0.0, 0.0, 1.0, 1.0],
        },
    );
    lib.exports.insert("pic".into(), 7);
    lib.root = Rc::new(Timeline {
        frames: vec![vec![Control::Script(a.bytes())]],
        ..Timeline::default()
    });
    let mut rt = Runtime::new(Player::new(Rc::new(lib)));
    rt.run_due();
    let b = var(&mut rt, "b");
    assert_eq!(number(&rt.avm.get(&mut rt.player, &b, "width")), 16.0);
}

#[test]
fn a_texture_font_without_glyph_shapes_keeps_its_codes() {
    // DefineFont3 as Scaleform leaves it: font 5, "F", 3 glyphs, the
    // offset table a single 0 then the code table's offset (4).
    let mut d = vec![5, 0, 0x80 | 0x04, 0, 1, b'F', 3, 0];
    d.extend([0, 0, 4, 0]);
    for c in ['A', 'B', 'C'] {
        d.extend((c as u16).to_le_bytes());
    }
    d.extend([0; 6]);
    for a in [100i16, 200, 300] {
        d.extend(a.to_le_bytes());
    }
    let (id, font) = super::text::Font::parse(&d).unwrap();
    assert_eq!(id, 5);
    assert_eq!(font.codes.len(), 3);
    assert_eq!(font.codes[&'B'], 1);
    assert_eq!(font.advances.len(), 3);
}

#[test]
fn key_listeners_hear_key_presses() {
    // A listener whose onKeyDown keeps Key.getCode() in `got`.
    let lib = Library {
        root: Rc::new(Timeline {
            frames: vec![Vec::new()],
            ..Timeline::default()
        }),
        ..Library::default()
    };
    let mut rt = Runtime::new(Player::new(Rc::new(lib)));
    let listener = Value::Object(rt.avm.object());
    let on_down = Object::new(
        ObjKind::Native(|avm, _, _, _| {
            let code = Value::Number(f64::from(avm.key_code));
            avm.global.borrow_mut().set("got", code);
            Value::Undefined
        }),
        None,
    );
    if let Value::Object(o) = &listener {
        o.borrow_mut().set("onKeyDown", Value::Object(on_down));
    }
    let key = Value::Object(rt.avm.global.clone());
    let key = rt.avm.get(&mut rt.player, &key, "Key");
    let add = rt.avm.get(&mut rt.player, &key, "addListener");
    rt.avm
        .call_value(&mut rt.player, &add, &key, std::slice::from_ref(&listener));
    rt.avm.call_value(&mut rt.player, &add, &key, &[listener]);
    rt.key_down(36);
    let global = Value::Object(rt.avm.global.clone());
    assert_eq!(number(&rt.avm.get(&mut rt.player, &global, "got")), 36.0);
    assert_eq!(rt.avm.key_listeners.len(), 1);
    assert_eq!(rt.avm.keys_down, vec![36]);
    rt.key_up(36);
    assert!(rt.avm.keys_down.is_empty());
}

#[test]
fn a_word_wrapped_field_measures_its_wrapped_lines() {
    let field = EditText {
        bounds: [0.0, 0.0, 604.0, 40.0],
        height: 20.0,
        word_wrap: true,
        ..EditText::default()
    };
    // 92 characters at 11 px: 1012 px in 600, two lines of 24 px.
    let hint = "x".repeat(92);
    let round = |(w, h): (f32, f32)| (w.round(), h.round());
    assert_eq!(round(field.text_size(&hint)), (600.0, 48.0));
    // A short line, and two explicit ones.
    assert_eq!(round(field.text_size("Loading")), (77.0, 24.0));
    assert_eq!(field.text_size("a\nb").1, 48.0);
    // Without word wrap, one line however long.
    let one = EditText {
        word_wrap: false,
        ..field
    };
    assert_eq!(round(one.text_size(&hint)), (1012.0, 24.0));
}

#[test]
fn going_back_keeps_an_instance_the_target_frame_has_too() {
    let lib = library(vec![vec![put(1, 2, at(0.0, 0.0), "clip")]], sprite_frames());
    let mut p = Player::new(lib);
    let clip = p.child(p.root, "clip").unwrap();
    let square = p.child(clip, "square").unwrap();
    p.goto(clip, 1);
    // A script's change to it (as a text field's text would be).
    p.clip_mut(square).unwrap().visible = false;
    p.goto(clip, 0);
    // The same instance, its change kept, placed as frame one has it.
    let again = p.clip(clip).unwrap().children[&(1 + TIMELINE_DEPTH)];
    assert_eq!(again, square);
    assert!(!p.clip(square).unwrap().visible);
    assert_eq!(p.clip(square).unwrap().matrix.tx, 0.0);
}

#[test]
fn a_text_fields_x_and_y_name_its_box_corner() {
    // got = t._y; t._y = 50;
    let mut a = Asm::default();
    a.set("got", |a| {
        a.str("t").op(0x1C).str("_y").op(0x4E);
    });
    a.str("t").op(0x1C).str("_y").int(50).op(0x4F);
    let mut lib = Rc::try_unwrap(library(
        vec![vec![
            put(1, 3, at(10.0, 20.0), "t"),
            Control::Script(a.bytes()),
        ]],
        sprite_frames(),
    ))
    .unwrap();
    lib.texts.insert(
        3,
        EditText {
            bounds: [-2.0, -2.0, 100.0, 20.0],
            ..EditText::default()
        },
    );
    let mut rt = Runtime::new(Player::new(Rc::new(lib)));
    // The box's top is 2 px above the field's origin: `_y` reads 18, and
    // setting it to 50 puts the origin at 52.
    assert_eq!(number(&var(&mut rt, "got")), 18.0);
    let t = rt.player.child(rt.player.root, "t").unwrap();
    assert_eq!(rt.player.clip(t).unwrap().matrix.ty, 52.0);
}

/// A runtime with text field `t` (character 3: `def`, showing `text`)
/// whose first frame runs `script`.
fn text_field(def: EditText, text: &str, script: Asm) -> (Runtime, ClipId) {
    let mut lib = Rc::try_unwrap(library(
        vec![vec![
            put(1, 3, at(10.0, 20.0), "t"),
            Control::Script(script.bytes()),
        ]],
        sprite_frames(),
    ))
    .unwrap();
    lib.texts.insert(
        3,
        EditText {
            initial: text.into(),
            ..def
        },
    );
    let rt = Runtime::new(Player::new(Rc::new(lib)));
    let t = rt.player.child(rt.player.root, "t").unwrap();
    (rt, t)
}

#[test]
fn a_text_fields_width_sizes_its_box_not_its_text() {
    // t._width = 30; got = t._width;
    let mut a = Asm::default();
    a.str("t").op(0x1C).str("_width").int(30).op(0x4F);
    a.set("got", |a| {
        a.str("t").op(0x1C).str("_width").op(0x4E);
    });
    let def = EditText {
        bounds: [-2.0, -2.0, 100.0, 20.0],
        ..EditText::default()
    };
    let (mut rt, t) = text_field(def, "abc", a);
    assert_eq!(number(&var(&mut rt, "got")), 30.0);
    let c = rt.player.clip(t).unwrap();
    // Unscaled: the box is 30 wide from its left edge.
    assert_eq!(c.matrix.a, 1.0);
    assert_eq!(c.text_box, Some([-2.0, -2.0, 28.0, 20.0]));
}

#[test]
fn auto_size_fits_the_box_round_its_text() {
    // t.autoSize = "center";
    let mut a = Asm::default();
    a.str("t").op(0x1C).str("autoSize").str("center").op(0x4F);
    let def = EditText {
        bounds: [-2.0, -2.0, 100.0, 20.0],
        height: 20.0,
        ..EditText::default()
    };
    // Estimated 11 px a character: "abcd" 44 wide, 48 with the gutters,
    // centred on the box's 102: from 25.
    let (rt, t) = text_field(def.clone(), "abcd", a.clone());
    assert_eq!(
        rt.player.clip(t).unwrap().text_box,
        Some([25.0, -2.0, 73.0, 26.0])
    );
    // A wrapping field keeps its width and grows down.
    let wraps = EditText {
        word_wrap: true,
        ..def
    };
    let (rt, t) = text_field(wraps, "abcd", a);
    assert_eq!(
        rt.player.clip(t).unwrap().text_box,
        Some([-2.0, -2.0, 100.0, 26.0])
    );
}

#[test]
fn auto_size_fits_again_when_the_text_is_set() {
    // t.autoSize = "right"; t.text = "abcd";
    let mut a = Asm::default();
    a.str("t").op(0x1C).str("autoSize").str("right").op(0x4F);
    a.str("t").op(0x1C).str("text").str("abcd").op(0x4F);
    let def = EditText {
        bounds: [-2.0, -2.0, 100.0, 20.0],
        height: 20.0,
        ..EditText::default()
    };
    // Empty, it shrank to its right edge; "abcd" grows it back leftwards.
    let (rt, t) = text_field(def, "", a);
    assert_eq!(
        rt.player.clip(t).unwrap().text_box,
        Some([52.0, -2.0, 100.0, 26.0])
    );
}

#[test]
fn text_width_measures_the_text_as_drawn() {
    let def = EditText {
        bounds: [-2.0, -2.0, 400.0, 20.0],
        height: 20.0,
        ..EditText::default()
    };
    let (mut rt, t) = text_field(def, "[0x1]", Asm::default());
    rt.player.translate = Some(Translator(Rc::new(|s: &str| s.replace("[0x1]", "ab"))));
    assert_eq!(rt.player.text_size(t).map(|(w, _)| w.round()), Some(22.0));
}

#[test]
fn a_wrapped_centred_line_keeps_its_break_space() {
    let mut font = Font {
        ascent: 0.75,
        descent: 0.25,
        advances: vec![0.5, 0.5],
        ..Font::default()
    };
    font.codes.insert('A', 0);
    font.codes.insert(' ', 1);
    let g = text::TextureGlyph {
        uv: [0.0, 0.0, 24.0 / 512.0, 48.0 / 256.0],
        origin: [0.0, 36.0 / 256.0],
    };
    font.texture = Some(("f".into(), [512.0, 256.0], 48.0, vec![Some(g), None]));
    let def = EditText {
        bounds: [0.0, 0.0, 50.0, 60.0],
        height: 24.0,
        colour: [1.0; 4],
        align: 2,
        leading: 2.0,
        word_wrap: true,
        multiline: true,
        ..EditText::default()
    };
    // 12 px a character: "A A A" (60) breaks in 50 as "A A " and "A".
    let mut out = Vec::new();
    text::layout(
        &def,
        "A A A",
        &[],
        &font,
        &Matrix::IDENTITY,
        &ColourTransform::IDENTITY,
        None,
        &mut out,
    );
    assert_eq!(out.len(), 6);
    // Centred in 46 px past the 2 px gutter: the first line 48 wide with
    // its space (x 1, not 7), the last 12 (x 19).
    assert!((bounds(&out[..2]).unwrap()[0] - 1.0).abs() < 1e-4);
    assert!((bounds(&out[4..]).unwrap()[0] - 19.0).abs() < 1e-4);
    // textWidth leaves the space out; textHeight has no leading under the
    // last line: 2 x 26 - 2.
    let (w, h) = text::measure(&def, "A A A", &font);
    assert_eq!((w, h), (36.0, 50.0));
}

fn bump(avm: &mut super::avm1::Avm, name: &str) {
    let g = Rc::clone(&avm.global);
    let n = match g.borrow().own(name) {
        Some(Value::Number(n)) => n,
        _ => 0.0,
    };
    g.borrow_mut().set(name, Value::Number(n + 1.0));
}

fn count(rt: &mut Runtime, name: &str) -> f64 {
    match rt.avm.global.borrow().own(name) {
        Some(Value::Number(n)) => n,
        _ => 0.0,
    }
}

#[test]
fn the_mouse_rolls_over_presses_and_releases_buttons() {
    // A 20 px square at (10, 10): a button by its handlers.
    let lib = library(
        vec![vec![put(1, 2, at(10.0, 10.0), "btn")]],
        sprite_frames(),
    );
    let mut rt = Runtime::new(Player::new(lib));
    let btn = rt.player.child(rt.player.root, "btn").unwrap();
    let o = rt.avm.clip_object(btn);
    let handlers: [(&str, super::avm1::Native); 5] = [
        ("onRollOver", |avm, _, _, _| {
            bump(avm, "over");
            Value::Undefined
        }),
        ("onRollOut", |avm, _, _, _| {
            bump(avm, "out");
            Value::Undefined
        }),
        ("onPress", |avm, _, _, _| {
            bump(avm, "press");
            Value::Undefined
        }),
        ("onRelease", |avm, _, _, _| {
            bump(avm, "release");
            Value::Undefined
        }),
        ("onReleaseOutside", |avm, _, _, _| {
            bump(avm, "outside");
            Value::Undefined
        }),
    ];
    for (name, f) in handlers {
        let f = Object::new(ObjKind::Native(f), None);
        o.borrow_mut().set(name, Value::Object(f));
    }
    // An instance hears every mouse down, button or not.
    let root = rt.avm.clip_object(rt.player.root);
    let down = Object::new(
        ObjKind::Native(|avm, _, _, _| {
            bump(avm, "down");
            Value::Undefined
        }),
        None,
    );
    root.borrow_mut().set("onMouseDown", Value::Object(down));
    assert_eq!(rt.button_at([15.0, 15.0]), Some(btn));
    assert_eq!(rt.button_at([5.0, 5.0]), None);
    rt.mouse_move([15.0, 15.0]);
    assert_eq!(count(&mut rt, "over"), 1.0);
    // `_xmouse` is in the instance's own space.
    let v = Value::Object(Rc::clone(&o));
    assert_eq!(number(&rt.avm.get(&mut rt.player, &v, "_xmouse")), 5.0);
    rt.mouse_down();
    rt.mouse_up();
    assert_eq!(
        [
            count(&mut rt, "press"),
            count(&mut rt, "release"),
            count(&mut rt, "down")
        ],
        [1.0, 1.0, 1.0]
    );
    // Pressed, then let go off it.
    rt.mouse_down();
    rt.mouse_move([60.0, 60.0]);
    assert_eq!(count(&mut rt, "out"), 1.0);
    rt.mouse_up();
    assert_eq!(
        (count(&mut rt, "outside"), count(&mut rt, "release")),
        (1.0, 1.0)
    );
    // hitTest: on its square, by shape and by box.
    let hit = rt.avm.get(&mut rt.player, &v, "hitTest");
    let at =
        |x: f64, y: f64, shape: bool| vec![Value::Number(x), Value::Number(y), Value::Bool(shape)];
    for (x, y, want) in [(20.0, 20.0, true), (40.0, 20.0, false)] {
        let r = rt.avm.call_value(&mut rt.player, &hit, &v, &at(x, y, true));
        assert!(matches!(r, Value::Bool(b) if b == want), "({x}, {y})");
    }
    // Hidden, it is no button.
    rt.player.clip_mut(btn).unwrap().visible = false;
    assert_eq!(rt.button_at([15.0, 15.0]), None);
}
