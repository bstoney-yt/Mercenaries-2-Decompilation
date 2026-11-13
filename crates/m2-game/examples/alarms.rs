//! Runs the alarm towers of your own installation headless: how many
//! entities have the `Alarm` component and run its script, their layers
//! loaded; then at each tower in turn the player stands beside it and
//! takes its prompt (`Junk.ToggleAlarm`, as `[ContextAction.UseAlarm]`
//! does), and what the alarm did is counted: whether the script raised it
//! (its lamp, `Vehicle.SetParts`, is not seen here; the spawners it turns
//! on are, as `TweakSpawners` commands) and the scripts' errors.
//!
//! ```text
//! cargo run -p m2-game --example alarms -- <vz.wad> [--layers n]
//! ```
//!
//! Only the first `n` (12) layers with towers are loaded, other than the
//! static ones. Prints names, counts and the scripts' errors only.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::BufReader;

use m2_assets::WadArchive;
use m2_game::api::Command;
use m2_game::mission::{ALARM, GameData, Mission};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let wad = args.first().ok_or("usage: alarms <vz.wad>")?;
    let mut archive = WadArchive::open(BufReader::new(File::open(wad)?))?;
    let data = GameData::read(&mut archive)?;
    let mut towers: Vec<(String, u32, String, [f32; 3])> = Vec::new();
    let mut scripted = 0;
    for (layer, entities) in &data.layers {
        for e in entities.iter().filter(|e| e.alarm) {
            scripted += usize::from(e.script == Some(ALARM));
            towers.push((layer.clone(), e.guid, e.name.clone(), e.at));
        }
    }
    let layers: BTreeSet<String> = towers.iter().map(|t| t.0.clone()).collect();
    println!(
        "{} entities have an alarm, {scripted} run Alarm, in {} layers",
        towers.len(),
        layers.len()
    );
    let fixed = towers
        .iter()
        .filter(|t| m2_game::layers::LayerState::is_static(&t.0))
        .count();
    println!(
        "{fixed} of them in the static layers, which the port places without running their scripts"
    );
    // A sample: every layer at once runs out of Lua references.
    let sample: usize = args
        .iter()
        .position(|a| a == "--layers")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| n.parse().ok())
        .unwrap_or(12);
    let layers: BTreeSet<String> = layers
        .into_iter()
        .filter(|l| !m2_game::layers::LayerState::is_static(l))
        .take(sample)
        .collect();
    towers.retain(|t| layers.contains(&t.0));
    println!("loading {} layers, {} towers", layers.len(), towers.len());
    let mut mission = Mission::new(data, "Mattias", [0.0, 0.0, 0.0])?;
    mission.set_hero("Mattias", "mattias", 120.0);
    mission.start();
    for layer in &layers {
        mission.preload(layer);
    }
    let mut errors = 0;
    let mut run = |m: &mut Mission, seconds: f64| -> Vec<Command> {
        let mut out = Vec::new();
        let end = m.now() + seconds;
        while m.now() < end {
            m.tick(0.25);
            out.extend(m.take_commands());
            for line in m.take_log() {
                let t = line.trim_start();
                let print = t.starts_with("print ") || t.starts_with("command ");
                if !print && (line.contains("Alarm") || line.contains("alarm")) {
                    errors += 1;
                    if errors <= 10 {
                        println!("{line}");
                    }
                }
            }
        }
        out
    };
    run(&mut mission, 3.0);
    let (mut on, mut tweaked, mut prompts) = (0, 0, 0);
    for (_, guid, _, at) in &towers {
        mission.put_player([at[0] + 3.0, at[1], at[2]]);
        run(&mut mission, 1.0);
        prompts += usize::from(mission.world().borrow().context_actions.contains_key(guid));
        let toggle: mlua::Function = mission.lua().load("Junk.ToggleAlarm").eval()?;
        toggle.call::<()>(m2_game::events::guid_value(*guid))?;
        let after = run(&mut mission, 2.0);
        on += usize::from(mission.world().borrow().alarms.on.get(guid) == Some(&true));
        tweaked += usize::from(
            after
                .iter()
                .any(|c| matches!(c, Command::TweakSpawners { .. })),
        );
    }
    println!("towers\twith a prompt\tflag on after the prompt\tspawners turned on near it");
    println!("{}\t{prompts}\t{on}\t{tweaked}", towers.len());
    println!("alarm errors logged: {errors}");
    Ok(())
}
