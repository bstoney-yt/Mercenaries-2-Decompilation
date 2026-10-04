//! Reads the game's own HUD layout from your installation and lists it or
//! draws it.
//!
//! ```text
//! cargo run -p m2-game --example hud -- <vz.wad> tree [layout hash]
//! cargo run -p m2-game --example hud -- <vz.wad> png  <out.png> [options]
//!     --size <w>x<h>     screen size (default 1280x720)
//!     --layout <hash>    layout script (default the HUD, 4cd95654)
//!     --all              draw hidden widgets too
//!     --relative         children's boxes are relative to their parent's
//!     --outline          outline every widget's box
//! ```
//!
//! `tree` prints each widget with its type, box, anchors, texture and UVs,
//! font and text, and handlers. `png` draws the layout as it starts: images
//! from the game's textures (tinted, UV-cropped), text fields, the minimap
//! and Flash widgets as outlines. The image holds the game's own artwork:
//! keep it out of the repository (`dumps/` and `local/` are ignored).

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::process::ExitCode;

use m2_assets::lua::{Chunk, SCRIPT_TYPE};
use m2_assets::texture::{BODY, INFO};
use m2_assets::{BlockContents, Texture, Ucfx, WadArchive, name_hash};
use m2_game::gui::{self, HUD_LAYOUT, Widget};

/// Type hash of texture assets.
const TEXTURE_TYPE: u32 = 0xf011_157a;

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
            let mut o = Options {
                width: 1280,
                height: 720,
                layout: HUD_LAYOUT,
                all: false,
                relative: false,
                outline: false,
            };
            let mut it = options.iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--size" => {
                        let s = it.next().ok_or("--size needs WxH")?;
                        let (w, h) = s.split_once('x').ok_or("--size needs WxH")?;
                        (o.width, o.height) = (w.parse()?, h.parse()?);
                    }
                    "--layout" => {
                        o.layout =
                            u32::from_str_radix(it.next().ok_or("--layout needs a hash")?, 16)?
                    }
                    "--all" => o.all = true,
                    "--relative" => o.relative = true,
                    "--outline" => o.outline = true,
                    other => return Err(format!("unknown option {other}").into()),
                }
            }
            let widgets = gui::load_layout(&find_script(&mut wad, o.layout)?)?;
            let mut canvas = Canvas::new(o.width, o.height);
            let mut textures: HashMap<String, Option<Image>> = HashMap::new();
            let mut missing = Vec::new();
            for w in &widgets {
                draw(
                    &mut wad,
                    &o,
                    &mut canvas,
                    &mut textures,
                    &mut missing,
                    w,
                    [0.0; 2],
                )?;
            }
            std::fs::write(out, png(o.width, o.height, &canvas.rgba))?;
            if !missing.is_empty() {
                missing.sort();
                missing.dedup();
                eprintln!("textures not found: {}", missing.join(", "));
            }
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
    relative: bool,
    outline: bool,
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
    let c = w.colour();
    if c != [1.0; 4] {
        s += &format!(" rgba [{:.2} {:.2} {:.2} {:.2}]", c[0], c[1], c[2], c[3]);
    }
    for (event, handler) in w.handlers() {
        s += &format!(" on {event}={handler}");
    }
    // Fields the accessors above don't cover.
    const SHOWN: [&str; 21] = [
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
        "Red",
        "Green",
        "Blue",
        "EventHandlers",
    ];
    for (k, v) in &w.fields {
        if !SHOWN.iter().any(|s| s.eq_ignore_ascii_case(k)) && k != "TranslucencyLevel" {
            s += &format!(" {k}={v:?}");
        }
    }
    s
}

#[allow(clippy::too_many_arguments)]
fn draw<R: Read + Seek>(
    wad: &mut WadArchive<R>,
    o: &Options,
    canvas: &mut Canvas,
    textures: &mut HashMap<String, Option<Image>>,
    missing: &mut Vec<String>,
    w: &Widget,
    origin: [f32; 2],
) -> Result<(), Error> {
    if !o.all && !w.visible() {
        return Ok(());
    }
    let rect = w.rect().map(|r| {
        [
            r[0] + origin[0],
            r[1] + origin[1],
            r[2] + origin[0],
            r[3] + origin[1],
        ]
    });
    if let Some(r) = rect {
        let px = gui::place(r, w.anchors(), o.width as f32, o.height as f32);
        let colour = w.colour();
        match w.kind().as_str() {
            "image" | "sprite" => {
                if let Some(name) = w.texture() {
                    let image = textures
                        .entry(name.clone())
                        .or_insert_with(|| load_texture(wad, name_hash(&name)));
                    match image {
                        Some(img) => canvas.blit(img, px, w.uv(), colour),
                        None => {
                            missing.push(name);
                            canvas.outline(px, [255, 0, 255, 255]);
                        }
                    }
                } else {
                    canvas.fill(px, colour);
                }
            }
            "text" => canvas.outline(px, [255, 255, 0, 160]),
            "minimap" => canvas.outline(px, [0, 255, 0, 200]),
            "flash" => canvas.outline(px, [0, 160, 255, 200]),
            _ => {}
        }
        if o.outline {
            canvas.outline(px, [255, 255, 255, 90]);
        }
    }
    let child_origin = match (o.relative, rect) {
        (true, Some(r)) => [r[0], r[1]],
        _ => origin,
    };
    for c in &w.children {
        draw(wad, o, canvas, textures, missing, c, child_origin)?;
    }
    Ok(())
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
