//! The game's own interface layouts: the HUD and the screens built like
//! it (binoculars, the satellite view, the PDA) are trees of native
//! widgets, not Flash movies.
//!
//! A layout is a compiled script in `vz.wad`'s `guilayouts` block that
//! fills the global `LocalWidgetList` with nested tables, one per widget:
//!
//! ```text
//! name                        the widget's name ("Health Counter")
//! WidgetType                  image, text, sprite, flash, minimap (none: a container)
//! x1, y1, x2, y2              its box on a 640 x 480 virtual screen
//! HorizontalAnchor, VerticalAnchor
//! Red, Green, Blue, TranslucencyLevel
//! visible
//! texture, textureFile, u1, v1, u2, v2, rotation, nTileWidth, nTileHeight
//! font, text, scale, Justification
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

    /// Whether it starts shown (absent means shown).
    pub fn visible(&self) -> bool {
        match self.field("visible") {
            Some(Field::Bool(b)) => *b,
            Some(Field::Number(n)) => *n != 0.0,
            _ => true,
        }
    }

    /// Colour and opacity, 0..=1. `TranslucencyLevel` is read as opacity
    /// in 0..=255 or 0..=1, whichever range it is in.
    pub fn colour(&self) -> [f32; 4] {
        let channel = |name: &str| {
            self.number(name)
                .map_or(1.0, |v| if v > 1.0 { v / 255.0 } else { v })
        };
        [
            channel("Red"),
            channel("Green"),
            channel("Blue"),
            channel("TranslucencyLevel"),
        ]
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

/// Maps a virtual-screen box onto a `width` x `height` screen. The
/// virtual screen is scaled to the screen's height; on a wider screen a
/// box keeps its distance from the side its anchor names (the middle for
/// centre), so start-anchored widgets hug the left edge and end-anchored
/// ones the right.
pub fn place(rect: [f32; 4], anchors: (Anchor, Anchor), width: f32, height: f32) -> [f32; 4] {
    let s = height / VIRTUAL_SIZE.1;
    let shift = |a: Anchor, screen: f32, virt: f32| match a {
        Anchor::Start => 0.0,
        Anchor::Centre => (screen - virt * s) / 2.0,
        Anchor::End => screen - virt * s,
    };
    let dx = shift(anchors.0, width, VIRTUAL_SIZE.0);
    let dy = shift(anchors.1, height, VIRTUAL_SIZE.1);
    [
        rect[0] * s + dx,
        rect[1] * s + dy,
        rect[2] * s + dx,
        rect[3] * s + dy,
    ]
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
                      Red = 255, Green = 200, Blue = 0, TranslucencyLevel = 128 },
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
        // 4:3 is the virtual screen scaled.
        assert_eq!(
            place(r, (Anchor::End, Anchor::End), 1280.0, 960.0),
            [1080.0, 760.0, 1260.0, 940.0]
        );
        // 16:9 at 720 lines: scale 1.5, right-anchored keeps its margin.
        let p = place(r, (Anchor::End, Anchor::Start), 1280.0, 720.0);
        assert_eq!(p, [1280.0 - 100.0 * 1.5, 570.0, 1280.0 - 10.0 * 1.5, 705.0]);
        let p = place(
            [0.0, 0.0, 640.0, 480.0],
            (Anchor::Centre, Anchor::Start),
            1280.0,
            720.0,
        );
        assert_eq!(p, [160.0, 0.0, 1120.0, 720.0]);
    }

    #[test]
    fn no_list_is_an_error() {
        assert!(matches!(
            load_layout(&compile("x = 1")),
            Err(GuiError::NoList)
        ));
    }
}
