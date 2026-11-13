//! Lists what each hero plays for the actions the 360 code asks for by
//! name: the stance and action table's handle for each, then the on-foot
//! table's clip for the hero and the class of weapon held, with the
//! clip's length and events. For checking the heroes' clips against the
//! tables without running the game.
//!
//! ```text
//! cargo run --release -p m2-engine --example heroactions -- "<install dir>"
//! ```
//!
//! Prints names, hashes and numbers only.

use std::collections::HashMap;
use std::path::Path;

use m2_assets::animation::ANIMATION_TYPE;
use m2_assets::animation_table::{self as table, AnimationTable};
use m2_assets::{Clip, name_hash};
use m2_engine::Install;
use m2_engine::hero_action::{
    CARRY_GAITS, CROUCH_FIDGET, CROUCH_GAITS, SLOT_ACTIONS, SWIM_GAITS, ThrowWay, throw_action,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: heroactions <install dir>")?;
    let install = Install::find(Path::new(&root))?;
    let mut source = install.open_world()?;
    let actions =
        AnimationTable::parse(&source.asset(table::ACTIONS, table::ANIMATION_TABLE_TYPE)?)?;
    let foot = AnimationTable::parse(
        &source.asset(table::CHARACTER_ANIMATIONS, table::ANIMATION_TABLE_TYPE)?,
    )?;
    // Every clip the on-foot table lists that the archive holds.
    let list = foot.column(table::ANIMATION).ok_or("no Animation column")?;
    let mut clips: HashMap<u32, Option<Clip>> = HashMap::new();
    for row in 1..foot.rows() {
        for &c in foot.list(row, list) {
            clips.entry(c).or_insert_with(|| {
                source
                    .asset(c, ANIMATION_TYPE)
                    .ok()
                    .and_then(|a| Clip::parse(&a).ok())
            });
        }
    }
    let fits = |c: u32| clips.get(&c).is_some_and(Option::is_some);
    let describe = |p: Option<table::CharacterClip>| match p {
        None => "none".to_owned(),
        Some(p) => {
            let c = clips[&p.clip].as_ref().expect("fits");
            let events: Vec<String> = c
                .events
                .iter()
                .map(|e| format!("{}@{:.2}", e.name, e.time))
                .collect();
            format!(
                "{:08x} row {} {:?} {:.2}s [{}]",
                p.clip,
                p.row,
                p.by,
                c.header.duration,
                events.join(" ")
            )
        }
    };
    let hero = name_hash("Hero");
    println!("who\tclass\tstance\taction\thandle\tclip");
    for who in ["mattias", "Jennifer", "Chris"] {
        for class in ["rifle", "pistol", "rocket"] {
            let key = (hero, name_hash(who));
            let class_hash = name_hash(class);
            let mut named: Vec<(&str, &str)> =
                SLOT_ACTIONS.iter().map(|a| (a.stance, a.action)).collect();
            named.extend([
                ("Upright", "JumpTakeoffMoving"),
                ("Upright", "JumpTakeoffStationary"),
                ("Upright", "Jump"),
                ("InAir", "Jump"),
                ("Upright", "Die"),
            ]);
            for (stance, action) in named {
                let h = table::action_handle(&actions, stance, action);
                let p = h.and_then(|h| table::handle_clip(&foot, h, key, class_hash, fits));
                println!(
                    "{who}\t{class}\t{stance}\t{action}\t{}\t{}",
                    h.map_or("none".to_owned(), |h| format!("{h:08x}")),
                    describe(p)
                );
            }
            // The locomotion nodes' gait handles, by stance.
            let crouch = CROUCH_GAITS.map(name_hash);
            let nodes: [(&str, &[u32]); 4] = [
                ("Swim", &SWIM_GAITS),
                ("Carrying", &CARRY_GAITS),
                ("Crouched", &crouch),
                ("Crouched fidget", &[name_hash(CROUCH_FIDGET)]),
            ];
            for (stance, handles) in nodes {
                for (i, &h) in handles.iter().enumerate() {
                    let p = table::handle_clip(&foot, h, key, class_hash, fits);
                    println!("{who}	{class}	{stance}	gait {i}	{h:08x}	{}", describe(p));
                }
            }
            for moving in [false, true] {
                for way in ThrowWay::ALL {
                    let action = throw_action(moving);
                    let h = table::action_handle_toward(&actions, "Upright", action, way.hash());
                    let p = h.and_then(|h| {
                        table::handle_clip_using(&foot, h, key, class_hash, table::GRENADE, fits)
                    });
                    println!(
                        "{who}\t{class}\tUpright\t{action} {}\t{}\t{}",
                        way.name(),
                        h.map_or("none".to_owned(), |h| format!("{h:08x}")),
                        describe(p)
                    );
                }
            }
        }
    }
    Ok(())
}
