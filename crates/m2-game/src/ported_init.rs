//! Missing natives the scripts call, fifth gap-list batch: initialisers
//! and loaders ported from the 360 code. Each function names its 360
//! address.

use mlua::{Function, Lua, MultiValue, Table, Value};

use crate::api::ScriptWorld;

type Handler = fn(&mut ScriptWorld, &Lua, &[Value]) -> mlua::Result<MultiValue>;

/// How many source music entry states the music keeps (+0xd3f0 counts to
/// 4).
pub const MUSIC_ENTRY_STATES: usize = 4;

/// What these natives keep for the game to read.
#[derive(Debug, Default, Clone)]
pub struct InitState {
    /// `Sound.AddSourceMusicEntryState`: up to four hashes, cleared by
    /// `Sound.ClearSourceMusicEntryStates`.
    pub music_entry_states: Vec<u32>,
    /// `Sound.OpenStreamFile`: (name, second name) pairs opened, in order.
    pub stream_files: Vec<(String, String)>,
    /// `Disguise.Init`'s `InvestigatorCallback` (0x83134548).
    pub investigator_callback: Option<Function>,
    /// `FactionZone.Init`'s `TresspasserCallback` (0x83134298; the scripts'
    /// spelling).
    pub trespasser_callback: Option<Function>,
    /// `Player.SetSatelliteScanMode`: on, and the point given.
    pub satellite_scan: Option<(bool, [f32; 3])>,
    /// `Graphics.InitTinyGeometry` calls.
    pub tiny_geometry: u32,
    /// `Gui.LoadFont`: fonts asked for, by hash, in order.
    pub fonts: Vec<u32>,
}

pub(crate) fn install(
    lib: &dyn Fn(&str) -> mlua::Result<Table>,
    add: &dyn Fn(&Table, &str, Handler) -> mlua::Result<()>,
) -> mlua::Result<()> {
    let sound = lib("Sound")?;
    add(&sound, "ClearSourceMusicEntryStates", |w, _, _| {
        w.init.music_entry_states.clear();
        nothing()
    })?;
    add(&sound, "AddSourceMusicEntryState", add_music_entry_state)?;
    add(&sound, "OpenStreamFile", open_stream_file)?;
    add(&lib("Disguise")?, "Init", disguise_init)?;
    add(&lib("FactionZone")?, "Init", faction_zone_init)?;
    add(
        &lib("Player")?,
        "SetSatelliteScanMode",
        set_satellite_scan_mode,
    )?;
    add(&lib("Graphics")?, "InitTinyGeometry", |w, _, _| {
        w.init.tiny_geometry += 1;
        nothing()
    })?;
    add(&lib("Gui")?, "LoadFont", load_font)?;
    Ok(())
}

fn guid(v: Option<&Value>) -> Option<u32> {
    crate::events::as_guid(v?)
}

fn boolean(v: Option<&Value>) -> Option<bool> {
    match v? {
        Value::Boolean(b) => Some(*b),
        _ => None,
    }
}

fn number(v: Option<&Value>) -> Option<f32> {
    match v? {
        Value::Integer(i) => Some(*i as f32),
        Value::Number(n) => Some(*n as f32),
        _ => None,
    }
}

fn text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.to_string_lossy()),
        _ => None,
    }
}

/// A hash argument: a name (hashed as the archive's names are) or the
/// number itself.
fn hash(v: Option<&Value>) -> Option<u32> {
    match v? {
        Value::String(s) => Some(m2_assets::name_hash(&s.to_string_lossy())),
        Value::Integer(i) => Some(*i as u32),
        Value::Number(n) if n.is_finite() => Some(*n as i64 as u32),
        _ => None,
    }
}

fn nothing() -> mlua::Result<MultiValue> {
    Ok(MultiValue::new())
}

fn one(v: Value) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_vec(vec![v]))
}

fn nil() -> mlua::Result<MultiValue> {
    one(Value::Nil)
}

/// `Sound.AddSourceMusicEntryState(hash)` (0x82341e88): adds it while
/// fewer than four are kept; returns nothing, or nil without a hash.
fn add_music_entry_state(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let Some(h) = hash(a.first()) else {
        return nil();
    };
    if w.init.music_entry_states.len() < MUSIC_ENTRY_STATES {
        w.init.music_entry_states.push(h);
    }
    nothing()
}

/// `Sound.OpenStreamFile(name, name)` (0x82343da0): opens the stream;
/// returns nothing, or nil without two strings.
fn open_stream_file(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    match (text(a.first()), text(a.get(1))) {
        (Some(x), Some(y)) => {
            w.init.stream_files.push((x, y));
            nothing()
        }
        _ => nil(),
    }
}

/// The function at `key` of a table argument, and whether there was a
/// table: the shape of `Disguise.Init` and `FactionZone.Init`.
fn callback_in(a: &[Value], key: &str) -> mlua::Result<(bool, Option<Function>)> {
    match a.first() {
        Some(Value::Table(t)) => Ok((true, t.raw_get::<Option<Function>>(key).ok().flatten())),
        _ => Ok((false, None)),
    }
}

/// `Disguise.Init({ InvestigatorCallback = f })` (0x82340b20): keeps the
/// callback (none if the table has none) and returns true; false, keeping
/// the old one, without a table.
fn disguise_init(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let (table, f) = callback_in(a, "InvestigatorCallback")?;
    if table {
        w.init.investigator_callback = f;
    }
    one(Value::Boolean(table))
}

/// `FactionZone.Init({ TresspasserCallback = f })` (0x82340bf8): as
/// `Disguise.Init`, for the faction zones' trespass callback.
fn faction_zone_init(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let (table, f) = callback_in(a, "TresspasserCallback")?;
    if table {
        w.init.trespasser_callback = f;
    }
    one(Value::Boolean(table))
}

/// `Player.SetSatelliteScanMode(player, on, x, y, z)` (0x8233c730): hands
/// the player's record the switch and the point (0x8243ab10, not traced
/// further); returns nothing, or nil without them.
fn set_satellite_scan_mode(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let point = (number(a.get(2)), number(a.get(3)), number(a.get(4)));
    match (guid(a.first()), boolean(a.get(1)), point) {
        (Some(_), Some(on), (Some(x), Some(y), Some(z))) => {
            w.init.satellite_scan = Some((on, [x, y, z]));
            nothing()
        }
        _ => nil(),
    }
}

/// `Gui.LoadFont(hash)` (0x8231aa58): asks for the font when the GUI is up
/// and always returns nil.
fn load_font(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    if let Some(h) = hash(a.first()) {
        w.init.fonts.push(h);
    }
    nil()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{Object, Shared, install};
    use crate::script::ScriptHost;

    #[test]
    fn initialisers_and_loaders_answer_as_the_360() {
        let host = ScriptHost::new().unwrap();
        let world = Shared::default();
        {
            let mut w = world.borrow_mut();
            w.insert(1, Object::new("Mattias", [0.0; 3]));
            w.player = Some(1);
        }
        install(host.lua(), &world).unwrap();
        let lua = host.lua();
        let n = |code: &str| -> usize {
            lua.load(format!("return select('#', {code})"))
                .eval()
                .unwrap()
        };
        let got = |code: &str| -> Value { lua.load(format!("return {code}")).eval().unwrap() };
        // Music entry states: four at most, then cleared.
        for s in ["a", "b", "c", "d", "e"] {
            assert_eq!(n(&format!("Sound.AddSourceMusicEntryState('{s}')")), 0);
        }
        assert_eq!(world.borrow().init.music_entry_states.len(), 4);
        assert_eq!(got("Sound.AddSourceMusicEntryState()"), Value::Nil);
        assert_eq!(n("Sound.ClearSourceMusicEntryStates()"), 0);
        assert!(world.borrow().init.music_entry_states.is_empty());
        // Streams.
        assert_eq!(n("Sound.OpenStreamFile('a', 'b')"), 0);
        assert_eq!(got("Sound.OpenStreamFile('a')"), Value::Nil);
        // Callbacks kept from their tables.
        assert_eq!(
            got("FactionZone.Init({ TresspasserCallback = function() end })"),
            Value::Boolean(true)
        );
        assert!(world.borrow().init.trespasser_callback.is_some());
        assert_eq!(got("Disguise.Init({})"), Value::Boolean(true));
        assert!(world.borrow().init.investigator_callback.is_none());
        assert_eq!(got("Disguise.Init()"), Value::Boolean(false));
        // Satellite scan.
        assert_eq!(
            n("Player.SetSatelliteScanMode(Player.GetLocalPlayer(), true, 1, 2, 3)"),
            0
        );
        assert_eq!(
            world.borrow().init.satellite_scan,
            Some((true, [1.0, 2.0, 3.0]))
        );
        assert_eq!(
            got("Player.SetSatelliteScanMode(Player.GetLocalPlayer())"),
            Value::Nil
        );
        // Loaders.
        assert_eq!(n("Graphics.InitTinyGeometry()"), 0);
        assert_eq!(world.borrow().init.tiny_geometry, 1);
        assert_eq!(n("Gui.LoadFont('font')"), 1);
        assert_eq!(world.borrow().init.fonts, [m2_assets::name_hash("font")]);
    }
}
