//! Scaleform GFx movies: the Flash (SWF) files the game's interface is
//! built from, with Scaleform's own extension tags.
//!
//! A movie starts with an 8-byte header: a 3-byte signature, the SWF
//! version and the length of the whole uncompressed file. `FWS` and `GFX`
//! bodies follow as is; `CWS` and `CFX` bodies are one zlib stream. The
//! body is a frame rectangle, the frame rate (8.8 fixed point) and frame
//! count, then a list of tags, each a `u16` of `code << 6 | length` with
//! `length == 0x3f` meaning a `u32` length follows.
//!
//! [`Movie::parse`] reads the header and the tag list; [`Movie::summary`]
//! walks the tags (and the timelines of sprites) for what describes the
//! interface: characters and their bounds, where instances are placed and
//! what they are called, text fields and the variables they show, fonts,
//! exported and imported symbols, frame labels, images stored outside the
//! movie (Scaleform's `DefineExternalImage`), and the names ActionScript
//! uses (constant pools, functions, pushed strings). It keeps names and
//! numbers only, never the ActionScript itself.
//!
//! # Provenance
//!
//! Clean-room from the public SWF file format specification (version 10)
//! and the published list of Scaleform GFx extension tags. Nothing here
//! was read from the game's executable.

use std::collections::BTreeMap;

use crate::bytes::{OutOfBounds, Reader};
use crate::ucfx::{Ucfx, UcfxError};

/// Errors from parsing a movie.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum GfxError {
    #[error("not a movie (signature {0:02x?})")]
    Signature([u8; 3]),
    #[error("truncated: needed {len} bytes at {offset:#x}")]
    Truncated { offset: usize, len: usize },
    #[error("body does not inflate: {0}")]
    Inflate(String),
    #[error("container: {0}")]
    Ucfx(#[from] UcfxError),
    #[error("no {0} section")]
    Missing(&'static str),
    #[error("header says {declared} bytes, body gives {actual}")]
    Length { declared: u32, actual: usize },
}

impl From<OutOfBounds> for GfxError {
    fn from(e: OutOfBounds) -> Self {
        Self::Truncated {
            offset: e.offset,
            len: e.len,
        }
    }
}

/// Asset type hash of movies in the archives. Each is a `UCFX` container
/// with one section, `data`, holding the movie file; the asset's name hash
/// is [`crate::name_hash`] of the movie's file name without its extension
/// (`minimap` for `minimap.gfx`).
pub const MOVIE_TYPE: u32 = 0xfe0e_8320;

/// How far past its declared length a compressed movie may inflate.
const INFLATE_SLACK: usize = 64 * 1024;

/// Reads the movie stored in a movie asset ([`MOVIE_TYPE`]).
pub fn movie_from_asset(asset: &[u8]) -> Result<Movie, GfxError> {
    let ucfx = Ucfx::parse(asset)?;
    let data = ucfx.top_level(*b"data").ok_or(GfxError::Missing("data"))?;
    Movie::parse(data)
}

/// The signatures a movie can start with.
pub const SIGNATURES: [&[u8; 3]; 4] = [b"FWS", b"CWS", b"GFX", b"CFX"];

/// Whether `bytes` starts like a movie: a known signature, a version in
/// 3..=15 and a length at least the header's.
pub fn looks_like_movie(bytes: &[u8]) -> bool {
    bytes.len() >= 8
        && SIGNATURES.iter().any(|s| bytes[..3] == s[..])
        && (3..=15).contains(&bytes[3])
        && u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) >= 8
}

/// Tag codes this module reads.
pub mod tag {
    pub const END: u16 = 0;
    pub const SHOW_FRAME: u16 = 1;
    pub const DEFINE_SHAPE: u16 = 2;
    pub const PLACE_OBJECT: u16 = 4;
    pub const REMOVE_OBJECT: u16 = 5;
    pub const DEFINE_BITS: u16 = 6;
    pub const DEFINE_BUTTON: u16 = 7;
    pub const SET_BACKGROUND_COLOR: u16 = 9;
    pub const DEFINE_FONT: u16 = 10;
    pub const DEFINE_TEXT: u16 = 11;
    pub const DO_ACTION: u16 = 12;
    pub const DEFINE_SOUND: u16 = 14;
    pub const DEFINE_BITS_LOSSLESS: u16 = 20;
    pub const DEFINE_BITS_JPEG2: u16 = 21;
    pub const DEFINE_SHAPE2: u16 = 22;
    pub const PLACE_OBJECT2: u16 = 26;
    pub const REMOVE_OBJECT2: u16 = 28;
    pub const DEFINE_SHAPE3: u16 = 32;
    pub const DEFINE_TEXT2: u16 = 33;
    pub const DEFINE_BUTTON2: u16 = 34;
    pub const DEFINE_BITS_JPEG3: u16 = 35;
    pub const DEFINE_BITS_LOSSLESS2: u16 = 36;
    pub const DEFINE_EDIT_TEXT: u16 = 37;
    pub const DEFINE_SPRITE: u16 = 39;
    pub const FRAME_LABEL: u16 = 43;
    pub const DEFINE_MORPH_SHAPE: u16 = 46;
    pub const DEFINE_FONT2: u16 = 48;
    pub const EXPORT_ASSETS: u16 = 56;
    pub const IMPORT_ASSETS: u16 = 57;
    pub const DO_INIT_ACTION: u16 = 59;
    pub const DEFINE_FONT3: u16 = 75;
    pub const IMPORT_ASSETS2: u16 = 71;
    pub const PLACE_OBJECT3: u16 = 70;
    pub const SYMBOL_CLASS: u16 = 76;
    pub const DO_ABC: u16 = 82;
    pub const DEFINE_SHAPE4: u16 = 83;
    pub const DEFINE_MORPH_SHAPE2: u16 = 84;
    pub const DEFINE_FONT_NAME: u16 = 88;
    pub const DEFINE_FONT4: u16 = 91;
    // Scaleform GFx extensions.
    pub const GFX_EXPORTER_INFO: u16 = 1000;
    pub const GFX_DEFINE_EXTERNAL_IMAGE: u16 = 1001;
    pub const GFX_FONT_TEXTURE_INFO: u16 = 1002;
    pub const GFX_DEFINE_EXTERNAL_GRADIENT: u16 = 1003;
    pub const GFX_DEFINE_GRADIENT_MAP: u16 = 1004;
    pub const GFX_DEFINE_COMPACTED_FONT: u16 = 1005;
    pub const GFX_DEFINE_EXTERNAL_SOUND: u16 = 1006;
    pub const GFX_DEFINE_EXTERNAL_STREAM_SOUND: u16 = 1007;
    pub const GFX_DEFINE_SUB_IMAGE: u16 = 1008;
    pub const GFX_DEFINE_EXTERNAL_IMAGE2: u16 = 1009;

    /// A readable name for a tag code.
    pub fn name(code: u16) -> &'static str {
        match code {
            END => "End",
            SHOW_FRAME => "ShowFrame",
            DEFINE_SHAPE => "DefineShape",
            PLACE_OBJECT => "PlaceObject",
            REMOVE_OBJECT => "RemoveObject",
            DEFINE_BITS => "DefineBits",
            DEFINE_BUTTON => "DefineButton",
            8 => "JPEGTables",
            SET_BACKGROUND_COLOR => "SetBackgroundColor",
            DEFINE_FONT => "DefineFont",
            DEFINE_TEXT => "DefineText",
            DO_ACTION => "DoAction",
            13 => "DefineFontInfo",
            DEFINE_SOUND => "DefineSound",
            15 => "StartSound",
            17 => "DefineButtonSound",
            18 => "SoundStreamHead",
            19 => "SoundStreamBlock",
            DEFINE_BITS_LOSSLESS => "DefineBitsLossless",
            DEFINE_BITS_JPEG2 => "DefineBitsJPEG2",
            DEFINE_SHAPE2 => "DefineShape2",
            23 => "DefineButtonCxform",
            24 => "Protect",
            PLACE_OBJECT2 => "PlaceObject2",
            REMOVE_OBJECT2 => "RemoveObject2",
            DEFINE_SHAPE3 => "DefineShape3",
            DEFINE_TEXT2 => "DefineText2",
            DEFINE_BUTTON2 => "DefineButton2",
            DEFINE_BITS_JPEG3 => "DefineBitsJPEG3",
            DEFINE_BITS_LOSSLESS2 => "DefineBitsLossless2",
            DEFINE_EDIT_TEXT => "DefineEditText",
            DEFINE_SPRITE => "DefineSprite",
            FRAME_LABEL => "FrameLabel",
            45 => "SoundStreamHead2",
            DEFINE_MORPH_SHAPE => "DefineMorphShape",
            DEFINE_FONT2 => "DefineFont2",
            EXPORT_ASSETS => "ExportAssets",
            IMPORT_ASSETS => "ImportAssets",
            58 => "EnableDebugger",
            DO_INIT_ACTION => "DoInitAction",
            60 => "DefineVideoStream",
            61 => "VideoFrame",
            62 => "DefineFontInfo2",
            64 => "EnableDebugger2",
            65 => "ScriptLimits",
            66 => "SetTabIndex",
            69 => "FileAttributes",
            PLACE_OBJECT3 => "PlaceObject3",
            IMPORT_ASSETS2 => "ImportAssets2",
            73 => "DefineFontAlignZones",
            74 => "CSMTextSettings",
            DEFINE_FONT3 => "DefineFont3",
            SYMBOL_CLASS => "SymbolClass",
            77 => "Metadata",
            78 => "DefineScalingGrid",
            DO_ABC => "DoABC",
            DEFINE_SHAPE4 => "DefineShape4",
            DEFINE_MORPH_SHAPE2 => "DefineMorphShape2",
            86 => "DefineSceneAndFrameLabelData",
            87 => "DefineBinaryData",
            DEFINE_FONT_NAME => "DefineFontName",
            89 => "StartSound2",
            90 => "DefineBitsJPEG4",
            DEFINE_FONT4 => "DefineFont4",
            GFX_EXPORTER_INFO => "GFx:ExporterInfo",
            GFX_DEFINE_EXTERNAL_IMAGE => "GFx:DefineExternalImage",
            GFX_FONT_TEXTURE_INFO => "GFx:FontTextureInfo",
            GFX_DEFINE_EXTERNAL_GRADIENT => "GFx:DefineExternalGradient",
            GFX_DEFINE_GRADIENT_MAP => "GFx:DefineGradientMap",
            GFX_DEFINE_COMPACTED_FONT => "GFx:DefineCompactedFont",
            GFX_DEFINE_EXTERNAL_SOUND => "GFx:DefineExternalSound",
            GFX_DEFINE_EXTERNAL_STREAM_SOUND => "GFx:DefineExternalStreamSound",
            GFX_DEFINE_SUB_IMAGE => "GFx:DefineSubImage",
            GFX_DEFINE_EXTERNAL_IMAGE2 => "GFx:DefineExternalImage2",
            _ => "?",
        }
    }
}

/// A rectangle in twips (1/20 pixel).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect {
    pub x_min: i32,
    pub x_max: i32,
    pub y_min: i32,
    pub y_max: i32,
}

impl Rect {
    /// Width and height in pixels.
    pub fn size_px(&self) -> (f32, f32) {
        (
            (self.x_max - self.x_min) as f32 / 20.0,
            (self.y_max - self.y_min) as f32 / 20.0,
        )
    }
}

/// A 2D transform: `x' = a*x + c*y + tx`, `y' = b*x + d*y + ty`, with the
/// translation in twips.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub tx: i32,
    pub ty: i32,
}

impl Default for Matrix {
    fn default() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            tx: 0,
            ty: 0,
        }
    }
}

/// One tag of a movie or sprite: its code and where its data is in
/// [`Movie::body`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tag {
    pub code: u16,
    pub offset: usize,
    pub len: usize,
}

/// A parsed movie.
#[derive(Debug, Clone, PartialEq)]
pub struct Movie {
    /// The signature (`b"GFX"`, `b"CFX"`, `b"FWS"`, `b"CWS"`).
    pub signature: [u8; 3],
    pub version: u8,
    /// The whole file uncompressed, header included, so offsets match the
    /// declared length.
    pub body: Vec<u8>,
    pub frame_size: Rect,
    pub frame_rate: f32,
    pub frame_count: u16,
    /// The root timeline's tags.
    pub tags: Vec<Tag>,
}

impl Movie {
    /// Parses a movie, inflating a compressed body.
    pub fn parse(bytes: &[u8]) -> Result<Self, GfxError> {
        let mut r = Reader::at(bytes, 0);
        let signature: [u8; 3] = r.array()?;
        if !SIGNATURES.iter().any(|s| **s == signature) {
            return Err(GfxError::Signature(signature));
        }
        let version = r.u8()?;
        let declared = r.u32()?;
        let body = if signature[0] == b'C' {
            // The game's compressed movies inflate to more than the
            // declared length: the stream runs on to a 4 KiB boundary with
            // stale bytes, which are dropped.
            let want = (declared as usize).saturating_sub(8);
            let inflated = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(
                &bytes[8..],
                want + INFLATE_SLACK,
            )
            .map_err(|e| GfxError::Inflate(format!("{:?}", e.status)))?;
            let mut body = bytes[..8].to_vec();
            body.extend_from_slice(&inflated[..want.min(inflated.len())]);
            body
        } else {
            // The game's stored movies declare their length without the
            // 8-byte header: the tags run 8 bytes past it, to an End tag.
            // Others carry padding after them inside archives.
            let with_header = declared as usize + 8;
            let len = if bytes.get(with_header - 2..with_header) == Some(&[0, 0])
                && read_tags(bytes, header_end(bytes)?, with_header)
                    .is_ok_and(|t| t.last().is_some_and(|t| t.code == tag::END))
            {
                with_header
            } else {
                (declared as usize).min(bytes.len())
            };
            bytes[..len].to_vec()
        };
        if body.len() != declared as usize && body.len() != declared as usize + 8 {
            return Err(GfxError::Length {
                declared,
                actual: body.len(),
            });
        }
        let mut bits = Bits::at(&body, 8);
        let frame_size = bits.rect()?;
        let mut r = Reader::at(&body, bits.byte_pos());
        let frame_rate = f32::from(r.u16()?) / 256.0;
        let frame_count = r.u16()?;
        let tags = read_tags(&body, r.pos(), body.len())?;
        Ok(Self {
            signature,
            version,
            body,
            frame_size,
            frame_rate,
            frame_count,
            tags,
        })
    }

    /// Whether the movie is Scaleform's own variant (`GFX`/`CFX`).
    pub fn is_gfx(&self) -> bool {
        self.signature[1..] == *b"FX"
    }

    /// A tag's data.
    pub fn data(&self, tag: &Tag) -> &[u8] {
        &self.body[tag.offset..tag.offset + tag.len]
    }

    /// Walks every tag, sprites' timelines included.
    pub fn summary(&self) -> Summary {
        let mut s = Summary::default();
        self.walk(&self.tags, 0, &mut s);
        s
    }

    fn walk(&self, tags: &[Tag], timeline: u16, s: &mut Summary) {
        let mut frame = 0u16;
        for t in tags {
            *s.tag_counts.entry(t.code).or_default() += 1;
            let data = self.data(t);
            match read_tag(t.code, data, timeline, frame, s) {
                Ok(Some(sprite_tags)) => {
                    let base = t.offset;
                    let nested: Vec<Tag> = sprite_tags
                        .into_iter()
                        .map(|n| Tag {
                            offset: base + n.offset,
                            ..n
                        })
                        .collect();
                    let id = u16::from_le_bytes([data[0], data[1]]);
                    self.walk(&nested, id, s);
                }
                Ok(None) => {}
                Err(_) => s.errors.push((timeline, t.code, t.offset)),
            }
            if t.code == tag::SHOW_FRAME {
                frame += 1;
            }
        }
    }
}

/// Where the tags start in an uncompressed movie: after the header, the
/// frame rectangle, the frame rate and the frame count.
fn header_end(bytes: &[u8]) -> Result<usize, GfxError> {
    let mut bits = Bits::at(bytes, 8);
    bits.rect()?;
    Ok(bits.byte_pos() + 4)
}

/// Reads tags from `start` to `end`, stopping after an `End` tag.
fn read_tags(body: &[u8], start: usize, end: usize) -> Result<Vec<Tag>, GfxError> {
    let mut r = Reader::at(&body[..end], start);
    let mut tags = Vec::new();
    while r.remaining() >= 2 {
        let head = r.u16()?;
        let code = head >> 6;
        let mut len = usize::from(head & 0x3f);
        if len == 0x3f {
            len = r.u32()? as usize;
        }
        let offset = r.pos();
        r.take(len)?;
        tags.push(Tag { code, offset, len });
        if code == tag::END {
            break;
        }
    }
    Ok(tags)
}

/// What kind of thing a character id names.
#[derive(Debug, Clone, PartialEq)]
pub enum Character {
    Shape { bounds: Rect },
    MorphShape,
    Sprite { frames: u16 },
    Button,
    Text { bounds: Rect },
    EditText(EditText),
    Font { name: String },
    Bitmap { tag: u16 },
    ExternalImage(ExternalImage),
    SubImage { image: u16, rect: [u16; 4] },
    Sound,
    Other { tag: u16 },
}

/// A text field: where it is, what it shows initially and the variable it
/// is bound to.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EditText {
    pub bounds: Rect,
    pub font: Option<u16>,
    pub font_class: Option<String>,
    /// Text height in twips.
    pub height: Option<u16>,
    pub color: Option<[u8; 4]>,
    pub max_length: Option<u16>,
    /// 0 left, 1 right, 2 centre, 3 justify.
    pub align: Option<u8>,
    pub variable: String,
    pub initial_text: Option<String>,
    pub html: bool,
    pub multiline: bool,
    pub use_outlines: bool,
}

/// An image Scaleform's exporter moved out of the movie into its own file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExternalImage {
    pub format: u16,
    pub width: u16,
    pub height: u16,
    pub export_name: String,
    pub file_name: String,
}

/// An instance placed on a timeline (`PlaceObject2`/`3`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Placement {
    /// The sprite whose timeline it is on (0 for the root).
    pub timeline: u16,
    pub frame: u16,
    pub depth: u16,
    /// Set when a new character is placed; absent when one is only moved.
    pub character: Option<u16>,
    pub matrix: Option<Matrix>,
    pub name: Option<String>,
    pub class_name: Option<String>,
    pub clip_depth: Option<u16>,
    pub moved: bool,
}

/// Names and layout gathered from a movie.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Summary {
    pub tag_counts: BTreeMap<u16, u32>,
    pub characters: BTreeMap<u16, Character>,
    pub placements: Vec<Placement>,
    /// `(timeline, frame, label)`.
    pub frame_labels: Vec<(u16, u16, String)>,
    /// `(character, name)`.
    pub exports: Vec<(u16, String)>,
    /// `(url, character, name)`.
    pub imports: Vec<(String, u16, String)>,
    /// `(character, class)`.
    pub symbol_classes: Vec<(u16, String)>,
    /// Font names from `DefineFontName` and Scaleform's compacted fonts.
    pub font_names: Vec<(u16, String)>,
    /// Exporter info: `(version, flags, bitmap format, prefix, swf name)`.
    pub exporter: Option<(u16, u32, u16, String, String)>,
    /// Font texture files named by `FontTextureInfo`.
    pub font_textures: Vec<String>,
    /// Names from ActionScript constant pools, function definitions and
    /// pushed strings, each once, in first-seen order.
    pub action_names: Vec<String>,
    pub background: Option<[u8; 3]>,
    pub has_abc: bool,
    /// Tags that did not parse: `(timeline, code, offset)`.
    pub errors: Vec<(u16, u16, usize)>,
}

impl Summary {
    fn name(&mut self, s: String) {
        if !s.is_empty() && !self.action_names.contains(&s) {
            self.action_names.push(s);
        }
    }

    /// Placements whose instance has a name, with the character placed.
    pub fn named_instances(&self) -> impl Iterator<Item = &Placement> {
        self.placements.iter().filter(|p| p.name.is_some())
    }
}

fn string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A length-prefixed (`u8`) string, as Scaleform's tags store them.
fn pstring(r: &mut Reader) -> Result<String, OutOfBounds> {
    let len = r.u8()?;
    Ok(string(r.take(usize::from(len))?))
}

/// Reads one tag into the summary. Returns a sprite's tags (offsets
/// relative to the tag's data) for the caller to walk.
fn read_tag(
    code: u16,
    data: &[u8],
    timeline: u16,
    frame: u16,
    s: &mut Summary,
) -> Result<Option<Vec<Tag>>, OutOfBounds> {
    let mut r = Reader::at(data, 0);
    match code {
        tag::SET_BACKGROUND_COLOR => s.background = Some(r.array()?),
        tag::DEFINE_SHAPE | tag::DEFINE_SHAPE2 | tag::DEFINE_SHAPE3 | tag::DEFINE_SHAPE4 => {
            let id = r.u16()?;
            let bounds = Bits::at(data, 2).rect()?;
            s.characters.insert(id, Character::Shape { bounds });
        }
        tag::DEFINE_MORPH_SHAPE | tag::DEFINE_MORPH_SHAPE2 => {
            s.characters.insert(r.u16()?, Character::MorphShape);
        }
        tag::DEFINE_TEXT | tag::DEFINE_TEXT2 => {
            let id = r.u16()?;
            let bounds = Bits::at(data, 2).rect()?;
            s.characters.insert(id, Character::Text { bounds });
        }
        tag::DEFINE_BUTTON | tag::DEFINE_BUTTON2 => {
            s.characters.insert(r.u16()?, Character::Button);
        }
        tag::DEFINE_BITS
        | tag::DEFINE_BITS_JPEG2
        | tag::DEFINE_BITS_JPEG3
        | tag::DEFINE_BITS_LOSSLESS
        | tag::DEFINE_BITS_LOSSLESS2
        | 90 => {
            s.characters
                .insert(r.u16()?, Character::Bitmap { tag: code });
        }
        tag::DEFINE_SOUND => {
            s.characters.insert(r.u16()?, Character::Sound);
        }
        tag::DEFINE_FONT | tag::DEFINE_FONT4 => {
            s.characters.insert(
                r.u16()?,
                Character::Font {
                    name: String::new(),
                },
            );
        }
        tag::DEFINE_FONT2 | tag::DEFINE_FONT3 => {
            let id = r.u16()?;
            let _flags = r.u8()?;
            let _language = r.u8()?;
            let name = pstring(&mut r)?.trim_end_matches('\0').to_owned();
            s.characters.insert(id, Character::Font { name });
        }
        tag::DEFINE_FONT_NAME => {
            let id = r.u16()?;
            let name = string(r.cstr()?);
            s.font_names.push((id, name));
        }
        tag::DEFINE_EDIT_TEXT => {
            let id = r.u16()?;
            let mut bits = Bits::at(data, 2);
            let bounds = bits.rect()?;
            let mut r = Reader::at(data, bits.byte_pos());
            let f1 = r.u8()?;
            let f2 = r.u8()?;
            let mut e = EditText {
                bounds,
                html: f2 & 0x02 != 0,
                multiline: f1 & 0x20 != 0,
                use_outlines: f2 & 0x01 != 0,
                ..EditText::default()
            };
            if f1 & 0x01 != 0 {
                e.font = Some(r.u16()?);
            }
            if f2 & 0x80 != 0 {
                e.font_class = Some(string(r.cstr()?));
            }
            if f1 & 0x01 != 0 || f2 & 0x80 != 0 {
                e.height = Some(r.u16()?);
            }
            if f1 & 0x04 != 0 {
                e.color = Some(r.array()?);
            }
            if f1 & 0x02 != 0 {
                e.max_length = Some(r.u16()?);
            }
            if f2 & 0x20 != 0 {
                e.align = Some(r.u8()?);
                r.take(8)?;
            }
            e.variable = string(r.cstr()?);
            if f1 & 0x80 != 0 {
                e.initial_text = Some(string(r.cstr()?));
            }
            s.characters.insert(id, Character::EditText(e));
        }
        tag::DEFINE_SPRITE => {
            let id = r.u16()?;
            let frames = r.u16()?;
            s.characters.insert(id, Character::Sprite { frames });
            let tags = read_tags(data, 4, data.len()).map_err(|_| OutOfBounds {
                offset: 4,
                len: data.len(),
            })?;
            return Ok(Some(tags));
        }
        tag::FRAME_LABEL => {
            s.frame_labels.push((timeline, frame, string(r.cstr()?)));
        }
        tag::EXPORT_ASSETS | tag::SYMBOL_CLASS => {
            let count = r.u16()?;
            for _ in 0..count {
                let id = r.u16()?;
                let name = string(r.cstr()?);
                if code == tag::EXPORT_ASSETS {
                    s.exports.push((id, name));
                } else {
                    s.symbol_classes.push((id, name));
                }
            }
        }
        tag::IMPORT_ASSETS | tag::IMPORT_ASSETS2 => {
            let url = string(r.cstr()?);
            if code == tag::IMPORT_ASSETS2 {
                r.take(2)?;
            }
            let count = r.u16()?;
            for _ in 0..count {
                let id = r.u16()?;
                let name = string(r.cstr()?);
                s.imports.push((url.clone(), id, name));
            }
        }
        tag::PLACE_OBJECT2 | tag::PLACE_OBJECT3 => {
            s.placements.push(read_place(code, data, timeline, frame)?);
        }
        tag::DO_ACTION => read_actions(data, s)?,
        tag::DO_INIT_ACTION => read_actions(data.get(2..).unwrap_or_default(), s)?,
        tag::DO_ABC => s.has_abc = true,
        tag::GFX_EXPORTER_INFO => {
            let version = r.u16()?;
            let flags = if version >= 0x10a { r.u32()? } else { 0 };
            let format = r.u16()?;
            let prefix = pstring(&mut r)?;
            let swf = pstring(&mut r)?;
            s.exporter = Some((version, flags, format, prefix, swf));
        }
        tag::GFX_DEFINE_EXTERNAL_IMAGE | tag::GFX_DEFINE_EXTERNAL_IMAGE2 => {
            let id = if code == tag::GFX_DEFINE_EXTERNAL_IMAGE {
                u32::from(r.u16()?)
            } else {
                r.u32()?
            };
            let image = ExternalImage {
                format: r.u16()?,
                width: r.u16()?,
                height: r.u16()?,
                export_name: pstring(&mut r)?,
                // Some of the game's movies end the tag after the export
                // name.
                file_name: if r.remaining() > 0 {
                    pstring(&mut r)?
                } else {
                    String::new()
                },
            };
            s.characters
                .insert(id as u16, Character::ExternalImage(image));
        }
        tag::GFX_DEFINE_SUB_IMAGE => {
            let id = r.u16()?;
            let image = r.u16()?;
            let rect = [r.u16()?, r.u16()?, r.u16()?, r.u16()?];
            s.characters.insert(id, Character::SubImage { image, rect });
        }
        tag::GFX_FONT_TEXTURE_INFO => {
            let _texture = r.u32()?;
            let _format = r.u16()?;
            s.font_textures.push(pstring(&mut r)?);
        }
        tag::GFX_DEFINE_COMPACTED_FONT => {
            let id = r.u16()?;
            s.characters.insert(
                id,
                Character::Font {
                    name: String::new(),
                },
            );
        }
        tag::GFX_DEFINE_EXTERNAL_SOUND | tag::GFX_DEFINE_EXTERNAL_STREAM_SOUND => {
            s.characters.insert(r.u16()?, Character::Sound);
        }
        _ => {}
    }
    Ok(None)
}

fn read_place(code: u16, data: &[u8], timeline: u16, frame: u16) -> Result<Placement, OutOfBounds> {
    let mut r = Reader::at(data, 0);
    let f1 = r.u8()?;
    let f2 = if code == tag::PLACE_OBJECT3 {
        r.u8()?
    } else {
        0
    };
    let mut p = Placement {
        timeline,
        frame,
        depth: r.u16()?,
        moved: f1 & 0x01 != 0,
        ..Placement::default()
    };
    if code == tag::PLACE_OBJECT3 && (f2 & 0x08 != 0 || (f2 & 0x10 != 0 && f1 & 0x02 != 0)) {
        p.class_name = Some(string(r.cstr()?));
    }
    if f1 & 0x02 != 0 {
        p.character = Some(r.u16()?);
    }
    let mut pos = r.pos();
    if f1 & 0x04 != 0 {
        let mut bits = Bits::at(data, pos);
        p.matrix = Some(bits.matrix()?);
        pos = bits.byte_pos();
    }
    if f1 & 0x08 != 0 {
        let mut bits = Bits::at(data, pos);
        bits.cxform_with_alpha()?;
        pos = bits.byte_pos();
    }
    let mut r = Reader::at(data, pos);
    if f1 & 0x10 != 0 {
        r.u16()?;
    }
    if f1 & 0x20 != 0 {
        p.name = Some(string(r.cstr()?));
    }
    if f1 & 0x40 != 0 {
        p.clip_depth = Some(r.u16()?);
    }
    Ok(p)
}

/// Collects names from an action list: constant pools, function names and
/// parameters, and pushed strings.
fn read_actions(data: &[u8], s: &mut Summary) -> Result<(), OutOfBounds> {
    let mut r = Reader::at(data, 0);
    while r.remaining() > 0 {
        let op = r.u8()?;
        if op == 0 {
            // ActionEnd; function bodies follow their definitions inline,
            // so keep going to the end of the tag.
            continue;
        }
        if op < 0x80 {
            continue;
        }
        let len = usize::from(r.u16()?);
        let body = r.take(len)?;
        let mut a = Reader::at(body, 0);
        match op {
            // ConstantPool
            0x88 => {
                let count = a.u16()?;
                for _ in 0..count {
                    let n = string(a.cstr()?);
                    s.name(n);
                }
            }
            // DefineFunction
            0x9b => {
                s.name(string(a.cstr()?));
                let params = a.u16()?;
                for _ in 0..params {
                    s.name(string(a.cstr()?));
                }
            }
            // DefineFunction2
            0x8e => {
                s.name(string(a.cstr()?));
                let params = a.u16()?;
                a.u8()?;
                a.u16()?;
                for _ in 0..params {
                    a.u8()?;
                    s.name(string(a.cstr()?));
                }
            }
            // Push: only the string literals.
            0x96 => {
                while a.remaining() > 0 {
                    match a.u8()? {
                        0 => {
                            let n = string(a.cstr()?);
                            s.name(n);
                        }
                        1 | 7 => {
                            a.take(4)?;
                        }
                        2 | 3 => {}
                        4 | 5 | 8 => {
                            a.u8()?;
                        }
                        6 => {
                            a.take(8)?;
                        }
                        9 => {
                            a.u16()?;
                        }
                        _ => break,
                    }
                }
            }
            // GetURL: the target, often a movie to load.
            0x83 => {
                s.name(string(a.cstr()?));
                s.name(string(a.cstr()?));
            }
            // GotoLabel, SetTarget
            0x8c | 0x8b => s.name(string(a.cstr()?)),
            _ => {}
        }
    }
    Ok(())
}

/// A big-endian bit reader over SWF's packed records.
struct Bits<'a> {
    buf: &'a [u8],
    bit: usize,
}

impl<'a> Bits<'a> {
    fn at(buf: &'a [u8], byte: usize) -> Self {
        Self { buf, bit: byte * 8 }
    }

    /// The next whole byte after the bits read.
    fn byte_pos(&self) -> usize {
        self.bit.div_ceil(8)
    }

    fn ub(&mut self, n: u32) -> Result<u32, OutOfBounds> {
        let mut v = 0u32;
        for _ in 0..n {
            let byte = *self.buf.get(self.bit / 8).ok_or(OutOfBounds {
                offset: self.bit / 8,
                len: 1,
            })?;
            v = (v << 1) | u32::from((byte >> (7 - self.bit % 8)) & 1);
            self.bit += 1;
        }
        Ok(v)
    }

    fn sb(&mut self, n: u32) -> Result<i32, OutOfBounds> {
        let v = self.ub(n)?;
        Ok(if n > 0 && n < 32 && v & (1 << (n - 1)) != 0 {
            (v | !((1u32 << n) - 1)) as i32
        } else {
            v as i32
        })
    }

    /// A 16.16 fixed-point value.
    fn fb(&mut self, n: u32) -> Result<f32, OutOfBounds> {
        Ok(self.sb(n)? as f32 / 65536.0)
    }

    fn rect(&mut self) -> Result<Rect, OutOfBounds> {
        let n = self.ub(5)?;
        Ok(Rect {
            x_min: self.sb(n)?,
            x_max: self.sb(n)?,
            y_min: self.sb(n)?,
            y_max: self.sb(n)?,
        })
    }

    fn matrix(&mut self) -> Result<Matrix, OutOfBounds> {
        let mut m = Matrix::default();
        if self.ub(1)? == 1 {
            let n = self.ub(5)?;
            m.a = self.fb(n)?;
            m.d = self.fb(n)?;
        }
        if self.ub(1)? == 1 {
            let n = self.ub(5)?;
            m.b = self.fb(n)?;
            m.c = self.fb(n)?;
        }
        let n = self.ub(5)?;
        m.tx = self.sb(n)?;
        m.ty = self.sb(n)?;
        Ok(m)
    }

    fn cxform_with_alpha(&mut self) -> Result<(), OutOfBounds> {
        let add = self.ub(1)?;
        let mult = self.ub(1)?;
        let n = self.ub(4)?;
        for _ in 0..4 * (add + mult) {
            self.sb(n)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A big-endian bit writer, the inverse of [`Bits`].
    #[derive(Default)]
    struct BitWriter {
        out: Vec<u8>,
        bit: usize,
    }

    impl BitWriter {
        fn put(&mut self, n: u32, v: i64) {
            for i in (0..n).rev() {
                if self.bit.is_multiple_of(8) {
                    self.out.push(0);
                }
                if (v >> i) & 1 != 0 {
                    *self.out.last_mut().unwrap() |= 0x80 >> (self.bit % 8);
                }
                self.bit += 1;
            }
        }

        fn rect(mut self, r: [i64; 4]) -> Vec<u8> {
            self.put(5, 16);
            for v in r {
                self.put(16, v);
            }
            self.out
        }
    }

    fn tag(code: u16, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        if data.len() < 0x3f {
            out.extend_from_slice(&((code << 6) | data.len() as u16).to_le_bytes());
        } else {
            out.extend_from_slice(&((code << 6) | 0x3f).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        }
        out.extend_from_slice(data);
        out
    }

    fn cstr(s: &str) -> Vec<u8> {
        let mut v = s.as_bytes().to_vec();
        v.push(0);
        v
    }

    fn movie(signature: &[u8; 3], tags: &[Vec<u8>]) -> Vec<u8> {
        let mut body = BitWriter::default().rect([0, 1280 * 20, 0, 720 * 20]);
        body.extend_from_slice(&(30u16 << 8).to_le_bytes());
        body.extend_from_slice(&1u16.to_le_bytes());
        for t in tags {
            body.extend_from_slice(t);
        }
        body.extend_from_slice(&tag(tag::END, &[]));
        let mut out = signature.to_vec();
        out.push(8);
        out.extend_from_slice(&(body.len() as u32 + 8).to_le_bytes());
        if signature[0] == b'C' {
            out.extend_from_slice(&miniz_oxide::deflate::compress_to_vec_zlib(&body, 6));
        } else {
            out.extend_from_slice(&body);
        }
        out
    }

    fn hud_tags() -> Vec<Vec<u8>> {
        // A text field bound to a variable, with a font and initial text.
        let mut edit = 7u16.to_le_bytes().to_vec();
        edit.extend(BitWriter::default().rect([0, 2000, 0, 400]));
        edit.extend_from_slice(&[0x80 | 0x04 | 0x01, 0x00]);
        edit.extend_from_slice(&3u16.to_le_bytes());
        edit.extend_from_slice(&240u16.to_le_bytes());
        edit.extend_from_slice(&[255, 200, 0, 255]);
        edit.extend(cstr("_root.ammo"));
        edit.extend(cstr("30"));

        // A sprite placing the field as "ammoText" at (100, 50) px.
        let mut place = vec![0x02 | 0x04 | 0x20];
        place.extend_from_slice(&1u16.to_le_bytes());
        place.extend_from_slice(&7u16.to_le_bytes());
        let mut m = BitWriter::default();
        m.put(1, 0);
        m.put(1, 0);
        m.put(5, 16);
        m.put(16, 2000);
        m.put(16, 1000);
        place.extend(m.out);
        place.extend(cstr("ammoText"));
        let mut sprite = 9u16.to_le_bytes().to_vec();
        sprite.extend_from_slice(&1u16.to_le_bytes());
        sprite.extend(tag(tag::FRAME_LABEL, &cstr("show")));
        sprite.extend(tag(tag::PLACE_OBJECT2, &place));
        sprite.extend(tag(tag::SHOW_FRAME, &[]));
        sprite.extend(tag(tag::END, &[]));

        let mut export = 1u16.to_le_bytes().to_vec();
        export.extend_from_slice(&9u16.to_le_bytes());
        export.extend(cstr("WeaponBox"));

        let mut image = 12u16.to_le_bytes().to_vec();
        image.extend_from_slice(&1u16.to_le_bytes());
        image.extend_from_slice(&64u16.to_le_bytes());
        image.extend_from_slice(&32u16.to_le_bytes());
        image.extend_from_slice(&[4]);
        image.extend_from_slice(b"icon");
        image.extend_from_slice(&[8]);
        image.extend_from_slice(b"icon.dds");

        let mut pool = 2u16.to_le_bytes().to_vec();
        pool.extend(cstr("setHealth"));
        pool.extend(cstr("gauge"));
        let mut action = vec![0x88];
        action.extend_from_slice(&(pool.len() as u16).to_le_bytes());
        action.extend(pool);
        let mut push = vec![0];
        push.extend(cstr("onLoad"));
        action.push(0x96);
        action.extend_from_slice(&(push.len() as u16).to_le_bytes());
        action.extend(push);
        action.push(0);

        vec![
            tag(tag::SET_BACKGROUND_COLOR, &[1, 2, 3]),
            tag(tag::DEFINE_EDIT_TEXT, &edit),
            tag(tag::DEFINE_SPRITE, &sprite),
            tag(tag::EXPORT_ASSETS, &export),
            tag(tag::GFX_DEFINE_EXTERNAL_IMAGE, &image),
            tag(tag::DO_ACTION, &action),
            tag(tag::SHOW_FRAME, &[]),
        ]
    }

    fn check(m: &Movie) {
        assert_eq!(m.version, 8);
        assert_eq!(m.frame_size.size_px(), (1280.0, 720.0));
        assert_eq!(m.frame_rate, 30.0);
        assert_eq!(m.frame_count, 1);
        let s = m.summary();
        assert!(s.errors.is_empty(), "{:?}", s.errors);
        assert_eq!(s.background, Some([1, 2, 3]));
        let Some(Character::EditText(e)) = s.characters.get(&7) else {
            panic!("no text field: {:?}", s.characters);
        };
        assert_eq!(e.variable, "_root.ammo");
        assert_eq!(e.initial_text.as_deref(), Some("30"));
        assert_eq!(e.font, Some(3));
        assert_eq!(e.height, Some(240));
        assert_eq!(e.color, Some([255, 200, 0, 255]));
        assert_eq!(s.characters.get(&9), Some(&Character::Sprite { frames: 1 }));
        assert_eq!(s.frame_labels, vec![(9, 0, "show".to_owned())]);
        let p = s.named_instances().next().unwrap();
        assert_eq!(p.timeline, 9);
        assert_eq!(p.character, Some(7));
        assert_eq!(p.name.as_deref(), Some("ammoText"));
        let mx = p.matrix.unwrap();
        assert_eq!((mx.tx, mx.ty, mx.a, mx.d), (2000, 1000, 1.0, 1.0));
        assert_eq!(s.exports, vec![(9, "WeaponBox".to_owned())]);
        let Some(Character::ExternalImage(i)) = s.characters.get(&12) else {
            panic!("no external image");
        };
        assert_eq!((i.width, i.height), (64, 32));
        assert_eq!(i.file_name, "icon.dds");
        assert_eq!(s.action_names, ["setHealth", "gauge", "onLoad"]);
        assert_eq!(s.tag_counts[&tag::SHOW_FRAME], 2);
    }

    #[test]
    fn parses_stored_movie() {
        let bytes = movie(b"GFX", &hud_tags());
        assert!(looks_like_movie(&bytes));
        let m = Movie::parse(&bytes).unwrap();
        assert!(m.is_gfx());
        check(&m);
    }

    #[test]
    fn parses_compressed_movie() {
        let bytes = movie(b"CFX", &hud_tags());
        let m = Movie::parse(&bytes).unwrap();
        assert!(m.is_gfx());
        check(&m);
        let plain = Movie::parse(&movie(b"CWS", &hud_tags())).unwrap();
        assert!(!plain.is_gfx());
        check(&plain);
    }

    #[test]
    fn compressed_movie_drops_bytes_past_its_length() {
        let bytes = movie(b"CFX", &hud_tags());
        let declared = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        let mut body = miniz_oxide::inflate::decompress_to_vec_zlib(&bytes[8..]).unwrap();
        body.resize(body.len().next_multiple_of(4096), 0xcd);
        let mut padded = bytes[..8].to_vec();
        padded.extend(miniz_oxide::deflate::compress_to_vec_zlib(&body, 6));
        let m = Movie::parse(&padded).unwrap();
        assert_eq!(m.body.len(), declared);
        check(&m);
    }

    #[test]
    fn stored_movie_ignores_trailing_padding() {
        let mut bytes = movie(b"FWS", &hud_tags());
        bytes.extend_from_slice(&[0; 13]);
        check(&Movie::parse(&bytes).unwrap());
    }

    #[test]
    fn stored_movie_may_declare_its_length_without_the_header() {
        let mut bytes = movie(b"GFX", &hud_tags());
        let declared = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) - 8;
        bytes[4..8].copy_from_slice(&declared.to_le_bytes());
        bytes.extend_from_slice(&[0; 5]);
        let m = Movie::parse(&bytes).unwrap();
        assert_eq!(m.body.len(), declared as usize + 8);
        check(&m);
    }

    #[test]
    fn external_image_may_have_no_file_name() {
        let mut image = vec![5, 0, 13, 0, 0, 2, 128, 0, 4];
        image.extend_from_slice(b"card");
        let m = Movie::parse(&movie(
            b"GFX",
            &[tag(tag::GFX_DEFINE_EXTERNAL_IMAGE, &image)],
        ))
        .unwrap();
        let s = m.summary();
        assert!(s.errors.is_empty());
        let Some(Character::ExternalImage(i)) = s.characters.get(&5) else {
            panic!("no external image");
        };
        assert_eq!((i.export_name.as_str(), i.file_name.as_str()), ("card", ""));
        assert_eq!((i.width, i.height), (512, 128));
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(
            Movie::parse(b"ABC\x08\x10\0\0\0"),
            Err(GfxError::Signature(*b"ABC"))
        );
        let mut bytes = movie(b"GFX", &hud_tags());
        bytes.truncate(40);
        assert!(matches!(Movie::parse(&bytes), Err(GfxError::Length { .. })));
        assert!(!looks_like_movie(b"GFX\x08"));
    }
}
