//! Prints the words that follow a cue's name hash in a cue table: a
//! survey of the mixed cues' headers (their loop points), against what
//! the 360's sound engine reads there (`0x82836FA0`).
//!
//! ```text
//! cargo run --release -p m2-engine --example mixhead -- "<install dir>" <table name> <cue name>...
//! ```
//!
//! Prints offsets, words and their values as floats only.

use std::path::Path;

use m2_assets::name_hash;
use m2_engine::Install;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [root, table, cues @ ..] = &args[..] else {
        return Err("usage: mixhead <install dir> <table name> <cue name>...".into());
    };
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let data = source.asset(name_hash(table), m2_assets::voice::CUE_TABLE_TYPE)?;
    let word = |at: usize| {
        data.get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let list = word(0x18).unwrap_or(0) as usize;
    for cue in cues {
        let hash = name_hash(cue);
        let Some(at) = (list..data.len().saturating_sub(4))
            .step_by(4)
            .find(|&at| word(at) == Some(hash))
        else {
            println!("{cue} ({hash:08x}): not in the table's cue list");
            continue;
        };
        println!("{cue} ({hash:08x}) at {at:#x}:");
        for k in 0..40 {
            let Some(w) = word(at + 4 * k) else { break };
            println!("  +{:#04x}  {w:08x}  {}", 4 * k, f32::from_bits(w));
        }
    }
    Ok(())
}
