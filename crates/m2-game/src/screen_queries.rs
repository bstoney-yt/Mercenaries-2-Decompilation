//! What the pause screen and the PDA map ask about the player:
//! `Player.GetControlBindingType` (which controller page to show) and
//! `Player.GetAllTargetMarkerPos` (the players' map target markers).
//!
//! From the 360 code:
//! - `GetControlBindingType(player)` (0x8233dc90) reads a guid (0x82308a80;
//!   nil when there is none) and finds the player record by it, falling
//!   back to the local player's (0x823da2a0, then 0x83658024). The record's
//!   controlled object (+0x18, the seat `GetControlledObject` turns into its
//!   vehicle) decides the answer: none is "human"; otherwise the first of
//!   six component tables that holds that object names it, in this order:
//!   "car" (0x836531ac), "tank" (0x83653244), "helicopter" (0x836534cc),
//!   "livingworld" (0x83653290), "boat" (0x836531f8), "ladder"
//!   (0x83653518). An object in none of them is "seat".
//! - `GetAllTargetMarkerPos()` (0x8233f470) goes through the players
//!   (0x82467ab0 by number) and, for each one bound to a controller
//!   (record +0x24 not -1), adds a table: `{ false }` when the player has
//!   no target marker (+0x444), else `{ true, x, z }`, the marker's
//!   position (0x82381d40: the object's +0x3c and +0x44). It answers the
//!   list (0x82309a70 packs them). The marker is the minimap icon
//!   0x82440018 places ("MiniMap_Icon_Marker_A/B", event
//!   "SetTargetMarker"). `MrxGuiPda` draws entry 1 as "target1_mc" and the
//!   rest as "target2_mc".
//!
//! [`ScreenState`] keeps what the game tells the scripts: which component
//! each control object has ([`set_control_kind`]) and her target marker
//! ([`set_target_marker`]). Single player: one player, always bound.
//!
//! Not ported: the game doesn't call either setter yet, so a seat answers
//! "seat" and the list holds `{ false }`. Which seats carry the driving
//! components (the driver's, presumably) is data in the 360's templates
//! and isn't traced; the game is to register the kind for each seat that
//! has one. Ladders and the living-world control aren't modelled here.

use std::collections::BTreeMap;

use mlua::{Lua, MultiValue, Table, Value};

use crate::api::ScriptWorld;
use crate::events::as_guid;

type Handler = fn(&mut ScriptWorld, &Lua, &[Value]) -> mlua::Result<MultiValue>;

/// The control components `GetControlBindingType` tells apart, in the
/// 360's order of lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlKind {
    Car,
    Tank,
    Helicopter,
    LivingWorld,
    Boat,
    Ladder,
}

impl ControlKind {
    /// The answer's text.
    pub fn name(self) -> &'static str {
        match self {
            Self::Car => "car",
            Self::Tank => "tank",
            Self::Helicopter => "helicopter",
            Self::LivingWorld => "livingworld",
            Self::Boat => "boat",
            Self::Ladder => "ladder",
        }
    }
}

/// What the game has told the scripts, in the script host's app data.
#[derive(Debug, Default)]
pub struct ScreenState {
    /// The control component of each control object (a seat).
    pub control_kinds: BTreeMap<u32, ControlKind>,
    /// Her target marker on the map, (x, z).
    pub target_marker: Option<[f32; 2]>,
}

fn state(lua: &Lua) -> mlua::AppDataRefMut<'_, ScreenState> {
    if lua.app_data_ref::<ScreenState>().is_none() {
        lua.set_app_data(ScreenState::default());
    }
    lua.app_data_mut::<ScreenState>().expect("set above")
}

/// Gives `object` (a seat) its control component, or takes it away.
pub fn set_control_kind(lua: &Lua, object: u32, kind: Option<ControlKind>) {
    let mut s = state(lua);
    match kind {
        Some(k) => s.control_kinds.insert(object, k),
        None => s.control_kinds.remove(&object),
    };
}

/// Places her target marker at (x, z), or clears it.
pub fn set_target_marker(lua: &Lua, at: Option<[f32; 2]>) {
    state(lua).target_marker = at;
}

fn get_control_binding_type(
    w: &mut ScriptWorld,
    lua: &Lua,
    a: &[Value],
) -> mlua::Result<MultiValue> {
    // Any guid finds her record: no record of its own falls back to the
    // local player's.
    if a.first().and_then(as_guid).is_none() {
        return Ok(MultiValue::from_iter([Value::Nil]));
    }
    let Some(her) = w.player else {
        return Ok(MultiValue::from_iter([Value::Nil]));
    };
    let name = match w.seats.seat_of(her) {
        None => "human",
        Some(seat) => state(lua)
            .control_kinds
            .get(&seat)
            .map_or("seat", |k| k.name()),
    };
    Ok(MultiValue::from_iter([Value::String(
        lua.create_string(name)?,
    )]))
}

fn get_all_target_marker_pos(
    _: &mut ScriptWorld,
    lua: &Lua,
    _: &[Value],
) -> mlua::Result<MultiValue> {
    let marker = state(lua).target_marker;
    let entry = lua.create_table()?;
    match marker {
        None => entry.raw_push(false)?,
        Some([x, z]) => {
            entry.raw_push(true)?;
            entry.raw_push(f64::from(x))?;
            entry.raw_push(f64::from(z))?;
        }
    }
    let all = lua.create_table()?;
    all.raw_push(entry)?;
    Ok(MultiValue::from_iter([Value::Table(all)]))
}

pub(crate) fn install(
    lib: &dyn Fn(&str) -> mlua::Result<Table>,
    add: &dyn Fn(&Table, &str, Handler) -> mlua::Result<()>,
) -> mlua::Result<()> {
    let player = lib("Player")?;
    add(&player, "GetControlBindingType", get_control_binding_type)?;
    add(&player, "GetAllTargetMarkerPos", get_all_target_marker_pos)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{Object, Shared, install as install_api};
    use crate::script::ScriptHost;
    use crate::vehicle_seats::SeatParams;

    fn setup() -> (ScriptHost, Shared) {
        let host = ScriptHost::new().unwrap();
        let world: Shared = std::rc::Rc::default();
        {
            let mut w = world.borrow_mut();
            w.insert(1, Object::new("Mattias", [0.0; 3]));
            w.player = Some(1);
            w.insert(50, Object::new("jeep", [0.0; 3]));
        }
        install_api(host.lua(), &world).unwrap();
        (host, world)
    }

    /// The pause screen's `tControlMap[Player.GetControlBindingType(p)]`.
    #[test]
    fn the_control_page_follows_what_she_controls() {
        let (host, world) = setup();
        let lua = host.lua();
        let ask = "return Player.GetControlBindingType(Player.GetLocalPlayer())";
        let kind = || lua.load(ask).eval::<Value>().unwrap();
        let text = |v: Value| v.as_string().map(|s| s.to_string_lossy());
        assert_eq!(text(kind()).as_deref(), Some("human"));
        let seat = {
            let mut w = world.borrow_mut();
            let driver = SeatParams {
                driver: true,
                ..SeatParams::default()
            };
            w.seats.add(
                50,
                [
                    (1, driver, true, Vec::new()),
                    (2, SeatParams::default(), true, Vec::new()),
                ],
            );
            w.seats.sit(1, 50, None, true);
            w.seats.seat_of(1).unwrap()
        };
        // A seat with no driving component.
        assert_eq!(text(kind()).as_deref(), Some("seat"));
        set_control_kind(lua, seat, Some(ControlKind::Car));
        assert_eq!(text(kind()).as_deref(), Some("car"));
        // A guid that is no player's still finds hers; no guid is nil.
        let any: Value = lua
            .load("return Player.GetControlBindingType(Sys.StringToGuid('0x50'))")
            .eval()
            .unwrap();
        assert_eq!(text(any).as_deref(), Some("car"));
        let none: Value = lua
            .load("return Player.GetControlBindingType()")
            .eval()
            .unwrap();
        assert_eq!(none, Value::Nil);
    }

    /// `MrxGuiPda` reads `{ { has, x, z }, ... }` with `ipairs`.
    #[test]
    fn target_markers_list_one_entry_per_bound_player() {
        let (host, _world) = setup();
        let lua = host.lua();
        let ask = r#"
            local t = Player.GetAllTargetMarkerPos()
            local e = t[1]
            return #t, e[1], e[2], e[3]
        "#;
        let (n, has, x, z): (i64, bool, Option<f64>, Option<f64>) = lua.load(ask).eval().unwrap();
        assert_eq!((n, has, x, z), (1, false, None, None));
        set_target_marker(lua, Some([120.0, -40.5]));
        let got: (i64, bool, Option<f64>, Option<f64>) = lua.load(ask).eval().unwrap();
        assert_eq!(got, (1, true, Some(120.0), Some(-40.5)));
    }
}
