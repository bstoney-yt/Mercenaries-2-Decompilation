//! The game's own interface layouts: the HUD and the screens built like
//! it (binoculars, the satellite view, the PDA) are trees of native
//! widgets, not Flash movies.
//!
//! A layout is a compiled script in `vz.wad`'s `guilayouts` block that
//! fills the global `LocalWidgetList` with nested tables, one per widget:
//!
//! ```text
//! name                        the widget's name ("Health Counter")
//! WidgetType                  image, text, sprite, flash, minimap; absent: a
//!                             plain widget that only groups its children
//! container                   true: a plain, invisible widget whatever
//!                             WidgetType says (draws nothing itself)
//! x1, y1, x2, y2              its box on a 640 x 480 virtual screen, in
//!                             screen coordinates for children too
//! HorizontalAnchor            left, center, right
//! VerticalAnchor              top, center, bottom
//! RedLevel, GreenLevel, BlueLevel, TranslucencyLevel
//!                             tint and opacity, 0..=255 (255 opaque)
//! visible                     shown if > 0 (layouts use 1 and -1)
//! texture                     an image's texture name (textureFile is the
//!                             source path, dropped when loading)
//! u1, v1, u2, v2              its texture rectangle (default 0, 0, 1, 1);
//!                             u2 < u1 or v2 < v1 mirrors the image
//! rotation                    degrees, 0 in every layout
//! nTileWidth, nTileHeight     repeat the texture every so many virtual units
//! textureWidth, textureHeight a sprite's texture size: its frame is
//!                             (u2 - u1) x (v2 - v1) of that
//! font, text, scale, Justification
//!                             a text widget's font name, text (an id in
//!                             brackets is looked up), size factor and left,
//!                             center or right
//! nRadius, nTextureWidth, nWorldXMin..., sBorderTexture
//!                             a minimap's set-up (the HUD's sets none)
//! EventHandlerFile            the script module handling its events
//! EventHandlers               { GameEvent = Module.Function, ... }
//! Children                    more widgets
//! ```
//!
//! [`load_layout`] runs such a script in a fresh VM with every other
//! global stubbed and reads the tree back. Handlers named as
//! `Module.Function` come back as that text. Every field is kept as read
//! (keys as the script spells them); the accessors on [`Widget`] read the
//! ones above whatever their case.
//!
//! The game builds widgets from these tables in `MrxGuiBase`
//! (`LoadAndAddWidgetFromLayoutFileData`): it creates the native widget
//! for the type, sets texture, rotation and texture rectangle (images and
//! sprites) or text, font, scale and justification (text), the box, the
//! anchoring, the visibility (then hides containers) and the colour, adds
//! the handlers and builds the children, each with its own visibility:
//! showing or hiding a widget does not touch its children. Only top-level
//! widgets are then corrected for the screen's shape; see [`Correction`].
//!
//! Layout script hashes: the HUD [`HUD_LAYOUT`], binoculars and sniper
//! scope `95708d95`, the satellite view `0d60703a`, the PDA `b1f189b4`.

use std::collections::BTreeMap;

use m2_assets::lua::Chunk;
use mlua::{Table, Value};

use crate::script::{ScriptError, ScriptHost, stub_name};

/// The in-game HUD's layout script.
pub const HUD_LAYOUT: u32 = 0x4cd9_5654;

/// The virtual screen layouts are drawn on.
pub const VIRTUAL_SIZE: (f32, f32) = (640.0, 480.0);

/// A field's value as the layout script set it.
#[derive(Clone, Debug, PartialEq)]
pub enum Field {
    Bool(bool),
    Number(f64),
    Text(String),
    /// An engine or script function named `Module.Function`, or
    /// `<function>` for one defined in the layout script itself.
    Function(String),
    /// A table that is not a widget.
    Table(BTreeMap<String, Field>),
}

impl Field {
    pub fn as_f32(&self) -> Option<f32> {
        match self {
            Self::Number(n) => Some(*n as f32),
            Self::Bool(b) => Some(f32::from(u8::from(*b))),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(s) | Self::Function(s) => Some(s),
            _ => None,
        }
    }
}

/// One widget and its children.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Widget {
    /// The key it was stored under in its parent's table.
    pub key: String,
    pub fields: BTreeMap<String, Field>,
    pub children: Vec<Widget>,
}

/// Where a widget's box is measured from on a screen wider or narrower
/// than 4:3.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Anchor {
    #[default]
    Start,
    Centre,
    End,
}

impl Widget {
    /// A field by name, ignoring case.
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.get(name).or_else(|| {
            self.fields
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v)
        })
    }

    pub fn number(&self, name: &str) -> Option<f32> {
        self.field(name).and_then(Field::as_f32)
    }

    pub fn text_field(&self, name: &str) -> Option<&str> {
        self.field(name).and_then(Field::as_str)
    }

    pub fn name(&self) -> &str {
        self.text_field("name").unwrap_or(&self.key)
    }

    /// `WidgetType`, lower case; empty for a plain container.
    pub fn kind(&self) -> String {
        self.text_field("WidgetType")
            .map(str::to_ascii_lowercase)
            .unwrap_or_default()
    }

    /// The box on the virtual screen: `[x1, y1, x2, y2]`.
    pub fn rect(&self) -> Option<[f32; 4]> {
        Some([
            self.number("x1")?,
            self.number("y1")?,
            self.number("x2")?,
            self.number("y2")?,
        ])
    }

    /// Whether it starts shown: `visible > 0`, as the game reads it
    /// (absent means shown). Containers start hidden whatever this says;
    /// see [`draws`](Self::draws).
    pub fn visible(&self) -> bool {
        match self.field("visible") {
            Some(Field::Bool(b)) => *b,
            Some(Field::Number(n)) => *n > 0.0,
            _ => true,
        }
    }

    /// `container = true`: the game makes it a plain widget, hidden.
    pub fn container(&self) -> bool {
        matches!(self.field("container"), Some(Field::Bool(true)))
    }

    /// Whether the widget itself draws when the layout loads: shown and
    /// not a container. Its children decide for themselves.
    pub fn draws(&self) -> bool {
        self.visible() && !self.container()
    }

    /// `nTileWidth`, `nTileHeight` of a (non-container) image: the
    /// texture repeats every so many virtual units.
    pub fn tile(&self) -> Option<(f32, f32)> {
        let (w, h) = (self.number("nTileWidth")?, self.number("nTileHeight")?);
        (w > 0.0 && h > 0.0).then_some((w, h))
    }

    /// A text widget's justification, as an anchor (left, centre, right).
    pub fn justification(&self) -> Anchor {
        anchor(self.field("Justification"))
    }

    /// A text widget's size factor (1 when unset).
    pub fn scale(&self) -> f32 {
        self.number("scale").unwrap_or(1.0)
    }

    /// `RedLevel`, `GreenLevel`, `BlueLevel` and `TranslucencyLevel` as
    /// the layout sets them, 0..=255 (255 where unset).
    pub fn levels(&self) -> [f32; 4] {
        ["RedLevel", "GreenLevel", "BlueLevel", "TranslucencyLevel"]
            .map(|name| self.number(name).unwrap_or(255.0))
    }

    /// Colour and opacity, 0..=1: [`levels`](Self::levels) over 255.
    pub fn colour(&self) -> [f32; 4] {
        self.levels().map(|v| (v / 255.0).clamp(0.0, 1.0))
    }

    /// The texture's name: `texture`, else `textureFile` without its
    /// directory and extension.
    pub fn texture(&self) -> Option<String> {
        if let Some(t) = self.text_field("texture").filter(|t| !t.is_empty()) {
            return Some(t.to_owned());
        }
        let file = self.text_field("textureFile").filter(|t| !t.is_empty())?;
        let base = file.rsplit(['\\', '/']).next().unwrap_or(file);
        Some(base.split('.').next().unwrap_or(base).to_owned())
    }

    /// The texture rectangle `[u1, v1, u2, v2]`, whole texture if unset.
    pub fn uv(&self) -> [f32; 4] {
        [
            self.number("u1").unwrap_or(0.0),
            self.number("v1").unwrap_or(0.0),
            self.number("u2").unwrap_or(1.0),
            self.number("v2").unwrap_or(1.0),
        ]
    }

    pub fn anchors(&self) -> (Anchor, Anchor) {
        (
            anchor(self.field("HorizontalAnchor")),
            anchor(self.field("VerticalAnchor")),
        )
    }

    /// Event name to handler (`Module.Function`).
    pub fn handlers(&self) -> BTreeMap<String, String> {
        let Some(Field::Table(t)) = self.field("EventHandlers") else {
            return BTreeMap::new();
        };
        t.iter()
            .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
            .collect()
    }

    /// This widget and every descendant, depth first, with their depth.
    pub fn walk(&self) -> Vec<(usize, &Widget)> {
        let mut out = Vec::new();
        let mut stack = vec![(0, self)];
        while let Some((depth, w)) = stack.pop() {
            out.push((depth, w));
            stack.extend(w.children.iter().rev().map(|c| (depth + 1, c)));
        }
        out
    }
}

/// Reads an anchor field: a name (`left`, `center`, `right`, `top`,
/// `bottom`...) or a number (0 start, 1 centre, 2 end).
fn anchor(field: Option<&Field>) -> Anchor {
    match field {
        Some(Field::Number(n)) => match *n as i64 {
            1 => Anchor::Centre,
            2 => Anchor::End,
            _ => Anchor::Start,
        },
        Some(Field::Text(s)) => {
            let s = s.to_ascii_lowercase();
            if s.contains("cent") || s.contains("mid") {
                Anchor::Centre
            } else if s.contains("right") || s.contains("bottom") {
                Anchor::End
            } else {
                Anchor::Start
            }
        }
        _ => Anchor::Start,
    }
}

/// How a top-level widget and everything under it map from the virtual
/// screen onto a real one.
///
/// The widget's anchor point (its left edge, middle or right edge, and top,
/// middle or bottom) moves with the screen: it lands where the same
/// fraction of the screen is. Sizes scale with the screen's height, so the
/// widget keeps its shape, and its children keep their offsets from the
/// anchor point at that scale. On a 16:9 screen a left-anchored box keeps
/// its left edge at the same fraction of the width and grows rightwards
/// at height scale; a right-anchored one does the same from its right edge.
///
/// This matches the HUD in footage of the game at 16:9 (health counter,
/// ammo box, minimap, message box, reticle, within a few pixels).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Correction {
    /// Screen pixels per virtual unit.
    pub scale: f32,
    /// Added after scaling.
    pub offset: [f32; 2],
}

impl Correction {
    /// The correction of a top-level widget with box `rect` and `anchors`
    /// on a `width` x `height` screen.
    pub fn new(rect: [f32; 4], anchors: (Anchor, Anchor), width: f32, height: f32) -> Self {
        let scale = height / VIRTUAL_SIZE.1;
        let axis = |a: Anchor, lo: f32, hi: f32, screen: f32, virt: f32| {
            let point = match a {
                Anchor::Start => lo,
                Anchor::Centre => (lo + hi) / 2.0,
                Anchor::End => hi,
            };
            point * screen / virt - point * scale
        };
        Self {
            scale,
            offset: [
                axis(anchors.0, rect[0], rect[2], width, VIRTUAL_SIZE.0),
                axis(anchors.1, rect[1], rect[3], height, VIRTUAL_SIZE.1),
            ],
        }
    }

    /// The correction of `w` as a top-level widget (identity scale for one
    /// without a box).
    pub fn for_widget(w: &Widget, width: f32, height: f32) -> Self {
        match w.rect() {
            Some(r) => Self::new(r, w.anchors(), width, height),
            None => Self {
                scale: height / VIRTUAL_SIZE.1,
                offset: [0.0; 2],
            },
        }
    }

    /// A virtual-screen box in screen pixels.
    pub fn apply(&self, rect: [f32; 4]) -> [f32; 4] {
        [
            rect[0] * self.scale + self.offset[0],
            rect[1] * self.scale + self.offset[1],
            rect[2] * self.scale + self.offset[0],
            rect[3] * self.scale + self.offset[1],
        ]
    }
}

/// Maps a top-level widget's box onto a `width` x `height` screen; see
/// [`Correction`].
pub fn place(rect: [f32; 4], anchors: (Anchor, Anchor), width: f32, height: f32) -> [f32; 4] {
    Correction::new(rect, anchors, width, height).apply(rect)
}

/// Errors from loading a layout.
#[derive(Debug, thiserror::Error)]
pub enum GuiError {
    #[error(transparent)]
    Script(#[from] ScriptError),
    #[error(transparent)]
    Lua(#[from] mlua::Error),
    #[error("the script left no LocalWidgetList")]
    NoList,
}

/// Runs a layout script and reads its widget tree: the top-level widgets
/// of `LocalWidgetList`, in key order for a keyed list, else list order.
pub fn load_layout(chunk: &Chunk) -> Result<Vec<Widget>, GuiError> {
    let host = ScriptHost::new()?;
    host.stub_missing_globals()?;
    let f = host.load(chunk, "guilayout")?;
    f.call::<()>(())?;
    let list = host.lua().globals().raw_get::<Value>("LocalWidgetList")?;
    let Value::Table(list) = list else {
        return Err(GuiError::NoList);
    };
    widgets(&list, 0)
}

/// Deepest nesting read; layouts are a few levels deep.
const MAX_DEPTH: usize = 32;

fn widgets(t: &Table, depth: usize) -> Result<Vec<Widget>, GuiError> {
    let mut out = Vec::new();
    for pair in t.pairs::<Value, Value>() {
        let (k, v) = pair?;
        if let Value::Table(w) = v {
            out.push(widget(key(&k), &w, depth)?);
        }
    }
    // Lua's iteration order is arbitrary: list parts first in order, then
    // keys sorted, so a dump is stable.
    out.sort_by(|a, b| {
        let (na, nb) = (a.key.parse::<u64>(), b.key.parse::<u64>());
        match (na, nb) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            (Ok(_), Err(_)) => std::cmp::Ordering::Less,
            (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
            _ => a.key.cmp(&b.key),
        }
    });
    Ok(out)
}

fn widget(key: String, t: &Table, depth: usize) -> Result<Widget, GuiError> {
    let mut w = Widget {
        key,
        ..Widget::default()
    };
    for pair in t.pairs::<Value, Value>() {
        let (k, v) = pair?;
        let k = self::key(&k);
        if k.eq_ignore_ascii_case("Children") {
            if let Value::Table(c) = &v
                && depth < MAX_DEPTH
            {
                w.children = widgets(c, depth + 1)?;
            }
            continue;
        }
        if let Some(f) = field(&v, depth)? {
            w.fields.insert(k, f);
        }
    }
    Ok(w)
}

fn key(k: &Value) -> String {
    match k {
        Value::String(s) => s.to_string_lossy(),
        Value::Integer(i) => i.to_string(),
        Value::Number(n) if n.fract() == 0.0 => format!("{}", *n as i64),
        Value::Number(n) => n.to_string(),
        Value::Boolean(b) => b.to_string(),
        other => format!("<{}>", other.type_name()),
    }
}

fn field(v: &Value, depth: usize) -> Result<Option<Field>, GuiError> {
    Ok(Some(match v {
        Value::Boolean(b) => Field::Bool(*b),
        Value::Integer(i) => Field::Number(*i as f64),
        Value::Number(n) => Field::Number(*n),
        Value::String(s) => Field::Text(s.to_string_lossy()),
        Value::Function(_) => Field::Function("<function>".to_owned()),
        Value::Table(t) => {
            if let Some(name) = stub_name(v) {
                return Ok(Some(Field::Function(name)));
            }
            if depth >= MAX_DEPTH {
                return Ok(None);
            }
            let mut map = BTreeMap::new();
            for pair in t.pairs::<Value, Value>() {
                let (k, v) = pair?;
                if let Some(f) = field(&v, depth + 1)? {
                    map.insert(key(&k), f);
                }
            }
            Field::Table(map)
        }
        _ => return Ok(None),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compiles Lua source with the host's own compiler, as a stand-in for
    /// a layout script from the archive.
    fn compile(src: &str) -> Chunk {
        let lua = mlua::Lua::new();
        let f = lua.load(src).into_function().unwrap();
        Chunk::parse(&f.dump(false)).unwrap()
    }

    const LAYOUT: &str = r#"
        LocalWidgetList = {
            Health = {
                name = "Health Counter", WidgetType = "none",
                x1 = 10, y1 = 400, x2 = 90, y2 = 470,
                HorizontalAnchor = "left", VerticalAnchor = "bottom",
                EventHandlerFile = "HealthCounter",
                EventHandlers = { PlayerHealthChanged = HealthCounter.Update },
                Children = {
                    { name = "Gauge", WidgetType = "image", texture = "global_gui_hud02",
                      u1 = 0.5, v1 = 0, u2 = 0.75, v2 = 0.25,
                      x1 = 10, y1 = 400, x2 = 90, y2 = 470,
                      RedLevel = 255, GreenLevel = 200, BlueLevel = 0, TranslucencyLevel = 128 },
                    { name = "Number", WidgetType = "text", font = "english_20",
                      text = "100", visible = false, Justification = "center" },
                },
            },
            Radar = { name = "Radar", WidgetType = "minimap",
                      textureFile = "textures\\hud\\radar_ring.tga",
                      HorizontalAnchor = "right", x1 = 540, y1 = 380, x2 = 630, y2 = 470 },
        }
    "#;

    #[test]
    fn reads_widget_tree() {
        let list = load_layout(&compile(LAYOUT)).unwrap();
        assert_eq!(list.len(), 2);
        let health = &list[0];
        assert_eq!(health.name(), "Health Counter");
        assert_eq!(health.kind(), "none");
        assert_eq!(health.rect(), Some([10.0, 400.0, 90.0, 470.0]));
        assert_eq!(health.anchors(), (Anchor::Start, Anchor::End));
        assert_eq!(
            health
                .handlers()
                .get("PlayerHealthChanged")
                .map(String::as_str),
            Some("HealthCounter.Update")
        );
        assert_eq!(health.children.len(), 2);
        let gauge = &health.children[0];
        assert_eq!(gauge.texture().as_deref(), Some("global_gui_hud02"));
        assert_eq!(gauge.uv(), [0.5, 0.0, 0.75, 0.25]);
        let c = gauge.colour();
        assert!((c[1] - 200.0 / 255.0).abs() < 1e-6 && (c[3] - 128.0 / 255.0).abs() < 1e-6);
        let number = &health.children[1];
        assert!(!number.visible());
        assert_eq!(number.text_field("text"), Some("100"));
        let radar = &list[1];
        assert_eq!(radar.texture().as_deref(), Some("radar_ring"));
        assert_eq!(radar.anchors().0, Anchor::End);
        assert_eq!(health.walk().len(), 3);
    }

    #[test]
    fn places_by_anchor_on_wide_screens() {
        let r = [540.0, 380.0, 630.0, 470.0];
        // 4:3 is the virtual screen scaled, whatever the anchors.
        for a in [Anchor::Start, Anchor::Centre, Anchor::End] {
            assert_eq!(
                place(r, (a, a), 1280.0, 960.0),
                [1080.0, 760.0, 1260.0, 940.0]
            );
        }
        // 16:9 at 720 lines: scale 1.5; the right edge lands at 630/640 of
        // the width and the box grows leftwards from it.
        let p = place(r, (Anchor::End, Anchor::Start), 1280.0, 720.0);
        assert_eq!(p, [1260.0 - 135.0, 570.0, 1260.0, 705.0]);
        // Left-anchored: the left edge at 540/640 of the width.
        let p = place(r, (Anchor::Start, Anchor::End), 1280.0, 720.0);
        assert_eq!(p, [1080.0, 570.0, 1080.0 + 135.0, 705.0]);
        // Centred: the middle stays the middle.
        let p = place(
            [0.0, 0.0, 640.0, 480.0],
            (Anchor::Centre, Anchor::Start),
            1280.0,
            720.0,
        );
        assert_eq!(p, [160.0, 0.0, 1120.0, 720.0]);
    }

    #[test]
    fn children_move_with_their_top_level_widget() {
        let top = [460.0, 40.0, 600.0, 110.0];
        let c = Correction::new(top, (Anchor::End, Anchor::Start), 1280.0, 720.0);
        assert_eq!(c.apply(top), [1200.0 - 210.0, 60.0, 1200.0, 165.0]);
        // A child 10 units in from the right edge keeps that at scale 1.5.
        let child = c.apply([500.0, 50.0, 590.0, 70.0]);
        assert_eq!(child, [1200.0 - 150.0, 75.0, 1200.0 - 15.0, 105.0]);
    }

    #[test]
    fn containers_and_hidden_widgets_do_not_draw() {
        let list = load_layout(&compile(
            r#"LocalWidgetList = {
                { name = "a", WidgetType = "image", visible = 1, container = true },
                { name = "b", WidgetType = "image", visible = -1, container = false },
                { name = "c", WidgetType = "text", visible = 1, container = false,
                  Justification = "right", scale = 0.67, nTileWidth = 6, nTileHeight = 8 },
            }"#,
        ))
        .unwrap();
        assert!(list[0].visible() && list[0].container() && !list[0].draws());
        assert!(!list[1].visible() && !list[1].draws());
        assert!(list[2].draws());
        assert_eq!(list[2].justification(), Anchor::End);
        assert!((list[2].scale() - 0.67).abs() < 1e-6);
        assert_eq!(list[2].tile(), Some((6.0, 8.0)));
    }

    #[test]
    fn no_list_is_an_error() {
        assert!(matches!(
            load_layout(&compile("x = 1")),
            Err(GuiError::NoList)
        ));
    }
}
