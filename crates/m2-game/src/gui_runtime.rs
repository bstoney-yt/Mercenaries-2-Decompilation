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

use m2_assets::{Font, StringTable, name_hash};
use mlua::{Function, IntoLuaMulti, Lua, MultiValue, Table, Value};

use crate::gui::{Anchor, Correction, VIRTUAL_SIZE};
use crate::script::{safe_function, show_arg};

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
    /// Taken off the screen by `ActivateWidget(id, false)` (as
    /// `WidgetManager.RemoveWidget` does): not drawn. Its update callback
    /// still runs: the PDA's close (`MrxGuiPda.Close`) removes it and then
    /// counts its reopening cooldown down in its `GuiUpdate`.
    pub inactive: bool,
    /// `SetMovieEndCallback`: what runs when its movie ends (`PlayMovie`
    /// plays none here, so it ends at once).
    pub movie_end: Option<u32>,
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
    /// A sprite run (`AnimateSprite`) under way.
    pub sprite_run: Option<SpriteRun>,
    /// `SetImagePieSliceRender` arguments, as given.
    pub pie_slice: Option<Vec<f32>>,
    pub text: String,
    pub font: String,
    pub scale: f32,
    pub justification: Anchor,
    /// Wrapped at the widget's width (`SetTextWrapping`, which the
    /// scripts' `TextWidget:Wrap` turns on).
    pub wraps: bool,
    pub swf: Option<String>,
    /// `CallFlashScriptFunction` calls not yet taken by the host
    /// ([`Screen::take_flash_calls`]): the movie's function and its
    /// arguments.
    pub flash_calls: Vec<(String, Vec<FlashArg>)>,
    /// `SetFlashCallback`: the movie's `fscommand` names and the callbacks
    /// they call ([`flash_command`]).
    flash_handlers: BTreeMap<String, u32>,
    /// `PauseFlash` stops the movie's timeline, `PlayFlash` starts it.
    pub flash_paused: bool,
    /// How many times `RestartFlash` (or a new `SetFlashSwfFile`) asked
    /// for the movie from its start.
    pub flash_restarts: u32,
    /// The PDA map's blips as last sent to the movie, by name: the movie's
    /// `UpdateBlip` finds a blip by a name it overwrites on the first
    /// update (`pointsArray[j].splice(0, ..., internalName, ...)`), so it
    /// is sent only what changed, as the engine's per-frame updates must.
    pda_blips: BTreeMap<String, Vec<FlashArg>>,
    pub movie: Option<String>,
    /// `PlayMovie` started its movie and no `StopMovie` has stopped it.
    pub movie_playing: bool,
    /// `SetWidgetFullscreen`: drawn over the whole screen, whatever its box.
    pub fullscreen: bool,
    /// `MinimapCreate` arguments as text.
    pub minimap: Option<Vec<String>>,
    /// The latest arguments of each other `Minimap*` native (range,
    /// player location, rotation, objectives...), as text.
    pub minimap_calls: BTreeMap<String, Vec<String>>,
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
            inactive: false,
            movie_end: None,
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
            sprite_run: None,
            pie_slice: None,
            text: String::new(),
            font: String::new(),
            scale: 1.0,
            justification: Anchor::Start,
            wraps: false,
            swf: None,
            flash_calls: Vec::new(),
            flash_handlers: BTreeMap::new(),
            flash_paused: false,
            flash_restarts: 0,
            pda_blips: BTreeMap::new(),
            movie: None,
            movie_playing: false,
            fullscreen: false,
            minimap: None,
            minimap_calls: BTreeMap::new(),
            update: None,
            interp: None,
        }
    }
}

/// An argument the scripts pass to a Flash movie's function
/// (`FlashWidget:CallActionScriptCallback(name, { ... })`).
#[derive(Clone, Debug, PartialEq)]
pub enum FlashArg {
    Nil,
    Bool(bool),
    Number(f64),
    Text(String),
}

impl FlashArg {
    fn of(v: &Value) -> Self {
        match v {
            Value::Boolean(b) => FlashArg::Bool(*b),
            Value::Integer(n) => FlashArg::Number(*n as f64),
            Value::Number(n) => FlashArg::Number(*n),
            Value::String(s) => FlashArg::Text(s.to_string_lossy()),
            _ => FlashArg::Nil,
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
        /// Only the wedge from the centre between these angles shows
        /// (`SetImagePieSliceRender`): degrees clockwise from straight up.
        pie: Option<[f32; 2]>,
    },
    /// An image widget without a texture: a filled rectangle.
    Fill {
        widget: u32,
        colour: [f32; 4],
        rect: [f32; 4],
    },
    /// Text in a bitmap font, in the box `rect`, from its top, each line
    /// justified; `size` is screen pixels per font pixel. `text` is the
    /// whole text (lines split by `\n`), `spans` the same in colours.
    Text {
        widget: u32,
        font: String,
        text: String,
        spans: Vec<TextSpan>,
        rect: [f32; 4],
        size: f32,
        colour: [f32; 4],
        justification: Anchor,
    },
    /// A Flash movie's place (`swf` once set). A `fullscreen` movie's
    /// rect is the whole screen.
    Flash {
        widget: u32,
        swf: Option<String>,
        rect: [f32; 4],
        fullscreen: bool,
    },
    /// The minimap: its circle's box, and what the scripts last told it
    /// (`MinimapUpdate`, `MinimapSetRange`). The map itself is the
    /// engine's to draw.
    Minimap {
        widget: u32,
        rect: [f32; 4],
        /// Where the player is (world x, y, z), if told.
        player: Option<[f32; 3]>,
        /// The rotation passed on with it, as the host sent it in
        /// `GuiMinimapUpdate`.
        rotation: f32,
        /// Metres from the centre to the rim, if set.
        range: Option<f32>,
    },
    /// A video's place, and whether it plays (`PlayMovie`, until
    /// `StopMovie`).
    Movie {
        widget: u32,
        file: Option<String>,
        rect: [f32; 4],
        playing: bool,
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
    /// Where the player aims, -1..=1 across the screen and up it
    /// (`Gui.GetReticlePosition`).
    reticle: [f32; 2],
    /// Which way the camera looks, degrees (`Player.GetCameraXZHeading`).
    heading: f32,
    /// The keys bound to control words the fonts have no picture for
    /// (`l-stick up`: `T`), by word in lower case ([`Screen::set_key_names`]).
    key_names: BTreeMap<String, String>,
}

impl Screen {
    /// Names the keys bound to control words (`[l-stick up]`):
    /// a text shows such a word as its key in brackets (`[T]`) where the
    /// fonts have no button picture for it ([`button_glyph`]).
    pub fn set_key_names(&mut self, names: impl IntoIterator<Item = (String, String)>) {
        self.key_names = names
            .into_iter()
            .map(|(word, key)| (word.to_ascii_lowercase(), key))
            .collect();
    }

    /// Sets where the player aims: -1..=1 from the left edge to the right
    /// and from the bottom to the top (0, 0 is the middle), as
    /// `Gui.GetReticlePosition` answers. Fire `GuiReticlePositionChange`
    /// with `{ nX, nY }` too when it moves.
    pub fn set_reticle(&mut self, x: f32, y: f32) {
        self.reticle = [x, y];
    }

    /// Sets which way the camera looks, as `Player.GetCameraXZHeading`
    /// answers: degrees about the vertical, growing as it turns left. The
    /// hit arcs turn by it each frame (`MrxGuiHudDamageIndicator` draws an
    /// arc at the damage's direction plus this), so they stay on whoever
    /// fired as the camera turns; widgets turn clockwise by their rotation.
    pub fn set_camera_heading(&mut self, degrees: f32) {
        self.heading = degrees;
    }

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

    /// The Flash calls the scripts made since the last call, by widget, in
    /// the order made.
    pub fn take_flash_calls(&mut self) -> Vec<(u32, String, Vec<FlashArg>)> {
        let mut calls = Vec::new();
        for (&id, w) in &mut self.widgets {
            for (name, args) in w.flash_calls.drain(..) {
                calls.push((id, name, args));
            }
        }
        calls
    }

    /// Sends a widget's PDA blips again (`UpdateBlip` each), for a map
    /// movie loaded after they were sent.
    pub fn resend_pda_blips(&mut self, id: u32) {
        if let Some(w) = self.widgets.get_mut(&id) {
            for args in w.pda_blips.values() {
                w.flash_calls.push(("UpdateBlip".to_owned(), args.clone()));
            }
        }
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
        for w in self.widgets.values_mut() {
            if let Some(run) = &mut w.sprite_run {
                run.elapsed += dt;
                let (frame, done) = run.frame();
                w.sprite_frame = frame;
                if done {
                    w.sprite_run = None;
                }
            }
        }
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

    /// The text a text widget shows, without colours (see
    /// [`spans`](Self::spans)); lines are separated by `\n`.
    pub fn display_text(&self, text: &str) -> String {
        self.spans(text).into_iter().map(|s| s.text).collect()
    }

    /// A text as the game shows it, in coloured spans. The markup:
    /// `[n]` a new line; `[green]`, `[white]`... the colour from there on;
    /// `[Id]` the text with that id (`[0x8c4d842e]` by its key hash);
    /// `[Id:a:b]` that text with `%d`, `%s`... filled from the arguments
    /// in order (`[SHELL.Common.Money:125:0:[0xe00c096a]]` is
    /// `$125.0 K`). Unknown ids stay as written.
    pub fn spans(&self, text: &str) -> Vec<TextSpan> {
        let mut out = vec![TextSpan::default()];
        self.expand(text, &mut out, 0);
        out.retain(|s| !s.text.is_empty() || s.icon.is_some());
        out
    }

    fn expand(&self, text: &str, out: &mut Vec<TextSpan>, depth: usize) {
        let push = |out: &mut Vec<TextSpan>, s: &str| {
            if let Some(last) = out.last_mut() {
                last.text.push_str(s);
            }
        };
        let mut rest = text;
        while let Some(open) = rest.find('[') {
            push(out, &rest[..open]);
            let Some(len) = bracket_len(&rest[open..]) else {
                push(out, &rest[open..]);
                return;
            };
            let inner = &rest[open + 1..open + len - 1];
            rest = &rest[open + len..];
            let parts = split_top(inner);
            let id = parts[0];
            if id == "n" && parts.len() == 1 {
                push(out, "\n");
            } else if let Some(c) = colour_name(id).filter(|_| parts.len() == 1) {
                out.push(TextSpan {
                    colour: Some(c),
                    ..TextSpan::default()
                });
            } else if let Some(glyph) = button_glyph(id).filter(|_| parts.len() == 1) {
                push(out, glyph.encode_utf8(&mut [0; 4]));
            } else if let Some(key) = self
                .key_names
                .get(&id.to_ascii_lowercase())
                .filter(|_| parts.len() == 1)
            {
                push(out, &format!("[{key}]"));
            } else if let Some(icon) = inline_icon(id).filter(|_| parts.len() == 1) {
                // In the colour so far, which goes on after it.
                let colour = out.last().and_then(|s| s.colour);
                out.push(TextSpan {
                    colour,
                    icon: Some(icon.to_owned()),
                    ..TextSpan::default()
                });
                out.push(TextSpan {
                    colour,
                    ..TextSpan::default()
                });
            } else if let Some(template) = self.lookup(id).filter(|_| depth < 8) {
                let args: Vec<String> = parts[1..]
                    .iter()
                    .map(|a| {
                        let mut sub = vec![TextSpan::default()];
                        self.expand(a, &mut sub, depth + 1);
                        sub.into_iter().map(|s| s.text).collect()
                    })
                    .collect();
                let filled = fill(&template, &args);
                self.expand(&filled, out, depth + 1);
            } else {
                push(out, &format!("[{inner}]"));
            }
        }
        push(out, rest);
    }

    fn lookup(&self, id: &str) -> Option<String> {
        let s = self.strings.as_ref()?;
        id.strip_prefix("0x")
            .and_then(|h| u32::from_str_radix(h, 16).ok())
            .and_then(|k| s.get(k))
            .or_else(|| s.lookup(id))
            .map(str::to_owned)
    }

    /// Widget `id`'s text in spans as drawn: wrapped at its width, at
    /// spaces, when it wraps ([`Widget::wraps`]).
    pub fn text_spans(&self, id: u32) -> Vec<TextSpan> {
        let Some(w) = self.widgets.get(&id) else {
            return Vec::new();
        };
        let spans = self.spans(&w.text);
        if !w.wraps {
            return spans;
        }
        let r = self.rect(id);
        let font = self.fonts.get(&w.font.to_ascii_lowercase());
        let unit = VIRTUAL_SIZE.1 / FONT_LINES * w.scale;
        let line_px = font.map_or(30.0, |f| f32::from(f.line_height));
        let width =
            |t: &str| unit * font.map_or(t.chars().count() as f32 * 12.0, |f| f.text_width(t));
        wrap_spans(spans, (r[2] - r[0]).max(1.0), width, line_px * unit)
    }

    /// Text width in virtual units, by the font if given one.
    fn text_width(&self, id: u32) -> f32 {
        let Some(w) = self.widgets.get(&id) else {
            return 0.0;
        };
        let text: String = self.text_spans(id).into_iter().map(|s| s.text).collect();
        let font = self.fonts.get(&w.font.to_ascii_lowercase());
        // The widest line.
        let px = text
            .split('\n')
            .map(|line| match font {
                Some(f) => f.text_width(line),
                None => line.chars().count() as f32 * 12.0,
            })
            .fold(0.0, f32::max);
        px * VIRTUAL_SIZE.1 / FONT_LINES * w.scale
    }

    /// Text height in virtual units: a line's for each line (wrapped
    /// ones too: the text buffers size their messages by it).
    fn text_height(&self, id: u32) -> f32 {
        let Some(w) = self.widgets.get(&id) else {
            return 0.0;
        };
        let px = self
            .fonts
            .get(&w.font.to_ascii_lowercase())
            .map_or(30.0, |f| f32::from(f.line_height));
        let text: String = self.text_spans(id).into_iter().map(|s| s.text).collect();
        let lines = text.trim_end_matches('\n').split('\n').count().max(1);
        px * lines as f32 * VIRTUAL_SIZE.1 / FONT_LINES * w.scale
    }

    /// The correction of the widget's nearest corrected ancestor (itself
    /// included). A widget never corrected draws in the virtual screen
    /// centred on the screen: as a centre-anchored one does, so a widget
    /// the scripts make later and place where a corrected one is (the hit
    /// arcs over the reticle) lands on it. A child of a full-screen
    /// container (the front end's, `SetFullscreen("pan and scan")`) is
    /// placed on the screen by its own anchors, as a top-level widget is:
    /// the container is the screen.
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
            if w.parent
                .and_then(|p| self.widgets.get(&p))
                .is_some_and(|p| p.fullscreen && p.kind == Kind::Widget)
            {
                return Correction::new(self.rect(i), w.anchors, width, height);
            }
            at = w.parent;
        }
        Correction {
            scale,
            offset: [
                (width - VIRTUAL_SIZE.0 * scale) / 2.0,
                (height - VIRTUAL_SIZE.1 * scale) / 2.0,
            ],
        }
    }

    /// What to draw on a `width` x `height` screen, back to front.
    pub fn draw_list(&self, width: f32, height: f32) -> Vec<DrawItem> {
        let mut out = Vec::new();
        for &id in &self.order {
            let Some(w) = self.widgets.get(&id) else {
                continue;
            };
            if !w.visible || w.sleeping || w.inactive {
                continue;
            }
            let c = self.correction(id, width, height);
            let rect = if w.fullscreen {
                [0.0, 0.0, width, height]
            } else {
                c.apply(self.rect(id))
            };
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
                            pie: pie(w),
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
                            pie: None,
                        });
                    }
                }
                Kind::Text => {
                    if !w.text.is_empty() {
                        let spans = self.text_spans(id);
                        out.push(DrawItem::Text {
                            widget: id,
                            font: w.font.clone(),
                            text: spans.iter().map(|s| s.text.as_str()).collect(),
                            spans,
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
                    fullscreen: w.fullscreen,
                }),
                Kind::Minimap => {
                    let call = |name: &str| {
                        w.minimap_calls
                            .get(name)
                            .map(|v| v.iter().map(|s| s.parse::<f32>().ok()).collect::<Vec<_>>())
                    };
                    // (centre x, y, player x, y, z, focus x, y, z, rotation).
                    let update = call("MinimapUpdate").unwrap_or_default();
                    let at = |i: usize| update.get(i).copied().flatten();
                    out.push(DrawItem::Minimap {
                        widget: id,
                        rect,
                        player: at(2).zip(at(3)).zip(at(4)).map(|((x, y), z)| [x, y, z]),
                        rotation: at(8).unwrap_or(0.0),
                        range: call("MinimapSetRange").and_then(|r| r.first().copied().flatten()),
                    });
                }
                Kind::Movie => out.push(DrawItem::Movie {
                    widget: id,
                    file: w.movie.clone(),
                    rect,
                    playing: w.movie_playing,
                }),
            }
        }
        out
    }

    fn log_unknown(&mut self, name: &str) {
        self.unknown.insert(name.to_owned());
    }
}

/// Part of a text in one colour (`None`: the widget's colour). `text` may
/// hold `\n`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextSpan {
    pub text: String,
    /// RGB, 0..=1.
    pub colour: Option<[f32; 3]>,
    /// An inline icon in its place instead of text: the texture, a line
    /// high and square, tinted by the colour.
    pub icon: Option<String>,
}

/// `spans` with a new line put for the space before each word that would
/// take its line past `max` (as `width` measures text; an icon is
/// `icon` wide). A word wider than `max` keeps a line to itself.
fn wrap_spans(
    spans: Vec<TextSpan>,
    max: f32,
    width: impl Fn(&str) -> f32,
    icon: f32,
) -> Vec<TextSpan> {
    let mut out: Vec<TextSpan> = Vec::with_capacity(spans.len());
    // The width of the line so far.
    let mut line = 0.0f32;
    for span in spans {
        if span.icon.is_some() {
            line += icon;
            out.push(span);
            continue;
        }
        let mut text = String::with_capacity(span.text.len());
        for (i, para) in span.text.split('\n').enumerate() {
            if i > 0 {
                text.push('\n');
                line = 0.0;
            }
            for (j, word) in para.split(' ').enumerate() {
                let space = if j > 0 { width(" ") } else { 0.0 };
                let w = width(word);
                if line > 0.0 && line + space + w > max {
                    // The break goes where the space was.
                    if j == 0 {
                        // At a span's start: the space ended the last one.
                        if let Some(prev) = out.iter_mut().rev().find(|s| s.icon.is_none())
                            && prev.text.ends_with(' ')
                        {
                            prev.text.pop();
                        }
                    }
                    text.push('\n');
                    line = w;
                } else {
                    if j > 0 {
                        text.push(' ');
                    }
                    line += space + w;
                }
                text.push_str(word);
            }
        }
        out.push(TextSpan { text, ..span });
    }
    out
}

/// The length of the bracketed token `s` starts with, nested brackets
/// included; `None` if it never closes.
fn bracket_len(s: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// Splits a token's inside at the colons outside nested brackets.
fn split_top(s: &str) -> Vec<&str> {
    let (mut out, mut depth, mut start) = (Vec::new(), 0, 0);
    for (i, c) in s.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => depth -= 1,
            ':' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// Fills `%d`, `%s`, `%S`, `%i`, `%f` (any case) from `args` in order;
/// `%%` is a percent sign.
fn fill(template: &str, args: &[String]) -> String {
    let mut out = String::new();
    let mut args = args.iter();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.peek().map(char::to_ascii_lowercase) {
            Some('d' | 's' | 'i' | 'f' | 'u') => {
                chars.next();
                out.push_str(args.next().map_or("", String::as_str));
            }
            Some('%') => {
                chars.next();
                out.push('%');
            }
            _ => out.push('%'),
        }
    }
    out
}

/// The colour a markup word names (approximate shades).
fn colour_name(word: &str) -> Option<[f32; 3]> {
    let rgb = |r: u8, g: u8, b: u8| [r, g, b].map(|v| f32::from(v) / 255.0);
    Some(match word.to_ascii_lowercase().as_str() {
        "white" => rgb(255, 255, 255),
        "green" => rgb(96, 208, 64),
        "red" => rgb(216, 32, 32),
        "yellow" => rgb(255, 208, 64),
        "orange" => rgb(255, 144, 32),
        "blue" => rgb(64, 144, 255),
        "grey" | "gray" => rgb(160, 160, 160),
        "black" => rgb(0, 0, 0),
        // Objectives' colours, as `MrxGuiInterface.tObjectiveMessageConfigs`
        // gives them in hex beside the words (FFC800, 33CC99).
        "objt" => rgb(0xff, 0xc8, 0x00),
        "2ndobjt" => rgb(0x33, 0xcc, 0x99),
        _ => return None,
    })
}

/// The button page's glyphs below U+E014 that [`button_glyph`] gives no
/// word: U+E00C is an empty black disc and U+E00F is not on the page.
pub const UNMATCHED_GLYPHS: [char; 2] = ['\u{e00c}', '\u{e00f}'];

/// The glyph a control word stands for on the PC (`[pfire] Shoot`): the
/// HUD fonts keep the PC's button pictures on a page of their own
/// (`english_18`'s second, `pc\common_18_buttons`), at private-use
/// characters U+E000 on.
///
/// Which pad button each word is comes from the game's data: the HUD
/// movies' text formatter (`formatText` in `topbar` and `minimap`, vz.wad)
/// swaps each word for a pad picture loaded by name, `[confirm]` for
/// `button_A`, `[bash]`, `[honk]` and `[cancel]` for `button_B`,
/// `[reload]` for `button_X`, `[action]` for `button_Y`, `[left stick]`
/// for `button_joystick_L` (as `[move]`), `[sswitch]` for
/// `button_Dpad_left`, `[support]` for `button_joystick_L_press`. Those
/// are the console's pictures; the PC font page has its own, and the four
/// round ones (U+E000 to U+E003, the only face-button discs on it) are
/// taken to be A, B, X and Y in that order, the formatter's own order
/// (`b1` to `b4`). The pictures agree where they can be read (U+E001 a
/// fist for the bash, U+E002 a magazine for the reload), but no data ties
/// a code point to a pad button: inferred, unchecked on the PC.
/// `[jump]` is not in the formatter; it is given U+E000, the face disc
/// no other word takes (`[confirm]` is the pistol on the PC, below).
/// `[support]` is given the headset (U+E01C), the support radio, by
/// picture: the page has no disc of its own for the left stick's press.
///
/// The rest are matched by picture: the PC footage's satellite help shows
/// `[move]` as the four arrows and `[confirm]` as the pistol (the fire
/// button confirms), and the others are the pictures that say what the
/// words do. Words without a glyph show the key bound to them, if the
/// host named it ([`Screen::set_key_names`]), else stay as written.
pub fn button_glyph(word: &str) -> Option<char> {
    Some(match word.to_ascii_lowercase().as_str() {
        "jump" => '\u{e000}',
        "bash" | "honk" | "cancel" => '\u{e001}',
        "reload" => '\u{e002}',
        "action" => '\u{e003}',
        "crouch" => '\u{e004}',
        // Fire, and what the fire button does.
        "pfire" | "confirm" => '\u{e005}',
        "sfire" => '\u{e006}',
        "pswitch" => '\u{e007}',
        "move" | "left stick" => '\u{e008}',
        "aim" => '\u{e009}',
        "sprint" => '\u{e00a}',
        "zoom" => '\u{e00b}',
        "pda" => '\u{e00d}',
        "pause" => '\u{e00e}',
        "dpad left" | "sswitch" => '\u{e010}',
        "dpad right" => '\u{e011}',
        "dpad up" => '\u{e012}',
        "dpad down" => '\u{e013}',
        "support" => '\u{e01c}',
        _ => return None,
    })
}

/// The texture an inline icon word stands for (`MrxUtil.tInlineIcons`:
/// `[objdeliver]`, and `[objdeliver2]` for secondary objectives), by the
/// objective icon textures' names (`HUD_objective_deliverable`...).
fn inline_icon(word: &str) -> Option<&'static str> {
    let w = word.to_ascii_lowercase();
    Some(match w.strip_suffix('2').unwrap_or(&w) {
        "objaction" => "HUD_objective_action",
        "objoutpost" => "HUD_objective_outpost",
        "objdeliver" => "HUD_objective_deliverable",
        "objdestroy" => "HUD_objective_destroy",
        "objdefend" => "HUD_objective_defend",
        "objverify" => "HUD_objective_verify",
        _ => return None,
    })
}

/// An image's pie slice, as `SetImagePieSliceRender(id, from, to)` gives
/// it: `MrxGuiHudReticle` cuts the target-health ring to 130..230 (centred
/// on 180, under the reticle as footage shows it) and shortens it from the
/// first end as health falls.
fn pie(w: &Widget) -> Option<[f32; 2]> {
    match w.pie_slice.as_deref() {
        Some([from, to, ..]) => Some([*from, *to]),
        _ => None,
    }
}

/// A sprite run (`AnimateSprite(id, from, to, seconds, loop)`, a sprite
/// widget's `PlayAnimation`): frames `from` to `to` (either way) in
/// `seconds` all told, again and again if `looping`, else stopping at
/// `to`. That `seconds` is the whole run's, not each frame's, is read off
/// the scripts' calls: three frames of sparks in 0.25 s, the laser
/// reticle's 31-frame circle in its lock time.
#[derive(Clone, Debug, PartialEq)]
pub struct SpriteRun {
    pub from: u32,
    pub to: u32,
    pub seconds: f32,
    pub looping: bool,
    pub elapsed: f32,
}

impl SpriteRun {
    /// The frame now, and whether the run is over.
    fn frame(&self) -> (u32, bool) {
        let count = self.from.abs_diff(self.to) + 1;
        let step = if self.seconds > 0.0 {
            (self.elapsed / self.seconds * count as f32).floor() as u32
        } else {
            count
        };
        let (step, done) = if self.looping {
            (step % count, false)
        } else {
            (step.min(count - 1), step >= count)
        };
        let frame = if self.to >= self.from {
            self.from + step
        } else {
            self.from - step
        };
        (frame, done)
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
/// An anchor as `Widget:SetAnchoring` passes it (-1 left or top, 0 centre,
/// 1 right or bottom), or a justification's name.
fn anchor_of(v: Option<&Value>) -> Anchor {
    let n = match v {
        Some(Value::Integer(i)) => Some(*i as f64),
        Some(Value::Number(n)) => Some(*n),
        _ => None,
    };
    if let Some(n) = n {
        return match n {
            n if n < 0.0 => Anchor::Start,
            n if n > 0.0 => Anchor::End,
            _ => Anchor::Centre,
        };
    }
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

/// The box `SetWidgetLocation(id, x1, y1, x2, y2)` asks for: without `x2`
/// and `y2` the widget moves to `x1`, `y1` and keeps its size (the
/// reticle follows the aim so).
fn new_location(s: &Screen, a: &Args) -> [f32; 4] {
    let old = s.rect(a.id(0));
    let (x1, y1) = (a.f(1).unwrap_or(old[0]), a.f(2).unwrap_or(old[1]));
    match (a.f(3), a.f(4)) {
        (Some(x2), Some(y2)) => [x1, y1, x2, y2],
        _ => [x1, y1, x1 + old[2] - old[0], y1 + old[3] - old[1]],
    }
}

/// Keeps a `Minimap*` call's latest arguments on its widget.
fn record_minimap(s: &mut Screen, name: &str, a: &Args) -> mlua::Result<MultiValue> {
    if let Some(w) = s.widgets.get_mut(&a.id(0)) {
        w.minimap_calls.insert(name.to_owned(), a.texts(1));
    }
    Ok(MultiValue::new())
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
    let extra: Vec<Value> = extra.iter().map(|v| Value::Number(*v)).collect();
    call_back_with(lua, handle, extra)
}

/// Calls a kept callback with its data then `extra`.
fn call_back_with(lua: &Lua, handle: u32, extra: Vec<Value>) -> mlua::Result<()> {
    let table: Table = lua.named_registry_value(CALLBACKS)?;
    let Some(entry) = table.raw_get::<Option<Table>>(handle)? else {
        return Ok(());
    };
    let f: Function = entry.raw_get(1)?;
    let mut args: Vec<Value> = match entry.raw_get::<Option<Table>>(2)? {
        Some(d) => d.sequence_values::<Value>().collect::<mlua::Result<_>>()?,
        None => Vec::new(),
    };
    args.extend(extra);
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

/// The names the scripts gave their widgets, by id. The natives are
/// created without one (`_GuiInternal.CreateImageWidget()`); the layout's
/// `name` stays on the scripts' side, in the widget object MrxGuiBase's
/// `WidgetIdIndex[uId]` holds (`BasicData.name`).
pub fn script_names(lua: &Lua) -> BTreeMap<u32, String> {
    let mut names = BTreeMap::new();
    let index = lua
        .globals()
        .get::<Option<Table>>("MrxGuiBase")
        .ok()
        .flatten()
        .and_then(|m| m.get::<Option<Table>>("WidgetIdIndex").ok().flatten());
    let Some(index) = index else {
        return names;
    };
    for (id, widget) in index.pairs::<Value, Table>().flatten() {
        let id = match id {
            Value::Integer(i) => i as u32,
            Value::Number(n) => n as u32,
            _ => continue,
        };
        let name = widget
            .get::<Option<Table>>("BasicData")
            .ok()
            .flatten()
            .and_then(|b| b.get::<Option<String>>("name").ok().flatten());
        if let Some(name) = name {
            names.insert(id, name);
        }
    }
    names
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
    // `Gui.GetReticlePosition(owner)`: the aim point, in the plain `Gui`
    // library table (made here if the host has none yet).
    let gui = match lua.globals().raw_get::<Value>("Gui")? {
        Value::Table(t) if t.metatable().is_none() => t,
        _ => {
            let t = lua.create_table()?;
            lua.globals().raw_set("Gui", &t)?;
            t
        }
    };
    let s = Rc::clone(&screen);
    gui.raw_set(
        "GetReticlePosition",
        safe_function(lua, move |lua, _| {
            let [x, y] = s.borrow().reticle;
            ret(lua, (x, y))
        })?,
    )?;
    // `Player.GetCameraXZHeading(owner)`, likewise in `Player`.
    let player = match lua.globals().raw_get::<Value>("Player")? {
        Value::Table(t) if t.metatable().is_none() => t,
        _ => {
            let t = lua.create_table()?;
            lua.globals().raw_set("Player", &t)?;
            t
        }
    };
    let s = Rc::clone(&screen);
    player.raw_set(
        "GetCameraXZHeading",
        safe_function(lua, move |lua, _| ret(lua, s.borrow().heading))?,
    )?;
    // `String.GetHash(name)`, likewise in `String`: the support scripts
    // key their recruits by it (`MrxSupportManager.IsRecruitAvailable`;
    // the support menu greys out an item whose recruit it can't find).
    // The archive's name hash: what the scripts compare are their own
    // hashes, so any consistent hash would do.
    let string = match lua.globals().raw_get::<Value>("String")? {
        Value::Table(t) if t.metatable().is_none() => t,
        _ => {
            let t = lua.create_table()?;
            lua.globals().raw_set("String", &t)?;
            t
        }
    };
    string.raw_set(
        "GetHash",
        safe_function(lua, |lua, args| match args.front() {
            Some(Value::String(s)) => ret(lua, name_hash(&s.to_string_lossy())),
            _ => ret(lua, Value::Nil),
        })?,
    )?;
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

/// Delivers a Flash movie's `fscommand(command, argument)` from widget
/// `widget` to the callback the scripts set for `command`
/// (`FlashWidget:SetFlashEventHandler(command, handler, data)` sets
/// `_FlashCallback` with `{ widget, handler, data }`, which calls
/// `handler(widget, argument, unpack(data))`). Returns whether one was
/// set.
pub fn flash_command(lua: &Lua, widget: u32, command: &str, argument: &str) -> mlua::Result<bool> {
    let Some(screen) = screen(lua) else {
        return Ok(false);
    };
    let handle = screen
        .borrow()
        .widgets
        .get(&widget)
        .and_then(|w| w.flash_handlers.get(command).copied());
    let Some(handle) = handle else {
        return Ok(false);
    };
    call_back_with(
        lua,
        handle,
        vec![Value::String(lua.create_string(argument)?)],
    )?;
    Ok(true)
}

/// Delivers an engine event to the handlers registered for it, with
/// `args` after each handler's own data (`GuiHealthUpdate` with health,
/// maximum and whether in a vehicle). Returns how many handlers ran.
///
/// Don't fire `GuiInitialization`: the game's own loading initialises
/// every widget, and handlers run twice record wrong animation points.
pub fn fire(lua: &Lua, event: &str, args: impl IntoLuaMulti) -> mlua::Result<usize> {
    let args = args.into_lua_multi(lua)?;
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
        // Not drawn until added: the scripts' widget objects start
        // disabled (`bEnabled = false`), and `MrxGui.AddWidget` activates
        // them (`ActivateWidget(id, true)`), `RemoveWidget` deactivates.
        // The text buffers make each message's widget as it is queued
        // and add it only when its turn comes.
        if let Some(w) = s.widgets.get_mut(&id) {
            w.inactive = true;
        }
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
    "ActivateWidget" => |s, _, a| {
        // `WidgetManager.AddWidget` activates, `RemoveWidget` deactivates.
        let active = a.get(1).is_none_or(|v| !matches!(v, Value::Nil | Value::Boolean(false)));
        if let Some(w) = s.widgets.get_mut(&a.id(0)) {
            w.inactive = !active;
        }
        Ok(MultiValue::new())
    },
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
        let r = new_location(s, a);
        s.set_rect(a.id(0), r);
        Ok(MultiValue::new())
    },
    "GetWidgetLocation" => |s, l, a| {
        let r = s.rect(a.id(0));
        ret(l, (r[0], r[1], r[2], r[3]))
    },
    // The corrected box is the box: corrections apply when drawing.
    "SetWidgetCorrectedLocation" => |s, _, a| {
        let r = new_location(s, a);
        s.set_rect(a.id(0), r);
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
        // As `SetWidgetAnchoring` takes them: `Widget:SetAnchoring` passes
        // back what this answers for a side it is not given.
        let n = |a| match a {
            Anchor::Start => -1,
            Anchor::Centre => 0,
            Anchor::End => 1,
        };
        ret(l, (n(h), n(v)))
    },
    "CorrectWidgetForResolution" => |s, _, a| {
        // A widget with a parent moves with it: `MrxGuiBase` corrects every
        // widget on a resolution change, and children corrected by their
        // own anchors would come apart from their frames.
        let id = a.id(0);
        let r = s.rect(id);
        if let Some(w) = s.widgets.get_mut(&id)
            && w.parent.is_none()
        {
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
    // The text cut into lines that fit the widget's width, at spaces
    // (`TextWidget:SplitIntoLines` makes a widget of each).
    "SplitText" => |s, _, a| {
        let id = a.id(0);
        let Some(w) = s.widget(id) else {
            return Ok(MultiValue::new());
        };
        let r = s.rect(id);
        let max = (r[2] - r[0]).max(1.0);
        let text = s.display_text(&w.text);
        let font = s.fonts.get(&w.font.to_ascii_lowercase());
        let width = |t: &str| {
            let px = font.map_or(t.chars().count() as f32 * 12.0, |f| f.text_width(t));
            px * VIRTUAL_SIZE.1 / FONT_LINES * w.scale
        };
        let mut lines = Vec::new();
        for para in text.split('\n') {
            let mut line = String::new();
            for word in para.split(' ') {
                let candidate = if line.is_empty() { word.to_owned() } else { format!("{line} {word}") };
                if !line.is_empty() && width(&candidate) > max {
                    lines.push(std::mem::replace(&mut line, word.to_owned()));
                } else {
                    line = candidate;
                }
            }
            lines.push(line);
        }
        // A new text widget for each line, in the widget's font, scale,
        // colour and justification: `SplitIntoLines` wraps each id it is
        // given in a `TextWidget` (`TextWidget.new(nil, uId)`), and the
        // text buffers show the lines one after another.
        let (font, scale, colour, justification) =
            (w.font.clone(), w.scale, w.colour, w.justification);
        let mut out = MultiValue::new();
        for line in lines {
            let line_id = s.create(Kind::Text, String::new());
            if let Some(n) = s.widgets.get_mut(&line_id) {
                n.text = line;
                n.font = font.clone();
                n.scale = scale;
                n.colour = colour;
                n.justification = justification;
            }
            out.push_back(Value::Integer(line_id as mlua::Integer));
        }
        Ok(out)
    },
    // Recorded as calls only: the text and sprite animations and the clock
    // wipe draw as their full image or text (see the module docs).
    "AnimateText" => |s, _, a| {
        s.log_unknown("AnimateText (drawn whole)");
        let _ = a;
        Ok(MultiValue::new())
    },
    "HaltTextAnimation" => |_, _, _| Ok(MultiValue::new()),
    "AnimateSprite" => |s, _, a| {
        let run = SpriteRun {
            from: a.num(1).map_or(0, |f| f.max(0.0) as u32),
            to: a.num(2).map_or(0, |f| f.max(0.0) as u32),
            seconds: a.f(3).unwrap_or(0.0),
            looping: a.truthy(4),
            elapsed: 0.0,
        };
        if let Some(w) = s.widgets.get_mut(&a.id(0)) {
            w.sprite_frame = run.from;
            w.sprite_run = Some(run);
        }
        Ok(MultiValue::new())
    },
    "HaltSpriteAnimation" => |s, _, a| {
        if let Some(w) = s.widgets.get_mut(&a.id(0)) {
            w.sprite_run = None;
        }
        Ok(MultiValue::new())
    },
    "SetImageClockAnimation" => |s, _, a| {
        s.log_unknown("SetImageClockAnimation (drawn whole)");
        let _ = a;
        Ok(MultiValue::new())
    },
    "SetImageClockCallback" => |_, _, _| Ok(MultiValue::new()),
    "GetImageClockElapsed" => |_, l, _| ret(l, 0),
    // Settings with nothing to draw differently here.
    "SetWidgetViewport" => |_, _, _| Ok(MultiValue::new()),
    "SetWidgetIgnoresPause" => |_, _, _| Ok(MultiValue::new()),
    "SetWidgetFullscreen" => |s, _, a| {
        if let Some(w) = s.widgets.get_mut(&a.id(0)) {
            w.fullscreen = a.truthy(1);
        }
        Ok(MultiValue::new())
    },
    "SetWidgetHighlightable" => |_, _, _| Ok(MultiValue::new()),
    "SetWidgetUseNewRescale" => |_, _, _| Ok(MultiValue::new()),
    // (widget, command, function, data): `fscommand(command, arg)` from
    // the movie calls `function(unpack(data), arg)`.
    "SetFlashCallback" => |s, l, a| {
        let id = a.id(0);
        let Some(command) = a.text(1) else {
            return Ok(MultiValue::new());
        };
        let handle = keep_callback(s, l, a.get(2), a.get(3))?;
        let old = match s.widgets.get_mut(&id) {
            Some(w) => match handle {
                Some(h) => w.flash_handlers.insert(command, h),
                None => w.flash_handlers.remove(&command),
            },
            None => handle,
        };
        if let Some(old) = old {
            forget_callback(l, old)?;
        }
        Ok(MultiValue::new())
    },
    "SetFlashTesselationAllowed" => |_, _, _| Ok(MultiValue::new()),
    "PlayFlash" => |s, _, a| {
        if let Some(w) = s.widgets.get_mut(&a.id(0)) {
            w.flash_paused = false;
        }
        Ok(MultiValue::new())
    },
    "PauseFlash" => |s, _, a| {
        if let Some(w) = s.widgets.get_mut(&a.id(0)) {
            w.flash_paused = true;
        }
        Ok(MultiValue::new())
    },
    "RestartFlash" => |s, _, a| {
        if let Some(w) = s.widgets.get_mut(&a.id(0)) {
            w.flash_restarts += 1;
            w.flash_calls.clear();
            w.pda_blips.clear();
        }
        Ok(MultiValue::new())
    },
    // The movie ends at once: its end callback runs at the next tick
    // (the cinematics' scripts wait on it, as 02_AOB_M does).
    "PlayMovie" => |s, _, a| {
        let w = s.widgets.get_mut(&a.id(0));
        let cb = w.and_then(|w| {
            w.movie_playing = true;
            w.movie_end.take()
        });
        if let Some(cb) = cb {
            s.due.push((cb, Vec::new()));
        }
        Ok(MultiValue::new())
    },
    // (widget, list): the PDA map's blips, which the engine hands its
    // movie one `AddBlip` each; logged too.
    "AddPdaMapBlips" => |s, _, a| {
        let mut shown = Vec::new();
        if let Some(Value::Table(list)) = a.get(1) {
            for entry in list.clone().sequence_values::<Value>().flatten() {
                let Value::Table(entry) = entry else { continue };
                let mut fields: Vec<String> = entry
                    .pairs::<Value, Value>()
                    .flatten()
                    .map(|(k, v)| format!("{}={}", show_arg(&k), show_arg(&v)))
                    .collect();
                fields.sort();
                shown.push(format!("{{{}}}", fields.join(" ")));
            }
        }
        s.log.push(format!(
            "AddPdaMapBlips({}): {} blips {}",
            a.id(0),
            shown.len(),
            shown.join(" ")
        ));
        // Each entry is `AddBlip`'s arguments in order (internal name,
        // texture, x, y, rotation, label, description, faction...).
        if let (Some(Value::Table(list)), Some(w)) = (a.get(1), s.widgets.get_mut(&a.id(0))) {
            for entry in list.clone().sequence_values::<Value>().flatten() {
                if let Value::Table(entry) = entry {
                    let args: Vec<FlashArg> = entry
                        .sequence_values::<Value>()
                        .flatten()
                        .map(|v| FlashArg::of(&v))
                        .collect();
                    if let Some(FlashArg::Text(name)) = args.first() {
                        w.pda_blips.insert(name.clone(), args.clone());
                    }
                    w.flash_calls.push(("AddBlip".to_owned(), args));
                }
            }
        }
        Ok(MultiValue::new())
    },
    // (widget, entry): one blip moved or changed, its entry as in
    // `AddPdaMapBlips`; the movie's `UpdateBlip` takes the same arguments.
    "UpdatePdaBlip" => |s, _, a| {
        if let (Some(Value::Table(entry)), Some(w)) = (a.get(1), s.widgets.get_mut(&a.id(0))) {
            let args: Vec<FlashArg> = entry
                .clone()
                .sequence_values::<Value>()
                .flatten()
                .map(|v| FlashArg::of(&v))
                .collect();
            let name = match args.first() {
                Some(FlashArg::Text(n)) => n.clone(),
                _ => String::new(),
            };
            if w.pda_blips.get(&name) != Some(&args) {
                w.pda_blips.insert(name, args.clone());
                w.flash_calls.push(("UpdateBlip".to_owned(), args));
            }
        }
        Ok(MultiValue::new())
    },
    // (widget, name).
    "RemovePdaBlip" => |s, _, a| {
        if let (Some(v), Some(w)) = (a.get(1), s.widgets.get_mut(&a.id(0))) {
            if let FlashArg::Text(name) = FlashArg::of(v) {
                w.pda_blips.remove(&name);
            }
            w.flash_calls
                .push(("RemoveBlip".to_owned(), vec![FlashArg::of(v)]));
        }
        Ok(MultiValue::new())
    },
    // (widget, function, data).
    "SetMovieEndCallback" => |s, l, a| {
        let cb = keep_callback(s, l, a.get(1), a.get(2))?;
        let old = match s.widgets.get_mut(&a.id(0)) {
            Some(w) => std::mem::replace(&mut w.movie_end, cb),
            None => cb,
        };
        if let Some(old) = old {
            forget_callback(l, old)?;
        }
        Ok(MultiValue::new())
    },
    "PauseMovie" => |_, _, _| Ok(MultiValue::new()),
    "StopMovie" => |s, _, a| {
        if let Some(w) = s.widgets.get_mut(&a.id(0)) {
            w.movie_playing = false;
        }
        Ok(MultiValue::new())
    },
    "SetMinimapBorder" => |_, _, _| Ok(MultiValue::new()),
    "SetMinimapOwner" => |_, _, _| Ok(MultiValue::new()),
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
    "SetTextWrapping" => |s, _, a| {
        if let Some(w) = s.widgets.get_mut(&a.id(0)) {
            w.wraps = a.truthy(1);
        }
        Ok(MultiValue::new())
    },
    "GetTextWidth" => |s, l, a| ret(l, s.text_width(a.id(0))),
    "GetTextHeight" => |s, l, a| ret(l, s.text_height(a.id(0))),
    "SetFlashSwfFile" => |s, l, a| {
        let id = a.id(0);
        if let Some(w) = s.widgets.get_mut(&id) {
            w.swf = a.text(1);
            w.flash_restarts += 1;
            w.flash_calls.clear();
            w.pda_blips.clear();
        }
        // Loaded at once: the callback runs at the next tick.
        if let Some(cb) = keep_callback(s, l, a.get(2), a.get(3))? {
            s.due.push((cb, Vec::new()));
        }
        Ok(MultiValue::new())
    },
    // (widget, function, arguments...), as `CallActionScriptCallback`
    // unpacks them; a table in their place is taken as the arguments.
    "CallFlashScriptFunction" => |s, _, a| {
        let id = a.id(0);
        let name = a.text(1).unwrap_or_default();
        let rest = match a.get(2) {
            // Up to its last whole-number key, holes passed as undefined
            // (`{ bRetry, nil, nPlayers, sMessage, bHost }`).
            Some(Value::Table(t)) if a.0.len() == 3 => {
                let n = t
                    .pairs::<Value, Value>()
                    .filter_map(Result::ok)
                    .filter_map(|(k, _)| match k {
                        Value::Integer(i) if i >= 1 => Some(i),
                        _ => None,
                    })
                    .max()
                    .unwrap_or(0);
                (1..=n)
                    .map(|i| FlashArg::of(&t.raw_get::<Value>(i).unwrap_or(Value::Nil)))
                    .collect()
            }
            _ => a.0.iter().skip(2).map(FlashArg::of).collect(),
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
    "MinimapSetRange" => |s, _, a| record_minimap(s, "MinimapSetRange", a),
    "MinimapSetPlayerLocation" => |s, _, a| record_minimap(s, "MinimapSetPlayerLocation", a),
    "MinimapSetRotation" => |s, _, a| record_minimap(s, "MinimapSetRotation", a),
    "MinimapSetFocusLocation" => |s, _, a| record_minimap(s, "MinimapSetFocusLocation", a),
    "MinimapUpdate" => |s, _, a| record_minimap(s, "MinimapUpdate", a),
    "MinimapAddObjective" => |s, _, a| record_minimap(s, "MinimapAddObjective", a),
    "MinimapRemoveObjective" => |s, _, a| record_minimap(s, "MinimapRemoveObjective", a),
    // (x, y, radius, texture, texture width, height, horizontal and
    // vertical anchor, world x min, max, z min, max): makes the minimap,
    // its circle centred on (x, y), and answers its id
    // (`MinimapWidget:SetUpMinimap`).
    "MinimapCreate" => |s, l, a| {
        let id = s.create(Kind::Minimap, String::new());
        let (x, y, r) = (a.f(0).unwrap_or(0.0), a.f(1).unwrap_or(0.0), a.f(2).unwrap_or(0.0));
        s.set_rect(id, [x - r, y - r, x + r, y + r]);
        if let Some(w) = s.widgets.get_mut(&id) {
            w.anchors = (anchor_of(a.get(6)), anchor_of(a.get(7)));
            w.minimap = Some(a.texts(0));
        }
        ret(l, id)
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::ScriptHost;

    fn plain(spans: &[TextSpan]) -> String {
        spans.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn wrapping_breaks_at_the_spaces_that_would_overflow() {
        let span = |t: &str, c: Option<[f32; 3]>| TextSpan {
            text: t.into(),
            colour: c,
            icon: None,
        };
        let width = |t: &str| t.chars().count() as f32;
        let one = vec![span("aaa bbb ccc ddd", None)];
        assert_eq!(plain(&wrap_spans(one, 7.0, width, 1.0)), "aaa bbb\nccc ddd");
        // Across colours, the space that ended a span is the break.
        let two = vec![span("aaa bbb ", None), span("ccc", Some([1.0, 0.0, 0.0]))];
        let wrapped = wrap_spans(two, 8.0, width, 1.0);
        assert_eq!(plain(&wrapped), "aaa bbb\nccc");
        assert_eq!(wrapped[1].colour, Some([1.0, 0.0, 0.0]));
        // Lines already short enough, and a word too long, stay as they are.
        let three = vec![span("a\nlongword b", None)];
        assert_eq!(plain(&wrap_spans(three, 4.0, width, 1.0)), "a\nlongword\nb");
    }

    fn host() -> (ScriptHost, ScreenRef) {
        let host = ScriptHost::new().unwrap();
        let screen = install(host.lua()).unwrap();
        (host, screen)
    }

    fn run(host: &ScriptHost, code: &str) {
        host.lua().load(code).exec().unwrap();
    }

    /// Activates every widget made so far, as the scripts'
    /// `MrxGui.AddWidget` does for the ones they show.
    fn added(screen: &ScreenRef) {
        for w in screen.borrow_mut().widgets.values_mut() {
            w.inactive = false;
        }
    }

    #[test]
    fn widgets_draw_once_added() {
        let (host, screen) = host();
        run(
            &host,
            r#"
            local G = _GuiInternal
            queued = G.CreateTextWidget()
            shown = G.CreateTextWidget()
            G.SetTextText(queued, "later")
            G.SetTextText(shown, "now")
            G.ActivateWidget(shown, true)
            "#,
        );
        let texts = || {
            screen
                .borrow()
                .draw_list(640.0, 480.0)
                .into_iter()
                .filter_map(|i| match i {
                    DrawItem::Text { text, .. } => Some(text),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(texts(), ["now"]);
        run(&host, "_GuiInternal.ActivateWidget(queued, true)");
        assert_eq!(texts(), ["later", "now"]);
    }

    #[test]
    fn sprite_runs_step_through_their_frames() {
        let (host, screen) = host();
        run(
            &host,
            r#"
            local G = _GuiInternal
            spark = G.CreateSpriteWidget()
            arrow = G.CreateSpriteWidget()
            G.AnimateSprite(spark, 0, 2, 0.3, true)
            G.AnimateSprite(arrow, 0, 3, 0.4, false)
            "#,
        );
        let id = |name: &str| host.lua().globals().get::<u32>(name).unwrap();
        let (spark, arrow) = (id("spark"), id("arrow"));
        let frame = |w: u32| screen.borrow().widget(w).unwrap().sprite_frame;
        tick(host.lua(), 0.15);
        // Halfway through the three frames, and through the four.
        assert_eq!((frame(spark), frame(arrow)), (1, 1));
        tick(host.lua(), 0.2);
        // The spark loops round to its first frame; the arrow stops at its
        // last and stays.
        assert_eq!((frame(spark), frame(arrow)), (0, 3));
        tick(host.lua(), 1.0);
        assert_eq!(frame(arrow), 3);
        assert!(screen.borrow().widget(arrow).unwrap().sprite_run.is_none());
        run(&host, "_GuiInternal.HaltSpriteAnimation(spark)");
        let halted = frame(spark);
        tick(host.lua(), 0.1);
        assert_eq!(frame(spark), halted);
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
        added(&screen);
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
    fn text_markup_expands_ids_arguments_lines_and_colours() {
        let mut s = Screen::default();
        let mut strings = StringTable::default();
        strings.strings.insert(
            m2_assets::name_hash("SHELL.Common.Money"),
            "$%D.%D %S".to_owned(),
        );
        strings.strings.insert(0xe00c_096a, "K".to_owned());
        strings
            .strings
            .insert(m2_assets::name_hash("Generic.Credit"), "Credit".to_owned());
        s.set_strings(strings);
        let text =
            "[green]+[SHELL.Common.Money:25:0:[0xe00c096a]][n][white][Generic.Credit] [nope]";
        assert_eq!(s.display_text(text), "+$25.0 K\nCredit [nope]");
        let spans = s.spans(text);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].text, "+$25.0 K\n");
        assert!(spans[0].colour.is_some_and(|c| c[1] > c[0]));
        assert_eq!(spans[1].colour, Some([1.0; 3]));
        assert_eq!(fill("%d%%-%s", &["5".to_owned()]), "5%-");
    }

    #[test]
    fn fullscreen_widgets_cover_the_screen() {
        let (host, screen) = host();
        run(
            &host,
            r#"
            local G = _GuiInternal
            f = G.CreateFlashWidget()
            G.SetWidgetFullscreen(f, true)
            "#,
        );
        added(&screen);
        let items = screen.borrow().draw_list(1280.0, 720.0);
        let Some(DrawItem::Flash {
            rect, fullscreen, ..
        }) = items.first()
        else {
            panic!("{items:?}");
        };
        assert!(*fullscreen);
        assert_eq!(*rect, [0.0, 0.0, 1280.0, 720.0]);
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
        added(&screen);
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

    #[test]
    fn objective_messages_have_their_colour_and_icon() {
        let s = Screen::default();
        let spans = s.spans("[objt][objdeliver] Go");
        let yellow = Some([1.0, 200.0 / 255.0, 0.0]);
        assert_eq!(spans.len(), 2, "{spans:?}");
        assert_eq!(spans[0].icon.as_deref(), Some("HUD_objective_deliverable"));
        assert_eq!(spans[0].colour, yellow);
        assert_eq!((spans[1].text.as_str(), spans[1].colour), (" Go", yellow));
        assert_eq!(s.display_text("[2ndobjt][objverify2] x"), " x");
    }

    #[test]
    fn control_words_are_the_fonts_button_glyphs() {
        let s = Screen::default();
        assert_eq!(
            s.display_text("Press [pfire] at[n][move] Move view"),
            "Press \u{e005} at\n\u{e008} Move view"
        );
        // Not a control word: as written.
        assert_eq!(s.display_text("[beep] Beep"), "[beep] Beep");
        // The face buttons, by the HUD movies' formatter: A B X Y.
        assert_eq!(
            s.display_text("[jump] Handbrake[n][reload] Reload"),
            "\u{e000} Handbrake\n\u{e002} Reload"
        );
        for (w, g) in [
            ("bash", '\u{e001}'),
            ("honk", '\u{e001}'),
            ("cancel", '\u{e001}'),
            ("action", '\u{e003}'),
            ("left stick", '\u{e008}'),
            ("sswitch", '\u{e010}'),
            ("support", '\u{e01c}'),
        ] {
            assert_eq!(button_glyph(w), Some(g), "{w}");
        }
        let words = [
            "jump",
            "bash",
            "reload",
            "action",
            "crouch",
            "pfire",
            "sfire",
            "pswitch",
            "move",
            "aim",
            "sprint",
            "zoom",
            "pda",
            "pause",
            "dpad left",
            "dpad right",
            "dpad up",
            "dpad down",
        ];
        let given: Vec<char> = words.iter().filter_map(|w| button_glyph(w)).collect();
        for g in UNMATCHED_GLYPHS {
            assert!(!given.contains(&g), "{g:?}");
        }
        assert_eq!(given.len() + UNMATCHED_GLYPHS.len(), 0x14);
    }

    #[test]
    fn control_words_without_a_picture_name_their_keys() {
        let mut s = Screen::default();
        s.set_key_names([
            ("L-Stick Up".to_owned(), "T".to_owned()),
            ("pfire".to_owned(), "F".to_owned()),
        ]);
        assert_eq!(s.display_text("[l-stick up] Look up"), "[T] Look up");
        // A word the fonts have a picture for keeps it; others as written.
        assert_eq!(
            s.display_text("[pfire] [l-stick down]"),
            "\u{e005} [l-stick down]"
        );
    }

    #[test]
    fn a_played_movie_ends_at_the_next_tick() {
        let (host, _screen) = host();
        run(
            &host,
            r#"
            local G = _GuiInternal
            movie = G.CreateImageWidget()
            ended = 0
            G.SetMovieEndCallback(movie, function(n) ended = ended + n end, { 2 })
            "#,
        );
        tick(host.lua(), 0.1);
        run(&host, "_GuiInternal.PlayMovie(movie)");
        assert_eq!(host.lua().globals().get::<i64>("ended").unwrap(), 0);
        tick(host.lua(), 0.1);
        tick(host.lua(), 0.1);
        assert_eq!(host.lua().globals().get::<i64>("ended").unwrap(), 2);
    }

    #[test]
    fn pda_blip_updates_reach_the_map_movie() {
        let (host, screen) = host();
        run(
            &host,
            r#"
            local G = _GuiInternal
            pda = G.CreateFlashWidget()
            G.SetFlashSwfFile(pda, "topbar")
            G.UpdatePdaBlip(pda, { "player1_mc", "player1_mc", 35, 40 })
            G.UpdatePdaBlip(pda, { "player1_mc", "player1_mc", 35, 40 })
            G.RemovePdaBlip(pda, "target1_mc")
            "#,
        );
        let pda = host.lua().globals().get::<u32>("pda").unwrap();
        let calls = screen.borrow_mut().take_flash_calls();
        assert_eq!(
            calls,
            vec![
                (
                    pda,
                    "UpdateBlip".to_owned(),
                    vec![
                        FlashArg::Text("player1_mc".into()),
                        FlashArg::Text("player1_mc".into()),
                        FlashArg::Number(35.0),
                        FlashArg::Number(40.0),
                    ]
                ),
                (
                    pda,
                    "RemoveBlip".to_owned(),
                    vec![FlashArg::Text("target1_mc".into())]
                ),
            ]
        );
    }

    #[test]
    fn flash_calls_reach_the_host_and_fscommands_reach_the_scripts() {
        let (host, screen) = host();
        // As `FlashWidget:CallActionScriptCallback` and
        // `SetFlashEventHandler` call the natives, with `_FlashCallback`
        // doing what the game's does.
        run(
            &host,
            r#"
            local G = _GuiInternal
            function _FlashCallback(widget, handler, data, arg)
                local args = {}
                if type(data) == "table" then
                    for i, v in ipairs(data) do args[i] = v end
                end
                if type(handler) == "function" then
                    table.insert(args, 1, arg)
                    table.insert(args, 1, widget)
                    handler(unpack(args))
                end
            end
            fan = G.CreateFlashWidget()
            G.SetFlashSwfFile(fan, "fanfare_contract")
            G.CallFlashScriptFunction(fan, "setText", "Contract", 3, true)
            got = nil
            G.SetFlashCallback(fan, "done", _FlashCallback,
                { "me", function(w, arg, extra) got = w .. arg .. extra end, { "!" } })
            "#,
        );
        let fan = host.lua().globals().get::<u32>("fan").unwrap();
        let calls = screen.borrow_mut().take_flash_calls();
        assert_eq!(
            calls,
            vec![(
                fan,
                "setText".to_owned(),
                vec![
                    FlashArg::Text("Contract".into()),
                    FlashArg::Number(3.0),
                    FlashArg::Bool(true)
                ]
            )]
        );
        assert!(screen.borrow_mut().take_flash_calls().is_empty());
        assert!(flash_command(host.lua(), fan, "done", "1").unwrap());
        assert!(!flash_command(host.lua(), fan, "other", "1").unwrap());
        let got: String = host.lua().globals().get("got").unwrap();
        assert_eq!(got, "me1!");
        run(&host, "_GuiInternal.PauseFlash(fan)");
        assert!(screen.borrow().widget(fan).unwrap().flash_paused);
    }

    #[test]
    fn the_minimap_is_made_and_told_where_the_player_is() {
        let (host, screen) = host();
        run(
            &host,
            r#"
            local G = _GuiInternal
            map = G.MinimapCreate(534, 386, 50, "VZ", 2800, 2800, "left", "top",
                -4096, 4096, -4096, 4096)
            assert(type(map) == "number")
            G.MinimapUpdate(map, 534, 386, 10, 2, -30, 10, 2, -30, 45)
            G.MinimapSetRange(map, 150)
            "#,
        );
        let items = screen.borrow().draw_list(640.0, 480.0);
        let [
            DrawItem::Minimap {
                rect,
                player,
                rotation,
                range,
                ..
            },
        ] = items.as_slice()
        else {
            panic!("{items:?}");
        };
        assert_eq!(*rect, [484.0, 336.0, 584.0, 436.0]);
        assert_eq!(*player, Some([10.0, 2.0, -30.0]));
        assert_eq!((*rotation, *range), (45.0, Some(150.0)));
    }

    #[test]
    fn removed_widgets_do_not_draw() {
        let (host, screen) = host();
        run(
            &host,
            r#"
            local G = _GuiInternal
            img = G.CreateImageWidget()
            G.SetWidgetLocation(img, 0, 0, 10, 10)
            G.ActivateWidget(img, false)
            "#,
        );
        assert!(screen.borrow().draw_list(640.0, 480.0).is_empty());
        run(&host, "_GuiInternal.ActivateWidget(img, true)");
        assert_eq!(screen.borrow().draw_list(640.0, 480.0).len(), 1);
    }

    #[test]
    fn anchors_come_as_numbers_and_uncorrected_widgets_centre() {
        let (host, screen) = host();
        run(
            &host,
            r#"
            local G = _GuiInternal
            -- As `Widget:SetAnchoring("right", "top")` passes it.
            right = G.CreateWidget()
            G.SetWidgetLocation(right, 460, 40, 600, 110)
            G.SetWidgetAnchoring(right, 1, -1)
            local h, v = G.GetWidgetAnchoring(right)
            assert(h == 1 and v == -1)
            img = G.CreateImageWidget()
            G.SetWidgetLocation(img, 460, 40, 600, 110)
            G.AddWidgetChild(right, img)
            G.CorrectWidgetForResolution(right)
            -- Made later, never corrected, over the screen's middle.
            late = G.CreateImageWidget()
            G.SetWidgetLocation(late, 300, 220, 340, 260)
            "#,
        );
        added(&screen);
        let s = screen.borrow();
        let rects: Vec<[f32; 4]> = s
            .draw_list(1280.0, 720.0)
            .iter()
            .map(|i| match i {
                DrawItem::Fill { rect, .. } => *rect,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(rects[0], [1200.0 - 210.0, 60.0, 1200.0, 165.0]);
        assert_eq!(rects[1], [640.0 - 30.0, 330.0, 640.0 + 30.0, 390.0]);
    }
}
