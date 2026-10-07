//! The engine's `Airstrike` library: what the support scripts call to
//! bring a strike in. Each strike's script (`MrxClusterBomb`,
//! `MrxCarpetBomb`, `MrxSatelliteGuidedBomb`, ...) flies its aircraft in
//! with `Flyby`, and when it is over the target (the callback) lets its
//! ordnance go with `SpawnOrdnance`, `SpawnTargettedOrdnance`,
//! `ConeSpawn` or `SpawnCarpetBombLine`; the ordnance's callback hears it
//! go off. The designator scripts hand her the designator with
//! `EquipDesignator`, and the support manager (the script of what she
//! throws: `"Smoke Grenade Projectile"`'s `ObjectScript` is
//! `MrxSupportManager`) asks `FindDesignatorOwner` whose call it was.
//!
//! Argument orders, read off the scripts' calls (the engine's own code
//! is not read):
//!
//! ```text
//! Flyby(vehicle, x1, z1, x2, z2, altitude, speed, callback, data) -> jet
//! SpawnOrdnance(template, x, y, z, vx, vy, vz, fuse, fuse value, owner,
//!               callback, data) -> ordnance
//! SpawnTargettedOrdnance(template, x, y, z, tx, ty, tz, target, fuse,
//!               fuse value, owner, callback, data) -> ordnance
//! ConeSpawn(template, x, y, z, dx, dy, dz, cone degrees, count, speed)
//! SpawnCarpetBombLine(x, y, z, heading, owner, nil, count, spacing)
//!               -> next x, next z
//! SpawnDirectedObject(effect, x, y, z, dx, dy, dz) -> object
//! EquipDesignator(owner, designation type) -> weapon
//! ```
//!
//! What is inferred rather than read: a flyby starts at its altitude (the
//! scripts give the start only as x and z); the daisy cutter puts a number
//! before its fuse where the others put it after (the first string is
//! taken as the fuse, and a number after it as its value); `ConeSpawn`'s
//! count and speed are the two numbers after the cone (the cluster bomb
//! passes 15 or 30 degrees, then a count, then 10 or 20); a carpet bomb
//! line drops `count` "Carpet Bomb Projectile"s `spacing` metres apart
//! along the heading and answers the point after the last.

use std::collections::HashMap;

use mlua::{Function, Lua, MultiValue, Table, Value};

use crate::api::{Command, Object, ScriptWorld};

/// What carpet bomb lines drop: the template the database has for it.
pub const CARPET_BOMB: &str = "Carpet Bomb Projectile";

/// When ordnance goes off, as the strike scripts say.
#[derive(Clone, Debug, PartialEq)]
pub enum Fuse {
    /// On hitting anything (`"impact"`).
    Impact,
    /// On coming this near to what is in its way (`"obstructed"`, 30: the
    /// cluster bomb opens 30 m up).
    Obstructed(f32),
    /// After flying this far (`"distance"`: the fuel air bomb).
    Distance(f32),
    /// A fuse the game doesn't know yet, by its name.
    Other(String, Option<f32>),
}

impl Fuse {
    fn of(name: Option<&str>, value: Option<f32>) -> Self {
        match (name.map(str::to_ascii_lowercase).as_deref(), value) {
            (None | Some("impact"), _) => Fuse::Impact,
            (Some("obstructed"), v) => Fuse::Obstructed(v.unwrap_or(0.0)),
            (Some("distance"), Some(v)) => Fuse::Distance(v),
            (Some(other), v) => Fuse::Other(other.to_owned(), v),
        }
    }
}

/// How ordnance sets off.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Launch {
    /// At this velocity (m/s), as the script gave it.
    Velocity([f32; 3]),
    /// At its template's speed along this unit direction.
    Along([f32; 3]),
}

/// What the game owes the strike scripts, and the designators out.
#[derive(Debug, Default)]
pub struct Strikes {
    /// Callbacks for aircraft reaching the end of their flyby, by the
    /// aircraft's guid.
    arrivals: Vec<(u32, Function, Option<Table>)>,
    /// Callbacks for ordnance going off, by its guid.
    bursts: Vec<(u32, Function, Option<Table>)>,
    /// Designators thrown: who threw each.
    pub designators: HashMap<u32, u32>,
    /// The designator weapon each owner holds.
    pub equipped: HashMap<u32, u32>,
    /// For cones of bomblets.
    seed: u32,
}

impl Strikes {
    /// A number in [0, 1).
    fn random(&mut self) -> f32 {
        self.seed = self
            .seed
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Takes the callbacks waiting on `guid`: its flyby's end, or its
    /// going off.
    pub(crate) fn take(&mut self, guid: u32, arrival: bool) -> Vec<(Function, Option<Table>)> {
        let list = if arrival {
            &mut self.arrivals
        } else {
            &mut self.bursts
        };
        let (done, keep) = std::mem::take(list)
            .into_iter()
            .partition(|(g, _, _)| *g == guid);
        *list = keep;
        done.into_iter().map(|(_, f, d)| (f, d)).collect()
    }
}

type Handler = fn(&mut ScriptWorld, &Lua, &[Value]) -> mlua::Result<MultiValue>;

/// Puts the library's functions in `table` with `add`.
pub(crate) fn install(
    table: &Table,
    add: &dyn Fn(&Table, &str, Handler) -> mlua::Result<()>,
) -> mlua::Result<()> {
    add(table, "Flyby", flyby)?;
    add(table, "SpawnOrdnance", spawn_ordnance)?;
    add(table, "SpawnTargettedOrdnance", spawn_targetted)?;
    add(table, "ConeSpawn", cone_spawn)?;
    add(table, "SpawnCarpetBombLine", carpet_line)?;
    add(table, "SpawnDirectedObject", directed_object)?;
    add(table, "EquipDesignator", equip_designator)?;
    add(table, "FindDesignatorOwner", |w, _, a| {
        let owner = id(a.first()).and_then(|g| w.strikes.designators.get(&g).copied());
        values([guid_value(owner)])
    })?;
    add(table, "RemoveDesignator", |w, _, a| {
        if let Some(who) = id(a.first()) {
            w.strikes.equipped.remove(&who);
            w.commands.push(Command::RemoveDesignator { who });
        }
        values([])
    })?;
    // The designator weapon, given back its round when a strike is
    // refused (anti-air at the target).
    add(table, "RefillDesignator", |w, _, a| {
        if let Some(weapon) = id(a.first())
            && let Some(c) = w.inventory.carried.get_mut(&weapon)
        {
            c.reserve = c.reserve.max(1);
            let who = c.owner;
            if let Some(name) = w.objects.get(&c.template).map(|o| o.name.clone()) {
                w.commands.push(Command::Reserve {
                    who,
                    weapon: name,
                    rounds: 1,
                });
            }
        }
        values([])
    })?;
    Ok(())
}

fn flyby(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let Some(what) = template_name(w, a.first()) else {
        return values([Value::Nil]);
    };
    let n = |i: usize| num(a.get(i)).unwrap_or(0.0);
    let altitude = n(5);
    let from = [n(1), altitude, n(2)];
    let to = [n(3), altitude, n(4)];
    let guid = w.spawn_guid();
    w.insert(guid, Object::new(&what, from));
    w.commands.push(Command::Flyby {
        guid,
        what,
        from,
        to,
        speed: n(6),
    });
    if let Some(Value::Function(f)) = a.get(7) {
        let data = match a.get(8) {
            Some(Value::Table(t)) => Some(t.clone()),
            _ => None,
        };
        w.strikes.arrivals.push((guid, f.clone(), data));
    }
    values([guid_value(Some(guid))])
}

/// A strike call's callback and the data it is called with.
type Callback = (Function, Option<Table>);

/// Reads the arguments after where and how fast: the fuse (the first
/// string) and its value (a number after it), the owner (the first guid
/// after the fuse), the callback and its data.
fn after_launch(rest: &[Value]) -> (Fuse, Option<u32>, Option<Callback>) {
    let fuse_at = rest.iter().position(|v| matches!(v, Value::String(_)));
    let (name, value) = match fuse_at {
        Some(i) => (
            match &rest[i] {
                Value::String(s) => Some(s.to_string_lossy()),
                _ => None,
            },
            num(rest.get(i + 1)),
        ),
        None => (None, None),
    };
    let tail = &rest[fuse_at.map_or(0, |i| i + 1)..];
    let owner = tail
        .iter()
        .find(|v| matches!(v, Value::LightUserData(_)))
        .and_then(|v| id(Some(v)));
    let callback = tail.iter().position(|v| v.is_function()).map(|i| {
        let Value::Function(f) = &tail[i] else {
            unreachable!()
        };
        let data = match tail.get(i + 1) {
            Some(Value::Table(t)) => Some(t.clone()),
            _ => None,
        };
        (f.clone(), data)
    });
    (Fuse::of(name.as_deref(), value), owner, callback)
}

/// Makes a piece of ordnance and has the game fly it.
#[allow(clippy::too_many_arguments)]
fn launch(
    w: &mut ScriptWorld,
    what: String,
    at: [f32; 3],
    launch: Launch,
    fuse: Fuse,
    target: Option<(Option<u32>, [f32; 3])>,
    owner: Option<u32>,
    callback: Option<Callback>,
) -> u32 {
    let guid = w.spawn_guid();
    w.insert(guid, Object::new(&what, at));
    w.commands.push(Command::Ordnance {
        guid,
        what,
        at,
        launch,
        fuse,
        target,
        owner,
    });
    if let Some((f, data)) = callback {
        w.strikes.bursts.push((guid, f, data));
    }
    guid
}

fn spawn_ordnance(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let Some(what) = template_name(w, a.first()) else {
        return values([Value::Nil]);
    };
    let (Some(at), Some(v)) = (point(a, 1), point(a, 4)) else {
        return values([Value::Nil]);
    };
    let (fuse, owner, callback) = after_launch(a.get(7..).unwrap_or_default());
    let guid = launch(
        w,
        what,
        at,
        Launch::Velocity(v),
        fuse,
        None,
        owner,
        callback,
    );
    values([guid_value(Some(guid))])
}

fn spawn_targetted(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let Some(what) = template_name(w, a.first()) else {
        return values([Value::Nil]);
    };
    let (Some(at), Some(aim)) = (point(a, 1), point(a, 4)) else {
        return values([Value::Nil]);
    };
    let target = id(a.get(7));
    let (fuse, owner, callback) = after_launch(a.get(8..).unwrap_or_default());
    let guid = launch(
        w,
        what,
        at,
        Launch::Along(unit(sub(aim, at)).unwrap_or([0.0, -1.0, 0.0])),
        fuse,
        Some((target, aim)),
        owner,
        callback,
    );
    values([guid_value(Some(guid))])
}

fn cone_spawn(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let Some(what) = template_name(w, a.first()) else {
        return values([]);
    };
    let (Some(at), Some(d)) = (point(a, 1), point(a, 4)) else {
        return values([]);
    };
    let Some(d) = unit(d) else {
        return values([]);
    };
    let cone = num(a.get(7)).unwrap_or(0.0).to_radians();
    let count = num(a.get(8)).unwrap_or(1.0).max(0.0) as u32;
    let speed = num(a.get(9)).unwrap_or(0.0);
    // Two directions square to `d` and each other.
    let side = unit(cross(d, [0.0, 1.0, 0.0])).unwrap_or([1.0, 0.0, 0.0]);
    let up = cross(side, d);
    for _ in 0..count {
        // Spread evenly over the cone's cap.
        let cos = 1.0 - w.strikes.random() * (1.0 - cone.cos());
        let sin = (1.0 - cos * cos).max(0.0).sqrt();
        let turn = w.strikes.random() * std::f32::consts::TAU;
        let (s, c) = turn.sin_cos();
        let dir: [f32; 3] = std::array::from_fn(|i| d[i] * cos + (side[i] * c + up[i] * s) * sin);
        launch(
            w,
            what.clone(),
            at,
            Launch::Velocity(dir.map(|x| x * speed)),
            Fuse::Impact,
            None,
            None,
            None,
        );
    }
    values([])
}

fn carpet_line(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let Some(at) = point(a, 0) else {
        return values([]);
    };
    let heading = num(a.get(3)).unwrap_or(0.0);
    let owner = id(a.get(4));
    let count = num(a.get(6)).unwrap_or(1.0).max(0.0) as u32;
    let spacing = num(a.get(7)).unwrap_or(0.0);
    let (s, c) = heading.sin_cos();
    for k in 0..count {
        let k = k as f32 * spacing;
        launch(
            w,
            CARPET_BOMB.to_owned(),
            [at[0] + s * k, at[1], at[2] + c * k],
            Launch::Along([0.0, -1.0, 0.0]),
            Fuse::Impact,
            None,
            owner,
            None,
        );
    }
    let k = count as f32 * spacing;
    values([
        Value::Number(f64::from(at[0] + s * k)),
        Value::Number(f64::from(at[2] + c * k)),
        Value::Number(f64::from(at[1])),
    ])
}

fn directed_object(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let Some(what) = template_name(w, a.first()) else {
        return values([Value::Nil]);
    };
    let (Some(at), Some(d)) = (point(a, 1), point(a, 4)) else {
        return values([Value::Nil]);
    };
    let guid = w.spawn_guid();
    w.insert(guid, Object::new(&what, at));
    w.commands.push(Command::DirectedEffect {
        guid,
        what,
        at,
        direction: unit(d).unwrap_or([0.0, 1.0, 0.0]),
    });
    values([guid_value(Some(guid))])
}

fn equip_designator(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let (Some(who), Some(Value::String(kind))) = (id(a.first()), a.get(1)) else {
        return values([Value::Nil]);
    };
    let kind = kind.to_string_lossy();
    let template = w.template(&kind);
    let at = w.objects.get(&who).map_or([0.0; 3], |o| o.position);
    let weapon = w.spawn_guid();
    w.insert(weapon, Object::new(&kind, at));
    w.inventory.carried.insert(
        weapon,
        crate::inventory::Carried {
            owner: who,
            template,
            reserve: 1,
        },
    );
    w.inventory.weapons.entry(who).or_default().push(weapon);
    w.strikes.equipped.insert(who, weapon);
    w.commands
        .push(Command::EquipDesignator { who, weapon, kind });
    values([guid_value(Some(weapon))])
}

/// A template named by name or by its guid (`Pg.GetGuidByName`'s).
fn template_name(w: &ScriptWorld, v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.to_string_lossy()),
        v => w.objects.get(&id(Some(v))?).map(|o| o.name.clone()),
    }
}

fn point(a: &[Value], from: usize) -> Option<[f32; 3]> {
    Some([
        num(a.get(from))?,
        num(a.get(from + 1))?,
        num(a.get(from + 2))?,
    ])
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn unit(v: [f32; 3]) -> Option<[f32; 3]> {
    let l = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    (l > 1e-6).then(|| v.map(|x| x / l))
}

fn id(v: Option<&Value>) -> Option<u32> {
    crate::events::as_guid(v?)
}

fn num(v: Option<&Value>) -> Option<f32> {
    match v? {
        Value::Integer(i) => Some(*i as f32),
        Value::Number(n) => Some(*n as f32),
        _ => None,
    }
}

fn guid_value(g: Option<u32>) -> Value {
    g.map_or(Value::Nil, crate::events::guid_value)
}

fn values<I: IntoIterator<Item = Value>>(v: I) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_iter(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuses_read_as_the_scripts_write_them() {
        assert_eq!(Fuse::of(Some("impact"), Some(2.0)), Fuse::Impact);
        assert_eq!(
            Fuse::of(Some("obstructed"), Some(30.0)),
            Fuse::Obstructed(30.0)
        );
        assert_eq!(Fuse::of(Some("distance"), Some(12.0)), Fuse::Distance(12.0));
        assert_eq!(Fuse::of(None, None), Fuse::Impact);
    }
}
