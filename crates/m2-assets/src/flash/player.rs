//! A movie played: its timelines frame by frame, each sprite instance
//! with a playhead of its own.
//!
//! A [`Library`] holds what a movie defines: shapes (with their meshes),
//! the timelines of its sprites and of the movie itself (each frame's
//! placements, removals and scripts, and the frames' labels), the images
//! it names, its text fields and the symbols it exports by name.
//!
//! A [`Player`] keeps the instances on stage in an arena of [`Clip`]s:
//! the movie's own timeline at the root, and under each sprite instance
//! what its timeline (or a script) has placed, by depth. [`Player::step`]
//! advances every playing timeline at the movie's frame rate, looping at
//! the end; [`Player::goto`] moves a playhead: forward by applying the
//! frames passed, backward by clearing what the timeline placed and
//! replaying from the first frame. Only the frame landed on queues its
//! scripts ([`Player::take_scripts`]), as the Flash player runs them.
//! [`Player::draw`] lays out what is on stage as triangles.

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use super::text::{EditText, Font, FontBook, layout};
use super::{
    Bits, ColourTransform, Image, Matrix, Shape, ShapeTriangle, Triangle, bounds, draw_shape, tags,
};
use crate::gfx::{Character, Movie, tag};

/// What a frame of a timeline does, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    Place(Place),
    Remove(u16),
    /// A script (`DoAction`): its action bytes.
    Script(Rc<[u8]>),
}

/// A `PlaceObject`: a new character at a depth, or the one there changed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Place {
    pub depth: u16,
    pub character: Option<u16>,
    pub matrix: Option<Matrix>,
    pub colour: Option<ColourTransform>,
    pub name: Option<String>,
    pub clip_depth: Option<u16>,
    /// Changes the character at the depth (with `character`, replaces it).
    pub moving: bool,
}

/// A timeline: what each frame does, and the frames' labels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Timeline {
    pub frames: Vec<Vec<Control>>,
    pub labels: HashMap<String, usize>,
}

impl Timeline {
    /// Reads a timeline's tags.
    pub fn parse(tags: &[(u16, &[u8])]) -> Timeline {
        let mut t = Timeline {
            frames: vec![Vec::new()],
            labels: HashMap::new(),
        };
        for &(code, data) in tags {
            let frame = t.frames.len() - 1;
            let controls = &mut t.frames[frame];
            match code {
                tag::SHOW_FRAME => t.frames.push(Vec::new()),
                tag::FRAME_LABEL => {
                    let name = Bits::new(data).cstr();
                    t.labels.entry(name.to_ascii_lowercase()).or_insert(frame);
                }
                tag::PLACE_OBJECT => controls.push(Control::Place(place1(data))),
                tag::PLACE_OBJECT2 | tag::PLACE_OBJECT3 => {
                    controls.push(Control::Place(place23(data, code == tag::PLACE_OBJECT3)));
                }
                tag::REMOVE_OBJECT => {
                    let mut b = Bits::new(data);
                    b.u16();
                    controls.push(Control::Remove(b.u16()));
                }
                tag::REMOVE_OBJECT2 => controls.push(Control::Remove(Bits::new(data).u16())),
                tag::DO_ACTION => controls.push(Control::Script(data.into())),
                _ => {}
            }
        }
        // What follows the last ShowFrame is no frame.
        if t.frames.len() > 1 && t.frames.last().is_some_and(Vec::is_empty) {
            t.frames.pop();
        }
        t
    }

    /// The frame a label (any case) or a number (1-based) names.
    pub fn frame_of(&self, label: &str) -> Option<usize> {
        self.labels
            .get(&label.to_ascii_lowercase())
            .copied()
            .or_else(|| {
                label
                    .trim()
                    .parse::<usize>()
                    .ok()
                    .map(|n| n.saturating_sub(1))
            })
    }
}

fn place1(data: &[u8]) -> Place {
    let mut b = Bits::new(data);
    let character = b.u16();
    let depth = b.u16();
    let matrix = b.matrix();
    let colour = (!b.done()).then(|| b.cxform(false));
    Place {
        depth,
        character: Some(character),
        matrix: Some(matrix),
        colour,
        ..Place::default()
    }
}

fn place23(data: &[u8], three: bool) -> Place {
    let mut b = Bits::new(data);
    let flags = b.u8();
    let flags3 = if three { b.u8() } else { 0 };
    let depth = b.u16();
    if three && (flags3 & 8 != 0 || (flags3 & 16 != 0 && flags & 2 != 0)) {
        b.cstr();
    }
    let character = (flags & 2 != 0).then(|| b.u16());
    let matrix = (flags & 4 != 0).then(|| b.matrix());
    let colour = (flags & 8 != 0).then(|| b.cxform(true));
    if flags & 16 != 0 {
        b.u16();
    }
    let name = (flags & 32 != 0).then(|| b.cstr());
    let clip_depth = (flags & 64 != 0).then(|| b.u16());
    Place {
        depth,
        character,
        matrix,
        colour,
        name,
        clip_depth,
        moving: flags & 1 != 0,
    }
}

/// What a movie defines.
#[derive(Clone, Debug, Default)]
pub struct Library {
    pub size: [f32; 2],
    pub frame_rate: f32,
    pub shapes: HashMap<u16, (Shape, Vec<ShapeTriangle>)>,
    pub sprites: HashMap<u16, Rc<Timeline>>,
    pub images: HashMap<u16, Image>,
    /// Images it defines that aren't drawn: bitmaps embedded in the movie,
    /// and parts of images it doesn't name.
    pub missing_images: Vec<u16>,
    pub texts: HashMap<u16, EditText>,
    /// Fonts by character id: defined here, or imported (by name).
    pub fonts: HashMap<u16, Font>,
    /// Symbols exported by name (`attachMovie`'s linkage names), any case.
    pub exports: HashMap<String, u16>,
    /// Fonts imported from other movies: character id, exported name.
    pub font_imports: Vec<(u16, String)>,
    /// `DoInitAction` scripts: the sprite they belong to, and the bytes.
    pub init_scripts: Vec<(u16, Rc<[u8]>)>,
    pub root: Rc<Timeline>,
}

impl Library {
    /// What `movie` defines.
    pub fn new(movie: &Movie) -> Self {
        let (w, h) = movie.frame_size.size_px();
        let mut lib = Library {
            size: [w, h],
            frame_rate: if movie.frame_rate > 0.0 {
                movie.frame_rate
            } else {
                30.0
            },
            ..Library::default()
        };
        let summary = movie.summary();
        for (id, c) in &summary.characters {
            if let Character::ExternalImage(e) = c {
                let stem = e
                    .file_name
                    .rsplit_once('.')
                    .map_or(e.file_name.as_str(), |(s, _)| s);
                let texture = if stem.is_empty() {
                    e.export_name.clone()
                } else {
                    stem.to_owned()
                };
                lib.images.insert(
                    *id,
                    Image {
                        texture,
                        width: f32::from(e.width),
                        height: f32::from(e.height),
                        part: [0.0, 0.0, 1.0, 1.0],
                    },
                );
            }
        }
        // Parts of those images (left, top, right, bottom in its pixels).
        for (id, c) in &summary.characters {
            if let Character::SubImage { image, rect } = c
                && let Some(whole) = lib.images.get(image).cloned()
            {
                let [l, t, r, b] = rect.map(f32::from);
                let (w, h) = (whole.width.max(1.0), whole.height.max(1.0));
                lib.images.insert(
                    *id,
                    Image {
                        texture: whole.texture,
                        width: (r - l).max(1.0),
                        height: (b - t).max(1.0),
                        part: [l / w, t / h, r / w, b / h],
                    },
                );
            }
        }
        lib.missing_images = summary
            .characters
            .iter()
            .filter(|(id, c)| {
                matches!(c, Character::Bitmap { .. } | Character::SubImage { .. })
                    && !lib.images.contains_key(id)
            })
            .map(|(id, _)| *id)
            .collect();
        for (id, name) in summary.exports {
            lib.exports.insert(name.to_ascii_lowercase(), id);
        }
        for (_, id, name) in summary.imports {
            lib.font_imports.push((id, name));
        }
        let tags: Vec<(u16, &[u8])> = movie.tags.iter().map(|t| (t.code, movie.data(t))).collect();
        lib.add(&tags);
        lib.root = Rc::new(Timeline::parse(&tags));
        lib
    }

    /// The characters defined in `tags`.
    fn add(&mut self, tags: &[(u16, &[u8])]) {
        for &(code, data) in tags {
            let version = match code {
                tag::DEFINE_SHAPE => 1,
                tag::DEFINE_SHAPE2 => 2,
                tag::DEFINE_SHAPE3 => 3,
                tag::DEFINE_SHAPE4 => 4,
                tag::DEFINE_SPRITE => {
                    let Some(id) = data.get(..2).map(|b| u16::from_le_bytes([b[0], b[1]])) else {
                        continue;
                    };
                    let inner = self::tags(data.get(4..).unwrap_or(&[]));
                    self.add(&inner);
                    self.sprites.insert(id, Rc::new(Timeline::parse(&inner)));
                    continue;
                }
                tag::DEFINE_EDIT_TEXT => {
                    if let Some((id, t)) = EditText::parse(data) {
                        self.texts.insert(id, t);
                    }
                    continue;
                }
                tag::DEFINE_FONT3 | tag::DEFINE_FONT2 => {
                    if let Some((id, f)) = Font::parse(data) {
                        self.fonts.insert(id, f);
                    }
                    continue;
                }
                tag::DO_INIT_ACTION => {
                    if let Some(id) = data.get(..2).map(|b| u16::from_le_bytes([b[0], b[1]])) {
                        self.init_scripts.push((id, data[2..].into()));
                    }
                    continue;
                }
                _ => continue,
            };
            let (id, shape) = Shape::parse(data, version);
            let mesh = shape.mesh();
            self.shapes.insert(id, (shape, mesh));
        }
    }
}

/// Where scripts see the timeline's depths: depth 1 of a timeline is
/// -16383 to them, leaving 0 and up to what they place.
pub const TIMELINE_DEPTH: i32 = -16384;

/// An instance on stage, by its index in the player's arena.
pub type ClipId = usize;

/// What an instance shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// A sprite or the movie: a timeline (or none, for one a script made).
    Sprite,
    Shape(u16),
    /// A text field: its definition, and the text it shows.
    Text(u16, String),
    /// A character not drawn (a morph shape, a button...).
    Other,
}

/// An instance on stage.
#[derive(Clone, Debug)]
pub struct Clip {
    /// The movie it belongs to (whose characters it and its children are).
    pub lib: Rc<Library>,
    pub character: u16,
    pub kind: Kind,
    pub parent: Option<ClipId>,
    pub depth: i32,
    pub name: String,
    pub matrix: Matrix,
    pub colour: ColourTransform,
    pub visible: bool,
    pub clip_depth: Option<i32>,
    /// Placed by its parent's timeline (cleared when that goes back).
    pub from_timeline: bool,
    pub timeline: Option<Rc<Timeline>>,
    /// The frame shown (0-based), and whether the playhead moves.
    pub frame: usize,
    pub playing: bool,
    /// Whether its first frame has been entered.
    pub entered: bool,
    pub children: BTreeMap<i32, ClipId>,
    /// What scripts have drawn on it (the drawing API), as triangles in
    /// its own space.
    pub drawing: Vec<Triangle>,
}

impl Clip {
    fn new(
        lib: Rc<Library>,
        character: u16,
        kind: Kind,
        parent: Option<ClipId>,
        depth: i32,
    ) -> Self {
        Clip {
            lib,
            character,
            kind,
            parent,
            depth,
            name: String::new(),
            matrix: Matrix::IDENTITY,
            colour: ColourTransform::IDENTITY,
            visible: true,
            clip_depth: None,
            from_timeline: false,
            timeline: None,
            frame: 0,
            playing: true,
            entered: false,
            children: BTreeMap::new(),
            drawing: Vec::new(),
        }
    }

    /// Frames in its timeline (1 for one without).
    pub fn frame_count(&self) -> usize {
        self.timeline.as_ref().map_or(1, |t| t.frames.len().max(1))
    }
}

/// At most this many frames are played in one [`Player::step`].
const MAX_FRAMES_PER_STEP: u32 = 4;

/// A movie on stage.
#[derive(Clone, Debug)]
pub struct Player {
    pub lib: Rc<Library>,
    pub clips: Vec<Option<Clip>>,
    pub root: ClipId,
    /// The movies loaded into levels (`_level0` the first), by level.
    pub levels: BTreeMap<i32, ClipId>,
    /// Seconds since the last frame played.
    clock: f32,
    /// Scripts due, in order: the instance they run on, and the bytes.
    scripts: Vec<(ClipId, Rc<[u8]>)>,
}

impl Player {
    /// A player of `lib`, on its first frame.
    pub fn new(lib: Rc<Library>) -> Self {
        let mut root = Clip::new(Rc::clone(&lib), 0, Kind::Sprite, None, 0);
        root.timeline = Some(Rc::clone(&lib.root));
        let mut p = Player {
            lib,
            clips: vec![Some(root)],
            root: 0,
            levels: BTreeMap::from([(0, 0)]),
            clock: 0.0,
            scripts: Vec::new(),
        };
        p.goto(0, 0);
        p
    }

    pub fn clip(&self, id: ClipId) -> Option<&Clip> {
        self.clips.get(id).and_then(Option::as_ref)
    }

    pub fn clip_mut(&mut self, id: ClipId) -> Option<&mut Clip> {
        self.clips.get_mut(id).and_then(Option::as_mut)
    }

    /// The scripts due since the last call, in the order they run.
    pub fn take_scripts(&mut self) -> Vec<(ClipId, Rc<[u8]>)> {
        std::mem::take(&mut self.scripts)
    }

    /// Plays `dt` seconds: every playing timeline moves on a frame each
    /// 1 / frame rate, looping at its end. Returns the frames played.
    pub fn step(&mut self, dt: f32) -> u32 {
        let frame = 1.0 / self.lib.frame_rate;
        self.clock += dt.max(0.0);
        let mut played = 0;
        while self.clock >= frame {
            self.clock -= frame;
            if played == MAX_FRAMES_PER_STEP {
                self.clock = 0.0;
                break;
            }
            self.advance();
            played += 1;
        }
        played
    }

    /// One frame of every playing timeline, parents before children.
    pub fn advance(&mut self) {
        for id in self.order() {
            let Some(c) = self.clip(id) else { continue };
            if c.playing && c.timeline.is_some() {
                let n = c.frame_count();
                if n > 1 {
                    let next = (c.frame + 1) % n;
                    self.goto(id, next);
                }
            }
        }
    }

    /// Every instance on stage, parents before their children, children
    /// by depth.
    pub fn order(&self) -> Vec<ClipId> {
        let mut out = Vec::new();
        let mut stack: Vec<ClipId> = self.levels.values().rev().copied().collect();
        while let Some(id) = stack.pop() {
            let Some(c) = self.clip(id) else { continue };
            out.push(id);
            stack.extend(c.children.values().rev());
        }
        out
    }

    /// Moves `id`'s playhead to `frame` (0-based, clamped), its children
    /// placed as the timeline has them there; queues that frame's scripts.
    pub fn goto(&mut self, id: ClipId, frame: usize) {
        let Some(c) = self.clip(id) else { return };
        let Some(timeline) = c.timeline.clone() else {
            return;
        };
        let frame = frame.min(timeline.frames.len().saturating_sub(1));
        let (from, entered) = (c.frame, c.entered);
        if entered && frame == from {
            return;
        }
        let start = if !entered || frame < from {
            // Back to the start: what the timeline placed goes.
            let placed: Vec<(i32, ClipId)> = c
                .children
                .iter()
                .filter(|(_, k)| self.clip(**k).is_some_and(|k| k.from_timeline))
                .map(|(d, k)| (*d, *k))
                .collect();
            for (depth, child) in placed {
                self.remove(id, depth, child);
            }
            0
        } else {
            from + 1
        };
        for f in start..=frame {
            for control in &timeline.frames[f] {
                match control {
                    Control::Place(p) => self.place(id, p),
                    Control::Remove(depth) => {
                        let depth = i32::from(*depth) + TIMELINE_DEPTH;
                        if let Some(child) =
                            self.clip(id).and_then(|c| c.children.get(&depth)).copied()
                        {
                            self.remove(id, depth, child);
                        }
                    }
                    Control::Script(bytes) if f == frame => {
                        self.scripts.push((id, Rc::clone(bytes)))
                    }
                    Control::Script(_) => {}
                }
            }
        }
        if let Some(c) = self.clip_mut(id) {
            c.frame = frame;
            c.entered = true;
        }
    }

    /// Takes `child` (at `depth`) off `parent`, and everything under it.
    pub fn remove(&mut self, parent: ClipId, depth: i32, child: ClipId) {
        if let Some(p) = self.clip_mut(parent)
            && p.children.get(&depth) == Some(&child)
        {
            p.children.remove(&depth);
        }
        let mut stack = vec![child];
        while let Some(id) = stack.pop() {
            if let Some(c) = self.clips.get_mut(id).and_then(Option::take) {
                stack.extend(c.children.values());
            }
        }
    }

    fn place(&mut self, parent: ClipId, p: &Place) {
        let depth = i32::from(p.depth) + TIMELINE_DEPTH;
        let existing = self
            .clip(parent)
            .and_then(|c| c.children.get(&depth))
            .copied();
        match (p.character, existing) {
            // A new character where one is already (not moving): kept.
            (Some(_), Some(_)) if !p.moving => {}
            // A new character, where there is none (or replacing one).
            (Some(character), _) => {
                // The old one's place kept when it is replaced.
                let old = existing.and_then(|e| self.clip(e).cloned());
                if let Some(e) = existing {
                    self.remove(parent, depth, e);
                }
                let id = self.add(parent, depth, character);
                if let Some(c) = self.clip_mut(id) {
                    c.from_timeline = true;
                    let keep = old.filter(|_| p.moving);
                    c.matrix = p
                        .matrix
                        .or(keep.as_ref().map(|o| o.matrix))
                        .unwrap_or(Matrix::IDENTITY);
                    c.colour = p
                        .colour
                        .or(keep.as_ref().map(|o| o.colour))
                        .unwrap_or(ColourTransform::IDENTITY);
                    c.clip_depth = p
                        .clip_depth
                        .map(|d| i32::from(d) + TIMELINE_DEPTH)
                        .or(keep.as_ref().and_then(|o| o.clip_depth));
                    c.name = p.name.clone().or(keep.map(|o| o.name)).unwrap_or_default();
                }
                // A sprite shows its first frame as it is placed.
                if self.clip(id).is_some_and(|c| c.timeline.is_some()) {
                    self.goto(id, 0);
                }
            }
            // The one there, changed.
            (None, Some(e)) => {
                if let Some(c) = self.clip_mut(e) {
                    if let Some(m) = p.matrix {
                        c.matrix = m;
                    }
                    if let Some(col) = p.colour {
                        c.colour = col;
                    }
                    if let Some(d) = p.clip_depth {
                        c.clip_depth = Some(i32::from(d) + TIMELINE_DEPTH);
                    }
                    if let Some(n) = &p.name {
                        c.name.clone_from(n);
                    }
                }
            }
            _ => {}
        }
    }

    /// A new instance of `character` at `depth` under `parent` (replacing
    /// none: the depth must be free), not yet on its first frame.
    pub fn add(&mut self, parent: ClipId, depth: i32, character: u16) -> ClipId {
        let lib = self
            .clip(parent)
            .map_or_else(|| Rc::clone(&self.lib), |p| Rc::clone(&p.lib));
        let kind = if lib.sprites.contains_key(&character) {
            Kind::Sprite
        } else if lib.shapes.contains_key(&character) {
            Kind::Shape(character)
        } else if let Some(t) = lib.texts.get(&character) {
            Kind::Text(character, t.initial.clone())
        } else {
            Kind::Other
        };
        let timeline = lib.sprites.get(&character).cloned();
        let mut c = Clip::new(lib, character, kind, Some(parent), depth);
        c.timeline = timeline;
        let id = self.clips.len();
        self.clips.push(Some(c));
        if let Some(p) = self.clip_mut(parent) {
            p.children.insert(depth, id);
        }
        id
    }

    /// An empty sprite (no timeline) at `depth` under `parent`, as
    /// `createEmptyMovieClip` makes one.
    pub fn add_empty(&mut self, parent: ClipId, depth: i32, name: &str) -> ClipId {
        if let Some(old) = self
            .clip(parent)
            .and_then(|c| c.children.get(&depth))
            .copied()
        {
            self.remove(parent, depth, old);
        }
        let lib = self
            .clip(parent)
            .map_or_else(|| Rc::clone(&self.lib), |p| Rc::clone(&p.lib));
        let mut c = Clip::new(lib, 0, Kind::Sprite, Some(parent), depth);
        c.name = name.to_owned();
        c.entered = true;
        let id = self.clips.len();
        self.clips.push(Some(c));
        if let Some(p) = self.clip_mut(parent) {
            p.children.insert(depth, id);
        }
        id
    }

    /// The child of `parent` named `name` (any case).
    pub fn child(&self, parent: ClipId, name: &str) -> Option<ClipId> {
        let p = self.clip(parent)?;
        p.children.values().copied().find(|&k| {
            self.clip(k)
                .is_some_and(|c| c.name.eq_ignore_ascii_case(name))
        })
    }

    /// `id`'s matrix to the stage, through its parents.
    pub fn world_matrix(&self, id: ClipId) -> Matrix {
        let mut m = Matrix::IDENTITY;
        let mut at = Some(id);
        while let Some(i) = at {
            let Some(c) = self.clip(i) else { break };
            m = c.matrix.then(&m);
            at = c.parent;
        }
        m
    }

    /// What is on stage as triangles, through `matrix`; text fields in the
    /// fonts `fonts` has.
    pub fn draw(&self, matrix: &Matrix, fonts: &FontBook, out: &mut Vec<Triangle>) {
        for &root in self.levels.values() {
            self.draw_clip(
                root,
                matrix,
                &ColourTransform::IDENTITY,
                None,
                fonts,
                out,
                0,
            );
        }
    }

    /// Loads `lib` into `level` (`loadMovieNum`), replacing what was there.
    pub fn load_level(&mut self, level: i32, lib: Rc<Library>) -> ClipId {
        if let Some(old) = self.levels.remove(&level) {
            self.remove_tree(old);
        }
        let mut root = Clip::new(Rc::clone(&lib), 0, Kind::Sprite, None, level);
        root.timeline = Some(Rc::clone(&lib.root));
        root.name = format!("_level{level}");
        let id = self.clips.len();
        self.clips.push(Some(root));
        self.levels.insert(level, id);
        if level == 0 {
            self.root = id;
            self.lib = lib;
        }
        self.goto(id, 0);
        id
    }

    /// Loads `lib` into instance `id` (`loadMovie`): its children and
    /// timeline replaced by the movie's.
    pub fn load_into(&mut self, id: ClipId, lib: Rc<Library>) {
        let children: Vec<(i32, ClipId)> = self
            .clip(id)
            .map(|c| c.children.iter().map(|(d, k)| (*d, *k)).collect())
            .unwrap_or_default();
        for (depth, child) in children {
            self.remove(id, depth, child);
        }
        if let Some(c) = self.clip_mut(id) {
            c.timeline = Some(Rc::clone(&lib.root));
            c.lib = lib;
            c.kind = Kind::Sprite;
            c.entered = false;
            c.frame = 0;
            c.playing = true;
            c.drawing.clear();
        }
        self.goto(id, 0);
    }

    fn remove_tree(&mut self, id: ClipId) {
        let mut stack = vec![id];
        while let Some(i) = stack.pop() {
            if let Some(c) = self.clips.get_mut(i).and_then(Option::take) {
                stack.extend(c.children.values());
            }
        }
    }

    /// The font a text field's font id stands for: the movie's own if it
    /// is drawn in a texture, else the one it imports by that name, else
    /// the font library's of its style.
    pub fn font_for<'a>(lib: &'a Library, id: u16, book: &'a FontBook) -> Option<&'a Font> {
        if let Some(f) = lib.fonts.get(&id).filter(|f| f.texture.is_some()) {
            return Some(f);
        }
        if let Some(f) = lib
            .font_imports
            .iter()
            .find(|(i, _)| *i == id)
            .and_then(|(_, name)| book.get(name))
        {
            return Some(f);
        }
        let own = lib.fonts.get(&id);
        let style = match own.map(|f| (f.bold, f.italic)) {
            Some((true, true)) => "_bold_italic_font",
            Some((true, false)) => "_bold_font",
            Some((false, true)) => "_italic_font",
            _ => "_normal_font",
        };
        book.get(style)
            .or_else(|| own.and_then(|f| book.get(&f.name)))
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_clip(
        &self,
        id: ClipId,
        parent_matrix: &Matrix,
        parent_colour: &ColourTransform,
        clip: Option<[f32; 4]>,
        fonts: &FontBook,
        out: &mut Vec<Triangle>,
        nesting: usize,
    ) {
        let Some(c) = self.clip(id) else { return };
        if !c.visible || nesting > 64 {
            return;
        }
        let m = parent_matrix.then(&c.matrix);
        let col = parent_colour.then(&c.colour);
        match &c.kind {
            Kind::Shape(s) => {
                if let Some((shape, mesh)) = c.lib.shapes.get(s) {
                    draw_shape(shape, mesh, &c.lib.images, &m, &col, clip, out);
                }
            }
            Kind::Text(def, s) => {
                if let Some(t) = c.lib.texts.get(def)
                    && let Some(font) = t.font.and_then(|f| Self::font_for(&c.lib, f, fonts))
                {
                    layout(t, s, font, &m, &col, clip, out);
                }
            }
            Kind::Sprite | Kind::Other => {}
        }
        for t in &c.drawing {
            let corners = t.corners.map(|p| m.apply(p));
            super::push_clipped(
                out,
                corners,
                t.uv,
                col.apply(t.colour),
                t.texture.clone(),
                clip,
            );
        }
        // Masks under way among the children: up to which depth, the box.
        let mut masks: Vec<(i32, [f32; 4])> = Vec::new();
        for (&depth, &child) in &c.children {
            masks.retain(|(to, _)| depth <= *to);
            let Some(k) = self.clip(child) else { continue };
            if let Some(to) = k.clip_depth {
                let mut shape = Vec::new();
                self.draw_clip(child, &m, &col, None, fonts, &mut shape, nesting + 1);
                if let Some(b) = bounds(&shape) {
                    masks.push((to, b));
                }
                continue;
            }
            let inner = masks.iter().fold(clip, |acc, (_, b)| {
                Some(match acc {
                    Some(a) => [
                        a[0].max(b[0]),
                        a[1].max(b[1]),
                        a[2].min(b[2]),
                        a[3].min(b[3]),
                    ],
                    None => *b,
                })
            });
            self.draw_clip(child, &m, &col, inner, fonts, out, nesting + 1);
        }
    }
}
