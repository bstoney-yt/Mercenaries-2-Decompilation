//! What the island's destructible models send off as their part states
//! run ([`m2_engine::node_debris`]), against breaking up all at once.
//!
//! For every placed model whose states send parts off (`CreateObject`
//! naming `DebrisTemplate`): its breakup's chunks, whether each lies in a
//! part sent off, and whether each part sent off has a `PHY2` body of its
//! own (what the port sends it off as). Then, for the models named after
//! the install (model hashes, hex; a building, a wall, a fence, a palm and
//! a plant by default), a timeline: its part machines take `DestroyMsg`
//! and the fire-destroy message in turn and run 60 s, and each part sent
//! off is printed with when it left; a part sent twice, or a part never
//! sent, is counted.
//!
//! ```text
//! cargo run --release -p m2-engine --example sentoff -- "<install dir>" [model hash]...
//! ```
//!
//! Prints counts, names, hashes and times only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_engine::Install;
use m2_engine::node_debris::NodePieces;
use m2_engine::part_runner::Runner;
use m2_engine::world::WorldIndex;

/// `_docks_bld_warehouse`, `_commercial_walllong`, `_shanty_fencewood01`,
/// `_global_env_palmtree01`, `_jungle_env_plantlarge01`.
const KINDS: [u32; 5] = [
    0xfb84_e548,
    0xf66e_4a3c,
    0xf7dc_e388,
    0x799c_0ca2,
    0x0a36_2710,
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        return Err("usage: sentoff <install dir> [model hash]...".into());
    };
    let asked: Vec<u32> = args[1..]
        .iter()
        .filter_map(|a| u32::from_str_radix(a.trim_start_matches("0x"), 16).ok())
        .collect();
    let kinds: &[u32] = if asked.is_empty() { &KINDS } else { &asked };
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let world = WorldIndex::build(&mut source)?;
    let db = &world.template_db;
    let mut by_model: BTreeMap<u32, (u32, usize)> = BTreeMap::new();
    for o in &world.objects {
        by_model.entry(o.model).or_insert((o.template, 0)).1 += 1;
    }
    let (mut models, mut driven, mut placed_driven) = (0, 0, 0);
    for (&model, &(template, n)) in &by_model {
        let Ok(data) = source.model(model) else {
            continue;
        };
        let Ok(parsed) = data.parse() else { continue };
        let Some(pieces) = NodePieces::of(&parsed) else {
            continue;
        };
        models += 1;
        let Some(states) = parsed.states.as_ref() else {
            continue;
        };
        let chunks = states.chunks(&parsed.nodes);
        let in_part = |c: u16| {
            pieces
                .pieces
                .iter()
                .any(|p| p.nodes.get(usize::from(c)).copied().unwrap_or(false))
        };
        let ucfx = m2_assets::Ucfx::parse(&data.base).ok();
        let phy = ucfx
            .as_ref()
            .and_then(|u| u.top_level(*b"PHY2"))
            .and_then(|d| m2_assets::physics::Physics::parse(d).ok());
        let body_on = |p: &m2_engine::node_debris::NodePiece| {
            phy.as_ref().is_some_and(|phy| {
                phy.parts.iter().any(|q| {
                    p.nodes.get(q.node as usize).copied().unwrap_or(false)
                        && q.body.is_some_and(|b| phy.bodies[b].mass > 0.0)
                })
            })
        };
        let covered = chunks.iter().all(|&c| in_part(c));
        let bodied = pieces.pieces.iter().all(body_on);
        if covered && bodied {
            driven += 1;
            placed_driven += n;
        }
        if !kinds.contains(&model) {
            continue;
        }
        let name = db.names.get(&template).map_or("?", String::as_str);
        println!(
            "{model:08x} {name}: {n} placed; before: breaks into {} chunks at once; {} parts \
             sent off, chunks all in them {covered}, each with a PHY2 body {bodied}",
            chunks.len(),
            pieces.pieces.len()
        );
        let names: Vec<u32> = parsed.nodes.iter().map(|x| x.name_hash).collect();
        for (how, message) in [
            ("DestroyMsg", m2_assets::model::DESTROY_MSG),
            ("fire-destroy", m2_engine::destroy_route::FIRE_DESTROY_MSG),
        ] {
            let mut runner = Runner::new(
                states,
                |k| names.get(usize::from(k)).copied().unwrap_or(0),
                model,
            );
            runner.deliver(message);
            let mut sent: Vec<(f32, u32)> = Vec::new();
            let mut t = 0.0;
            loop {
                for c in runner.take_changes() {
                    if c.sent_off {
                        sent.push((t, c.machine));
                    }
                }
                if runner.settled() || t > 60.0 {
                    break;
                }
                runner.advance(0.1);
                t += 0.1;
            }
            let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
            for &(_, m) in &sent {
                *counts.entry(m).or_default() += 1;
            }
            let twice = counts.values().filter(|&&c| c > 1).count();
            let never = pieces
                .pieces
                .iter()
                .filter(|p| !counts.contains_key(&p.name))
                .count();
            let times: Vec<String> = sent
                .iter()
                .map(|(t, m)| format!("{m:08x}@{t:.1}s"))
                .collect();
            println!(
                "    after, {how}: {} parts sent off ({twice} twice, {never} never): {}",
                sent.len(),
                times.join(" ")
            );
        }
    }
    println!(
        "models sending parts off: {models}; breaking up as their states send them (every chunk \
         in a part, every part with a PHY2 body): {driven} ({placed_driven} placed)"
    );
    Ok(())
}
