//! Tallies the messages the island's models' part states send as they
//! are entered (`SendMessage`, [`m2_engine::StateMessage`]), by state,
//! message and second operand, with how many models send each, and what
//! the sound system does with each message
//! ([`m2_engine::spawn_sound::object_event`]).
//!
//! ```text
//! cargo run --release -p m2-engine --example sendmessages -- "<install dir>"
//! ```
//!
//! Prints hashes and counts only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_assets::model::MODEL_TYPE;
use m2_engine::{Install, StateMessage};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: sendmessages <install dir>")?;
    let install = Install::find(Path::new(&root))?;
    let mut source = install.open_world()?;
    let mut models: Vec<u32> = {
        let index = source.archive().index();
        index
            .assets()
            .iter()
            .filter(|a| index.type_hash(a) == MODEL_TYPE)
            .map(|a| a.name_hash)
            .collect()
    };
    models.sort_unstable();
    models.dedup();
    let mut sent: BTreeMap<(u32, u32, u32), usize> = BTreeMap::new();
    for &m in &models {
        let Ok(d) = source.model(m) else { continue };
        if std::env::var_os("SENDMSG_DEBUG").is_some() {
            debug(m, &d.base);
        }
        let Ok(model) = d.parse() else { continue };
        let mut seen = std::collections::HashSet::new();
        for s in StateMessage::of(&model) {
            if seen.insert((s.state, s.message, s.param)) {
                *sent.entry((s.state, s.message, s.param)).or_default() += 1;
            }
        }
    }
    println!("models\tstate\tmessage\tparam\tsound events");
    for ((state, message, param), n) in &sent {
        let events: Vec<String> = m2_engine::spawn_sound::object_event(*message)
            .iter()
            .map(|(start, e)| format!("{} {e:08x}", if *start { "start" } else { "stop" }))
            .collect();
        println!(
            "{n}\t{state:08x}\t{message:08x}\t{param:08x}\t{}",
            events.join(", ")
        );
    }
    Ok(())
}

/// Where in the model's sections each `SendMessage` command sits: the
/// fourccs leading to it and the last `CHDR`'s kind (debugging aid).
fn debug(model: u32, base: &[u8]) {
    let Ok(u) = m2_assets::ucfx::Ucfx::parse(base) else {
        return;
    };
    let mut chdr = 0;
    let mut stat = 0;
    for (i, s) in u.sections().iter().enumerate() {
        let Some(data) = s.data else { continue };
        let w: Vec<u32> = data
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        match &s.fourcc {
            b"CHDR" => chdr = w.first().copied().unwrap_or(0),
            b"STAT" => stat = w.first().copied().unwrap_or(0),
            _ => {}
        }
        if w.windows(2).any(|p| p == [2, 0xba20_f82c]) {
            println!(
                "debug {model:08x} section {i} {} after STAT {stat:08x} CHDR {chdr:08x}: {:08x?}",
                String::from_utf8_lossy(&s.fourcc),
                &w[..w.len().min(16)]
            );
        }
    }
}
