//! The game's widget library (`_GuiInternal`) rebuilt as a widget store,
//! so the game's own interface scripts build and drive the HUD.
//!
//! The scripts never draw: `MrxGuiBase` wraps each native widget in a Lua
//! object and calls `_GuiInternal` to create it, place it, colour it, set
//! its texture or text, animate it and tie it into a tree. This module is
//! that native side: a [`Screen`] of widgets the natives change, advanced by
//! [`tick`], and read back as a renderer-neutral [`DrawItem`] list by
//! [`Screen::draw_list`]. A renderer draws the quads and text runs.
//!
//! How the natives behave, as the scripts use them:
//!
//! - Boxes are on the 640 x 480 virtual screen. A child keeps its place
//!   relative to its parent: when the parent's box moves or changes size
//!   (an animation collapsing a frame, the reticle following the aim), the
//!   child's box is mapped the same way. `GetWidgetLocation` answers the
//!   box as mapped.
//! - `SetWidgetVisible` and `SetWidgetColor` with its last argument true
//!   apply to the widget and its current children: the layout loader hides
//!   containers before their children exist (and colours each widget with
//!   the argument false), while the handlers hide or fade whole groups. A
//!   negative colour channel leaves that channel as it is.
//! - `InterpolateWidget` moves a widget's box, colour, texture rectangle
//!   and rotation to a target over a time (a colour channel of -4096, or a
//!   missing value, stays), then calls back; `MrxGuiBase` queues its
//!   animation points on it. Colour changes reach the children as above.
//! - `SetWidgetUpdateCallback` registers a function called every tick with
//!   its data and the time step (the handlers' `GuiUpdate`).
//! - `CorrectWidgetForResolution` fixes a top-level widget's anchor point
//!   for the screen's shape; see [`crate::gui::Correction`].
//! - Every live widget is on the screen, in creation order (a parent
//!   before its children), with `PushWidgetToFront`/`Back` reordering;
//!   one draws if it is visible and not asleep.
//!
//! Natives not implemented answer as the old stand-in did (`Create*` a new
//! plain widget, `Get*` zeros, `Is*` false, anything else nothing) and are
//! listed once in [`Screen::unknown`].
//!
//! Events reach the handlers as the scripts register them: `Widget:
//! SetEventHandler` makes an `Event.CreatePersistent` of the event's name
//! for the engine's events, so [`fire`] delivers through the `Event`
//! library; `MrxGuiBase.Run()`, called by [`tick`], processes the events
//! the scripts queue themselves (`MrxGui.SendEvent`).
//!
//! # Provenance
//!
//! Behaviour read from how the game's scripts call `_GuiInternal`
//! (`MrxGuiBase` and the HUD handler scripts in a legally installed copy)
//! and checked against footage of the game. Nothing from the executable.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use m2_assets::{Font, StringTable};
use mlua::{Function, IntoLuaMulti, Lua, MultiValue, Table, Value};

use crate::gui::{Anchor, Correction, VIRTUAL_SIZE};
use crate::script::safe_function;

/// The screen, shared by the natives and the host.
pub type ScreenRef = Rc<RefCell<Screen>>;

/// The screen height the bitmap fonts are drawn for.
pub const FONT_LINES: f32 = 720.0;

/// Colour channels below zero (the scripts pass -255 and -4096) are left
/// as they are.
const KEEP: f64 = 0.0;

/// Registry table of callbacks: handle to `{ function, data }`.
const CALLBACKS: &str = "m2_gui_callbacks";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    /// A plain widget: groups its children, draws nothing.
    #[default]
    Widget,
    Image,
    Text,
    Sprite,
    Flash,
    Minimap,
    Movie,
}

/// A running `InterpolateWidget`.
#[derive(Clone, Debug, PartialEq)]
struct Interp {
    duration: f32,
    elapsed: f32,
    from_rect: [f32; 4],
    to_rect: [f32; 4],
    from_colour: [f32; 4],
    /// Per channel; `None` stays.
    to_colour: [Option<f32>; 4],
    from_uv: [f32; 4],
    to_uv: Option<[f32; 4]>,
    from_rotation: f32,
    to_rotation: Option<f32>,
    callback: Option<u32>,
}

/// One widget.
#[derive(Clone, Debug, PartialEq)]
pub struct Widget {
    pub kind: Kind,
    /// What it was created with (the scripts pass nothing, or a name).
    pub name: String,
    pub parent: Option<u32>,
    pub children: Vec<u32>,
    pub visible: bool,
    pub sleeping: bool,
    /// The box as last set on this widget, and its parent's box then:
    /// the widget's box now is this mapped from that to the parent's box
    /// now.
    own: [f32; 4],
    parent_ref: Option<[f32; 4]>,
    /// RGBA, 0..=255.
    pub colour: [f32; 4],
    pub anchors: (Anchor, Anchor),
    /// The anchor point fixed by `CorrectWidgetForResolution`.
    pub correction: Option<[f32; 2]>,
    pub texture: Option<String>,
    pub uv: [f32; 4],
    /// Degrees.
    pub rotation: f32,
    pub tile: Option<(f32, f32)>,
    pub sprite_texture_size: [f32; 2],
    pub sprite_frame_size: [f32; 2],
    pub sprite_frame: u32,
    /// `SetImagePieSliceRender` arguments, as given.
    pub pie_slice: Option<Vec<f32>>,
    pub text: String,
    pub font: String,
    pub scale: f32,
    pub justification: Anchor,
    pub swf: Option<String>,
    /// `CallFlashScriptFunction` calls: name and arguments as text.
    pub flash_calls: Vec<(String, Vec<String>)>,
    pub movie: Option<String>,
    /// `MinimapCreate` arguments as text.
    pub minimap: Option<Vec<String>>,
    update: Option<u32>,
    interp: Option<Interp>,
}

impl Widget {
    fn new(kind: Kind, name: String) -> Self {
        Self {
            kind,
            name,
            parent: None,
            children: Vec::new(),
            visible: true,
            sleeping: false,
            own: [0.0; 4],
            parent_ref: None,
            colour: [255.0; 4],
            anchors: (Anchor::Start, Anchor::Start),
            correction: None,
            texture: None,
            uv: [0.0, 0.0, 1.0, 1.0],
            rotation: 0.0,
            tile: None,
            sprite_texture_size: [0.0; 2],
            sprite_frame_size: [0.0; 2],
            sprite_frame: 0,
            pie_slice: None,
            text: String::new(),
            font: String::new(),
            scale: 1.0,
            justification: Anchor::Start,
            swf: None,
            flash_calls: Vec::new(),
            movie: None,
            minimap: None,
            update: None,
            interp: None,
        }
    }
}

/// Something to draw, in screen pixels, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum DrawItem {
    /// A textured rectangle. `uv` may run past 0..=1 (tiling) or
    /// backwards (mirroring); `colour` multiplies the texture, 0..=1.
    Quad {
        widget: u32,
        texture: String,
        uv: [f32; 4],
        colour: [f32; 4],
        rect: [f32; 4],
        rotation: f32,
    },
    /// An image widget without a texture: a filled rectangle.
    Fill {
        widget: u32,
        colour: [f32; 4],
        rect: [f32; 4],
    },
    /// One line of text in a bitmap font, in the box `rect`, from its top,
    /// justified; `size` is screen pixels per font pixel.
    Text {
        widget: u32,
        font: String,
        text: String,
        rect: [f32; 4],
        size: f32,
        colour: [f32; 4],
        justification: Anchor,
    },
    /// A Flash movie's place (`swf` once set).
    Flash {
        widget: u32,
        swf: Option<String>,
        rect: [f32; 4],
    },
    /// The minimap's place.
    Minimap { widget: u32, rect: [f32; 4] },
    /// A video's place.
    Movie {
        widget: u32,
        file: Option<String>,
        rect: [f32; 4],
    },
}

/// The widget store.
#[derive(Debug, Default)]
pub struct Screen {
    widgets: BTreeMap<u32, Widget>,
    next_id: u32,
    /// Drawing order.
    order: Vec<u32>,
    next_callback: u32,
    /// Callbacks due at the next tick: handle and extra arguments.
    due: Vec<(u32, Vec<f64>)>,
    /// Natives called that are not implemented, once each.
    unknown: BTreeSet<String>,
    fonts: BTreeMap<String, Font>,
    strings: Option<StringTable>,
    /// Errors from callbacks and the scripts' event processing.
    log: Vec<String>,
}

impl Screen {
    /// A widget by id.
    pub fn widget(&self, id: u32) -> Option<&Widget> {
        self.widgets.get(&id)
    }

    pub fn widgets(&self) -> impl Iterator<Item = (u32, &Widget)> {
        self.widgets.iter().map(|(&k, v)| (k, v))
    }

    /// Natives the scripts called that this does not implement.
    pub fn unknown(&self) -> &BTreeSet<String> {
        &self.unknown
    }

    /// Errors since the last call.
    pub fn take_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.log)
    }

    /// Gives the store a font, for text widths (`GetTextWidth`).
    pub fn set_font(&mut self, name: &str, font: Font) {
        self.fonts.insert(name.to_ascii_lowercase(), font);
    }

    /// Gives the store the language's strings, for texts written as ids
    /// in brackets.
    pub fn set_strings(&mut self, strings: StringTable) {
        self.strings = Some(strings);
    }

    /// A widget's box now, on the virtual screen.
    pub fn rect(&self, id: u32) -> [f32; 4] {
        let Some(w) = self.widgets.get(&id) else {
            return [0.0; 4];
        };
        match (w.parent, w.parent_ref) {
            (Some(p), Some(from)) if self.widgets.contains_key(&p) => {
                map_rect(w.own, from, self.rect(p))
            }
            _ => w.own,
        }
    }

    /// Ids of the widget and all its descendants.
    fn subtree(&self, id: u32) -> Vec<u32> {
        let mut out = vec![id];
        let mut i = 0;
        while i < out.len() {
            if let Some(w) = self.widgets.get(&out[i]) {
                out.extend(w.children.iter().copied());
            }
            i += 1;
        }
        out
    }

    fn create(&mut self, kind: Kind, name: String) -> u32 {
        self.next_id += 1;
        let id = self.next_id;
        self.widgets.insert(id, Widget::new(kind, name));
        self.order.push(id);
        id
    }

    fn delete(&mut self, id: u32) {
        let Some(w) = self.widgets.remove(&id) else {
            return;
        };
        if let Some(p) = w.parent.and_then(|p| self.widgets.get_mut(&p)) {
            p.children.retain(|&c| c != id);
        }
        for c in w.children {
            if let Some(cw) = self.widgets.get_mut(&c) {
                cw.parent = None;
                cw.parent_ref = None;
            }
        }
        self.order.retain(|&o| o != id);
    }

    fn set_rect(&mut self, id: u32, rect: [f32; 4]) {
        let parent = self.widgets.get(&id).and_then(|w| w.parent);
        let parent_rect = parent.map(|p| self.rect(p));
        if let Some(w) = self.widgets.get_mut(&id) {
            w.own = rect;
            w.parent_ref = parent_rect;
        }
    }

    fn add_child(&mut self, parent: u32, child: u32) {
        if parent == child
            || !self.widgets.contains_key(&parent)
            || !self.widgets.contains_key(&child)
            || self.subtree(child).contains(&parent)
        {
            return;
        }
        let abs = self.rect(child);
        if let Some(old) = self.widgets.get(&child).and_then(|w| w.parent)
            && let Some(o) = self.widgets.get_mut(&old)
        {
            o.children.retain(|&c| c != child);
        }
        let parent_rect = self.rect(parent);
        if let Some(p) = self.widgets.get_mut(&parent)
            && !p.children.contains(&child)
        {
            p.children.push(child);
        }
        if let Some(c) = self.widgets.get_mut(&child) {
            c.parent = Some(parent);
            c.own = abs;
            c.parent_ref = Some(parent_rect);
        }
    }

    fn remove_child(&mut self, parent: u32, child: u32) {
        let abs = self.rect(child);
        if let Some(p) = self.widgets.get_mut(&parent) {
            p.children.retain(|&c| c != child);
        }
        if let Some(c) = self.widgets.get_mut(&child)
            && c.parent == Some(parent)
        {
            c.parent = None;
            c.parent_ref = None;
            c.own = abs;
        }
    }

    fn set_colour(&mut self, id: u32, colour: [Option<f32>; 4], recursive: bool) {
        let ids = if recursive {
            self.subtree(id)
        } else {
            vec![id]
        };
        for i in ids {
            if let Some(w) = self.widgets.get_mut(&i) {
                for (c, v) in w.colour.iter_mut().zip(colour) {
                    if let Some(v) = v {
                        *c = v;
                    }
                }
            }
        }
    }

    fn set_visible(&mut self, id: u32, visible: bool) {
        for i in self.subtree(id) {
            if let Some(w) = self.widgets.get_mut(&i) {
                w.visible = visible;
            }
        }
    }

    /// Advances animations by `dt` seconds; returns the callbacks now due
    /// (completed animations, then the update callbacks with `dt`).
    fn advance(&mut self, dt: f32) -> Vec<(u32, Vec<f64>)> {
        let mut due = std::mem::take(&mut self.due);
        let ids: Vec<u32> = self.widgets.keys().copied().collect();
        for id in ids {
            let Some(mut it) = self.widgets.get_mut(&id).and_then(|w| w.interp.take()) else {
                continue;
            };
            it.elapsed += dt;
            let t = if it.duration <= 0.0 {
                1.0
            } else {
                (it.elapsed / it.duration).min(1.0)
            };
            self.apply_interp(id, &it, t);
            if t >= 1.0 {
                if let Some(cb) = it.callback {
                    due.push((cb, Vec::new()));
                }
            } else if let Some(w) = self.widgets.get_mut(&id) {
                w.interp = Some(it);
            }
        }
        for w in self.widgets.values() {
            if let Some(cb) = w.update
                && !w.sleeping
            {
                due.push((cb, vec![f64::from(dt)]));
            }
        }
        due
    }

    fn apply_interp(&mut self, id: u32, it: &Interp, t: f32) {
        let lerp = |a: f32, b: f32| a + (b - a) * t;
        let rect = [0, 1, 2, 3].map(|i| lerp(it.from_rect[i], it.to_rect[i]));
        self.set_rect(id, rect);
        let colour = [0, 1, 2, 3].map(|i| it.to_colour[i].map(|to| lerp(it.from_colour[i], to)));
        if colour.iter().any(Option::is_some) {
            self.set_colour(id, colour, true);
        }
        if let Some(w) = self.widgets.get_mut(&id) {
            if let Some(to) = it.to_uv {
                w.uv = [0, 1, 2, 3].map(|i| lerp(it.from_uv[i], to[i]));
            }
            if let Some(to) = it.to_rotation {
                w.rotation = lerp(it.from_rotation, to);
            }
        }
    }

    /// The text a text widget shows: an id in brackets looked up in the
    /// strings given, else as set.
    pub fn display_text(&self, text: &str) -> String {
        let id = text.strip_prefix('[').and_then(|t| t.strip_suffix(']'));
        match (id, &self.strings) {
            (Some(id), Some(s)) => s.lookup(id).unwrap_or(text).to_owned(),
            _ => text.to_owned(),
        }
    }

    /// Text width in virtual units, by the font if given one.
    fn text_width(&self, id: u32) -> f32 {
        let Some(w) = self.widgets.get(&id) else {
            return 0.0;
        };
        let text = self.display_text(&w.text);
        let px = match self.fonts.get(&w.font.to_ascii_lowercase()) {
            Some(f) => f.text_width(&text),
            None => text.chars().count() as f32 * 12.0,
        };
        px * VIRTUAL_SIZE.1 / FONT_LINES * w.scale
    }

    fn text_height(&self, id: u32) -> f32 {
        let Some(w) = self.widgets.get(&id) else {
            return 0.0;
        };
        let px = self
            .fonts
            .get(&w.font.to_ascii_lowercase())
            .map_or(30.0, |f| f32::from(f.line_height));
        px * VIRTUAL_SIZE.1 / FONT_LINES * w.scale
    }

    /// The correction of the widget's nearest corrected ancestor (itself
    /// included).
    fn correction(&self, id: u32, width: f32, height: f32) -> Correction {
        let scale = height / VIRTUAL_SIZE.1;
        let mut at = Some(id);
        while let Some(i) = at {
            let Some(w) = self.widgets.get(&i) else {
                break;
            };
            if let Some(p) = w.correction {
                return Correction {
                    scale,
                    offset: [
                        p[0] * width / VIRTUAL_SIZE.0 - p[0] * scale,
                        p[1] * height / VIRTUAL_SIZE.1 - p[1] * scale,
                    ],
                };
            }
            at = w.parent;
        }
        Correction {
            scale,
            offset: [0.0; 2],
        }
    }

    /// What to draw on a `width` x `height` screen, back to front.
    pub fn draw_list(&self, width: f32, height: f32) -> Vec<DrawItem> {
        let mut out = Vec::new();
        for &id in &self.order {
            let Some(w) = self.widgets.get(&id) else {
                continue;
            };
            if !w.visible || w.sleeping {
                continue;
            }
            let c = self.correction(id, width, height);
            let rect = c.apply(self.rect(id));
            let colour = w.colour.map(|v| (v / 255.0).clamp(0.0, 1.0));
            match w.kind {
                Kind::Widget => {}
                Kind::Image => match &w.texture {
                    Some(t) => {
                        let uv = match w.tile {
                            Some((tw, th)) => {
                                let r = self.rect(id);
                                [0.0, 0.0, (r[2] - r[0]) / tw, (r[3] - r[1]) / th]
                            }
                            None => w.uv,
                        };
                        out.push(DrawItem::Quad {
                            widget: id,
                            texture: t.clone(),
                            uv,
                            colour,
                            rect,
                            rotation: w.rotation,
                        });
                    }
                    None => out.push(DrawItem::Fill {
                        widget: id,
                        colour,
                        rect,
                    }),
                },
                Kind::Sprite => {
                    if let Some(t) = &w.texture {
                        out.push(DrawItem::Quad {
                            widget: id,
                            texture: t.clone(),
                            uv: sprite_uv(w),
                            colour,
                            rect,
                            rotation: w.rotation,
                        });
                    }
                }
                Kind::Text => {
                    if !w.text.is_empty() {
                        out.push(DrawItem::Text {
                            widget: id,
                            font: w.font.clone(),
                            text: self.display_text(&w.text),
                            rect,
                            size: c.scale * VIRTUAL_SIZE.1 / FONT_LINES * w.scale,
                            colour,
                            justification: w.justification,
                        });
                    }
                }
                Kind::Flash => out.push(DrawItem::Flash {
                    widget: id,
                    swf: w.swf.clone(),
                    rect,
                }),
                Kind::Minimap => out.push(DrawItem::Minimap { widget: id, rect }),
                Kind::Movie => out.push(DrawItem::Movie {
                    widget: id,
                    file: w.movie.clone(),
                    rect,
                }),
            }
        }
        out
    }

    fn log_unknown(&mut self, name: &str) {
        self.unknown.insert(name.to_owned());
    }
}

/// The texture rectangle of a sprite's current frame: frames left to
/// right, then down, from the frame rectangle's corner.
fn sprite_uv(w: &Widget) -> [f32; 4] {
    let [tw, th] = w.sprite_texture_size;
    let [fw, fh] = w.sprite_frame_size;
    if tw <= 0.0 || th <= 0.0 || fw <= 0.0 || fh <= 0.0 {
        return w.uv;
    }
    let cols = ((tw / fw).floor() as u32).max(1);
    let (col, row) = (w.sprite_frame % cols, w.sprite_frame / cols);
    let u = w.uv[0] + col as f32 * fw / tw;
    let v = w.uv[1] + row as f32 * fh / th;
    [u, v, u + fw / tw, v + fh / th]
}

/// Maps `r` as it sat in `from` onto `to`, per axis (a zero-size `from`
/// axis only moves).
fn map_rect(r: [f32; 4], from: [f32; 4], to: [f32; 4]) -> [f32; 4] {
    let axis = |v: f32, a0: f32, a1: f32, b0: f32, b1: f32| {
        if (a1 - a0).abs() < 1e-6 {
            v + (b0 - a0)
        } else {
            b0 + (v - a0) * (b1 - b0) / (a1 - a0)
        }
    };
    [
        axis(r[0], from[0], from[2], to[0], to[2]),
        axis(r[1], from[1], from[3], to[1], to[3]),
        axis(r[2], from[0], from[2], to[0], to[2]),
        axis(r[3], from[1], from[3], to[1], to[3]),
    ]
}

/// The anchor a script names (`left`, `center`, `right`, `top`, `bottom`).
fn anchor_of(v: Option<&Value>) -> Anchor {
    let Some(Value::String(s)) = v else {
        return Anchor::Start;
    };
    let s = s.to_string_lossy().to_ascii_lowercase();
    if s.contains("cent") || s.contains("mid") {
        Anchor::Centre
    } else if s.contains("right") || s.contains("bottom") {
        Anchor::End
    } else {
        Anchor::Start
    }
}

fn anchor_name(a: Anchor, horizontal: bool) -> &'static str {
    match (a, horizontal) {
        (Anchor::Start, true) => "left",
        (Anchor::Start, false) => "top",
        (Anchor::Centre, _) => "center",
        (Anchor::End, true) => "right",
        (Anchor::End, false) => "bottom",
    }
}

/// A native's arguments.
struct Args(Vec<Value>);

impl Args {
    fn get(&self, i: usize) -> Option<&Value> {
        self.0.get(i)
    }

    fn num(&self, i: usize) -> Option<f64> {
        match self.0.get(i) {
            Some(Value::Integer(n)) => Some(*n as f64),
            Some(Value::Number(n)) => Some(*n),
            _ => None,
        }
    }

    fn f(&self, i: usize) -> Option<f32> {
        self.num(i).map(|v| v as f32)
    }

    fn id(&self, i: usize) -> u32 {
        self.num(i).map_or(0, |v| v as u32)
    }

    fn text(&self, i: usize) -> Option<String> {
        match self.0.get(i) {
            Some(Value::String(s)) => Some(s.to_string_lossy()),
            Some(Value::Integer(n)) => Some(n.to_string()),
            Some(Value::Number(n)) => Some(n.to_string()),
            _ => None,
        }
    }

    fn truthy(&self, i: usize) -> bool {
        !matches!(
            self.0.get(i),
            None | Some(Value::Nil) | Some(Value::Boolean(false))
        )
    }

    /// Everything from `i` on, as text (for recorded calls).
    fn texts(&self, i: usize) -> Vec<String> {
        self.0
            .iter()
            .skip(i)
            .map(|v| match v {
                Value::String(s) => s.to_string_lossy(),
                Value::Integer(n) => n.to_string(),
                Value::Number(n) => n.to_string(),
                Value::Boolean(b) => b.to_string(),
                Value::Nil => "nil".to_owned(),
                other => format!("<{}>", other.type_name()),
            })
            .collect()
    }
}

fn ret<T: IntoLuaMulti>(lua: &Lua, v: T) -> mlua::Result<MultiValue> {
    v.into_lua_multi(lua)
}

/// Keeps a function and its data for calling later; `None` if `f` is not
/// a function.
fn keep_callback(
    screen: &mut Screen,
    lua: &Lua,
    f: Option<&Value>,
    data: Option<&Value>,
) -> mlua::Result<Option<u32>> {
    let Some(Value::Function(f)) = f else {
        return Ok(None);
    };
    screen.next_callback += 1;
    let handle = screen.next_callback;
    let table: Table = lua.named_registry_value(CALLBACKS)?;
    let entry = lua.create_table()?;
    entry.raw_set(1, f.clone())?;
    if let Some(d @ Value::Table(_)) = data {
        entry.raw_set(2, d.clone())?;
    }
    table.raw_set(handle, entry)?;
    Ok(Some(handle))
}

/// Calls a kept callback with its data then `extra`.
fn call_back(lua: &Lua, handle: u32, extra: &[f64]) -> mlua::Result<()> {
    let table: Table = lua.named_registry_value(CALLBACKS)?;
    let Some(entry) = table.raw_get::<Option<Table>>(handle)? else {
        return Ok(());
    };
    let f: Function = entry.raw_get(1)?;
    let mut args: Vec<Value> = match entry.raw_get::<Option<Table>>(2)? {
        Some(d) => d.sequence_values::<Value>().collect::<mlua::Result<_>>()?,
        None => Vec::new(),
    };
    args.extend(extra.iter().map(|v| Value::Number(*v)));
    f.call::<()>(MultiValue::from_vec(args))
}

fn forget_callback(lua: &Lua, handle: u32) -> mlua::Result<()> {
    let table: Table = lua.named_registry_value(CALLBACKS)?;
    table.raw_set(handle, Value::Nil)
}

/// The screen installed in `lua`, if any.
pub fn screen(lua: &Lua) -> Option<ScreenRef> {
    lua.app_data_ref::<ScreenRef>().map(|s| Rc::clone(&s))
}

type Native = fn(&mut Screen, &Lua, &Args) -> mlua::Result<MultiValue>;

/// Installs `_GuiInternal` over a new [`Screen`] (kept as the Lua state's
/// app data, see [`screen`]) and returns it.
pub fn install(lua: &Lua) -> mlua::Result<ScreenRef> {
    let screen: ScreenRef = Rc::new(RefCell::new(Screen::default()));
    lua.set_named_registry_value(CALLBACKS, lua.create_table()?)?;
    let g = lua.create_table()?;
    for (name, f) in NATIVES {
        let s = Rc::clone(&screen);
        let f: Native = *f;
        g.raw_set(
            *name,
            safe_function(lua, move |lua, args| {
                let args = Args(args.into_iter().collect());
                // Natives never call back into Lua while borrowed.
                f(&mut s.borrow_mut(), lua, &args)
            })?,
        )?;
    }
    // The current engine: `Widget:SetVisible` leaves the children to the
    // native.
    g.raw_set("nVersion", 1)?;
    let s = Rc::clone(&screen);
    let fallback = safe_function(lua, move |lua, args| {
        let Some(Value::String(k)) = args.into_iter().nth(1) else {
            return Ok(MultiValue::new());
        };
        let k = k.to_string_lossy();
        s.borrow_mut().log_unknown(&k);
        let s = Rc::clone(&s);
        let f = if k.starts_with("Create") {
            safe_function(lua, move |lua, _| {
                let id = s.borrow_mut().create(Kind::Widget, String::new());
                ret(lua, id)
            })?
        } else if k.starts_with("Get") {
            safe_function(lua, |lua, _| ret(lua, (0, 0, 0, 0)))?
        } else if k.starts_with("Is") {
            safe_function(lua, |lua, _| ret(lua, false))?
        } else {
            safe_function(lua, |_, _| Ok(MultiValue::new()))?
        };
        ret(lua, f)
    })?;
    let meta = lua.create_table()?;
    meta.raw_set("__index", fallback)?;
    g.set_metatable(Some(meta))?;
    lua.globals().raw_set("_GuiInternal", g)?;
    lua.set_app_data(Rc::clone(&screen));
    Ok(screen)
}

/// Advances the screen by `dt` seconds: animations step and call back when
/// done, update callbacks run, and `MrxGuiBase.Run()` processes the events
/// the scripts queued. Errors are kept in the log ([`Screen::take_log`]).
pub fn tick(lua: &Lua, dt: f64) {
    let Some(screen) = screen(lua) else {
        return;
    };
    let due = screen.borrow_mut().advance(dt as f32);
    let mut errors = Vec::new();
    for (handle, extra) in due {
        let once = extra.is_empty();
        if let Err(e) = call_back(lua, handle, &extra) {
            errors.push(format!("gui callback: {e}"));
        }
        if once {
            let _ = forget_callback(lua, handle);
        }
    }
    let run = lua
        .globals()
        .get::<Option<Table>>("MrxGuiBase")
        .ok()
        .flatten()
        .and_then(|m| m.get::<Option<Function>>("Run").ok().flatten());
    if let Some(run) = run
        && let Err(e) = run.call::<()>(())
    {
        errors.push(format!("MrxGuiBase.Run: {e}"));
    }
    screen.borrow_mut().log.extend(errors);
}

/// Delivers an engine event to the handlers registered for it, with
/// `args` after each handler's own data (`GuiHealthUpdate` with health,
/// maximum and whether in a vehicle). Returns how many handlers ran.
///
/// `GuiInitialization` is special: the game calls each layout's top-level
/// widgets' handlers as the layout loads, and the rest through the event;
/// this calls the handlers of widgets with a parent, in creation order.
pub fn fire(lua: &Lua, event: &str, args: impl IntoLuaMulti) -> mlua::Result<usize> {
    let args = args.into_lua_multi(lua)?;
    if event == "GuiInitialization" {
        return lua
            .load(
                r#"
                local n = 0
                local list = {}
                for _, w in pairs(WidgetIdIndex or {}) do list[#list + 1] = w end
                table.sort(list, function(a, b) return a.BasicData.uId < b.BasicData.uId end)
                for _, w in ipairs(list) do
                    local h = w.EventHandlers and w.EventHandlers.GuiInitialization
                    if h and w.ParentWidget then
                        n = n + 1
                        h(w)
                    end
                end
                return n
                "#,
            )
            .set_name("=m2-gui-init")
            .eval::<usize>();
    }
    let event_lib: Option<Table> = lua.globals().get("Event")?;
    let Some(fire) =
        event_lib.and_then(|e| e.raw_get::<Option<Function>>("__m2_fire").ok().flatten())
    else {
        return Ok(0);
    };
    let mut call = vec![Value::String(lua.create_string(event)?)];
    call.extend(args);
    fire.call::<usize>(MultiValue::from_vec(call))
}

macro_rules! natives {
    ($($name:literal => $f:expr,)*) => {
        const NATIVES: &[(&str, Native)] = &[$(($name, $f),)*];
    };
}

fn create(kind: Kind) -> impl Fn(&mut Screen, &Lua, &Args) -> mlua::Result<MultiValue> {
    move |s, lua, a| {
        let id = s.create(kind, a.text(0).unwrap_or_default());
        ret(lua, id)
    }
}

natives! {
    "CreateWidget" => |s, l, a| create(Kind::Widget)(s, l, a),
    "CreateImageWidget" => |s, l, a| create(Kind::Image)(s, l, a),
    "CreateTextWidget" => |s, l, a| create(Kind::Text)(s, l, a),
    "CreateSpriteWidget" => |s, l, a| create(Kind::Sprite)(s, l, a),
    "CreateFlashWidget" => |s, l, a| create(Kind::Flash)(s, l, a),
    "CreateMovieWidget" => |s, l, a| create(Kind::Movie)(s, l, a),
    "DeleteWidget" => |s, _, a| {
        s.delete(a.id(0));
        Ok(MultiValue::new())
    },
    "ActivateWidget" => |_, _, _| Ok(MultiValue::new()),
    "SetWidgetVisible" => |s, _, a| {
        s.set_visible(a.id(0), a.truthy(1));
        Ok(MultiValue::new())
    },
    "GetWidgetVisible" => |s, l, a| ret(l, s.widget(a.id(0)).is_some_and(|w| w.visible)),
    "SetWidgetSleep" => |s, _, a| {
        let sleep = a.truthy(1);
        for i in s.subtree(a.id(0)) {
            if let Some(w) = s.widgets.get_mut(&i) {
                w.sleeping = sleep;
            }
        }
        Ok(MultiValue::new())
    },
    "GetWidgetSleep" => |s, l, a| ret(l, s.widget(a.id(0)).is_some_and(|w| w.sleeping)),
    "SetWidgetLocation" => |s, _, a| {
        let id = a.id(0);
        let old = s.rect(id);
        let r = [
            a.f(1).unwrap_or(old[0]),
            a.f(2).unwrap_or(old[1]),
            a.f(3).unwrap_or(old[2]),
            a.f(4).unwrap_or(old[3]),
        ];
        s.set_rect(id, r);
        Ok(MultiValue::new())
    },
    "GetWidgetLocation" => |s, l, a| {
        let r = s.rect(a.id(0));
        ret(l, (r[0], r[1], r[2], r[3]))
    },
    // The corrected box is the box: corrections apply when drawing.
    "SetWidgetCorrectedLocation" => |s, _, a| {
        let id = a.id(0);
        let old = s.rect(id);
        let r = [
            a.f(1).unwrap_or(old[0]),
            a.f(2).unwrap_or(old[1]),
            a.f(3).unwrap_or(old[2]),
            a.f(4).unwrap_or(old[3]),
        ];
        s.set_rect(id, r);
        Ok(MultiValue::new())
    },
    "GetWidgetCorrectedLocation" => |s, l, a| {
        let r = s.rect(a.id(0));
        ret(l, (r[0], r[1], r[2], r[3]))
    },
    "SetWidgetColor" => |s, _, a| {
        let ch = |i| a.num(i).filter(|v| *v >= KEEP).map(|v| v as f32);
        s.set_colour(a.id(0), [ch(1), ch(2), ch(3), ch(4)], a.truthy(5));
        Ok(MultiValue::new())
    },
    "GetWidgetColor" => |s, l, a| {
        let c = s.widget(a.id(0)).map_or([255.0; 4], |w| w.colour);
        ret(l, (c[0], c[1], c[2], c[3]))
    },
    "SetWidgetAnchoring" => |s, _, a| {
        if let Some(w) = s.widgets.get_mut(&a.id(0)) {
            w.anchors = (anchor_of(a.get(1)), anchor_of(a.get(2)));
        }
        Ok(MultiValue::new())
    },
    "GetWidgetAnchoring" => |s, l, a| {
        let (h, v) = s.widget(a.id(0)).map_or((Anchor::Start, Anchor::Start), |w| w.anchors);
        ret(l, (anchor_name(h, true), anchor_name(v, false)))
    },
    "CorrectWidgetForResolution" => |s, _, a| {
        let id = a.id(0);
        let r = s.rect(id);
        if let Some(w) = s.widgets.get_mut(&id) {
            let point = |anchor, lo: f32, hi: f32| match anchor {
                Anchor::Start => lo,
                Anchor::Centre => (lo + hi) / 2.0,
                Anchor::End => hi,
            };
            w.correction = Some([point(w.anchors.0, r[0], r[2]), point(w.anchors.1, r[1], r[3])]);
        }
        Ok(MultiValue::new())
    },
    "AddWidgetChild" => |s, _, a| {
        s.add_child(a.id(0), a.id(1));
        Ok(MultiValue::new())
    },
    "SetWidgetChild" => |s, _, a| {
        s.add_child(a.id(0), a.id(1));
        Ok(MultiValue::new())
    },
    "RemoveWidgetChild" => |s, _, a| {
        s.remove_child(a.id(0), a.id(1));
        Ok(MultiValue::new())
    },
    "RemoveAllWidgetChildren" => |s, _, a| {
        let id = a.id(0);
        let children = s.widget(id).map(|w| w.children.clone()).unwrap_or_default();
        for c in children {
            s.remove_child(id, c);
        }
        Ok(MultiValue::new())
    },
    "GetWidgetChildren" => |s, l, a| {
        let children = s.widget(a.id(0)).map(|w| w.children.clone()).unwrap_or_default();
        let t = l.create_sequence_from(children.iter().copied())?;
        ret(l, (t, children.len()))
    },
    "PushWidgetToFront" => |s, _, a| {
        let ids = s.subtree(a.id(0));
        s.order.retain(|o| !ids.contains(o));
        s.order.extend(ids);
        Ok(MultiValue::new())
    },
    "PushWidgetToBack" => |s, _, a| {
        let ids = s.subtree(a.id(0));
        s.order.retain(|o| !ids.contains(o));
        let mut order = ids;
        order.append(&mut s.order);
        s.order = order;
        Ok(MultiValue::new())
    },
    "SetWidgetUpdateCallback" => |s, l, a| {
        let id = a.id(0);
        let old = s.widget(id).and_then(|w| w.update);
        if let Some(h) = old {
            forget_callback(l, h)?;
        }
        let cb = keep_callback(s, l, a.get(1), a.get(2))?;
        if let Some(w) = s.widgets.get_mut(&id) {
            w.update = cb;
        }
        Ok(MultiValue::new())
    },
    "InterpolateWidget" => |s, l, a| {
        // (id, time, x1, y1, x2, y2, r, g, b, a, callback, data, u1, v1,
        //  u2, v2, rotation, rotation direction, elapsed)
        let id = a.id(0);
        let Some(w) = s.widget(id) else {
            return Ok(MultiValue::new());
        };
        let from_rect = s.rect(id);
        let (from_colour, from_uv, from_rotation) = (w.colour, w.uv, w.rotation);
        let (x1, y1) = (a.f(2).unwrap_or(from_rect[0]), a.f(3).unwrap_or(from_rect[1]));
        let to_rect = match (a.f(4), a.f(5)) {
            (Some(x2), Some(y2)) => [x1, y1, x2, y2],
            // Moved, same size.
            _ => [
                x1,
                y1,
                x1 + from_rect[2] - from_rect[0],
                y1 + from_rect[3] - from_rect[1],
            ],
        };
        let ch = |i| a.num(i).filter(|v| *v >= KEEP).map(|v| v as f32);
        let to_uv = match (a.f(12), a.f(13), a.f(14), a.f(15)) {
            (Some(u1), Some(v1), Some(u2), Some(v2)) => Some([u1, v1, u2, v2]),
            _ => None,
        };
        let to_rotation = a.f(16).map(|r| {
            let mut d = r - from_rotation;
            match a.f(17) {
                Some(dir) if dir > 0.0 && d < 0.0 => d += 360.0,
                Some(dir) if dir < 0.0 && d > 0.0 => d -= 360.0,
                _ => {}
            }
            from_rotation + d
        });
        let callback = keep_callback(s, l, a.get(10), a.get(11))?;
        let it = Interp {
            duration: a.f(1).unwrap_or(0.0).max(0.0),
            elapsed: a.f(18).unwrap_or(0.0).max(0.0),
            from_rect,
            to_rect,
            from_colour,
            to_colour: [ch(6), ch(7), ch(8), ch(9)],
            from_uv,
            to_uv,
            from_rotation,
            to_rotation,
            callback,
        };
        if let Some(w) = s.widgets.get_mut(&id) {
            // A new animation replaces a running one, which never calls
            // back.
            if let Some(old) = w.interp.replace(it).and_then(|o| o.callback) {
                forget_callback(l, old)?;
            }
        }
        Ok(MultiValue::new())
    },
    "SetImageTexture" => |s, _, a| {
        if let Some(w) = s.widgets.get_mut(&a.id(0)) {
            w.texture = a.text(1).filter(|t| !t.is_empty());
        }
        Ok(MultiValue::new())
    },
    "SetSpriteTexture" => |s, _, a| {
        if let Some(w) = s.widgets.get_mut(&a.id(0)) {
            w.texture = a.text(1).filter(|t| !t.is_empty());
        }
        Ok(MultiValue::new())
    },
    "SetImageTextureCoordinates" => |s, _, a| {
        if let Some(w) = s.widgets.get_mut(&a.id(0)) {
            let old = w.uv;
            w.uv = [
                a.f(1).unwrap_or(old[0]),
                a.f(2).unwrap_or(old[1]),
                a.f(3).unwrap_or(old[2]),
                a.f(4).unwrap_or(old[3]),
            ];
        }
        Ok(MultiValue::new())
    },
    "GetImageTextureCoordinates" => |s, l, a| {
        let uv = s.widget(a.id(0)).map_or([0.0, 0.0, 1.0, 1.0], |w| w.uv);
        ret(l, (uv[0], uv[1], uv[2], uv[3]))
    },
    "SetImageRotation" => |s, _, a| {
        if let Some(w) = s.widgets.get_mut(&a.id(0)) {
            w.rotation = a.f(1).unwrap_or(0.0);
        }
        Ok(MultiValue::new())
    },
    "GetImageRotation" => |s, l, a| ret(l, s.widget(a.id(0)).map_or(0.0, |w| w.rotation)),
    "SetImageTiling" => |s, _, a| {
        if let Some(w) = s.widgets.get_mut(&a.id(0)) {
            w.tile = match (a.f(1), a.f(2)) {
                (Some(x), Some(y)) if x > 0.0 && y > 0.0 => Some((x, y)),
                _ => None,
            };
        }
        Ok(MultiValue::new())
    },
    "SetImagePieSliceRender" => |s, _, a| {
        if let Some(w) = s.widgets.get_mut(&a.id(0)) {
            w.pie_slice = Some((1..a.0.len()).filter_map(|i| a.f(i)).collect());
        }
        Ok(MultiValue::new())
    },
    "DisableImagePieSliceRender" => |s, _, a| {
        if let Some(w) = s.widgets.get_mut(&a.id(0)) {
            w.pie_slice = None;
        }
        Ok(MultiValue::new())
    },
    "SetImageTextureTransience" => |_, _, _| Ok(MultiValue::new()),
    "SetSpriteTextureSize" => |s, _, a| {
        if let Some(w) = s.widgets.get_mut(&a.id(0)) {
            w.sprite_texture_size = [a.f(1).unwrap_or(0.0), a.f(2).unwrap_or(0.0)];
        }
        Ok(MultiValue::new())
    },
    "SetSpriteFrameSize" => |s, _, a| {
        if let Some(w) = s.widgets.get_mut(&a.id(0)) {
            w.sprite_frame_size = [a.f(1).unwrap_or(0.0), a.f(2).unwrap_or(0.0)];
        }
        Ok(MultiValue::new())
    },
    "SetSpriteFrame" => |s, _, a| {
        if let Some(w) = s.widgets.get_mut(&a.id(0)) {
            w.sprite_frame = a.num(1).map_or(0, |f| f.max(0.0) as u32);
        }
        Ok(MultiValue::new())
    },
    "SetTextText" => |s, _, a| {
        if let Some(w) = s.widgets.get_mut(&a.id(0)) {
            w.text = a.text(1).unwrap_or_default();
        }
        Ok(MultiValue::new())
    },
    "GetTextText" => |s, l, a| ret(l, s.widget(a.id(0)).map(|w| w.text.clone()).unwrap_or_default()),
    "SetTextFont" => |s, _, a| {
        if let Some(w) = s.widgets.get_mut(&a.id(0)) {
            w.font = a.text(1).unwrap_or_default();
        }
        Ok(MultiValue::new())
    },
    "SetTextScale" => |s, _, a| {
        if let Some(w) = s.widgets.get_mut(&a.id(0)) {
            w.scale = a.f(1).unwrap_or(1.0);
        }
        Ok(MultiValue::new())
    },
    "GetTextScale" => |s, l, a| ret(l, s.widget(a.id(0)).map_or(1.0, |w| w.scale)),
    "SetTextJustification" => |s, _, a| {
        if let Some(w) = s.widgets.get_mut(&a.id(0)) {
            w.justification = anchor_of(a.get(1));
        }
        Ok(MultiValue::new())
    },
    "GetTextJustification" => |s, l, a| {
        let j = s.widget(a.id(0)).map_or(Anchor::Start, |w| w.justification);
        ret(l, anchor_name(j, true))
    },
    "SetTextWrapping" => |_, _, _| Ok(MultiValue::new()),
    "GetTextWidth" => |s, l, a| ret(l, s.text_width(a.id(0))),
    "GetTextHeight" => |s, l, a| ret(l, s.text_height(a.id(0))),
    "SetFlashSwfFile" => |s, l, a| {
        let id = a.id(0);
        if let Some(w) = s.widgets.get_mut(&id) {
            w.swf = a.text(1);
        }
        // Loaded at once: the callback runs at the next tick.
        if let Some(cb) = keep_callback(s, l, a.get(2), a.get(3))? {
            s.due.push((cb, Vec::new()));
        }
        Ok(MultiValue::new())
    },
    "CallFlashScriptFunction" => |s, _, a| {
        let id = a.id(0);
        let name = a.text(1).unwrap_or_default();
        let rest = match a.get(2) {
            Some(Value::Table(t)) => t
                .sequence_values::<Value>()
                .filter_map(Result::ok)
                .map(|v| Args(vec![v]).texts(0).concat())
                .collect(),
            _ => a.texts(2),
        };
        if let Some(w) = s.widgets.get_mut(&id) {
            w.flash_calls.push((name, rest));
        }
        Ok(MultiValue::new())
    },
    "SetMovieFile" => |s, _, a| {
        if let Some(w) = s.widgets.get_mut(&a.id(0)) {
            w.movie = a.text(1);
        }
        Ok(MultiValue::new())
    },
    "MinimapCreate" => |s, _, a| {
        if let Some(w) = s.widgets.get_mut(&a.id(0)) {
            w.kind = Kind::Minimap;
            w.minimap = Some(a.texts(1));
        }
        Ok(MultiValue::new())
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::ScriptHost;

    fn host() -> (ScriptHost, ScreenRef) {
        let host = ScriptHost::new().unwrap();
        let screen = install(host.lua()).unwrap();
        (host, screen)
    }

    fn run(host: &ScriptHost, code: &str) {
        host.lua().load(code).exec().unwrap();
    }

    #[test]
    fn natives_build_place_and_colour_widgets() {
        let (host, screen) = host();
        run(
            &host,
            r#"
            local G = _GuiInternal
            box = G.CreateWidget()
            img = G.CreateImageWidget()
            G.SetWidgetLocation(box, 100, 100, 200, 140, true)
            G.SetWidgetLocation(img, 110, 110, 150, 130, true)
            G.SetImageTexture(img, "global_gui_hud02")
            G.SetImageTextureCoordinates(img, 0.5, 0, 0.25, 1)
            G.SetWidgetColor(img, 255, 128, 0, 200, false)
            G.AddWidgetChild(box, img)
            G.SetWidgetVisible(box, false)
            G.SetWidgetVisible(box, true)
            G.SetWidgetColor(box, -255, -255, -255, 64, true)
            local list, n = G.GetWidgetChildren(box)
            assert(n == 1 and list[1] == img)
            local x1, y1, x2, y2 = G.GetWidgetLocation(img)
            assert(x1 == 110 and y2 == 130)
            G.Frobnicate(img)
            "#,
        );
        let s = screen.borrow();
        let items = s.draw_list(640.0, 480.0);
        assert_eq!(items.len(), 1);
        let DrawItem::Quad {
            texture,
            uv,
            colour,
            rect,
            ..
        } = &items[0]
        else {
            panic!("{items:?}");
        };
        assert_eq!(texture, "global_gui_hud02");
        assert_eq!(*uv, [0.5, 0.0, 0.25, 1.0]);
        assert_eq!(*rect, [110.0, 110.0, 150.0, 130.0]);
        // Colour kept, alpha set through the parent.
        assert!((colour[1] - 128.0 / 255.0).abs() < 1e-6);
        assert!((colour[3] - 64.0 / 255.0).abs() < 1e-6);
        assert!(s.unknown().contains("Frobnicate"));
    }

    #[test]
    fn children_follow_their_parent() {
        let (host, screen) = host();
        run(
            &host,
            r#"
            local G = _GuiInternal
            p = G.CreateWidget()
            c = G.CreateImageWidget()
            G.SetWidgetLocation(p, 0, 0, 100, 100)
            G.SetWidgetLocation(c, 10, 20, 30, 40)
            G.AddWidgetChild(p, c)
            -- Moved and squashed to zero height, then back.
            G.SetWidgetLocation(p, 50, 50, 150, 50)
            local _, y1, _, y2 = G.GetWidgetLocation(c)
            assert(y1 == 50 and y2 == 50)
            G.SetWidgetLocation(p, 50, 0, 150, 100)
            "#,
        );
        let s = screen.borrow();
        let c: u32 = host.lua().globals().get("c").unwrap();
        assert_eq!(s.rect(c), [60.0, 20.0, 80.0, 40.0]);
    }

    #[test]
    fn interpolation_steps_and_calls_back() {
        let (host, screen) = host();
        run(
            &host,
            r#"
            local G = _GuiInternal
            w = G.CreateImageWidget()
            G.SetWidgetLocation(w, 0, 0, 10, 10)
            done = 0
            G.InterpolateWidget(w, 1, 100, 0, nil, nil, -4096, -4096, -4096, 0,
                function(x) done = done + x end, { 5 })
            ticks = 0
            G.SetWidgetUpdateCallback(w, function(tag, dt) ticks = ticks + dt end, { "w" })
            "#,
        );
        tick(host.lua(), 0.5);
        {
            let s = screen.borrow();
            let w = s.widget(1).unwrap();
            assert_eq!(s.rect(1), [50.0, 0.0, 60.0, 10.0]);
            assert!((w.colour[3] - 127.5).abs() < 1e-3);
            assert_eq!(w.colour[0], 255.0);
        }
        tick(host.lua(), 0.5);
        tick(host.lua(), 0.5);
        let g = host.lua().globals();
        assert_eq!(g.get::<f64>("done").unwrap(), 5.0);
        assert!((g.get::<f64>("ticks").unwrap() - 1.5).abs() < 1e-9);
        assert_eq!(screen.borrow().rect(1), [100.0, 0.0, 110.0, 10.0]);
        assert!(screen.borrow_mut().take_log().is_empty());
    }

    #[test]
    fn draw_list_corrects_top_level_widgets_and_orders() {
        let (host, screen) = host();
        run(
            &host,
            r#"
            local G = _GuiInternal
            top = G.CreateWidget()
            G.SetWidgetLocation(top, 460, 40, 600, 110)
            G.SetWidgetAnchoring(top, "right", "top")
            t = G.CreateTextWidget()
            G.SetWidgetLocation(t, 500, 50, 590, 70)
            G.SetTextText(t, "[Ammo.Clip]")
            G.SetTextFont(t, "english_20")
            G.SetTextScale(t, 0.5)
            G.SetTextJustification(t, "right")
            G.AddWidgetChild(top, t)
            G.CorrectWidgetForResolution(top)
            back = G.CreateImageWidget()
            G.SetWidgetLocation(back, 0, 0, 640, 480)
            G.PushWidgetToBack(back)
            "#,
        );
        let mut s = screen.borrow_mut();
        let mut strings = StringTable::default();
        strings
            .strings
            .insert(m2_assets::name_hash("Ammo.Clip"), "29".to_owned());
        s.set_strings(strings);
        let items = s.draw_list(1280.0, 720.0);
        assert!(matches!(items[0], DrawItem::Fill { .. }));
        let DrawItem::Text {
            text,
            rect,
            size,
            justification,
            ..
        } = &items[1]
        else {
            panic!("{items:?}");
        };
        assert_eq!(text, "29");
        // The right edge at 600/640 of the width; the child 10 in from it.
        assert_eq!(*rect, [1200.0 - 150.0, 75.0, 1200.0 - 15.0, 105.0]);
        assert!((size - 0.5).abs() < 1e-6);
        assert_eq!(*justification, Anchor::End);
    }
}
