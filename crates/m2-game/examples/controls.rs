//! Counts the control words (`[reload]`, `[pfire]`...) in your
//! installation's English strings, and how many of their uses draw a
//! picture: the PC font's ([`gui_runtime::button_glyph`]) and the Xbox
//! pad's ([`gui_runtime::pad_picture`]).
//!
//! ```text
//! cargo run -p m2-game --example controls -- <vz.wad>
//! ```
//!
//! Prints counts and words only, no text from the game.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::process::ExitCode;

use m2_assets::strings::STRINGS_TYPE;
use m2_assets::{BlockContents, StringTable, WadArchive, name_hash};
use m2_game::gui_runtime::{button_glyph, pad_picture};
use m2_game::mission::LANGUAGE;

fn main() -> ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: controls <vz.wad>");
        return ExitCode::FAILURE;
    };
    match run(&path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// A bracketed word that is markup rather than a control: ids
/// (`[SHELL.Common.Money]`, `[0x8c4d842e]`), arguments, colours, new lines,
/// objective icons, numbers.
fn markup(word: &str) -> bool {
    let w = word.to_ascii_lowercase();
    w.contains('.')
        || w.contains(':')
        || w.starts_with("0x")
        || w.is_empty()
        || w.chars().all(|c| c.is_ascii_digit())
        || w == "n"
        || w.starts_with("obj")
        || w.ends_with("objt")
        || [
            "white", "green", "red", "yellow", "orange", "blue", "grey", "gray", "black",
        ]
        .contains(&w.as_str())
}

fn run(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut wad = WadArchive::open(BufReader::new(File::open(path)?))?;
    let language = name_hash(LANGUAGE);
    let mut table = None;
    for block in 0..wad.index().blocks().len() {
        let Ok(decoded) = wad.read_block(block) else {
            continue;
        };
        let Ok(contents) = BlockContents::parse(&decoded) else {
            continue;
        };
        for (entry, data) in contents.iter() {
            if entry.type_hash == STRINGS_TYPE && entry.name_hash == language {
                table = Some(StringTable::parse(data)?);
            }
        }
    }
    let table = table.ok_or("no English strings")?;
    // Uses of each word, lower case.
    let mut uses: BTreeMap<String, usize> = BTreeMap::new();
    for text in table.strings.values() {
        let mut rest = text.as_str();
        while let Some(open) = rest.find('[') {
            rest = &rest[open + 1..];
            let Some(close) = rest.find(']') else { break };
            let word = &rest[..close];
            if !word.contains('[') && !markup(word) {
                *uses.entry(word.to_ascii_lowercase()).or_default() += 1;
            }
            rest = &rest[close + 1..];
        }
    }
    let count = |f: &dyn Fn(&str) -> bool| -> usize {
        uses.iter().filter(|(w, _)| f(w)).map(|(_, n)| n).sum()
    };
    let total: usize = uses.values().sum();
    let pc = count(&|w| button_glyph(w).is_some());
    let pad = count(&|w| pad_picture(w).is_some());
    println!(
        "{} bracketed words, {total} uses: PC picture {pc}, pad picture {pad}",
        uses.len()
    );
    for (w, n) in &uses {
        println!(
            "  {w:>14} x{n:<3} pc {}  pad {}",
            button_glyph(w).map_or("-".into(), |g| format!("U+{:04X}", u32::from(g))),
            pad_picture(w).unwrap_or("-")
        );
    }
    Ok(())
}
