//! Reads the game's own HUD layout from your installation and lists it or
//! draws it.
//!
//! ```text
//! cargo run -p m2-game --example hud -- <vz.wad> tree [layout hash]
//! cargo run -p m2-game --example hud -- <vz.wad> sbs  <left.png> <right.png> <out.png>
//! cargo run -p m2-game --example hud -- <vz.wad> png  <out.png> [options]
//!     --size <w>x<h>         screen size (default 1280x720)
//!     --layout <hash>        layout script (default the HUD, 4cd95654)
//!     --all                  draw hidden widgets too (containers never draw)
//!     --outline              outline every widget's box
//!     --only <a,b,...>       draw only these top-level widgets
//!     --ingame               only what shows in ordinary play, with sample
//!                            values: reticle, ammo box, health, minimap
//!     --set <name>=<text>    show text in the text widget(s) called name
//!     --background <file>    draw over this PNG (8-bit RGB or RGBA, no
//!                            interlace), scaled to the screen size
//! ```
//!
//! `tree` prints each widget with its type, box, anchors, texture and UVs,
//! font and text, and handlers. `png` draws the layout as it starts:
//! images from the game's textures (tinted, UV-cropped or tiled), text in
//! the game's bitmap fonts (ids in brackets looked up in the English
//! strings), the minimap and Flash widgets as outlines. Each top-level
//! widget is placed for the screen's shape by its anchors and its children
//! move with it ([`gui::Correction`]). The image holds the game's own
//! artwork: keep it out of the repository (`dumps/` and `local/` are
//! ignored).

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::process::ExitCode;

use m2_assets::font::{FONT_TYPE, Font};
use m2_assets::lua::{Chunk, SCRIPT_TYPE};
use m2_assets::strings::STRINGS_TYPE;
use m2_assets::texture::{BODY, INFO};
use m2_assets::{BlockContents, StringTable, Texture, Ucfx, WadArchive, name_hash};
use m2_game::gui::{self, Anchor, Correction, HUD_LAYOUT, Widget};

/// Type hash of texture assets.
const TEXTURE_TYPE: u32 = 0xf011_157a;

/// The screen height the bitmap fonts are drawn for: a glyph's pixels are
/// screen pixels at 720 lines.
const FONT_LINES: f32 = 720.0;

/// What `--ingame` draws, and the values it shows.
const INGAME: [&str; 4] = ["reticle", "Guns", "Health Counter", "Radar"];
const INGAME_TEXT: [(&str, &str); 4] = [
    ("Clip Ammo", "29"),
    ("Stored Ammo", "290"),
    ("Counter", "98"),
    ("Map Label", " "),
];
/// Widgets the handlers hide as the HUD starts (target health, the
/// unreloadable-ammo count, the vehicle icon), so `--ingame` leaves them out.
const INGAME_HIDDEN: [&str; 6] = [
    "targetHealthBack",
    "healthbackstraight",
    "Unreloadable Ammo",
    "Vehicle Icon",
    "gunname",
    "trespass",
];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

type Error = Box<dyn std::error::Error>;

fn run(args: &[String]) -> Result<(), Error> {
    let [path, command, rest @ ..] = args else {
        return Err("usage: hud <vz.wad> tree [layout hash] | png <out.png> [options]".into());
    };
    let mut wad = WadArchive::open(BufReader::new(File::open(path)?))?;
    match command.as_str() {
        "tree" => {
            let layout = match rest.first() {
                Some(h) => u32::from_str_radix(h, 16)?,
                None => HUD_LAYOUT,
            };
            let widgets = gui::load_layout(&find_script(&mut wad, layout)?)?;
            for w in &widgets {
                for (depth, w) in w.walk() {
                    println!("{}{}", "  ".repeat(depth), describe(w));
                }
            }
            Ok(())
        }
        "png" => {
            let [out, options @ ..] = rest else {
                return Err("usage: png <out.png> [options]".into());
            };
            let o = Options::parse(options)?;
            let widgets = gui::load_layout(&find_script(&mut wad, o.layout)?)?;
            let mut canvas = match &o.background {
                Some(file) => Canvas::from_png(&std::fs::read(file)?, o.width, o.height)?,
                None => Canvas::new(o.width, o.height),
            };
            let mut assets = Assets {
                wad: &mut wad,
                textures: HashMap::new(),
                fonts: HashMap::new(),
                strings: None,
                missing: Vec::new(),
            };
            for w in &widgets {
                if !o.only.is_empty() && !o.only.iter().any(|n| n.eq_ignore_ascii_case(w.name())) {
                    continue;
                }
                let c = Correction::for_widget(w, o.width as f32, o.height as f32);
                draw(&mut assets, &o, &mut canvas, w, &c)?;
            }
            std::fs::write(out, png(o.width, o.height, &canvas.rgba))?;
            if !assets.missing.is_empty() {
                assets.missing.sort();
                assets.missing.dedup();
                eprintln!("not found: {}", assets.missing.join(", "));
            }
            println!("wrote {out}");
            Ok(())
        }
        "crop" => {
            // A region of a PNG, enlarged (nearest) by a whole factor.
            let [input, x, y, w, h, zoom, out] = rest else {
                return Err("usage: crop <in.png> <x> <y> <w> <h> <zoom> <out.png>".into());
            };
            let img = decode_png(&std::fs::read(input)?)?;
            let [x, y, w, h, z]: [u32; 5] =
                [x, y, w, h, zoom].map(|v| v.parse::<u32>().unwrap_or(0));
            let (w, h) = (
                w.min(img.width.saturating_sub(x)),
                h.min(img.height.saturating_sub(y)),
            );
            let z = z.max(1);
            let mut rgba = Vec::with_capacity((w * h * z * z * 4) as usize);
            for oy in 0..h * z {
                for ox in 0..w * z {
                    let i = (((y + oy / z) * img.width + x + ox / z) * 4) as usize;
                    rgba.extend_from_slice(&img.rgba[i..i + 4]);
                }
            }
            std::fs::write(out, png(w * z, h * z, &rgba))?;
            println!("wrote {out}");
            Ok(())
        }
        "sbs" => {
            // Two PNGs side by side at the left one's height.
            let [left, right, out] = rest else {
                return Err("usage: sbs <left.png> <right.png> <out.png>".into());
            };
            let a = decode_png(&std::fs::read(left)?)?;
            let b = decode_png(&std::fs::read(right)?)?;
            let h = a.height;
            let bw = b.width * h / b.height;
            let a = Canvas::from_png(&std::fs::read(left)?, a.width, h)?;
            let b = Canvas::from_png(&std::fs::read(right)?, bw, h)?;
            let w = a.width + 8 + b.width;
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h as usize {
                let row = |c: &Canvas| {
                    let s = y * c.width as usize * 4;
                    c.rgba[s..s + c.width as usize * 4].to_vec()
                };
                rgba.extend(row(&a));
                rgba.extend([0, 0, 0, 255].repeat(8));
                rgba.extend(row(&b));
            }
            std::fs::write(out, png(w, h, &rgba))?;
            println!("wrote {out}");
            Ok(())
        }
        other => Err(format!("unknown command {other}").into()),
    }
}

struct Options {
    width: u32,
    height: u32,
    layout: u32,
    all: bool,
    outline: bool,
    only: Vec<String>,
    hidden: Vec<String>,
    text: Vec<(String, String)>,
    background: Option<String>,
}

impl Options {
    fn parse(args: &[String]) -> Result<Self, Error> {
        let mut o = Options {
            width: 1280,
            height: 720,
            layout: HUD_LAYOUT,
            all: false,
            outline: false,
            only: Vec::new(),
            hidden: Vec::new(),
            text: Vec::new(),
            background: None,
        };
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let mut value = |what: &str| it.next().ok_or(format!("{a} needs {what}"));
            match a.as_str() {
                "--size" => {
                    let s = value("WxH")?;
                    let (w, h) = s.split_once('x').ok_or("--size needs WxH")?;
                    (o.width, o.height) = (w.parse()?, h.parse()?);
                }
                "--layout" => o.layout = u32::from_str_radix(value("a hash")?, 16)?,
                "--all" => o.all = true,
                "--outline" => o.outline = true,
                "--only" => o.only.extend(value("names")?.split(',').map(str::to_owned)),
                "--set" => {
                    let s = value("name=text")?;
                    let (k, v) = s.split_once('=').ok_or("--set needs name=text")?;
                    o.text.push((k.to_owned(), v.to_owned()));
                }
                "--background" => o.background = Some(value("a file")?.clone()),
                "--ingame" => {
                    o.only.extend(INGAME.map(str::to_owned));
                    o.hidden.extend(INGAME_HIDDEN.map(str::to_owned));
                    o.text
                        .extend(INGAME_TEXT.map(|(k, v)| (k.to_owned(), v.to_owned())));
                }
                other => return Err(format!("unknown option {other}").into()),
            }
        }
        Ok(o)
    }

    fn text_for(&self, w: &Widget) -> Option<&str> {
        self.text
            .iter()
            .rev()
            .find(|(k, _)| k.eq_ignore_ascii_case(w.name()))
            .map(|(_, v)| v.as_str())
    }
}

/// The archive and what has been read from it.
struct Assets<'a, R> {
    wad: &'a mut WadArchive<R>,
    textures: HashMap<u32, Option<Image>>,
    fonts: HashMap<String, Option<Font>>,
    strings: Option<Option<StringTable>>,
    missing: Vec<String>,
}

impl<R: Read + Seek> Assets<'_, R> {
    fn texture(&mut self, hash: u32) -> Option<&Image> {
        let wad = &mut *self.wad;
        self.textures
            .entry(hash)
            .or_insert_with(|| load_texture(wad, hash))
            .as_ref()
    }

    fn font(&mut self, name: &str) -> Option<Font> {
        let wad = &mut *self.wad;
        self.fonts
            .entry(name.to_owned())
            .or_insert_with(|| {
                let data = find_asset(wad, name_hash(name), FONT_TYPE)?;
                Font::parse(&data).ok()
            })
            .clone()
    }

    /// A text field's text: an id in brackets is looked up in the English
    /// strings, else shown as written.
    fn text(&mut self, text: &str) -> String {
        let Some(id) = text.strip_prefix('[').and_then(|t| t.strip_suffix(']')) else {
            return text.to_owned();
        };
        let wad = &mut *self.wad;
        let table = self.strings.get_or_insert_with(|| {
            let data = find_asset(wad, name_hash("english"), STRINGS_TYPE)?;
            StringTable::parse(&data).ok()
        });
        table
            .as_ref()
            .and_then(|t| t.lookup(id))
            .map_or_else(|| text.to_owned(), str::to_owned)
    }
}

/// An asset's bytes by name hash and type.
fn find_asset<R: Read + Seek>(wad: &mut WadArchive<R>, hash: u32, ty: u32) -> Option<Vec<u8>> {
    let blocks: Vec<usize> = wad
        .index()
        .assets_by_hash(hash)
        .filter(|a| wad.index().type_hash(a) == ty)
        .map(|a| usize::from(a.block))
        .collect();
    for b in blocks {
        let decoded = wad.read_block(b).ok()?;
        let contents = BlockContents::parse(&decoded).ok()?;
        if let Some(e) = contents.find(hash, ty) {
            return Some(contents.asset_data(e).to_vec());
        }
    }
    None
}
/// Finds a script by hash, looking in the GUI layout blocks first.
fn find_script<R: Read + Seek>(wad: &mut WadArchive<R>, hash: u32) -> Result<Chunk, Error> {
    let mut blocks: Vec<usize> = wad
        .index()
        .assets_by_hash(hash)
        .map(|a| usize::from(a.block))
        .collect();
    blocks.sort_unstable();
    blocks.dedup();
    for b in blocks {
        let decoded = wad.read_block(b)?;
        let contents = BlockContents::parse(&decoded)?;
        if let Some(e) = contents.find(hash, SCRIPT_TYPE) {
            return Ok(Chunk::from_asset(contents.asset_data(e))?);
        }
    }
    Err(format!("no script {hash:08x}").into())
}

fn describe(w: &Widget) -> String {
    let mut s = format!("{:?} <{}>", w.name(), w.kind());
    if !w.visible() {
        s += " hidden";
    }
    if let Some(r) = w.rect() {
        s += &format!(" [{} {} {} {}]", r[0], r[1], r[2], r[3]);
    }
    let (h, v) = w.anchors();
    s += &format!(" anchor {h:?}/{v:?}");
    if let Some(t) = w.texture() {
        let uv = w.uv();
        s += &format!(" tex {t} uv [{} {} {} {}]", uv[0], uv[1], uv[2], uv[3]);
    }
    if let Some(f) = w.text_field("font") {
        s += &format!(" font {f}");
    }
    if let Some(t) = w.text_field("text") {
        s += &format!(" text {t:?}");
    }
    let c = w.levels();
    if c != [255.0; 4] {
        s += &format!(" rgba [{} {} {} {}]", c[0], c[1], c[2], c[3]);
    }
    for (event, handler) in w.handlers() {
        s += &format!(" on {event}={handler}");
    }
    // Fields the accessors above don't cover.
    const SHOWN: [&str; 24] = [
        "name",
        "WidgetType",
        "visible",
        "x1",
        "y1",
        "x2",
        "y2",
        "HorizontalAnchor",
        "VerticalAnchor",
        "texture",
        "textureFile",
        "u1",
        "v1",
        "u2",
        "v2",
        "font",
        "text",
        "RedLevel",
        "GreenLevel",
        "BlueLevel",
        "TranslucencyLevel",
        "EventHandlers",
        "EventHandlerNames",
        "EventHandlerFile",
    ];
    for (k, v) in &w.fields {
        if !SHOWN.iter().any(|s| s.eq_ignore_ascii_case(k)) {
            s += &format!(" {k}={v:?}");
        }
    }
    s
}

/// Draws `w` and its children with the correction of their top-level
/// widget. A widget draws if it is shown and not a container (or with
/// `--all`, if not a container); its children decide for themselves.
fn draw<R: Read + Seek>(
    assets: &mut Assets<R>,
    o: &Options,
    canvas: &mut Canvas,
    w: &Widget,
    c: &Correction,
) -> Result<(), Error> {
    let hidden = o.hidden.iter().any(|n| n.eq_ignore_ascii_case(w.name()));
    let shown = !w.container() && !hidden && (o.all || w.visible());
    if let (true, Some(r)) = (shown, w.rect()) {
        let px = c.apply(r);
        let colour = w.colour();
        match w.kind().as_str() {
            "image" | "sprite" => match w.texture() {
                Some(name) => {
                    let uv = match w.tile() {
                        // Repeats every tile; the texture wraps.
                        Some((tw, th)) => [0.0, 0.0, (r[2] - r[0]) / tw, (r[3] - r[1]) / th],
                        None => w.uv(),
                    };
                    match assets.texture(name_hash(&name)) {
                        Some(img) => canvas.blit(img, px, uv, colour),
                        None => {
                            assets.missing.push(name);
                            canvas.outline(px, [255, 0, 255, 255]);
                        }
                    }
                }
                None => canvas.fill(px, colour),
            },
            "text" => {
                let text = o
                    .text_for(w)
                    .map(str::to_owned)
                    .unwrap_or_else(|| assets.text(w.text_field("text").unwrap_or_default()));
                let font = w.text_field("font").unwrap_or_default().to_owned();
                match assets.font(&font) {
                    Some(f) => {
                        let size = c.scale * gui::VIRTUAL_SIZE.1 / FONT_LINES * w.scale();
                        draw_text(
                            assets,
                            canvas,
                            &f,
                            &text,
                            px,
                            size,
                            w.justification(),
                            colour,
                        );
                    }
                    None => {
                        assets.missing.push(font);
                        canvas.outline(px, [255, 255, 0, 160]);
                    }
                }
            }
            "minimap" => canvas.outline(px, [0, 255, 0, 200]),
            "flash" => canvas.outline(px, [0, 160, 255, 200]),
            _ => {}
        }
    }
    if o.outline
        && let Some(r) = w.rect()
    {
        canvas.outline(c.apply(r), [255, 255, 255, 90]);
    }
    for child in &w.children {
        draw(assets, o, canvas, child, c)?;
    }
    Ok(())
}

/// Draws one line of text in `font` into the screen box `px`: from its
/// top, justified left, centre or right, `size` screen pixels per glyph
/// pixel, glyphs tinted by `colour`.
#[allow(clippy::too_many_arguments)]
fn draw_text<R: Read + Seek>(
    assets: &mut Assets<R>,
    canvas: &mut Canvas,
    font: &Font,
    text: &str,
    px: [f32; 4],
    size: f32,
    justify: Anchor,
    colour: [f32; 4],
) {
    let width = font.text_width(text) * size;
    let mut x = match justify {
        Anchor::Start => px[0],
        Anchor::Centre => (px[0] + px[2] - width) / 2.0,
        Anchor::End => px[2] - width,
    };
    for code in text.encode_utf16() {
        if let Some(g) = font.glyph(code)
            && let Some(m) = font.materials.get(usize::from(g.material))
            && let Some(page) = assets.texture(m.texture)
        {
            let y = px[1] + f32::from(g.y_offset) * size;
            let dst = [
                x,
                y,
                x + f32::from(g.width) * size,
                y + f32::from(g.height) * size,
            ];
            canvas.blit(page, dst, g.uv, colour);
        }
        x += font.advance(code) * size;
    }
}
/// RGBA8 pixels.
struct Image {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// Decodes a texture's top mip (with its streamed tiers if present).
fn load_texture<R: Read + Seek>(wad: &mut WadArchive<R>, hash: u32) -> Option<Image> {
    let mut blocks: Vec<usize> = wad
        .index()
        .assets_by_hash(hash)
        .flat_map(|a| std::iter::once(a.block).chain(a.alt_blocks()))
        .map(usize::from)
        .collect();
    blocks.sort_unstable();
    blocks.dedup();
    let mut base = None;
    let mut tiers: [Option<Vec<u8>>; 3] = Default::default();
    for b in blocks {
        let path = wad.index().blocks()[b].path.to_ascii_lowercase();
        let tier = ["_q0.", "_q1.", "_q2."]
            .iter()
            .position(|t| path.contains(t));
        let decoded = wad.read_block(b).ok()?;
        for (e, data) in BlockContents::parse(&decoded).ok()?.iter() {
            if e.type_hash != TEXTURE_TYPE || e.name_hash != hash {
                continue;
            }
            let u = Ucfx::parse(data).ok()?;
            if u.top_level(INFO).is_some() {
                base = Some(data.to_vec());
            } else if let (Some(t), Some(body)) = (tier, u.top_level(BODY)) {
                tiers[t] = Some(body.to_vec());
            }
        }
    }
    let base = base?;
    let tex = Texture::parse(&base).ok()?;
    let chain = tex.assemble([
        tiers[0].as_deref(),
        tiers[1].as_deref(),
        tiers[2].as_deref(),
    ]);
    let mip = match &chain {
        Ok(c) => tex.chain_mips(c).ok()?.into_iter().next()?,
        Err(_) => tex.resident_mips().into_iter().next()?,
    };
    let rgba = mip.to_rgba8(tex.info.format).ok()?;
    Some(Image {
        width: mip.width,
        height: mip.height,
        rgba,
    })
}

struct Canvas {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl Canvas {
    /// A mid-grey background, so dark and light HUD parts both show.
    fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            rgba: [90, 100, 90, 255].repeat((width * height) as usize),
        }
    }

    fn put(&mut self, x: i64, y: i64, c: [f32; 4]) {
        if x < 0 || y < 0 || x >= i64::from(self.width) || y >= i64::from(self.height) {
            return;
        }
        let i = (y as usize * self.width as usize + x as usize) * 4;
        let a = c[3].clamp(0.0, 1.0);
        for (d, c) in self.rgba[i..i + 3].iter_mut().zip(c) {
            let v = f32::from(*d);
            *d = (v + (c * 255.0 - v) * a).round() as u8;
        }
    }

    fn bounds(&self, r: [f32; 4]) -> (i64, i64, i64, i64) {
        let (x0, x1) = (r[0].min(r[2]), r[0].max(r[2]));
        let (y0, y1) = (r[1].min(r[3]), r[1].max(r[3]));
        (
            x0.round() as i64,
            y0.round() as i64,
            x1.round() as i64,
            y1.round() as i64,
        )
    }

    fn fill(&mut self, r: [f32; 4], c: [f32; 4]) {
        let (x0, y0, x1, y1) = self.bounds(r);
        for y in y0..y1 {
            for x in x0..x1 {
                self.put(x, y, c);
            }
        }
    }

    fn outline(&mut self, r: [f32; 4], c: [u8; 4]) {
        let c = c.map(|v| f32::from(v) / 255.0);
        let (x0, y0, x1, y1) = self.bounds(r);
        for x in x0..x1 {
            self.put(x, y0, c);
            self.put(x, y1 - 1, c);
        }
        for y in y0..y1 {
            self.put(x0, y, c);
            self.put(x1 - 1, y, c);
        }
    }

    /// Draws `uv` of `img` into `r`, tinted by `tint`, sampling nearest.
    /// A box drawn right to left or bottom to top flips the image.
    fn blit(&mut self, img: &Image, r: [f32; 4], uv: [f32; 4], tint: [f32; 4]) {
        let (x0, y0, x1, y1) = self.bounds(r);
        let (w, h) = ((x1 - x0).max(1) as f32, (y1 - y0).max(1) as f32);
        for y in y0..y1 {
            for x in x0..x1 {
                let fx = (x - x0) as f32 + 0.5;
                let fy = (y - y0) as f32 + 0.5;
                let mut s = fx / w;
                let mut t = fy / h;
                if r[2] < r[0] {
                    s = 1.0 - s;
                }
                if r[3] < r[1] {
                    t = 1.0 - t;
                }
                let u = uv[0] + (uv[2] - uv[0]) * s;
                let v = uv[1] + (uv[3] - uv[1]) * t;
                let tx = ((u.rem_euclid(1.0)) * img.width as f32) as u32 % img.width;
                let ty = ((v.rem_euclid(1.0)) * img.height as f32) as u32 % img.height;
                let i = ((ty * img.width + tx) * 4) as usize;
                let p = &img.rgba[i..i + 4];
                let c = [
                    f32::from(p[0]) / 255.0 * tint[0],
                    f32::from(p[1]) / 255.0 * tint[1],
                    f32::from(p[2]) / 255.0 * tint[2],
                    f32::from(p[3]) / 255.0 * tint[3],
                ];
                self.put(x, y, c);
            }
        }
    }
}

/// A PNG of RGBA8 pixels (filter 0, zlib via miniz_oxide).
fn png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut c = !0u32;
        for &b in data {
            c ^= u32::from(b);
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
        }
        !c
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |tag: &[u8; 4], data: &[u8]| {
        out.extend((data.len() as u32).to_be_bytes());
        let mut body = tag.to_vec();
        body.extend_from_slice(data);
        out.extend(&body);
        out.extend(crc32(&body).to_be_bytes());
    };
    let mut ihdr = width.to_be_bytes().to_vec();
    ihdr.extend(height.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    let mut raw = Vec::with_capacity(rgba.len() + height as usize);
    for row in rgba.chunks_exact(width as usize * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    chunk(
        b"IDAT",
        &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6),
    );
    chunk(b"IEND", &[]);
    out
}

impl Canvas {
    /// A PNG scaled (nearest) to `width` x `height`, as the background.
    fn from_png(bytes: &[u8], width: u32, height: u32) -> Result<Self, Error> {
        let img = decode_png(bytes)?;
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            let sy = (y * img.height / height) as usize;
            for x in 0..width {
                let sx = (x * img.width / width) as usize;
                let i = (sy * img.width as usize + sx) * 4;
                rgba.extend_from_slice(&img.rgba[i..i + 4]);
            }
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }
}

/// Decodes an 8-bit RGB or RGBA, non-interlaced PNG.
fn decode_png(bytes: &[u8]) -> Result<Image, Error> {
    if bytes.get(..8) != Some(b"\x89PNG\r\n\x1a\n".as_slice()) {
        return Err("not a PNG".into());
    }
    let (mut pos, mut header, mut idat) = (8, None, Vec::new());
    while pos + 8 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[pos..pos + 4].try_into()?) as usize;
        let tag = &bytes[pos + 4..pos + 8];
        let data = bytes.get(pos + 8..pos + 8 + len).ok_or("truncated PNG")?;
        match tag {
            b"IHDR" => header = Some(data.to_vec()),
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => break,
            _ => {}
        }
        pos += 12 + len;
    }
    let h = header.ok_or("PNG without IHDR")?;
    let width = u32::from_be_bytes(h[0..4].try_into()?);
    let height = u32::from_be_bytes(h[4..8].try_into()?);
    let channels = match (h[8], h[9], h[12]) {
        (8, 2, 0) => 3,
        (8, 6, 0) => 4,
        _ => return Err("only 8-bit RGB/RGBA non-interlaced PNGs".into()),
    };
    let raw = miniz_oxide::inflate::decompress_to_vec_zlib(&idat)
        .map_err(|e| format!("PNG data: {:?}", e.status))?;
    let stride = width as usize * channels;
    let mut prev = vec![0u8; stride];
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height as usize {
        let row = raw
            .get(y * (stride + 1)..(y + 1) * (stride + 1))
            .ok_or("PNG data too short")?;
        let (filter, src) = (row[0], &row[1..]);
        let mut cur = vec![0u8; stride];
        for i in 0..stride {
            let a = if i >= channels { cur[i - channels] } else { 0 };
            let b = prev[i];
            let c = if i >= channels { prev[i - channels] } else { 0 };
            let predict = match filter {
                0 => 0,
                1 => a,
                2 => b,
                3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
                4 => {
                    let p = i16::from(a) + i16::from(b) - i16::from(c);
                    let (pa, pb, pc) = (
                        (p - i16::from(a)).abs(),
                        (p - i16::from(b)).abs(),
                        (p - i16::from(c)).abs(),
                    );
                    if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    }
                }
                _ => return Err("bad PNG filter".into()),
            };
            cur[i] = src[i].wrapping_add(predict);
        }
        for px in cur.chunks_exact(channels) {
            rgba.extend_from_slice(&px[..3]);
            rgba.push(if channels == 4 { px[3] } else { 255 });
        }
        prev = cur;
    }
    Ok(Image {
        width,
        height,
        rgba,
    })
}
