//! What the launcher draws with, read from the player's install on a
//! thread of its own: the game's Scaleform texture fonts and a few of its
//! full-screen pictures. Nothing from the install is kept on disk or in
//! the repository; everything here is read at run time.
//!
//! # Provenance
//!
//! - From the game's data: the font movies (`_bold_font`, Gunplay, the
//!   game's heading and HUD face; `_shell_normal_font` and
//!   `_shell_bold_font`, Franklin Gothic Medium, the front end's body
//!   face; `fonts_enext` in `Loading.wad`, Gunplay again, if the shell's
//!   are missing) and the pictures in [`ART`], each by its asset name hash.
//!   Which picture is which was found by listing the install's textures
//!   by size and name (2048 x 1024 and up, or menu and briefing names).
//! - Invented: which pictures the banner shows and in what order, and
//!   the skull badge's use as the game's icon in the sidebar.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use m2_assets::flash::text::Font;
use m2_assets::flash::{self};
use m2_assets::gfx::{MOVIE_TYPE, movie_from_asset};
use m2_assets::name_hash;
use m2_engine::AssetSource;

/// The archives looked in, in order (the patch's before the world's).
pub const ARCHIVES: [&str; 4] = ["shell.wad", "Loading.wad", "vz-patch.wad", "vz.wad"];

/// What a font is used for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    /// Titles, tabs and the Play button: Gunplay.
    Heading,
    /// Everything else: Franklin Gothic Medium.
    Body,
    /// Names and numbers that stand out in body text.
    BodyBold,
}

/// The font movies for each role, the first found used.
pub const FONT_MOVIES: [(Role, &[&str]); 3] = [
    (
        Role::Heading,
        &["_bold_font", "_normal_font", "fonts_enext"],
    ),
    (Role::Body, &["_shell_normal_font", "_normal_font"]),
    (
        Role::BodyBold,
        &["_shell_bold_font", "_shell_normal_font", "_bold_font"],
    ),
];

/// A picture the launcher shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Art {
    /// The texture's name, as the install's index names it.
    pub name: &'static str,
    /// [`name_hash`] of it (checked by a test).
    pub hash: u32,
    /// Where in the game it comes from, for the banner's caption.
    pub caption: &'static str,
}

/// The banner's pictures, in the order it shows them.
pub const BANNER: [Art; 8] = [
    Art {
        name: "shell_mainmenu_dollarbill",
        hash: 0xff00_0cc4,
        caption: "Main menu",
    },
    Art {
        name: "lti_precache1",
        hash: 0x7329_d083,
        caption: "Loading",
    },
    Art {
        name: "gurcon001_briefing_i1f",
        hash: 0x9750_99d6,
        caption: "Guerrilla contract briefing",
    },
    Art {
        name: "oilcon001_background",
        hash: 0x751f_4e35,
        caption: "Universal Petroleum contract",
    },
    Art {
        name: "poi_anch_background",
        hash: 0x0b92_5cdd,
        caption: "Point of interest",
    },
    Art {
        name: "poi_gr_background",
        hash: 0x33c5_ee88,
        caption: "Point of interest",
    },
    Art {
        name: "poi_pr_background",
        hash: 0xed21_6113,
        caption: "Point of interest",
    },
    Art {
        name: "poi_up_background",
        hash: 0x755b_e510,
        caption: "Point of interest",
    },
];

/// The front end's character select picture (the Saves tab's new game).
pub const CHARACTERS: Art = Art {
    name: "shell_newgame_characters",
    hash: 0xe4b1_b90b,
    caption: "New game",
};

/// Mattias's head, which the main menu shows in the dollar bill's oval
/// (the bill's middle is see-through).
pub const BILL_HEAD: Art = Art {
    name: "shell_mainmenu_mattias_head",
    hash: 0xb07a_1459,
    caption: "",
};

/// The loading screen's skull, the sidebar's game icon.
pub const SKULL: Art = Art {
    name: "global_loading_skull",
    hash: 0x443f_889b,
    caption: "",
};

/// Pixels, RGBA8, top row first.
#[derive(Clone, Debug)]
pub struct Rgba {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

/// What came back from the install.
#[derive(Debug, Default)]
pub struct Loaded {
    /// Each role's font with its glyph page.
    pub fonts: Vec<(Role, Font, Rgba)>,
    /// Pictures by their [`Art::hash`].
    pub art: Vec<(u32, Rgba)>,
    /// The scripts' achievement list ([`crate::achievements::read_list`]),
    /// multiplayer ones included.
    pub achievements: Vec<crate::achievements::Entry>,
    /// What was found and what wasn't, for the log.
    pub log: Vec<String>,
}

type Source = AssetSource<BufReader<File>>;

/// Reads the fonts and pictures from the install's `data` folder.
pub fn load(data: &Path) -> Loaded {
    let mut out = Loaded::default();
    let mut sources: Vec<(&str, Source)> = Vec::new();
    for name in ARCHIVES {
        let path = data.join(name);
        match File::open(&path)
            .map_err(|e| e.to_string())
            .and_then(|f| AssetSource::new(BufReader::new(f)).map_err(|e| e.to_string()))
        {
            Ok(s) => sources.push((name, s)),
            Err(e) => out.log.push(format!("{}: {e}", path.display())),
        }
    }

    for (role, movies) in FONT_MOVIES {
        match movies.iter().find_map(|m| font(&mut sources, m)) {
            Some((movie, font, page)) => {
                out.log.push(format!(
                    "font {role:?}: {} from {movie} ({} glyphs, page {}x{})",
                    font.name,
                    font.codes.len(),
                    page.width,
                    page.height
                ));
                out.fonts.push((role, font, page));
            }
            None => out
                .log
                .push(format!("font {role:?}: none of {movies:?} found")),
        }
    }

    let script = sources.iter_mut().find_map(|(_, s)| {
        s.asset(crate::achievements::SCRIPT, m2_assets::lua::SCRIPT_TYPE)
            .ok()
    });
    match script.map(|a| m2_assets::lua::Chunk::from_asset(&a)) {
        Some(Ok(chunk)) => {
            out.achievements = crate::achievements::read_list(&chunk);
            out.log
                .push(format!("achievements: {} listed", out.achievements.len()));
        }
        Some(Err(e)) => out.log.push(format!("achievements: {e}")),
        None => out.log.push("achievements: script not found".to_owned()),
    }

    for art in BANNER.iter().chain([&CHARACTERS, &BILL_HEAD, &SKULL]) {
        match texture(&mut sources, art.hash) {
            Some(rgba) => {
                out.log.push(format!(
                    "art {} ({:08x}): {}x{}",
                    art.name, art.hash, rgba.width, rgba.height
                ));
                out.art.push((art.hash, rgba));
            }
            None => out
                .log
                .push(format!("art {} ({:08x}): not found", art.name, art.hash)),
        }
    }
    out
}

/// The font a font movie defines, with its glyph page: the one with the
/// most glyphs drawn into a texture.
fn font(sources: &mut [(&str, Source)], movie: &str) -> Option<(String, Font, Rgba)> {
    let hash = name_hash(movie);
    let parsed = sources.iter_mut().find_map(|(_, s)| {
        let data = s.asset(hash, MOVIE_TYPE).ok()?;
        movie_from_asset(&data).ok()
    })?;
    let mut book = flash::FontBook::default();
    book.add_movie(&parsed);
    let font = book
        .fonts
        .into_values()
        .filter(|f| f.texture.is_some())
        .max_by_key(|f| f.codes.len())?;
    let page_name = font.texture.as_ref()?.0.clone();
    let page = texture(sources, name_hash(&page_name))?;
    Some((movie.to_owned(), font, page))
}

/// Texture `hash` from the first archive holding it, at most 2048 wide.
fn texture(sources: &mut [(&str, Source)], hash: u32) -> Option<Rgba> {
    sources.iter_mut().find_map(|(_, s)| {
        let t = s.texture(hash, 2048).ok()?;
        let pixels = t.mips.into_iter().next()?;
        Some(Rgba {
            width: t.width as usize,
            height: t.height as usize,
            pixels,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_pictures_hash_is_its_names() {
        for art in BANNER.iter().chain([&CHARACTERS, &BILL_HEAD, &SKULL]) {
            assert_eq!(name_hash(art.name), art.hash, "{}", art.name);
        }
    }

    #[test]
    fn each_role_has_a_font_to_fall_back_on() {
        for (_, movies) in FONT_MOVIES {
            assert!(movies.len() >= 2);
        }
    }
}
