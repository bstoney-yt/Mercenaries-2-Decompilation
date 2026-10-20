//! What the scripts put on the player's screen: the mission's objective
//! list, objectives on the map and radar, markers over targets in the
//! world, and "objective added / completed" messages.
//!
//! The game draws these with its own (Flash) interface; scripts reach it
//! through `Net.SendEvent_*` calls (sent to every player in co-op) and
//! `Marker`. Here they fill a [`Hud`] the game draws its own way. Texts are
//! ids such as `[VzaCon001.Objectives.001]` into the language's string
//! table; [`render`] puts the table's text in, and [`readable`] makes
//! something showable of an id the table lacks.

use std::collections::BTreeMap;

use m2_assets::StringTable;
use mlua::{Lua, MultiValue, Table, Value};

use crate::api::Shared;
use crate::events::{as_guid, guid_value};
use crate::script::safe_function;

/// An objective on the map (the PDA) or radar.
#[derive(Clone, Debug, PartialEq)]
pub struct Objective {
    /// What it is about, if an object (a place, a target).
    pub target: Option<u32>,
    /// Text id, with progress appended (`"[VzaCon001.Objectives.003] (0/4)"`).
    pub text: String,
    /// The game's icon index.
    pub icon: i64,
    /// The mission it is for: its index in the mission data
    /// (`WifMissionData.GetMissionIndexFromId`; VzaCon001 is 36).
    pub mission: i64,
}

/// A blip on the radar.
#[derive(Clone, Debug, PartialEq)]
pub struct Blip {
    pub target: Option<u32>,
    /// Offset from the target, metres.
    pub offset: [f32; 3],
    pub colour: [u8; 3],
    pub icon: i64,
}

/// A disc on the ground round a target or a point (`Marker.AddDisc`).
#[derive(Clone, Debug, PartialEq)]
pub struct Disc {
    pub target: Option<u32>,
    /// Its point, when it was given one rather than an object.
    pub at: Option<[f32; 3]>,
    /// Metres.
    pub radius: f32,
    pub colour: [u8; 3],
    /// Over the ground, metres.
    pub lift: f32,
}

/// A marker over a target in the world.
#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub target: Option<u32>,
    /// `"HUD_objective_deliverable"`, `"HUD_objective_action"`,
    /// `"HUD_objective_destroy"`.
    pub icon: String,
    pub colour: [u8; 4],
    /// Height above the target, metres.
    pub height: f32,
    /// Shown from `near` to `far` metres away.
    pub near: f32,
    pub far: f32,
    /// The call's two sizes (its third and eleventh arguments: 32 and 32
    /// in the first contract's); which is for what isn't known yet.
    pub sizes: [f32; 2],
    /// The call's last argument ("2D only"), as given; what it changes
    /// isn't known yet.
    pub flat: bool,
    /// The call's twelfth argument, the mission it is for, as given (of a
    /// list, the first).
    pub mission: String,
}

/// What the scripts want shown.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hud {
    /// The current mission's objective list: text id and icon tag.
    pub mission: Vec<(String, String)>,
    /// Tutorial lines asked for and not yet shown
    /// (`Net.SetTutorialMessage`): each call's arguments as text, in
    /// order (see [`tutorial_text`]).
    pub tutorials: Vec<Vec<String>>,
    /// Objectives on the map, by the scripts' name for them (the target's
    /// guid as text, or `<mission>_PreMission`).
    pub objectives: BTreeMap<String, Objective>,
    pub radar: BTreeMap<String, Blip>,
    /// World markers by handle.
    pub markers: BTreeMap<u32, Marker>,
    /// Discs on the ground by handle (`Marker.AddDisc`).
    pub discs: BTreeMap<u32, Disc>,
    /// Objective messages not yet shown.
    pub messages: Vec<ObjectiveMessage>,
    next_marker: u32,
}

/// An objective message as the scripts send it
/// (`Net.SendEvent_ObjectiveMessage`, for `MrxGuiInterface.
/// DisplayObjectiveMessage` to show).
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectiveMessage {
    /// Its kind, a key of `MrxGuiInterface.tObjectiveMessageConfigs`:
    /// `"add"`, `"upd"`, `"cpl"`, `"ccl"`, `"bonus_add"`...
    pub kind: String,
    /// Its icon, an index into `MrxUtil.tInlineIcons` (`[objdeliver]`...).
    pub icon: i64,
    /// The objective's text id.
    pub text: String,
}

impl ObjectiveMessage {
    /// An objective completed (else added, updated or cancelled).
    pub fn done(&self) -> bool {
        self.kind.ends_with("cpl")
    }
}

/// Marker handles start here.
const FIRST_MARKER: u32 = 0x5000_0000;

/// A text with its ids replaced by the table's strings
/// (`[VzaCon001.Objectives.003] (0/4)` as `Secure the beach (0/4)`), and
/// ids the table lacks made [`readable`].
pub fn render(text: &str, table: &StringTable) -> String {
    render_with(text, |id| table.lookup(id).map(str::to_owned))
}

/// A text id made readable without the string table: icon tags dropped,
/// `[VzaCon001.Objectives.003] (0/4)` as `VzaCon001 objective 3 (0/4)`.
pub fn readable(text: &str) -> String {
    render_with(text, |_| None)
}

/// Line by line: the scripts join texts with line breaks, and the table's
/// own texts break with `[n]`. Lines left empty go.
fn render_with(text: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    text.split('\n')
        .map(|line| render_line(line, &lookup))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_line(text: &str, lookup: &impl Fn(&str) -> Option<String>) -> String {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let before = rest[..open].trim();
        if !before.is_empty() {
            out.push(before.to_owned());
        }
        let Some(close) = rest[open..].find(']') else {
            break;
        };
        let id = &rest[open + 1..open + close];
        rest = &rest[open + close + 1..];
        if let Some(found) = lookup(id) {
            let found = found.trim();
            if !found.is_empty() {
                out.push(found.replace("[n]", "\n"));
            }
            continue;
        }
        let parts: Vec<&str> = id.split('.').collect();
        match parts.as_slice() {
            // An icon tag: `[objdeliver]`.
            [_] => {}
            [mission, "Objectives", n] => {
                let n = n.trim_start_matches('0');
                out.push(format!(
                    "{mission} objective {}",
                    if n.is_empty() { "0" } else { n }
                ));
            }
            [.., last] => out.push(last.replace('_', " ")),
            [] => {}
        }
    }
    let rest = rest.trim();
    if !rest.is_empty() {
        out.push(rest.to_owned());
    }
    out.join(" ")
}

fn text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.to_string_lossy(),
        Some(Value::Integer(i)) => i.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// A script value as text, for a call whose arguments aren't known yet:
/// strings and numbers as they are, others by their type.
fn shown(v: &Value) -> String {
    match v {
        Value::String(s) => s.to_string_lossy(),
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Nil => "nil".to_owned(),
        other => other.type_name().to_owned(),
    }
}

/// The text id of a tutorial line, from its call's arguments
/// ([`Hud::tutorials`]): the first in brackets (`[Tutorial.OpenPDA]`),
/// none when the call clears the line.
pub fn tutorial_text(args: &[String]) -> Option<&str> {
    args.iter()
        .map(|a| a.trim())
        .find(|a| a.starts_with('[') && a.contains(']'))
}

fn number(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Integer(i) => Some(*i as f64),
        Value::Number(n) => Some(*n),
        _ => None,
    }
}

fn int(v: Option<&Value>) -> i64 {
    number(v).map_or(0, |n| n as i64)
}

fn byte(v: Option<&Value>) -> u8 {
    number(v).map_or(255, |n| n.clamp(0.0, 255.0) as u8)
}

fn guid(v: Option<&Value>) -> Option<u32> {
    as_guid(v?)
}

/// Installs the HUD calls on `Net` and `Marker`.
pub fn install(lua: &Lua, world: &Shared) -> mlua::Result<()> {
    let globals = lua.globals();
    let lib = |name: &str| -> mlua::Result<Table> {
        match globals.raw_get::<Value>(name)? {
            Value::Table(t) if t.metatable().is_none() => Ok(t),
            _ => {
                let t = lua.create_table()?;
                globals.raw_set(name, &t)?;
                Ok(t)
            }
        }
    };
    let add = |table: &Table,
               name: &str,
               f: fn(&mut Hud, &Lua, &[Value]) -> mlua::Result<MultiValue>|
     -> mlua::Result<()> {
        let w = std::rc::Rc::clone(world);
        table.set(
            name,
            safe_function(lua, move |lua, args| {
                let args: Vec<Value> = args.into_iter().collect();
                f(&mut w.borrow_mut().hud, lua, &args)
            })?,
        )
    };
    let none = || Ok(MultiValue::new());

    let net = lib("Net")?;
    // A tutorial line: the call's arguments are kept as they come (what
    // each is for isn't known yet; the text id is the one in brackets).
    add(&net, "SetTutorialMessage", |hud, _, a| {
        hud.tutorials.push(a.iter().map(shown).collect());
        Ok(MultiValue::new())
    })?;
    // (mission index, {{text id, icon tag}, ...}, repeatable): the whole
    // list, sent again whenever it changes.
    add(&net, "SendEvent_AddPDAMission", |hud, _, a| {
        hud.mission.clear();
        if let Some(Value::Table(list)) = a.get(1) {
            for pair in list.sequence_values::<Table>() {
                let pair = pair?;
                let get = |i| text(pair.raw_get::<Value>(i).ok().as_ref());
                hud.mission.push((get(1), get(2)));
            }
        }
        Ok(MultiValue::new())
    })?;
    // (name, target, text id, icon, mission index, state, sort order).
    add(&net, "SendEvent_AddPdaObjective", |hud, _, a| {
        hud.objectives.insert(
            text(a.first()),
            Objective {
                target: guid(a.get(1)),
                text: text(a.get(2)),
                icon: int(a.get(3)),
                mission: int(a.get(4)),
            },
        );
        Ok(MultiValue::new())
    })?;
    add(&net, "SendEvent_RemovePdaObjective", |hud, _, a| {
        hud.objectives.remove(&text(a.first()));
        Ok(MultiValue::new())
    })?;
    // (name, x, y, z, r, g, b, width, height, icon, target, sticky,
    // rotate, oriented, sort order).
    add(&net, "SendEvent_AddRadarObjective", |hud, _, a| {
        let f = |i: usize| number(a.get(i)).unwrap_or(0.0) as f32;
        hud.radar.insert(
            text(a.first()),
            Blip {
                target: guid(a.get(10)),
                offset: [f(1), f(2), f(3)],
                colour: [byte(a.get(4)), byte(a.get(5)), byte(a.get(6))],
                icon: int(a.get(9)),
            },
        );
        Ok(MultiValue::new())
    })?;
    add(&net, "SendEvent_RemoveRadarObjective", |hud, _, a| {
        hud.radar.remove(&text(a.first()));
        Ok(MultiValue::new())
    })?;
    // (icon index, kind, text id, player).
    add(&net, "SendEvent_ObjectiveMessage", |hud, _, a| {
        hud.messages.push(ObjectiveMessage {
            kind: text(a.get(1)),
            icon: int(a.first()),
            text: text(a.get(2)),
        });
        Ok(MultiValue::new())
    })?;
    // The network copies of world markers: the markers are already here.
    for name in [
        "SendEvent_AddMarkerObjective",
        "SendEvent_RemoveMarkerObjective",
    ] {
        net.set(name, safe_function(lua, move |_, _| none())?)?;
    }

    let marker = lib("Marker")?;
    // (target, icon, size, r, g, b, a, height, near, far, size, mission,
    // 2D only): returns the marker's handle.
    add(&marker, "AddBlip", |hud, _, a| {
        let handle = FIRST_MARKER + hud.next_marker;
        hud.next_marker += 1;
        let f = |i: usize, d: f64| number(a.get(i)).unwrap_or(d) as f32;
        hud.markers.insert(
            handle,
            Marker {
                target: guid(a.first()),
                icon: text(a.get(1)),
                colour: [
                    byte(a.get(3)),
                    byte(a.get(4)),
                    byte(a.get(5)),
                    byte(a.get(6)),
                ],
                height: f(7, 0.0),
                near: f(8, 5.0),
                far: f(9, 175.0),
                sizes: [f(2, 0.0), f(10, 0.0)],
                flat: matches!(a.get(12), Some(Value::Boolean(true))),
                // A list of missions (a race's gates pass one) gives its
                // first.
                mission: match a.get(11) {
                    Some(Value::Table(t)) => shown(&t.raw_get::<Value>(1).unwrap_or(Value::Nil)),
                    Some(v) => shown(v),
                    None => "nil".to_owned(),
                },
            },
        );
        Ok(MultiValue::from_vec(vec![guid_value(handle)]))
    })?;
    // Marker.AddDisc(target, radius, r, g, b, lift): a disc on the ground
    // round an object or a point, `radius` metres, in the colour, `lift`
    // metres over the ground (inferred from the 3 call sites: 0.02 to 0.1,
    // too small for anything else; the wardrobe's is 0.5 m across its
    // 1 m reach, a delivery's its drop-off radius `fDist`). Returns its
    // handle, which `Marker.Remove` takes as a blip's.
    add(&marker, "AddDisc", |hud, _, a| {
        let handle = FIRST_MARKER + hud.next_marker;
        hud.next_marker += 1;
        let f = |i: usize, d: f64| number(a.get(i)).unwrap_or(d) as f32;
        let at = match a.first() {
            Some(Value::Table(t)) => {
                let c = |k: &str, i: i64| {
                    t.get::<Option<f32>>(k)
                        .ok()
                        .flatten()
                        .or_else(|| t.get::<Option<f32>>(i).ok().flatten())
                };
                c("x", 1)
                    .zip(c("y", 2))
                    .zip(c("z", 3))
                    .map(|((x, y), z)| [x, y, z])
            }
            _ => None,
        };
        hud.discs.insert(
            handle,
            Disc {
                target: guid(a.first()),
                at,
                radius: f(1, 1.0),
                colour: [byte(a.get(2)), byte(a.get(3)), byte(a.get(4))],
                lift: f(5, 0.0),
            },
        );
        Ok(MultiValue::from_vec(vec![guid_value(handle)]))
    })?;
    add(&marker, "Remove", |hud, _, a| {
        if let Some(h) = guid(a.first()) {
            hud.markers.remove(&h);
            hud.discs.remove(&h);
        }
        Ok(MultiValue::new())
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{self, ScriptWorld};
    use crate::script::ScriptHost;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn texts_read_until_the_string_table_is_known() {
        assert_eq!(
            readable("[VzaCon001.Objectives.003] (0/4)"),
            "VzaCon001 objective 3 (0/4)"
        );
        assert_eq!(
            readable(
                "[objt][objdeliver] [objective.Objectivecompleted] [VzaCon001.Objectives.001]"
            ),
            "Objectivecompleted VzaCon001 objective 1"
        );
        assert_eq!(readable("plain"), "plain");
    }

    #[test]
    fn the_tables_text_replaces_the_id() {
        let mut table = StringTable::default();
        for (id, text) in [
            ("VzaCon001.Objectives.001", "Secure the beach"),
            ("objective.Objectivecompleted", "Objective completed:"),
        ] {
            table
                .strings
                .insert(m2_assets::name_hash(id), text.to_owned());
        }
        assert_eq!(
            render("[VzaCon001.Objectives.001] (0/4)", &table),
            "Secure the beach (0/4)"
        );
        assert_eq!(
            render(
                "[objt][objdeliver] [objective.Objectivecompleted] [VzaCon001.Objectives.002]",
                &table
            ),
            "Objective completed: VzaCon001 objective 2"
        );
    }

    #[test]
    fn line_breaks_stay() {
        let mut table = StringTable::default();
        for (id, text) in [("Test.First", "[pda] One[n]two"), ("Test.Second", "Three")] {
            table
                .strings
                .insert(m2_assets::name_hash(id), text.to_owned());
        }
        assert_eq!(
            render("[Test.First]\n[Test.Second]\n", &table),
            "[pda] One\ntwo\nThree"
        );
        assert_eq!(readable("[Test.First]\n[Test.Second]"), "First\nSecond");
    }

    #[test]
    fn objectives_and_markers_come_and_go() {
        let host = ScriptHost::new().unwrap();
        let world: Shared = Rc::new(RefCell::new(ScriptWorld::default()));
        api::install(host.lua(), &world).unwrap();
        host.lua()
            .load(
                r#"
                local beach = Sys.StringToGuid("734801")
                Net.SendEvent_AddPDAMission(36, {{"[VzaCon001.Objectives.001]", "[objdeliver]"}}, false)
                Net.SendEvent_AddPdaObjective("734801", beach, "[VzaCon001.Objectives.001]", 17, 36, 0, 2)
                Net.SendEvent_AddRadarObjective("734801", 0, 2, 0, 255, 200, 0, 10.667, 10.667, 2, beach, true, false, false, 5)
                Net.SendEvent_ObjectiveMessage(5, "add", "[VzaCon001.Objectives.001]", nil)
                handle = Marker.AddBlip(beach, "HUD_objective_deliverable", 32, 255, 200, 0, 255, 2, 5, 175, 32, "VzaCon001", false)
                Net.SendEvent_AddMarkerObjective(beach, handle, 255, 200, 0, 2, 8, 1, 16, false, 5, 175, 0)
                "#,
            )
            .exec()
            .unwrap();
        {
            let w = world.borrow();
            let hud = &w.hud;
            assert_eq!(
                hud.mission,
                [(
                    "[VzaCon001.Objectives.001]".to_owned(),
                    "[objdeliver]".to_owned()
                )]
            );
            assert_eq!(hud.objectives["734801"].target, Some(734_801));
            assert_eq!(hud.radar["734801"].colour, [255, 200, 0]);
            assert_eq!(
                hud.messages,
                [ObjectiveMessage {
                    kind: "add".to_owned(),
                    icon: 5,
                    text: "[VzaCon001.Objectives.001]".to_owned(),
                }]
            );
            let m = hud.markers.values().next().unwrap();
            assert_eq!((m.target, m.height, m.far), (Some(734_801), 2.0, 175.0));
            assert_eq!((m.sizes, m.flat), ([32.0, 32.0], false));
        }
        host.lua()
            .load(
                r#"
                Net.SendEvent_RemovePdaObjective("734801")
                Net.SendEvent_RemoveRadarObjective("734801")
                Marker.Remove(handle)
                "#,
            )
            .exec()
            .unwrap();
        let w = world.borrow();
        assert!(w.hud.objectives.is_empty() && w.hud.radar.is_empty() && w.hud.markers.is_empty());
    }

    #[test]
    fn tutorial_lines_keep_their_arguments() {
        let host = ScriptHost::new().unwrap();
        let world: Shared = Rc::new(RefCell::new(ScriptWorld::default()));
        api::install(host.lua(), &world).unwrap();
        host.lua()
            .load(
                r#"
                Net.SetTutorialMessage(nil, "[Tutorial.OpenPDA]", 8, true)
                Net.SetTutorialMessage(nil, "")
                "#,
            )
            .exec()
            .unwrap();
        let w = world.borrow();
        let t = &w.hud.tutorials;
        assert_eq!(t[0], ["nil", "[Tutorial.OpenPDA]", "8", "true"]);
        assert_eq!(tutorial_text(&t[0]), Some("[Tutorial.OpenPDA]"));
        assert_eq!(tutorial_text(&t[1]), None);
    }
}
