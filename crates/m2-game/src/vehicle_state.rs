//! The `Vehicle` library's calls that work a vehicle: its parts, its
//! turrets, its health and ammunition, its controls.
//!
//! From the 360 code (each reads its vehicle by guid and raises an
//! argument error, 0x82206F20, without one):
//! - `SetParts(vehicle, part, on)` (0x82348AF8): the part by name hash,
//!   `LightBrake` 1, `LightReverse` 2, `LightFront` 3, `CtrlRotation` 4,
//!   `LandingGear` 5, `Exhaust` 6, `ExhaustStart` 7, `ExhaustStop` 8,
//!   set through 0x82276828; returns true. An unknown name is an error.
//! - `SetTurretPitch(vehicle, turret, angle)` (0x82347D20) and
//!   `SetTurretYaw` (0x82347E00): a message to the turrets (0x831BB988)
//!   of the turret's name hash and the angle, mode 1 pitch, 2 yaw; true.
//! - `SpinHeli(vehicle)` (0x82348150): for a helicopter (kind 4) the
//!   input 0x30FBBF64 at -1 (0x831B7420); true. An error for anything
//!   else (here: for no object; the script world doesn't know kinds).
//! - `StartTankHijackMotion(vehicle)` (0x82348020) and
//!   `StopTankHijackMotion` (0x823480B8): the steering input 0x3483DBF1
//!   at -0.2, and at 0 (0x831B81D0); true.
//! - `IsHijackBad(seat)` (0x823488F0): false unless in co-op (0x82DE698F),
//!   where another rider of its vehicle being a player's makes it true.
//! - `RestoreHealth(vehicle)` (0x82348A38, 0x8222C9B8),
//!   `RestoreAmmo(vehicle)` (0x82348A98, 0x82285AF8): nothing returned.
//! - `ClearControls(vehicle)` (0x82348C70): its controls let go
//!   (0x822D3198, 0x822D31E8) and the message 0x6C5F1491 sent to it twice
//!   (0x831B7420, 0x831BA408); nothing returned.
//!
//! Not ported: `IsFlipped` (0x82348358, the physics object's vtable
//! +0x9C: the script world has no up vector), `CancelHijack` (0x823486D8,
//! a script call through the store 0x83658404). The script world keeps what was asked ([`VehicleCommand`]); the game
//! applies it. Inferred: a restored vehicle's health is its full health
//! at once in the script world.
//!
//! Other libraries' vehicle calls: `Pg.FastCollectGroundVehiclesExceptTanks`
//! (0x82335110, kind 2 only: `FastCollectCars`' kind), `Player.SetVehicleControlsLock`
//! (0x8233DBD8), `Player.VehicleDisguise` (0x823402B8), the `Gui` entrance
//! marker settings (0x8231C090, 0x8231C120, 0x8231C238) and
//! `Sound.SetVehicleEngineBoost` (0x823435C8). Not ported: when the
//! disguise callback is called, `Human.GetVehicleWeapon` (0x82325E08).

use mlua::{Lua, MultiValue, Table, Value};

use crate::api::{Command, ScriptWorld, Shared};
use crate::script::{ScriptHost, safe_function};

/// `SetParts`'s parts, by their hashes' names, numbered as 0x82348AF8 does.
pub const PARTS: [&str; 8] = [
    "LightBrake",
    "LightReverse",
    "LightFront",
    "CtrlRotation",
    "LandingGear",
    "Exhaust",
    "ExhaustStart",
    "ExhaustStop",
];

/// What a script asked of a vehicle.
#[derive(Clone, Debug, PartialEq)]
pub enum VehicleCommand {
    /// A part (1..=8, [`PARTS`]) on or off.
    Part {
        vehicle: u32,
        part: u8,
        on: bool,
    },
    /// A turret (by name hash) to an angle: its pitch, or its yaw.
    Turret {
        vehicle: u32,
        turret: u32,
        angle: f32,
        yaw: bool,
    },
    /// A helicopter's spin input (0x30FBBF64 at -1).
    Spin(u32),
    /// A tank's steering input (0x3483DBF1) held at this.
    TankHijackSteer {
        vehicle: u32,
        steer: f32,
    },
    RestoreHealth(u32),
    /// `Sound.SetVehicleEngineBoost` (0x823435C8): the vehicle's engine
    /// sound's boost (+0x30 of its sound object), while sound is on.
    EngineBoost {
        vehicle: u32,
        boost: f32,
    },
    RestoreAmmo(u32),
    ClearControls(u32),
}

fn id(v: Option<&Value>) -> Option<u32> {
    v.and_then(crate::events::as_guid)
}

fn text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.to_string_lossy()),
        _ => None,
    }
}

fn num(v: Option<&Value>) -> Option<f32> {
    match v? {
        Value::Integer(n) => Some(*n as f32),
        Value::Number(n) => Some(*n as f32),
        _ => None,
    }
}

/// 0x82348AF8's part number for `name`.
pub fn part(name: &str) -> Option<u8> {
    let h = m2_assets::name_hash(name);
    PARTS
        .iter()
        .position(|p| m2_assets::name_hash(p) == h)
        .map(|i| i as u8 + 1)
}

fn ok(v: bool) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_iter([Value::Boolean(v)]))
}

fn bad(name: &str) -> mlua::Error {
    mlua::Error::runtime(format!("Vehicle.{name}: bad argument"))
}

/// Installs the calls over `world`. Run after the other libraries are in.
pub fn install(host: &ScriptHost, world: &Shared) -> mlua::Result<()> {
    let lua = host.lua();
    let globals = lua.globals();
    let vehicle = match globals.get::<Value>("Vehicle")? {
        Value::Table(t) => t,
        _ => {
            let t = lua.create_table()?;
            globals.raw_set("Vehicle", &t)?;
            t
        }
    };
    let add = |table: &Table,
               name: &str,
               f: fn(&mut ScriptWorld, &Lua, &[Value]) -> mlua::Result<MultiValue>|
     -> mlua::Result<()> {
        let w = std::rc::Rc::clone(world);
        table.raw_set(
            name,
            safe_function(lua, move |lua, args| {
                let args: Vec<Value> = args.into_iter().collect();
                f(&mut w.borrow_mut(), lua, &args)
            })?,
        )
    };
    add(&vehicle, "SetParts", |w, _, a| {
        let (Some(vehicle), Some(part)) = (id(a.first()), text(a.get(1)).and_then(|n| part(&n)))
        else {
            return Err(bad("SetParts"));
        };
        let on = matches!(a.get(2), Some(Value::Boolean(true)));
        w.commands
            .push(Command::Vehicle(VehicleCommand::Part { vehicle, part, on }));
        ok(true)
    })?;
    for (name, yaw) in [("SetTurretPitch", false), ("SetTurretYaw", true)] {
        let f: fn(&mut ScriptWorld, &Lua, &[Value]) -> mlua::Result<MultiValue> = if yaw {
            |w, _, a| turret(w, a, true)
        } else {
            |w, _, a| turret(w, a, false)
        };
        add(&vehicle, name, f)?;
    }
    add(&vehicle, "SpinHeli", |w, _, a| {
        let Some(v) = id(a.first()).filter(|v| w.objects.contains_key(v)) else {
            return Err(bad("SpinHeli"));
        };
        w.commands.push(Command::Vehicle(VehicleCommand::Spin(v)));
        ok(true)
    })?;
    for (name, steer) in [
        ("StartTankHijackMotion", -0.2),
        ("StopTankHijackMotion", 0.0),
    ] {
        let f: fn(&mut ScriptWorld, &Lua, &[Value]) -> mlua::Result<MultiValue> = if steer != 0.0 {
            |w, _, a| tank_steer(w, a, -0.2)
        } else {
            |w, _, a| tank_steer(w, a, 0.0)
        };
        add(&vehicle, name, f)?;
    }
    // 0x823488F0: only in co-op (0x82DE698F) is a hijack bad (another
    // rider of the seat's vehicle is a player's character): one player.
    add(&vehicle, "IsHijackBad", |_, _, a| {
        if id(a.first()).is_none() {
            return Err(bad("IsHijackBad"));
        }
        ok(false)
    })?;
    add(&vehicle, "RestoreHealth", |w, _, a| {
        let Some(v) = id(a.first()) else {
            return Err(bad("RestoreHealth"));
        };
        if let Some(o) = w.objects.get_mut(&v) {
            o.health = o.max_health;
        }
        w.commands
            .push(Command::Vehicle(VehicleCommand::RestoreHealth(v)));
        Ok(MultiValue::new())
    })?;
    add(&vehicle, "RestoreAmmo", |w, _, a| {
        let Some(v) = id(a.first()) else {
            return Err(bad("RestoreAmmo"));
        };
        w.commands
            .push(Command::Vehicle(VehicleCommand::RestoreAmmo(v)));
        Ok(MultiValue::new())
    })?;
    let lib = |name: &str| -> mlua::Result<Table> {
        match globals.get::<Value>(name)? {
            Value::Table(t) => Ok(t),
            _ => {
                let t = lua.create_table()?;
                globals.raw_set(name, &t)?;
                Ok(t)
            }
        }
    };
    // 0x82335110: as `FastCollectGroundVehicles` (0x82334FE8, its kinds 3
    // and 2), only kind 2: the kind `FastCollectCars` searches
    // (0x82334EE8); `FastCollectTanks` is kind 3 (0x82334DE8),
    // `FastCollectBoats` kind 10 (0x82335310). The script world has no
    // physics classes: the cars are told by labels, a `Vehicle` neither
    // flying, `Tank` nor `Boat`.
    add(
        &lib("Pg")?,
        "FastCollectGroundVehiclesExceptTanks",
        |w, lua, a| {
            crate::api::collect_labelled(
                w,
                lua,
                a,
                &["Vehicle"],
                &["helicopter", "Plane", "Aircraft", "Tank", "Boat"],
            )
        },
    )?;
    let player = lib("Player")?;
    // 0x8233DBD8: her player record's +0x450, true.
    add(&player, "SetVehicleControlsLock", |w, _, a| {
        if id(a.first()).is_none_or(|p| Some(p) != w.player) {
            return Err(bad("SetVehicleControlsLock"));
        }
        w.vehicle_controls_locked = matches!(a.get(1), Some(Value::Boolean(true)));
        ok(true)
    })?;
    // 0x823402B8: `{Player = p, Callback = f, AutoRemove = b}` keeps `f`
    // for her vehicle disguise (the player's +0x420, `AutoRemove` +0x428
    // bit 4) and answers true; `Remove = true` drops it.
    add(&player, "VehicleDisguise", |w, lua, a| {
        let Some(Value::Table(t)) = a.first() else {
            return ok(false);
        };
        let who = t.get::<Value>("Player")?;
        if id(Some(&who)).is_none_or(|p| Some(p) != w.player) {
            return ok(false);
        }
        if matches!(t.get::<Value>("Remove")?, Value::Boolean(true)) {
            lua.unset_named_registry_value(DISGUISE_CALLBACK)?;
            w.vehicle_disguise_auto_remove = false;
            return ok(false);
        }
        match t.get::<Value>("Callback")? {
            Value::Function(f) => {
                lua.set_named_registry_value(DISGUISE_CALLBACK, f)?;
                w.vehicle_disguise_auto_remove =
                    matches!(t.get::<Value>("AutoRemove")?, Value::Boolean(true));
                ok(true)
            }
            _ => ok(false),
        }
    })?;
    let gui = lib("Gui")?;
    // 0x8231C090, 0x8231C120, 0x8231C238: per player (second argument,
    // 1 unless given) the entrance markers' distance (kept squared), on or
    // off, and size (0x20 unless given).
    add(
        &gui,
        "SetVehicleEntranceMarkerVisibleDistance",
        |w, _, a| {
            let Some(d) = num(a.first()) else {
                return Err(bad("SetVehicleEntranceMarkerVisibleDistance"));
            };
            let p = player_index(a.get(1));
            w.entrance_markers[p].distance_squared = d * d;
            Ok(MultiValue::new())
        },
    )?;
    add(&gui, "EnableVehicleEntranceMarkers", |w, _, a| {
        let Some(Value::Boolean(on)) = a.first() else {
            return Err(bad("EnableVehicleEntranceMarkers"));
        };
        let p = player_index(a.get(1));
        w.entrance_markers[p].on = *on;
        Ok(MultiValue::new())
    })?;
    add(&gui, "SetVehicleEntranceMarkerSize", |w, _, a| {
        let size = num(a.first()).map_or(0x20, |n| n as u8);
        let p = player_index(a.get(1));
        w.entrance_markers[p].size = size;
        Ok(MultiValue::new())
    })?;
    add(&lib("Sound")?, "SetVehicleEngineBoost", |w, _, a| {
        let (Some(vehicle), Some(boost)) = (id(a.first()), num(a.get(1))) else {
            return Err(mlua::Error::runtime(
                "Sound.SetVehicleEngineBoost: bad argument",
            ));
        };
        w.commands
            .push(Command::Vehicle(VehicleCommand::EngineBoost {
                vehicle,
                boost,
            }));
        Ok(MultiValue::new())
    })?;
    add(&vehicle, "ClearControls", |w, _, a| {
        let Some(v) = id(a.first()) else {
            return Err(bad("ClearControls"));
        };
        w.commands
            .push(Command::Vehicle(VehicleCommand::ClearControls(v)));
        Ok(MultiValue::new())
    })?;
    Ok(())
}

/// The Lua registry name `Player.VehicleDisguise`'s callback is kept by.
pub const DISGUISE_CALLBACK: &str = "m2.vehicle_disguise_callback";

/// A player number argument (1 or 2; 1 unless given) as an index.
fn player_index(v: Option<&Value>) -> usize {
    match v {
        Some(Value::Integer(2)) => 1,
        Some(Value::Number(n)) if *n == 2.0 => 1,
        _ => 0,
    }
}

/// The vehicle entrance markers' settings for a player (the `Gui` calls,
/// 0x82F14440 / 0x82F14454 / 0x82F1442C by player).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntranceMarkers {
    pub on: bool,
    pub distance_squared: f32,
    pub size: u8,
}

impl Default for EntranceMarkers {
    /// Inferred: on, at the 360's untouched globals' zero distance, size
    /// 0x20 (the size call's default).
    fn default() -> Self {
        Self {
            on: true,
            distance_squared: 0.0,
            size: 0x20,
        }
    }
}

fn turret(w: &mut ScriptWorld, a: &[Value], yaw: bool) -> mlua::Result<MultiValue> {
    let (Some(vehicle), Some(name), Some(angle)) = (id(a.first()), text(a.get(1)), num(a.get(2)))
    else {
        return Err(bad(if yaw {
            "SetTurretYaw"
        } else {
            "SetTurretPitch"
        }));
    };
    w.commands.push(Command::Vehicle(VehicleCommand::Turret {
        vehicle,
        turret: m2_assets::name_hash(&name),
        angle,
        yaw,
    }));
    ok(true)
}

fn tank_steer(w: &mut ScriptWorld, a: &[Value], steer: f32) -> mlua::Result<MultiValue> {
    let Some(vehicle) = id(a.first()) else {
        return Err(bad("TankHijackMotion"));
    };
    w.commands
        .push(Command::Vehicle(VehicleCommand::TankHijackSteer {
            vehicle,
            steer,
        }));
    ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_parts_are_the_hashes_0x82348af8_compares() {
        let h: Vec<u32> = PARTS.iter().map(|p| m2_assets::name_hash(p)).collect();
        assert_eq!(
            h,
            [
                0x8194_F448,
                0xDD45_B6C3,
                0x7139_75A4,
                0xC079_3C78,
                0x98F3_790F,
                0x4905_C3BD,
                0xE91F_1B2F,
                0x3273_505D
            ]
        );
        assert_eq!(part("lightfront"), Some(3));
        assert_eq!(part("Siren"), None);
    }

    #[test]
    fn the_scripts_work_a_vehicle() {
        let host = ScriptHost::new().unwrap();
        let world: Shared = Default::default();
        {
            let mut w = world.borrow_mut();
            let o = crate::api::Object {
                health: 10.0,
                max_health: 100.0,
                ..Default::default()
            };
            w.insert(7, o);
        }
        install(&host, &world).unwrap();
        let lua = host.lua();
        lua.globals()
            .set("v", crate::events::guid_value(7))
            .unwrap();
        let (part, yaw, spin): (bool, bool, bool) = lua
            .load(
                "return Vehicle.SetParts(v, 'LightFront', true), \
                 Vehicle.SetTurretYaw(v, 'main_turret', 1.5), Vehicle.SpinHeli(v)",
            )
            .eval()
            .unwrap();
        assert!(part && yaw && spin);
        assert!(
            !lua.load("return Vehicle.IsHijackBad(v)")
                .eval::<bool>()
                .unwrap()
        );
        lua.load("Vehicle.RestoreHealth(v) Vehicle.StartTankHijackMotion(v)")
            .exec()
            .unwrap();
        assert!(
            lua.load("Vehicle.SetParts(v, 'Siren', true)")
                .exec()
                .is_err()
        );
        let w = world.borrow();
        assert_eq!(w.objects[&7].health, 100.0);
        let asked: Vec<&VehicleCommand> = w
            .commands
            .iter()
            .filter_map(|c| match c {
                Command::Vehicle(v) => Some(v),
                _ => None,
            })
            .collect();
        assert_eq!(
            asked[..2],
            [
                &VehicleCommand::Part {
                    vehicle: 7,
                    part: 3,
                    on: true
                },
                &VehicleCommand::Turret {
                    vehicle: 7,
                    turret: m2_assets::name_hash("main_turret"),
                    angle: 1.5,
                    yaw: true
                }
            ]
        );
        assert!(asked.contains(&&VehicleCommand::TankHijackSteer {
            vehicle: 7,
            steer: -0.2
        }));
    }
}
