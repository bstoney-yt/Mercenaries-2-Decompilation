//! How the template database says a person fights and notices things:
//! the `Ai` component's character (how aggressive, how accurate, how fast
//! in a fight), `AiSkill`, `Perception` (how well they see and hear, and
//! how wide they look), `Health`, and the `AiBehavior` switches that turn
//! things off (no crouching, no grenades, no vehicles, ...). A layer's
//! entity may carry its own `AiBehavior`, which stands in for its
//! template's.
//!
//! The database keys fields by [`name_hash`] alone. The names here hash to
//! the fields' keys (found by trying words, as in [`tuning`](crate::tuning));
//! two-word names are reliable, the one three-word name (`NoExitVehicle`)
//! fits the others' pattern. What the engine does with each value is not
//! decoded: the alpha's soldiers read them as described on each field.

use std::collections::HashMap;

use m2_assets::{Layer, name_hash};

use crate::tuning::Table;

/// The `AiBehavior` switches. All off is a soldier who does everything.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AiFlags {
    pub no_crouch: bool,
    pub no_prone: bool,
    /// Doesn't run to an alarm.
    pub no_alarm: bool,
    /// Can't be captured (the game's non-lethal takedown).
    pub no_capture: bool,
    pub no_vehicle: bool,
    pub no_turret: bool,
    pub no_grenades: bool,
    pub no_follow: bool,
    pub no_exit_vehicle: bool,
    /// Never fights.
    pub pacifist: bool,
}

/// Picks one switch of [`AiFlags`].
type Switch = fn(&mut AiFlags) -> &mut bool;

/// The `AiBehavior` fields by name, with the switch each sets.
const FLAGS: [(&str, Switch); 10] = [
    ("NoCrouch", |f| &mut f.no_crouch),
    ("NoProne", |f| &mut f.no_prone),
    ("NoAlarm", |f| &mut f.no_alarm),
    ("NoCapture", |f| &mut f.no_capture),
    ("NoVehicle", |f| &mut f.no_vehicle),
    ("NoTurret", |f| &mut f.no_turret),
    ("NoGrenades", |f| &mut f.no_grenades),
    ("NoFollow", |f| &mut f.no_follow),
    ("NoExitVehicle", |f| &mut f.no_exit_vehicle),
    ("Pacifist", |f| &mut f.pacifist),
];

/// What a person template says of how they fight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AiProfile {
    /// `Health`: points to kill (VZ mooks 75, AT defenders 80).
    pub health: f32,
    /// `Ai`, all 0..1: VZ soldiers 0.3, 0.5, 0.4; Chinese 0.85, 0.75,
    /// 0.8; pirates 1, 1, 0.35.
    pub aggressive: f32,
    pub offensive: f32,
    pub accuracy: f32,
    /// `Ai` speeds, fractions of a full sprint: moving about in a fight,
    /// chasing, and sprinting (VZ 0.3, 0.6, 1).
    pub combat_speed: f32,
    pub pursue_speed: f32,
    pub sprint_speed: f32,
    /// `AiSkill`: VZ soldiers 60, guerrillas 65, OC elites 80, the
    /// heroes 95.
    pub skill: f32,
    /// `Perception`: how far they see and hear, as multiples (people 1,
    /// vehicles 1.25, helicopters 1.5), and how wide they look, degrees
    /// (people 180, vehicles and helicopters 360).
    pub vision: f32,
    pub hearing: f32,
    pub vision_fov: f32,
    pub flags: AiFlags,
    /// Carries grenades: an `EquipmentLink` names a grenade template
    /// (`Grenade (AI)`, as VZ soldiers have).
    pub grenades: bool,
}

impl Default for AiProfile {
    /// A plain VZ soldier's (`VZ Soldier`), for people no template
    /// speaks for.
    fn default() -> Self {
        Self {
            health: 100.0,
            aggressive: 0.3,
            offensive: 0.5,
            accuracy: 0.4,
            combat_speed: 0.3,
            pursue_speed: 0.6,
            sprint_speed: 1.0,
            skill: 60.0,
            vision: 1.0,
            hearing: 1.0,
            vision_fov: 180.0,
            flags: AiFlags::default(),
            grenades: true,
        }
    }
}

/// The `AiBehavior` switches of `record` in `table`.
pub(crate) fn flags(table: &Table<'_>, record: &[u8]) -> AiFlags {
    let mut flags = AiFlags::default();
    for (name, set) in FLAGS {
        if table.word(record, name_hash(name)).is_some_and(|w| w != 0) {
            *set(&mut flags) = true;
        }
    }
    flags
}

/// Every template with an `Ai` component's profile, by template id.
pub fn read(db: &Layer<'_>) -> HashMap<u32, AiProfile> {
    let ai = Table::new(db, "Ai");
    let behaviour = Table::new(db, "AiBehavior");
    let skill = Table::new(db, "AiSkill");
    let perception = Table::new(db, "Perception");
    let health = Table::new(db, "Health");
    let equipment = Table::new(db, "EquipmentLink");
    let names = db.names();
    // `EquipmentLink`'s template field is unnamed (54704554).
    let carries_grenades = |id: u32| {
        equipment.of_template(id).iter().any(|r| {
            equipment
                .word(r, 0x5470_4554)
                .and_then(|t| names.get(&t))
                .is_some_and(|n| n.to_ascii_lowercase().contains("grenade"))
        })
    };
    let first = |t: &Table<'_>, id: u32, name: &str| {
        t.of_template(id)
            .first()
            .and_then(|r| t.float(r, name_hash(name)))
    };
    ai.templates()
        .map(|id| {
            let d = AiProfile::default();
            let get = |t: &Table<'_>, name: &str, or: f32| first(t, id, name).unwrap_or(or);
            // `Health`'s points field is unnamed (d122919d).
            let points = health
                .of_template(id)
                .first()
                .and_then(|r| health.float(r, 0xd122_919d))
                .filter(|h| *h > 0.0);
            // `AiSkill`'s one field is unnamed (a5d8fea0).
            let skill_points = skill
                .of_template(id)
                .first()
                .and_then(|r| skill.float(r, 0xa5d8_fea0));
            let profile = AiProfile {
                health: points.unwrap_or(d.health),
                aggressive: get(&ai, "Aggressive", d.aggressive),
                offensive: get(&ai, "Offensive", d.offensive),
                accuracy: get(&ai, "Accuracy", d.accuracy),
                combat_speed: get(&ai, "CombatSpeed", d.combat_speed),
                pursue_speed: get(&ai, "PursueSpeed", d.pursue_speed),
                sprint_speed: get(&ai, "SprintSpeed", d.sprint_speed),
                skill: skill_points.unwrap_or(d.skill),
                vision: get(&perception, "Vision", d.vision),
                hearing: get(&perception, "Hearing", d.hearing),
                vision_fov: get(&perception, "VisionFOV", d.vision_fov),
                flags: behaviour
                    .of_template(id)
                    .first()
                    .map_or_else(AiFlags::default, |r| flags(&behaviour, r)),
                grenades: carries_grenades(id),
            };
            (id, profile)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guessed_names_hash_to_the_fields() {
        for (name, hash) in [
            ("Aggressive", 0xa854_ecd9),
            ("Offensive", 0xe83a_9c2a),
            ("Accuracy", 0xd182_5c8a),
            ("CombatSpeed", 0x733c_ca82),
            ("PursueSpeed", 0xdcef_65e0),
            ("SprintSpeed", 0xb13c_c96e),
            ("Hearing", 0x1efb_13c3),
            ("VisionFOV", 0x9c5d_90de),
            ("NoCrouch", 0x66fd_b15e),
            ("NoProne", 0x236d_1336),
            ("NoAlarm", 0x26eb_8797),
            ("NoVehicle", 0x742b_02b4),
            ("NoTurret", 0x6b6b_8daa),
            ("NoGrenades", 0xc51b_fdf9),
            ("NoFollow", 0x57f9_2df7),
            ("NoExitVehicle", 0x528a_268c),
        ] {
            assert_eq!(name_hash(name), hash, "{name}");
        }
    }
}
